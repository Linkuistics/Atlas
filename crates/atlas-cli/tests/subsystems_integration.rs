//! Dedicated end-to-end fixture tests for subsystem seeding:
//! rename-stability and cross-namespace collision halt.

use std::path::Path;
use std::sync::Arc;

use atlas_cli::pipeline::{run_index, IndexConfig};
use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_index::{
    load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemOverride,
    SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use component_ontology::EvidenceGrade;
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [7u8; 32],
        ontology_sha: [8u8; 32],
        model_id: "subsystems-test-backend".into(),
        backend_version: "v-test".into(),
    }
}

struct SimpleBackend {
    fingerprint: LlmFingerprint,
}

impl SimpleBackend {
    fn new() -> Arc<Self> {
        Arc::new(SimpleBackend {
            fingerprint: fingerprint(),
        })
    }
}

impl LlmBackend for SimpleBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "test",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({
                "purpose": "test surface",
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

fn write_cargo_lib(root: &Path, rel_path: &str) {
    let dir = root.join(rel_path);
    let name = dir.file_name().unwrap().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

// ---------------------------------------------------------------
// Rename-stability: glob re-matches after directory rename
// ---------------------------------------------------------------

#[test]
fn rename_stable_glob_re_matches_after_directory_rename() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Two Cargo libraries: one under services/auth/ (matched by glob),
    // one under libs/ (matched by id form).
    write_cargo_lib(root, "services/auth/handlers");
    write_cargo_lib(root, "libs/identity-core");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();

    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth-subsystem".into(),
                members: vec!["services/auth/*".into(), "identity-core".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    // First run: glob matches handlers, id form matches identity-core.
    run_index(
        &config,
        SimpleBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let out1 = load_or_default_subsystems(&config.output_dir.join("subsystems.yaml")).unwrap();
    assert_eq!(out1.subsystems.len(), 1);
    let s1 = &out1.subsystems[0];
    assert!(
        s1.members.contains(&"identity-core".to_string()),
        "first run: identity-core must be a member; members: {:?}",
        s1.members
    );
    assert!(
        s1.member_evidence
            .iter()
            .any(|e| e.matched_via == "services/auth/*"),
        "first run: glob services/auth/* must produce evidence; evidence: {:?}",
        s1.member_evidence
    );

    // Rename handlers → api (stays under services/auth/; glob still valid).
    std::fs::rename(
        root.join("services/auth/handlers"),
        root.join("services/auth/api"),
    )
    .unwrap();

    // Second run: glob services/auth/* re-matches the renamed directory;
    // id form identity-core resolves unchanged.
    run_index(
        &config,
        SimpleBackend::new(),
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .unwrap();

    let out2 = load_or_default_subsystems(&config.output_dir.join("subsystems.yaml")).unwrap();
    assert_eq!(out2.subsystems.len(), 1);
    let s2 = &out2.subsystems[0];
    assert!(
        s2.members.contains(&"identity-core".to_string()),
        "second run: identity-core must still be a member after rename; members: {:?}",
        s2.members
    );
    assert!(
        s2.member_evidence
            .iter()
            .any(|e| e.matched_via == "services/auth/*"),
        "second run: glob services/auth/* must still produce evidence after rename; evidence: {:?}",
        s2.member_evidence
    );
}

// ---------------------------------------------------------------
// Collision halt: subsystem id equals a component id
// ---------------------------------------------------------------

#[test]
fn pipeline_halts_on_subsystem_id_collision_with_component() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Single Cargo library; Atlas assigns it id "mylib" from the basename.
    write_cargo_lib(root, "mylib");

    let config = base_config(root);
    std::fs::create_dir_all(&config.output_dir).unwrap();

    // Subsystem id "mylib" collides with the component id "mylib".
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
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    let err = run_index(
        &config,
        SimpleBackend::new(),
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
