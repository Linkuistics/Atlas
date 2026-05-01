//! Integration tests for L2 candidate generation and L3 classification.
//!
//! Each test builds a self-contained fixture under a `tempfile::TempDir`
//! so the signals the engine sees are fully under the test's control.
//! The deterministic-rule tests install a `TestBackend` with no canned
//! responses — any accidental LLM dispatch errors loudly, which is
//! exactly what §4.1's "deterministic short-circuit" invariant asks
//! for.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_engine::{
    candidate_components_at, is_component, seed_filesystem, AtlasDatabase, ComponentKind,
};
use atlas_index::{
    AlwaysTrue, ComponentEntry, OverridesFile, PathSegment, PinValue, OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::{LlmFingerprint, PromptId, TestBackend};
use component_ontology::EvidenceGrade;
use serde_json::json;
use tempfile::TempDir;

fn default_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn build_db(backend: Arc<TestBackend>, root: &Path) -> AtlasDatabase {
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), default_fingerprint());
    seed_filesystem(&mut db, root, false).expect("seed_filesystem must succeed");
    db
}

/// Builds a DB seeded from `root` with a fresh TestBackend that has
/// no canned responses — any accidental LLM dispatch fails.
fn db_without_llm(root: &Path) -> AtlasDatabase {
    build_db(Arc::new(TestBackend::new()), root)
}

// ---------------------------------------------------------------------
// L2 — candidate enumeration
// ---------------------------------------------------------------------

#[test]
fn l2_emits_one_candidate_per_manifest_dir() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"a\"\n[lib]\n").unwrap();
    std::fs::create_dir_all(root.join("crates/inner")).unwrap();
    std::fs::write(
        root.join("crates/inner/Cargo.toml"),
        "[package]\nname=\"b\"\n[lib]\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();

    let candidates = candidate_components_at(&db, ws, root.clone());
    let dirs: Vec<PathBuf> = candidates.iter().map(|c| c.dir.clone()).collect();
    assert_eq!(dirs, vec![root.clone(), root.join("crates/inner")]);
}

#[test]
fn l2_rationale_bundle_scopes_manifests_to_candidate_dir() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"outer\"\n").unwrap();
    std::fs::create_dir_all(root.join("inner")).unwrap();
    std::fs::write(root.join("inner/Cargo.toml"), "[package]\nname=\"inner\"\n").unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let candidates = candidate_components_at(&db, ws, root.clone());
    assert_eq!(candidates.len(), 2);

    // Outer candidate's bundle contains only the outer Cargo.toml.
    let outer = candidates
        .iter()
        .find(|c| c.dir == root)
        .expect("outer candidate present");
    assert_eq!(
        outer.rationale_bundle.manifests,
        vec![root.join("Cargo.toml")]
    );

    // Inner candidate's bundle contains only the inner Cargo.toml.
    let inner = candidates
        .iter()
        .find(|c| c.dir == root.join("inner"))
        .expect("inner candidate present");
    assert_eq!(
        inner.rationale_bundle.manifests,
        vec![root.join("inner/Cargo.toml")]
    );
}

#[test]
fn l2_includes_dotgit_dir_as_candidate_even_without_manifests() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("repo/.git")).unwrap();
    std::fs::write(root.join("repo/README.md"), "# Repo\n").unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let candidates = candidate_components_at(&db, ws, root.clone());
    assert!(candidates.iter().any(|c| c.dir == root.join("repo")));
    let entry = candidates
        .iter()
        .find(|c| c.dir == root.join("repo"))
        .unwrap();
    assert!(entry.rationale_bundle.is_git_root);
    assert!(entry.rationale_bundle.manifests.is_empty());
}

