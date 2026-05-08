//! Phase 3 PR-5 sweep test — `related-components.yaml` cache-path retrofit.
//!
//! Acceptance criteria (plan §4 PR-5, §5 row):
//!
//! (a) After `run_index` completes, `<root>/.atlas/cache/related-components.yaml`
//!     exists and is non-empty (parseable as `RelatedComponentsFile`).
//! (b) The pre-PR-5 top-level `related-components.yaml` (sibling of
//!     `components.yaml` at the editorial tier) does **not** exist.
//!     I.e. nothing writes to the old non-cache location.
//!
//! The test uses the `tiny` fixture (two-crate Rust workspace) and the
//! `LenientBackend` canned-response helper from the sibling
//! `pipeline_integration.rs` test (duplicated here to keep this file
//! self-contained).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use component_ontology::RelatedComponentsFile;
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Minimal canned-response backend (mirrors LenientBackend in
// pipeline_integration.rs — self-contained copy so this file stands alone).
// ---------------------------------------------------------------------------

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [7u8; 32],
        ontology_sha: [8u8; 32],
        model_id: "pr5-sweep-backend".into(),
        backend_version: "v-pr5-sweep".into(),
    }
}

struct SweepBackend {
    fingerprint: LlmFingerprint,
    call_count: Mutex<usize>,
}

impl SweepBackend {
    fn new() -> Arc<Self> {
        Arc::new(SweepBackend {
            fingerprint: fingerprint(),
            call_count: Mutex::new(0),
        })
    }
}

impl LlmBackend for SweepBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        *self.call_count.lock().unwrap() += 1;
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "sweep-backend default classification",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({
                "purpose": "sweep-backend default surface",
                "notes": "",
            }),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "sweep backend declined",
            }),
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixture helpers — tiny two-crate fixture (same as pipeline_integration.rs).
// ---------------------------------------------------------------------------

fn tiny_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny")
}

fn copy_fixture_to_tmp(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_fixture_to_tmp(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn materialise_tiny_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_fixture_to_tmp(&tiny_fixture_root(), tmp.path());
    tmp
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

// ---------------------------------------------------------------------------
// PR-5 sweep test
// ---------------------------------------------------------------------------

/// PR-5 acceptance: after `run_index`, the cache-subdir path is populated and
/// the old top-level path does not exist.
#[test]
fn related_components_written_to_cache_subdir_not_top_level() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let backend = SweepBackend::new();
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed for PR-5 sweep test");

    // (a) cache/related-components.yaml must exist and be parseable.
    let cache_path = config.output_dir.join("cache/related-components.yaml");
    assert!(
        cache_path.exists(),
        "PR-5: expected cache/related-components.yaml at {} but it does not exist",
        cache_path.display()
    );
    let bytes = std::fs::read(&cache_path).expect("failed to read cache/related-components.yaml");
    assert!(
        !bytes.is_empty(),
        "PR-5: cache/related-components.yaml must not be empty"
    );
    let _parsed: RelatedComponentsFile = serde_yaml::from_slice(&bytes)
        .expect("cache/related-components.yaml must parse as RelatedComponentsFile");

    // (b) The old top-level path must NOT exist.
    let old_path = config.output_dir.join("related-components.yaml");
    assert!(
        !old_path.exists(),
        "PR-5: old top-level related-components.yaml must not exist after PR-5 retrofit; \
         found at {}",
        old_path.display()
    );
}

/// PR-5 acceptance: a dry-run must not write either path.
#[test]
fn dry_run_writes_neither_old_nor_new_path() {
    let tmp = materialise_tiny_fixture();
    let mut config = base_config(tmp.path());
    config.dry_run = true;

    let backend = SweepBackend::new();
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("dry-run must not error");

    let cache_path = config.output_dir.join("cache/related-components.yaml");
    assert!(
        !cache_path.exists(),
        "PR-5: dry-run must not write cache/related-components.yaml; found at {}",
        cache_path.display()
    );

    let old_path = config.output_dir.join("related-components.yaml");
    assert!(
        !old_path.exists(),
        "PR-5: dry-run must not write old top-level related-components.yaml; found at {}",
        old_path.display()
    );
}

/// PR-5 acceptance: a second `run_index` call (re-run) still reads from the
/// cache path and produces byte-identical output.
#[test]
fn second_run_reads_from_cache_path_and_is_byte_identical() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    // First run.
    run_index(
        &config,
        SweepBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("first run must succeed");

    let cache_path = config.output_dir.join("cache/related-components.yaml");
    let first_bytes = std::fs::read(&cache_path)
        .expect("cache/related-components.yaml must exist after first run");

    // Second run.
    run_index(
        &config,
        SweepBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("second run must succeed");

    let second_bytes = std::fs::read(&cache_path)
        .expect("cache/related-components.yaml must exist after second run");

    assert_eq!(
        first_bytes, second_bytes,
        "PR-5: cache/related-components.yaml must be byte-identical on re-run"
    );
}
