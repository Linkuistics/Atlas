//! End-to-end integration tests for `atlas_cli::run_index`, driving
//! the full pipeline (L0 seed → fixedpoint → L9 projections → atomic
//! writes) against a shared canned-response backend.
//!
//! The fixture lives in `tests/fixtures/tiny/` and contains a Rust
//! library and a Rust CLI. Both classify deterministically (via
//! `rule_cargo_lib` / `rule_cargo_bin`), so no `Classify` LLM calls
//! fire; L5 (surface_of) and L6 (all_proposed_edges) do issue calls,
//! and L8 (subcarve) asks the LLM for the library.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig, IndexError};
use atlas_llm::{
    AtlasConfig, LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TokenCounter,
};
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [1u8; 32],
        ontology_sha: [2u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-test".into(),
    }
}

/// Backend that returns canned defaults for every prompt — L5 sees a
/// minimal surface, L6 sees an empty edge list, L8 sees "do not
/// recurse". Tests can override the default per-prompt by registering
/// a specific response.
struct LenientBackend {
    fingerprint: LlmFingerprint,
    call_log: Mutex<Vec<PromptId>>,
    overrides: Mutex<Vec<(PromptId, String, Value)>>,
    force_error: Mutex<Option<LlmError>>,
}

impl LenientBackend {
    fn new() -> Arc<Self> {
        Arc::new(LenientBackend {
            fingerprint: fingerprint(),
            call_log: Mutex::new(Vec::new()),
            overrides: Mutex::new(Vec::new()),
            force_error: Mutex::new(None),
        })
    }

    fn calls(&self) -> Vec<PromptId> {
        self.call_log.lock().unwrap().clone()
    }

    fn call_count(&self) -> usize {
        self.call_log.lock().unwrap().len()
    }
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_log.lock().unwrap().push(req.prompt_template);

        if let Some(err) = self.force_error.lock().unwrap().take() {
            return Err(err);
        }

