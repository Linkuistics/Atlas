//! Regression test: decomposing L5 demand into a per-component loop
//! must produce byte-identical `components.yaml` output. Spec §7.3.
//!
//! ## What this test asserts
//!
//! 1. `components.yaml` is byte-identical across two consecutive
//!    `run_index` invocations on an unchanged fixture (regression for
//!    spec §7.3 stability).
//! 2. The per-component demand loop introduced in commit `af02e35`
//!    actually issues `Stage1Surface` backend calls — we filter the
//!    counter to `PromptId::Stage1Surface` so subcarve / Stage 2 traffic
//!    does not mask a regression where the loop has been removed.
//!
//! ## What this test does NOT assert
//!
//! It does not prove the "zero LLM calls on a clean re-run" caching
//! contract. `TestBackend::respond` keys off `canonical_key(&inputs)` —
//! the wildcard `json!({})` we register here never matches the real
//! `build_inputs(...)` output (which embeds component path, content
//! shas, peer ids), so every backend call returns
//! `LlmError::TestBackendMiss`. `LlmResponseCache` increments
//! `call_count` only on success (see `crates/atlas-engine/src/llm_cache.rs`),
//! so `summary.llm_calls == 0` on both runs is technically correct but
//! masks the per-component traffic. The cache-hit contract is covered
//! by `pipeline_integration::byte_identical_yaml*` via `LenientBackend`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use atlas_cli::pipeline::{run_index, IndexConfig};
use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_index::ComponentsFile;
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TestBackend};
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [1u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn write_lib(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
}

fn make_inner_backend() -> Arc<TestBackend> {
    let backend = TestBackend::with_fingerprint(fingerprint());
    backend.respond(PromptId::Stage1Surface, json!({}), json!({"purpose": "x"}));
    backend.respond(PromptId::Stage2Edges, json!({}), json!([]));
    Arc::new(backend)
}

/// Backend wrapper that counts every invocation regardless of outcome,
/// bucketed by `PromptId`. The plain `LlmResponseCache::call_count`
/// increments only on successful backend calls (see
/// `llm_cache.rs:111-114` — the `?` returns errors before the
/// increment), which is wrong for this test: every call into
/// `TestBackend` here returns `TestBackendMiss` because the canned
/// response is keyed by the wildcard `json!({})` and the real inputs
/// embed real fields. We need a counter that fires on misses too.
struct CountingBackend {
    inner: Arc<dyn LlmBackend>,
    stage1_calls: AtomicUsize,
    total_calls: AtomicUsize,
}

impl CountingBackend {
    fn new(inner: Arc<dyn LlmBackend>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            stage1_calls: AtomicUsize::new(0),
            total_calls: AtomicUsize::new(0),
        })
    }

    fn stage1_calls(&self) -> usize {
        self.stage1_calls.load(Ordering::Relaxed)
    }

    fn total_calls(&self) -> usize {
        self.total_calls.load(Ordering::Relaxed)
    }
}

impl LlmBackend for CountingBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.total_calls.fetch_add(1, Ordering::Relaxed);
        if matches!(req.prompt_template, PromptId::Stage1Surface) {
            self.stage1_calls.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.call(req)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }
}

#[test]
fn pipeline_run_twice_produces_identical_components_yaml() {
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "lib");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());

    let reporter = make_stderr_reporter(ProgressMode::Never, None);
    let inner1: Arc<dyn LlmBackend> = make_inner_backend();
    let backend1 = CountingBackend::new(inner1);
    let backend1_dyn: Arc<dyn LlmBackend> = backend1.clone();
    let _ = run_index(&config, backend1_dyn, None, Arc::clone(&reporter)).unwrap();
    let first = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    // The per-component demand loop (commit af02e35) must have issued
    // at least one Stage1Surface call. Filtering by `PromptId` is what
    // gives this assertion teeth: subcarve traffic during the
    // fixedpoint also goes through `LlmBackend::call`, so an unfiltered
    // `total_calls() >= 1` would still be true even if the demand loop
    // were removed. With this fixture (single library = one component)
    // the L6 batch returns early without demanding surfaces, so a
    // Stage1Surface call here can only have come from the per-component
    // loop in `pipeline.rs`.
    assert!(
        backend1.stage1_calls() >= 1,
        "demand loop must issue at least one Stage1Surface call \
         (stage1={}, total={})",
        backend1.stage1_calls(),
        backend1.total_calls(),
    );

    let reporter2 = make_stderr_reporter(ProgressMode::Never, None);
    let inner2: Arc<dyn LlmBackend> = make_inner_backend();
    let backend2 = CountingBackend::new(inner2);
    let backend2_dyn: Arc<dyn LlmBackend> = backend2.clone();
    let _ = run_index(&config, backend2_dyn, None, Arc::clone(&reporter2)).unwrap();
    let second = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    assert_eq!(
        first, second,
        "components.yaml must be byte-identical on no-op re-run"
    );

    let parsed_first: ComponentsFile = serde_yaml::from_slice(&first).unwrap();
    let parsed_second: ComponentsFile = serde_yaml::from_slice(&second).unwrap();
    assert_eq!(parsed_first, parsed_second);
}
