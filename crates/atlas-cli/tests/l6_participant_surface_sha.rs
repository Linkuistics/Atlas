//! PR-11 acceptance tests — L6 cache key includes participant surface shas.
//!
//! ## Acceptance criteria
//!
//! 1. **Same-root cache invalidation** — A two-crate workspace where
//!    crate-B depends on a contract from crate-A. After the first (cold)
//!    run, editing crate-A's defining binding causes the L6 batch to miss
//!    the persistent cache on the next run. A subsequent no-edit run hits
//!    the cache again.
//!
//! 2. **No-contract stability** — A workspace whose components carry no
//!    contract content (no serde-derived structs, no library APIs) produces
//!    the same L6 batch fingerprint regardless of whether PR-11's
//!    `add_participant_surface_sha` loop fires.  The loop is empty when no
//!    component has contract content, so the fingerprint is byte-identical
//!    to the PR-10 baseline (no participant-sha contribution).

use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::{
    all_components, compute_l6_batch_fingerprint, seed_filesystem, AtlasDatabase,
    FingerprintBuilder,
};
use atlas_index::Stage;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared test backend
// ---------------------------------------------------------------------------

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [11u8; 32],
        ontology_sha: [12u8; 32],
        model_id: "pr11-test-backend".into(),
        backend_version: "v-pr11".into(),
    }
}

/// Lenient backend that returns valid canned responses for every prompt
/// and logs every LLM call so tests can assert on call counts.
struct TrackingBackend {
    fingerprint: LlmFingerprint,
    call_log: Mutex<Vec<(PromptId, String)>>,
}

impl TrackingBackend {
    fn new() -> Arc<Self> {
        Arc::new(TrackingBackend {
            fingerprint: fingerprint(),
            call_log: Mutex::new(Vec::new()),
        })
    }

    fn stage2_calls(&self) -> usize {
        self.call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| *p == PromptId::Stage2Edges)
            .count()
    }
}

impl LlmBackend for TrackingBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let inputs_canonical = serde_json::to_string(&req.inputs).unwrap_or_default();
        self.call_log
            .lock()
            .unwrap()
            .push((req.prompt_template, inputs_canonical));
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "pr11 test",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({
                "purpose": "pr11 test component",
                "notes": "",
            }),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            }),
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Write a minimal Rust library crate with no public items (all private).
/// This produces a `SurfacesFile` where all four contract / library-api
/// fields are empty, satisfying `!surface_has_contract_content(sf)`.
fn write_private_crate(dir: &std::path::Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    // Only private items → no pub_items → no library_apis → empty surface.
    std::fs::write(dir.join("src/lib.rs"), "fn internal() -> u32 { 42 }\n").unwrap();
}

/// Write a minimal Rust library crate with a `pub fn` (no serde structs).
/// This produces a `library_apis` entry (non-empty pub_items) but no contracts.
fn write_plain_crate(dir: &std::path::Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
}

/// Write a Rust library crate that defines a serde-derived pub struct
/// `Foo`. The Rust-surface analyser detects this and emits a
/// `data-format` contract, which makes the surface fingerprint
/// load-bearing in the L6 key.
fn write_serde_crate(dir: &std::path::Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n\
             \n[dependencies]\nserde = \"1\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "#[derive(serde::Serialize, serde::Deserialize)]\npub struct Foo { pub x: u32 }\n",
    )
    .unwrap();
}

