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
    all_components, candidate_components_at, is_component, parse_embedded_component_kinds_yaml,
    render_kinds_for_prompt, render_lifecycle_scopes_for_prompt, seed_filesystem,
    surface_artefacts_of, AtlasDatabase, ComponentKind,
};
use atlas_index::{
    AlwaysTrue, ComponentEntry, OverridesFile, PathSegment, PinValue, OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TestBackend};
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
    let mut db = AtlasDatabase::new(backend, vec![root.to_path_buf()], default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
}

/// Builds a DB seeded from `root` with a fresh TestBackend that has
/// no canned responses — any accidental LLM dispatch fails.
fn db_without_llm(root: &Path) -> AtlasDatabase {
    build_db(Arc::new(TestBackend::new()), root)
}

/// A lenient LLM backend that returns minimal valid stubs for every
/// prompt. Used by surface-emission tests that need the full pipeline
/// (L3 + L5) without a real LLM. Mirrors
/// `surfaces_emission_rust::LenientBackend`.
struct LenientStubBackend {
    fingerprint: LlmFingerprint,
}

impl LenientStubBackend {
    fn new() -> Arc<Self> {
        Arc::new(LenientStubBackend {
            fingerprint: default_fingerprint(),
        })
    }
}

impl LlmBackend for LenientStubBackend {
    fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "typescript-package",
                "language": "typescript",
                "build_system": "npm",
                "evidence_grade": "strong",
                "evidence_fields": [],
                "rationale": "stub",
                "is_boundary": false,
            }),
            PromptId::Stage1Surface => json!({ "purpose": "stub", "notes": "" }),
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

/// Build a seeded database with a lenient stub backend so both
/// deterministic and LLM-reliant pipeline stages succeed.
fn build_db_lenient(root: &Path) -> AtlasDatabase {
    let backend = LenientStubBackend::new();
    let mut db = AtlasDatabase::new(backend, vec![root.to_path_buf()], default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
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
            id: component_ontology::ComponentId::parse("my-spec").unwrap(),
            parent: None,
            kind: "spec".into(),
            lifecycle_roles: Vec::new(),
            languages: std::collections::BTreeSet::new(),
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
        ..OverridesFile::default()
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
fn l3_package_json_with_tsconfig_classifies_as_typescript_package_without_llm_call() {
    // Phase 2 PR-1 acceptance: a `package.json` + `tsconfig.json` +
    // `src/index.ts` fixture is classified as `typescript-package`
    // by the new TS/JS classifier, deterministically, without any
    // LLM dispatch.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"my-pkg\",\"version\":\"0.1.0\"}",
    )
    .unwrap();
    std::fs::write(root.join("tsconfig.json"), "{\"compilerOptions\":{}}").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "export function hello(): string { return \"world\"; }\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::TypescriptPackage);
    assert!(
        c.languages.contains("typescript"),
        "expected typescript in languages, got {:?}",
        c.languages
    );
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
}

#[test]
fn l3_main_plus_tsconfig_classifies_as_typescript_package_without_llm_call() {
    // Integration-level pin for the precedence rule: "package.json with a
    // `main` field + adjacent `tsconfig.json` → typescript-package" (NOT
    // node-library). The unit-level pin lives in ts_js_classifier.rs; this
    // test fixes the rule at the L3 dispatcher level so a future refactor
    // of dispatch order or legacy_heuristics cannot silently flip the case.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"my-pkg\",\"version\":\"0.1.0\",\"main\":\"src/index.js\"}",
    )
    .unwrap();
    std::fs::write(root.join("tsconfig.json"), "{\"compilerOptions\":{}}").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "export function hello(): string { return \"world\"; }\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(
        c.kind,
        ComponentKind::TypescriptPackage,
        "expected typescript-package, got {:?} — main+tsconfig precedence rule broken",
        c.kind
    );
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
}