#[test]
fn l2_emits_candidate_for_overrides_addition_at_empty_dir() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    // The specs/ dir has nothing in it — no manifest, no .git, no
    // README. L2 must still emit a candidate because an addition
    // references it.
    std::fs::create_dir_all(root.join("specs/my-spec")).unwrap();
    // Drop a single non-manifest file so the walker visits the dir.
    std::fs::write(root.join("specs/my-spec/.keep"), "").unwrap();

    let mut db = db_without_llm(&root);
    let overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins: BTreeMap::new(),
        additions: vec![ComponentEntry {
            id: "my-spec".into(),
            parent: None,
            kind: "spec".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("specs/my-spec"),
                content_sha: "0".into(),
            }],
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "spec".into(),
            deleted: false,
        }],
    };
    db.set_components_overrides(overrides);

    let ws = db.workspace();
    let candidates = candidate_components_at(&db, ws, root.clone());
    assert!(
        candidates
            .iter()
            .any(|c| c.dir == root.join("specs/my-spec")),
        "addition should surface a candidate; got {candidates:#?}"
    );
}

// ---------------------------------------------------------------------
// L3 — deterministic rules (the TestBackend has no canned responses,
// so any LLM dispatch causes a test failure)
// ---------------------------------------------------------------------

#[test]
fn l3_cargo_lib_classifies_as_rust_library_without_llm_call() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"x\"\n[lib]\npath = \"src/lib.rs\"\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::RustLibrary);
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    assert!(c.is_boundary);
}

#[test]
fn l3_cargo_bin_classifies_as_rust_cli_without_llm_call() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"x\"\n[[bin]]\nname=\"tool\"\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::RustCli);
}

#[test]
fn l3_cargo_workspace_wins_over_lib_section() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"a\"]\n[lib]\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::Workspace);
}

#[test]
fn l3_package_json_with_bin_classifies_as_node_cli() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"a\",\"bin\":\"cli.js\"}",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::NodeCli);
}

#[test]
fn l3_package_json_with_main_only_classifies_as_node_package() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"a\",\"main\":\"i.js\"}",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::NodeLibrary);
}

#[test]
fn l3_pyproject_toml_classifies_as_python_package() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::PythonLibrary);
}

#[test]
fn l3_bare_git_without_readme_classifies_as_non_component() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("repo/.git")).unwrap();
    std::fs::write(root.join("repo/.gitkeep"), "").unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.join("repo"));
    assert_eq!(c.kind, ComponentKind::NonComponent);
    assert!(!c.is_boundary);
}

// ---------------------------------------------------------------------
// L3 — pin short-circuits
// ---------------------------------------------------------------------

fn overrides_with_kind_pin(id: &str, kind: &str) -> OverridesFile {
    let mut pins = BTreeMap::new();
    let mut field_pins = BTreeMap::new();
    field_pins.insert(
        "kind".to_string(),
        PinValue::Value {
            value: kind.to_string(),
            reason: Some("test".into()),
        },
    );
    pins.insert(id.to_string(), field_pins);
    OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins,
        additions: Vec::new(),
    }
}

#[test]
fn l3_pin_short_circuits_before_deterministic_rules() {
    // Even though Cargo.toml with [lib] would classify as RustLibrary,
    // a pin at the same dir wins — with no LLM call (asserted because
    // TestBackend has no canned responses, so any dispatch errors).
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("crates/foo")).unwrap();
    std::fs::write(
        root.join("crates/foo/Cargo.toml"),
        "[package]\nname=\"foo\"\n[lib]\n",
    )
    .unwrap();

    let mut db = db_without_llm(&root);
    db.set_components_overrides(overrides_with_kind_pin("crates/foo", "spec"));

    let ws = db.workspace();
    let c = is_component(&db, ws, root.join("crates/foo"));
    assert_eq!(c.kind, ComponentKind::Spec);
    assert_eq!(c.rationale, "human pin");
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
}