fn base_config(root: &std::path::Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

fn run_cold(config: &IndexConfig) -> usize {
    let backend = TrackingBackend::new();
    run_index(
        config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("cold run must succeed");
    backend.stage2_calls()
}

// ---------------------------------------------------------------------------
// Acceptance criterion #1: same-root cache invalidation
// ---------------------------------------------------------------------------

#[test]
fn same_root_cache_invalidates_on_serde_struct_edit() {
    // Two crates in one workspace root.  crate-a has a serde-derived
    // struct (contributing a participant_surface_sha).  crate-b is plain.
    //
    // Run order:
    //  1. cold  — backend invoked for Stage 2 (cache cold).
    //  2. edit crate-a's struct (add field `y: u32`).
    //  3. warm-miss — Stage 2 backend must be invoked again (cache miss
    //     because crate-a's surface fingerprint changed and it was
    //     contributed as a participant sha to the L6 key).
    //  4. no-op warm — Stage 2 backend must NOT be invoked (all-hit run).
    let tmp = TempDir::new().unwrap();
    write_serde_crate(&tmp.path().join("crate-a"), "crate-a");
    write_plain_crate(&tmp.path().join("crate-b"), "crate-b");

    let config = base_config(tmp.path());

    // Run 1: cold.
    let cold_stage2 = run_cold(&config);
    assert!(
        cold_stage2 >= 1,
        "cold run must invoke Stage 2 at least once for the test to be meaningful; \
         got {cold_stage2}"
    );

    // Edit crate-a's serde struct: add field `y`.
    std::fs::write(
        tmp.path().join("crate-a/src/lib.rs"),
        "#[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct Foo { pub x: u32, pub y: u64 }\n",
    )
    .unwrap();

    // Run 2: warm-miss — the L6 fingerprint includes crate-a's surface
    // sha, which changed, so the batch must recompute.
    let backend2 = TrackingBackend::new();
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("warm-miss run must succeed");
    let miss_stage2 = backend2.stage2_calls();
    assert!(
        miss_stage2 >= 1,
        "after editing crate-a's serde struct, L6 batch must miss the persistent cache \
         and invoke Stage 2 again; got {miss_stage2} Stage2Edges calls on warm-miss run"
    );

    // Run 3: no-op warm — no edits → all-hit.
    let backend3 = TrackingBackend::new();
    run_index(
        &config,
        backend3.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("no-op warm run must succeed");
    let noop_stage2 = backend3.stage2_calls();
    assert_eq!(
        noop_stage2, 0,
        "no-op re-run after warm-miss must hit the persistent cache for Stage 2; \
         got {noop_stage2} Stage2Edges calls"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criterion #3: no-contract stability
// ---------------------------------------------------------------------------

#[test]
fn no_contract_workspace_has_stable_l6_fingerprint() {
    // A workspace where no component has contract content (no serde-derived
    // structs, no library APIs) must produce the same L6 batch fingerprint
    // as if PR-11's `add_participant_surface_sha` loop were disabled.
    //
    // Implementation of option (b) from the brief:
    // - Build a two-crate database with plain (non-serde) crates.
    // - Call `compute_l6_batch_fingerprint` — the helper fires the
    //   participant loop but all components have empty surfaces, so no
    //   `add_participant_surface_sha` calls execute.
    // - Build a reference fingerprint using a bare `FingerprintBuilder`
    //   WITHOUT the participant loop, contributing only the same preamble
    //   and file-content shas.
    // - Assert the two are equal.  Any non-empty surface contribution would
    //   change the sha, proving the contribution was truly skipped.
    // Use all-private crates: no public items → no library_apis → empty
    // surfaces → `surface_has_contract_content` returns false for both
    // components → no `add_participant_surface_sha` calls fire.
    let tmp = TempDir::new().unwrap();
    write_private_crate(&tmp.path().join("alpha"), "alpha");
    write_private_crate(&tmp.path().join("beta"), "beta");

    let backend: Arc<dyn LlmBackend> = TrackingBackend::new();
    let mut db = AtlasDatabase::new(backend, tmp.path().to_path_buf(), fingerprint());
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

    let components = all_components(&db);
    let live: Vec<&atlas_index::ComponentEntry> =
        components.iter().filter(|c| !c.deleted).collect();

    // Need at least two components for the L6 batch to be meaningful.
    assert!(
        live.len() >= 2,
        "fixture must produce at least two components, got {}: {:?}",
        live.len(),
        live.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
    );

    // Minimal sentinel inputs — identical for both fingerprints so only
    // the participant-sha contribution would differ if it fires.
    let prompt_sha = "aaaa0000";
    let registry_sha = "bbbb1111";
    let fp = fingerprint();

    // PR-11 fingerprint: fires the participant loop (but all surfaces are
    // empty, so no participant calls execute).
    let pr11_fp = compute_l6_batch_fingerprint(&db, &live, prompt_sha, registry_sha, &fp);

    // Baseline fingerprint: same contributions WITHOUT the participant
    // loop.  This simulates the PR-10 fingerprint shape for a no-contract
    // workspace.
    let baseline_fp = {
        let mut fb = FingerprintBuilder::new(Stage::L6, "l6-driver", "1.0.0");
        fb.add_analyzer_registry_sha(&registry_sha.to_string());
        fb.add_llm_fingerprint(&fp);
        fb.add_prompt_sha(&prompt_sha.to_string());
        for c in &live {
            for seg in &c.path_segments {
                fb.add_file_content_sha(&seg.content_sha);
            }
        }
        // No `add_participant_surface_sha` calls — this is the baseline.
        fb.finalise()
    };

    assert_eq!(
        pr11_fp, baseline_fp,
        "a no-contract workspace must produce the same L6 fingerprint with and without \
         PR-11's participant_surface_sha loop — the loop contributes nothing when all \
         component surfaces are empty"
    );
}