#[test]
fn l5_typescript_package_surface_artefacts_include_exported_hello_symbol() {
    // Phase 2 PR-1 spec-review fix: `surface_artefacts_of` must drive
    // `extract_ts_js_surface` for typescript-package components and
    // return a `LibraryApi` whose `pub_items` includes every exported
    // symbol from the component's source files.
    //
    // Fixture: `package.json` + `tsconfig.json` + `src/index.ts` that
    // exports `hello`. The test asserts that after L5 extraction the
    // resulting `SurfaceArtefacts` contains a binding for `hello` and
    // a `LibraryApi` with `hello` in its `pub_items`.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"my-pkg\",\"version\":\"0.1.0\"}",
    )
    .unwrap();
    std::fs::write(root.join("tsconfig.json"), "{\"compilerOptions\":{}}").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "export function hello(): string { return \"world\"; }\n",
    )
    .unwrap();

    let db = build_db_lenient(&root);

    // Locate the component produced by L3 for the root fixture.
    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce at least one live component");
    let comp_id = comp.id.clone();

    // Drive L5 surface extraction through the production path.
    let artefacts = surface_artefacts_of(&db, comp_id);

    // At least one binding for `hello` must be present.
    let hello_binding = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "hello")
        .unwrap_or_else(|| {
            panic!(
                "expected a binding for `hello` in surface artefacts, got bindings: {:?}",
                artefacts.bindings
            )
        });
    assert_eq!(hello_binding.language, "typescript");

    // The `LibraryApi` must be present and list `hello` in `pub_items`.
    assert_eq!(
        artefacts.library_apis.len(),
        1,
        "expected exactly one LibraryApi, got {:?}",
        artefacts.library_apis
    );
    let api = &artefacts.library_apis[0];
    assert_eq!(api.language, "typescript");
    let pub_names: Vec<&str> = api.pub_items.iter().map(|p| p.name.as_str()).collect();
    assert!(
        pub_names.contains(&"hello"),
        "`hello` must appear in library_api pub_items; got: {pub_names:?}"
    );
}

#[test]
fn l3_pyproject_toml_classifies_as_python_package_without_llm_call() {
    // Phase 2 PR-3 acceptance: a `pyproject.toml` fixture is
    // classified as `python-package` at L3 deterministically by the
    // new `python-classifier`, with no LLM dispatch. The analyser
    // identity propagates through PR-4's id/version plumbing so
    // downstream consumers can attribute the verdict.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::PythonPackage);
    assert_eq!(c.analyser_id, "python-classifier");
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    assert!(c.languages.contains("python"));
    assert_eq!(c.build_system.as_deref(), Some("pyproject"));
}

#[test]
fn l3_setup_py_classifies_as_python_package() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("setup.py"),
        "from setuptools import setup\nsetup(name=\"x\")\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::PythonPackage);
    assert_eq!(c.analyser_id, "python-classifier");
}

// ---------------------------------------------------------------------
// L3 — Dart / Flutter classification (Phase 2 PR-7)
//
// F1 regression: ComponentKind::parse must recognise "dart-package" and
// "flutter-package" so dart_to_classification doesn't silently coerce
// every Dart-classified component to ComponentKind::NonComponent.
// ---------------------------------------------------------------------

#[test]
fn pubspec_classifies_through_classification_dart_package() {
    // Phase 2 PR-7 acceptance: a `pubspec.yaml` with no `flutter:` block
    // is classified as `ComponentKind::DartPackage` at L3 deterministically
    // by the `dart-classifier`, with no LLM dispatch. Mirrors the Python
    // PR-3 test `l3_pyproject_toml_classifies_as_python_package_without_llm_call`.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("pubspec.yaml"),
        "name: dart_pkg\nversion: 0.1.0\n\ndependencies:\n  meta: any\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(
        c.kind,
        ComponentKind::DartPackage,
        "pubspec.yaml without flutter: block must produce DartPackage, got {:?}",
        c.kind
    );
    assert_eq!(c.analyser_id, "dart-classifier");
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    assert!(
        c.languages.contains("dart"),
        "language must be dart; got {:?}",
        c.languages
    );
    assert_eq!(c.build_system.as_deref(), Some("pub"));
    assert!(c.is_boundary);
}