#[test]
fn l3_pin_short_circuits_for_override_addition_without_manifests() {
    // Override-adds a dir that has no signals, then pins its kind.
    // L3 at that dir returns the pin directly — no LLM required.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("specs/api")).unwrap();
    std::fs::write(root.join("specs/api/.keep"), "").unwrap();

    let mut db = db_without_llm(&root);
    let mut overrides = overrides_with_kind_pin("api", "spec");
    overrides.additions.push(ComponentEntry {
        id: "api".into(),
        parent: None,
        kind: "spec".into(),
        lifecycle_roles: Vec::new(),
        language: None,
        build_system: None,
        role: None,
        path_segments: vec![PathSegment {
            path: PathBuf::from("specs/api"),
            content_sha: "0".into(),
        }],
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: Vec::new(),
        rationale: "hand-authored".into(),
        deleted: false,
    });
    db.set_components_overrides(overrides);

    let ws = db.workspace();
    let c = is_component(&db, ws, root.join("specs/api"));
    assert_eq!(c.kind, ComponentKind::Spec);
    assert_eq!(c.rationale, "human pin");
}

#[test]
fn l3_suppress_pin_sets_is_boundary_false() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n[lib]\n").unwrap();

    let mut overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins: BTreeMap::new(),
        additions: Vec::new(),
    };
    let mut field_pins = BTreeMap::new();
    field_pins.insert(
        "suppress".to_string(),
        PinValue::Suppress {
            suppress: AlwaysTrue,
        },
    );
    overrides.pins.insert(
        root.file_name().unwrap().to_string_lossy().into_owned(),
        field_pins,
    );

    let mut db = db_without_llm(&root);
    db.set_components_overrides(overrides);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert!(!c.is_boundary);
}

// ---------------------------------------------------------------------
// L3 — LLM fallback
// ---------------------------------------------------------------------

fn canned_response(kind: &str, is_boundary: bool) -> serde_json::Value {
    json!({
        "kind": kind,
        "language": null,
        "build_system": null,
        "lifecycle_roles": ["runtime"],
        "role": null,
        "evidence_grade": "medium",
        "evidence_fields": ["llm"],
        "rationale": "delegated",
        "is_boundary": is_boundary,
    })
}

#[test]
fn l3_ambiguous_candidate_calls_llm_fallback() {
    // An .md-only directory with a README but no manifest has no
    // deterministic rule; L3 must dispatch to the LLM. The canned
    // response drives the classification.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("README.md"), "# Docs-only project\n## Purpose\n").unwrap();

    let backend = Arc::new(TestBackend::new());

    // Pre-register the backend with a canned response matching the
    // exact inputs L3 will build. We construct the same inputs
    // structure here — if L3's JSON shape drifts, this miss will
    // surface immediately.
    let inputs = json!({
        "dir_relative": "",
        "rationale_bundle": {
            "manifests": [],
            "is_git_root": false,
            "doc_headings": [
                { "path": "README.md", "level": 1, "text": "Docs-only project" },
                { "path": "README.md", "level": 2, "text": "Purpose" },
            ],
            "shebangs": [],
        },
        "manifest_contents": {},
    });
    backend.respond(
        PromptId::Classify,
        inputs,
        canned_response("docs-repo", true),
    );

    let db = build_db(backend, &root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::DocsRepo);
    assert!(c.is_boundary);
    assert_eq!(c.rationale, "delegated");
}

#[test]
fn l3_deterministic_fixtures_never_dispatch_to_llm() {
    // A single backend instance passed into a series of fixtures that
    // should each hit a deterministic rule. The backend has no canned
    // responses, so any accidental dispatch fails the test.
    let backend = Arc::new(TestBackend::new());

    for manifest_contents in &[
        "[package]\nname=\"x\"\n[lib]\n",
        "[package]\nname=\"x\"\n[[bin]]\nname=\"x\"\n",
        "[workspace]\nmembers=[]\n",
    ] {
        let td = TempDir::new().unwrap();
        let root = td.path().to_path_buf();
        std::fs::write(root.join("Cargo.toml"), manifest_contents).unwrap();

        let db = build_db(backend.clone(), &root);
        let ws = db.workspace();
        let c = is_component(&db, ws, root.clone());
        assert_ne!(c.kind, ComponentKind::NonComponent);
        assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    }
}