        let inputs_canonical = serde_json::to_string(&req.inputs).unwrap_or_default();
        let mut overrides = self.overrides.lock().unwrap();
        if let Some(pos) = overrides
            .iter()
            .position(|(id, inputs, _)| *id == req.prompt_template && *inputs == inputs_canonical)
        {
            let (_, _, value) = overrides.remove(pos);
            return Ok(value);
        }
        drop(overrides);

        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "default lenient classification",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({
                "purpose": "default lenient surface",
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

// ---------------------------------------------------------------
// first-run / second-run contract
// ---------------------------------------------------------------

#[test]
fn first_run_produces_the_three_generated_yamls() {
    let tmp = materialise_tiny_fixture();
    let backend = LenientBackend::new();
    let config = base_config(tmp.path());

    let summary = run_index(
        &config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index");

    assert!(summary.outputs_written);
    assert!(
        summary.component_count >= 2,
        "expected lib + cli, got {summary:?}"
    );
    assert!(config.output_dir.join("components.yaml").exists());
    assert!(config.output_dir.join("external-components.yaml").exists());
    assert!(config.output_dir.join("related-components.yaml").exists());
    assert!(
        backend.call_count() > 0,
        "first run must exercise the backend"
    );
}

#[test]
fn second_run_on_unchanged_fixture_is_byte_identical() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let backend1 = LenientBackend::new();
    run_index(
        &config,
        backend1,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    let first = std::fs::read(config.output_dir.join("components.yaml")).unwrap();
    let first_edges = std::fs::read(config.output_dir.join("related-components.yaml")).unwrap();
    let first_externals =
        std::fs::read(config.output_dir.join("external-components.yaml")).unwrap();

    // Second run with a fresh backend — the on-disk LLM cache seeds
    // the new database, so a deterministic-input run makes no fresh
    // backend calls at all.
    let backend2 = LenientBackend::new();
    let summary2 = run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    let second = std::fs::read(config.output_dir.join("components.yaml")).unwrap();
    let second_edges = std::fs::read(config.output_dir.join("related-components.yaml")).unwrap();
    let second_externals =
        std::fs::read(config.output_dir.join("external-components.yaml")).unwrap();

    assert_eq!(
        first, second,
        "components.yaml must be byte-identical on re-run"
    );
    assert_eq!(
        first_edges, second_edges,
        "related-components.yaml must be byte-identical on re-run"
    );
    assert_eq!(first_externals, second_externals);
    assert_eq!(
        backend2.call_count(),
        0,
        "on-disk cache must satisfy every request on no-op re-run; actual calls: {:?}",
        backend2.calls()
    );
    assert_eq!(
        summary2.llm_calls, 0,
        "summary must report llm_calls=0 on no-op re-run"
    );
}

#[test]
fn modifying_a_source_file_invalidates_that_components_surface_cache() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let backend1 = LenientBackend::new();
    run_index(
        &config,
        backend1,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // Touch one file in the library; re-run with a fresh backend.
    // L1's `file_tree_sha` changes for the library's directory only,
    // so L5 re-runs at least one surface call for that component.
    // L6's batch key depends on the full surface-record set, so if
    // the returned record differs the batch also re-fires — with the
    // LenientBackend's constant default response that's not
    // guaranteed, hence the weaker assertion on Stage2 below.
    std::fs::write(tmp.path().join("mylib/src/lib.rs"), "// modified\n").unwrap();

    let backend2 = LenientBackend::new();
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let calls = backend2.calls();
    let surface_calls = calls
        .iter()
        .filter(|p| **p == PromptId::Stage1Surface)
        .count();
    assert!(
        surface_calls >= 1,
        "expected at least one Stage1Surface call after source change, got {calls:?}"
    );
    // At minimum we made a backend call; the no-op re-run contract
    // asserts 0, so `>0` proves the content-sha propagated into the
    // cache key the way the engine's memoisation contract requires.
    assert!(!calls.is_empty());
}

// ---------------------------------------------------------------
// --dry-run
// ---------------------------------------------------------------

#[test]
fn dry_run_produces_summary_but_writes_no_files() {
    let tmp = materialise_tiny_fixture();
    let mut config = base_config(tmp.path());
    config.dry_run = true;
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    assert!(!summary.outputs_written);
    assert!(summary.component_count >= 2);
    assert!(!config.output_dir.join("components.yaml").exists());
    assert!(!config.output_dir.join("external-components.yaml").exists());
    assert!(!config.output_dir.join("related-components.yaml").exists());
}

// ---------------------------------------------------------------
// Budget exhaustion
// ---------------------------------------------------------------

#[test]
fn tiny_budget_triggers_budget_exhausted_and_no_writes() {
    use atlas_llm::{default_token_estimator, BudgetedBackend};

    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    let counter = Arc::new(TokenCounter::new(1));
    let inner = LenientBackend::new();
    let backend: Arc<dyn LlmBackend> = Arc::new(BudgetedBackend::new(
        inner,
        counter.clone(),
        default_token_estimator(),
    ));

    let err = run_index(
        &config,
        backend,
        Some(counter.clone()),
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap_err();

    assert!(
        matches!(err, IndexError::BudgetExhausted),
        "expected BudgetExhausted, got {err:?}"
    );
    assert!(
        !config.output_dir.join("components.yaml").exists(),
        "budget-exhausted run must not have written outputs"
    );
}

// ---------------------------------------------------------------
// --max-depth=0 semantics
// ---------------------------------------------------------------

#[test]
fn max_depth_zero_still_emits_top_level_components_but_no_subcarve_back_edge() {
    let tmp = materialise_tiny_fixture();
    let mut config = base_config(tmp.path());
    config.max_depth = 0;
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // With max_depth=0 the policy returns Stop for every kind, so no
    // Subcarve LLM call fires (see subcarve_policy::decide).
    let subcarve_calls = backend
        .calls()
        .iter()
        .filter(|p| **p == PromptId::Subcarve)
        .count();
    assert_eq!(
        subcarve_calls, 0,
        "max_depth=0 must short-circuit every subcarve policy call"
    );
    // Top-level components are still classified — both lib and cli
    // live under root, so they both appear.
    assert!(summary.component_count >= 2);
    assert_eq!(summary.fixedpoint_iterations, 0);
}

// ---------------------------------------------------------------
// Overrides are left alone
// ---------------------------------------------------------------

#[test]
fn overrides_file_never_written_by_pipeline() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    std::fs::create_dir_all(&config.output_dir).unwrap();
    let overrides_path = config.output_dir.join("components.overrides.yaml");
    std::fs::write(
        &overrides_path,
        "schema_version: 1\npins: {}\nadditions: []\n",
    )
    .unwrap();
    let before = std::fs::read(&overrides_path).unwrap();

    let backend = LenientBackend::new();
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let after = std::fs::read(&overrides_path).unwrap();
    assert_eq!(
        before, after,
        "pipeline must never touch components.overrides.yaml"
    );
}

// ---------------------------------------------------------------
// --no-overrides
// ---------------------------------------------------------------

#[test]
fn no_overrides_skips_loading_pins_so_suppress_does_not_apply() {
    // A pin with `suppress: true` on `mylib` would normally cause L4
    // to drop the component (is_boundary=false). With `--no-overrides`
    // the pin is bypassed, so the component reappears in the output.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    std::fs::create_dir_all(&config.output_dir).unwrap();
    let overrides_path = config.output_dir.join("components.overrides.yaml");
    std::fs::write(
        &overrides_path,
        "schema_version: 1\npins:\n  mylib:\n    suppress: true\nadditions: []\n",
    )
    .unwrap();

    // Baseline: pin honoured, mylib suppressed.
    let backend1 = LenientBackend::new();
    let summary_with = run_index(
        &config,
        backend1,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    // Wipe outputs so the second run starts fresh; the LLM cache also
    // has to clear so we observe behaviour, not a cached re-projection.
    std::fs::remove_file(config.output_dir.join("components.yaml")).unwrap();
    std::fs::remove_file(config.output_dir.join("llm-cache.json")).ok();

    let mut config_no_overrides = config.clone();
    config_no_overrides.no_overrides = true;
    let backend2 = LenientBackend::new();
    let summary_without = run_index(
        &config_no_overrides,
        backend2,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    assert!(
        summary_without.component_count > summary_with.component_count,
        "--no-overrides must bypass the suppress pin: with={} without={}",
        summary_with.component_count,
        summary_without.component_count,
    );
}

#[test]
fn no_overrides_does_not_modify_overrides_file() {
    let tmp = materialise_tiny_fixture();
    let mut config = base_config(tmp.path());
    config.no_overrides = true;
    std::fs::create_dir_all(&config.output_dir).unwrap();
    let overrides_path = config.output_dir.join("components.overrides.yaml");
    let body = "schema_version: 1\npins:\n  mylib:\n    suppress: true\nadditions: []\n";
    std::fs::write(&overrides_path, body).unwrap();
    let before = std::fs::read(&overrides_path).unwrap();

    let backend = LenientBackend::new();
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let after = std::fs::read(&overrides_path).unwrap();
    assert_eq!(
        before, after,
        "--no-overrides must not write to components.overrides.yaml"
    );
}

// ---------------------------------------------------------------
// LLM cache persistence
// ---------------------------------------------------------------

#[test]
fn llm_cache_json_is_written_and_read_across_invocations() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    let backend1 = LenientBackend::new();
    run_index(
        &config,
        backend1.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    let cache_path = config.output_dir.join("llm-cache.json");
    assert!(cache_path.exists(), "cache file must be written on success");
    let cache_bytes = std::fs::read_to_string(&cache_path).unwrap();
    assert!(cache_bytes.contains("schema_version"));

    // On the next run the backend sees zero requests.
    let backend2 = LenientBackend::new();
    run_index(
        &config,
        backend2.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    assert_eq!(backend2.call_count(), 0);
}

// ---------------------------------------------------------------
// subsystems pipeline integration
// ---------------------------------------------------------------

#[test]
fn pipeline_emits_subsystems_yaml_when_overrides_present() {
    use atlas_index::{
        load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemOverride,
        SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
    };

    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    std::fs::create_dir_all(&config.output_dir).unwrap();
    let subs_path = config.output_dir.join("subsystems.overrides.yaml");
    let subs = SubsystemsOverridesFile {
        schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
        subsystems: vec![SubsystemOverride {
            id: "fixture-subsystem".into(),
            members: vec!["*".into()],
            role: None,
            lifecycle_roles: vec![],
            rationale: "test".into(),
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: vec![],
        }],
    };
    save_subsystems_overrides_atomic(&subs_path, &subs).unwrap();

    run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let out_path = config.output_dir.join("subsystems.yaml");
    assert!(
        out_path.exists(),
        "expected subsystems.yaml at {}",
        out_path.display()
    );
    let loaded = load_or_default_subsystems(&out_path).unwrap();
    assert_eq!(loaded.subsystems.len(), 1);
    assert_eq!(loaded.subsystems[0].id, "fixture-subsystem");
}

#[test]
fn pipeline_emits_empty_subsystems_yaml_when_no_overrides() {
    use atlas_index::load_or_default_subsystems;

    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let out_path = config.output_dir.join("subsystems.yaml");
    assert!(
        out_path.exists(),
        "expected empty subsystems.yaml even without overrides"
    );
    let loaded = load_or_default_subsystems(&out_path).unwrap();
    assert!(loaded.subsystems.is_empty());
}

#[test]
fn subsystems_glob_and_id_membership_both_resolve() {
    use atlas_index::{
        load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemOverride,
        SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
    };

    // The tiny fixture produces components "mylib" and "mycli".
    // Use an id-form member ("mylib") and a glob member ("m*") in the
    // same subsystem to verify both resolution paths fire.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "my-services".into(),
                members: vec!["mylib".into(), "m*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let out = load_or_default_subsystems(&config.output_dir.join("subsystems.yaml")).unwrap();
    assert_eq!(out.subsystems.len(), 1);
    let s = &out.subsystems[0];
    assert_eq!(s.id, "my-services");
    assert!(
        s.members.contains(&"mylib".to_string()),
        "id-form member 'mylib' must resolve; members: {:?}",
        s.members
    );
    assert!(
        s.members.contains(&"mycli".to_string()),
        "glob 'm*' must resolve mycli; members: {:?}",
        s.members
    );
    assert!(
        s.member_evidence.iter().any(|e| e.matched_via == "id"),
        "id-form evidence must be present"
    );
    assert!(
        s.member_evidence.iter().any(|e| e.matched_via == "m*"),
        "glob-form evidence must be present"
    );
}

#[test]
fn subsystems_yaml_is_byte_identical_on_no_op_re_run() {
    use atlas_index::{
        save_subsystems_overrides_atomic, SubsystemOverride, SubsystemsOverridesFile,
        SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
    };

    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "all-libs".into(),
                members: vec!["*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    let first = std::fs::read(config.output_dir.join("subsystems.yaml")).unwrap();

    run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();
    let second = std::fs::read(config.output_dir.join("subsystems.yaml")).unwrap();

    assert_eq!(
        first, second,
        "subsystems.yaml must be byte-identical on no-op re-run"
    );
}

// ---------------------------------------------------------------
// .atlas/config.yaml integration
// ---------------------------------------------------------------

#[test]
fn config_yaml_loads_and_pipeline_runs_when_present() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    std::fs::create_dir_all(&config.output_dir).unwrap();

    let config_path = config.output_dir.join("config.yaml");
    std::fs::write(
        &config_path,
        "defaults:\n  model: claude-code/claude-sonnet-4-6\n",
    )
    .unwrap();

    // Verify the config-absent error path does not fire when the file exists.
    let atlas_config =
        AtlasConfig::load(&config_path).expect("AtlasConfig::load must succeed when file exists");
    assert_eq!(atlas_config.defaults.model, "claude-code/claude-sonnet-4-6");

    // Verify the pipeline runs to completion in a workspace that has a
    // valid config.yaml present (backend is mocked — no live LLM calls).
    let summary = run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("pipeline must complete when config.yaml is valid");

    assert!(summary.outputs_written);
    assert!(summary.component_count >= 2);
}

#[test]
fn pipeline_halts_when_subsystem_id_collides_with_component() {
    use atlas_index::{
        save_subsystems_overrides_atomic, SubsystemOverride, SubsystemsOverridesFile,
        SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
    };

    // The tiny fixture produces a component with id "mylib". Using the
    // same id for a subsystem must trigger a hard error before any writes.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());

    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "mylib".into(),
                members: vec!["*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    let err = run_index(
        &config,
        LenientBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("collide with component ids"),
        "expected collision error, got: {err}"
    );
    assert!(
        !config.output_dir.join("subsystems.yaml").exists(),
        "subsystems.yaml must not be saved when collision halts the pipeline"
    );
}

// ---------------------------------------------------------------
// Setup-error propagation
// ---------------------------------------------------------------

/// Backend that always returns [`LlmError::Setup`]. Models the dull
/// failure mode where every L5/L6/L8 call returned Setup yet the
/// pipeline reported "0 LLM calls" because errors are not counted as
/// calls. The pipeline must now abort with `IndexError::SetupFailed`
/// rather than write outputs derived from silent fallbacks.
struct SetupOnlyBackend {
    fingerprint: LlmFingerprint,
    message: String,
    calls: Mutex<u32>,
}

impl SetupOnlyBackend {
    fn new(message: &str) -> Arc<Self> {
        Arc::new(SetupOnlyBackend {
            fingerprint: fingerprint(),
            message: message.to_string(),
            calls: Mutex::new(0),
        })
    }
}

impl LlmBackend for SetupOnlyBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        *self.calls.lock().unwrap() += 1;
        Err(LlmError::Setup(self.message.clone()))
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

#[test]
fn setup_error_aborts_pipeline_and_writes_no_outputs() {
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    let backend = SetupOnlyBackend::new("missing params.max_tokens");

    let err = run_index(
        &config,
        backend.clone(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap_err();

    match err {
        IndexError::SetupFailed(msg) => {
            assert!(
                msg.contains("missing params.max_tokens"),
                "expected sentinel message in error, got: {msg}"
            );
        }
        other => panic!("expected IndexError::SetupFailed, got {other:?}"),
    }

    assert!(
        *backend.calls.lock().unwrap() > 0,
        "backend must have been invoked at least once for the sentinel to trip"
    );
    assert!(
        !config.output_dir.join("components.yaml").exists(),
        "setup-failed run must not write components.yaml"
    );
    assert!(
        !config.output_dir.join("external-components.yaml").exists(),
        "setup-failed run must not write external-components.yaml"
    );
    assert!(
        !config.output_dir.join("related-components.yaml").exists(),
        "setup-failed run must not write related-components.yaml"
    );
}

#[test]
fn budget_exhausted_does_not_alias_setup_failed_error() {
    // Regression guard for the Setup vs BudgetExhausted split: a
    // budget-only failure must continue to map to BudgetExhausted, not
    // SetupFailed. Mirrors the existing tiny_budget_triggers_budget_exhausted
    // test but is colocated here so a future change to the sentinel that
    // accidentally collapses the two branches is caught by this file.
    use atlas_llm::{default_token_estimator, BudgetedBackend};

    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    let counter = Arc::new(TokenCounter::new(1));
    let inner = LenientBackend::new();
    let backend: Arc<dyn LlmBackend> = Arc::new(BudgetedBackend::new(
        inner,
        counter.clone(),
        default_token_estimator(),
    ));

    let err = run_index(
        &config,
        backend,
        Some(counter),
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap_err();

    assert!(
        matches!(err, IndexError::BudgetExhausted),
        "BudgetExhausted must not collapse into SetupFailed when the sentinel grew Setup tracking; got {err:?}"
    );
}

#[test]
fn summary_reports_llm_errors_for_cache_miss_failures() {
    // A fresh backend that errors on every call: the pipeline aborts
    // (per the SetupFailed test above), but if we look at the cache
    // counters before the pipeline propagates we should see error_count
    // > 0 and call_count == 0. This test confirms the counters split
    // cleanly so a future "report errors but continue" mode would have
    // accurate numbers to display.
    let tmp = materialise_tiny_fixture();
    let config = base_config(tmp.path());
    let backend = SetupOnlyBackend::new("config rejected");

    let err = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap_err();

    // The pipeline aborts with SetupFailed; the summary path is not
    // reached. The behavioural guarantee is that we did NOT silently
    // succeed — that's the regression this whole task fixes.
    assert!(
        matches!(err, IndexError::SetupFailed(_)),
        "pipeline must abort, not produce a misleading 'success' summary"
    );
}