#[test]
fn pubspec_with_flutter_block_classifies_as_flutter_package() {
    // Phase 2 PR-7 acceptance: a `pubspec.yaml` with a top-level `flutter:`
    // block is classified as `ComponentKind::FlutterPackage` at L3
    // deterministically, with no LLM dispatch.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("pubspec.yaml"),
        "name: flutter_app\nversion: 1.0.0\n\nflutter:\n  uses-material-design: true\n\ndependencies:\n  flutter:\n    sdk: flutter\n",
    )
    .unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(
        c.kind,
        ComponentKind::FlutterPackage,
        "pubspec.yaml with flutter: block must produce FlutterPackage, got {:?}",
        c.kind
    );
    assert_eq!(c.analyser_id, "dart-classifier");
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    assert!(c.languages.contains("dart"));
    assert_eq!(c.build_system.as_deref(), Some("pub"));
    assert!(c.is_boundary);
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
    pins.insert(
        component_ontology::ComponentId::parse(id).unwrap(),
        field_pins,
    );
    OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins,
        additions: Vec::new(),
        ..OverridesFile::default()
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
        id: component_ontology::ComponentId::parse("api").unwrap(),
        parent: None,
        kind: "spec".into(),
        lifecycle_roles: Vec::new(),
        languages: std::collections::BTreeSet::new(),
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
        ..OverridesFile::default()
    };
    let mut field_pins = BTreeMap::new();
    field_pins.insert(
        "suppress".to_string(),
        PinValue::Suppress {
            suppress: AlwaysTrue,
        },
    );
    // Pin against the slugified form of the workspace root's basename
    // — that's the id form a user sees in components.yaml and is what
    // L3's pin lookup tries via `slugify_segment`.
    let basename = root.file_name().unwrap().to_string_lossy();
    let slug: String = basename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    overrides.pins.insert(
        component_ontology::ComponentId::parse(&slug).unwrap(),
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
    let kinds_yaml =
        parse_embedded_component_kinds_yaml().expect("embedded component-kinds YAML must parse");
    let inputs = json!({
        "DIR_RELATIVE": "",
        "RATIONALE_BUNDLE": {
            "manifests": [],
            "is_git_root": false,
            "doc_headings": [
                { "path": "README.md", "level": 1, "text": "Docs-only project" },
                { "path": "README.md", "level": 2, "text": "Purpose" },
            ],
            "shebangs": [],
        },
        "MANIFEST_CONTENTS": {},
        "COMPONENT_KINDS": render_kinds_for_prompt(&kinds_yaml),
        "LIFECYCLE_SCOPES": render_lifecycle_scopes_for_prompt(&kinds_yaml),
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

// ---------------------------------------------------------------------
// PR-5 acceptance criteria: Cargo and Dockerfile analysers run
// without any LLM dispatch.
// ---------------------------------------------------------------------

#[test]
fn pr5_cargo_lib_classified_via_registry_without_llm() {
    // PR-5 acceptance: a `Cargo.toml` containing a `[lib]` section
    // is classified `rust-library` by the Cargo analyser without an
    // LLM call. The TestBackend has no canned responses; we further
    // assert that `db.llm_cache().call_count()` is zero so a
    // future regression where the registry fell through to the LLM
    // surfaces immediately.
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
    assert!(
        c.evidence_fields.iter().any(|f| f.contains("[lib]")),
        "evidence_fields must mention `[lib]`; got {:?}",
        c.evidence_fields
    );
    assert_eq!(
        db.llm_cache().call_count(),
        0,
        "Cargo classifier must not dispatch to the LLM"
    );
}

#[test]
fn pr5_dockerfile_classified_as_docker_image_without_llm() {
    // PR-5 acceptance: a fixture with a `Dockerfile` produces a
    // `kind: docker-image` component without any LLM call.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let docker_text = "FROM rust:1.79 AS builder\n\
                       COPY src/ /app/src\n\
                       FROM debian:bookworm-slim\n\
                       COPY --from=builder /app/target/release/atlas /usr/local/bin/atlas\n\
                       EXPOSE 8080\n\
                       CMD [\"atlas\", \"--help\"]\n";
    std::fs::write(root.join("Dockerfile"), docker_text).unwrap();

    let db = db_without_llm(&root);
    let ws = db.workspace();
    let c = is_component(&db, ws, root.clone());
    assert_eq!(c.kind, ComponentKind::DockerImage);
    assert_eq!(c.evidence_grade, EvidenceGrade::Strong);
    assert!(c.is_boundary);
    assert_eq!(
        c.build_system.as_deref(),
        Some("docker"),
        "build_system must be docker; got {:?}",
        c.build_system
    );
    assert_eq!(
        db.llm_cache().call_count(),
        0,
        "Dockerfile classifier must not dispatch to the LLM"
    );
}
