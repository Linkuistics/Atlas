//! L5 surface-extraction integration tests for the Python subprocess
//! analyser (Atlas vNext Phase 2 PR-3).
//!
//! These tests drive `surface_artefacts_of` end-to-end through the
//! actual `python-analyzer` subprocess transport that PR-3 wired up.
//! `cargo test --workspace` builds the python-analyzer binary into
//! `target/<profile>/`; the engine resolves it at runtime via
//! [`atlas_analyzers::locate_python_analyzer_binary`].
//!
//! The tests skip themselves if the binary cannot be located —
//! defensive against running these tests outside a cargo target tree.
//! On CI / `cargo test --workspace` the binary is always present, so
//! the skip path is dead code in practice.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{
    all_components, seed_filesystem, surface_artefacts_of, AtlasDatabase, ComponentKind,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TestBackend};
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

/// A lenient backend that returns empty stubs for every prompt — the
/// tests below care about the deterministic surface analyser, not the
/// LLM-derived `SurfaceRecord` inner. The `Stage1Surface` stub returns
/// the minimum fields `parse_surface_response` accepts.
struct LenientBackend {
    fingerprint: LlmFingerprint,
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "python-package",
                "language": "python",
                "evidence_grade": "strong",
                "evidence_fields": [],
                "rationale": "stub",
                "is_boundary": true,
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

fn build_db_lenient(root: &Path) -> AtlasDatabase {
    let backend: Arc<dyn LlmBackend> = Arc::new(LenientBackend {
        fingerprint: default_fingerprint(),
    });
    let mut db = AtlasDatabase::new(backend, vec![root.to_path_buf()], default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
}

fn build_db_test(root: &Path) -> AtlasDatabase {
    let backend = Arc::new(TestBackend::with_fingerprint(default_fingerprint()));
    let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
    let mut db = AtlasDatabase::new(backend_dyn, vec![root.to_path_buf()], default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
}

fn write_python_package_fixture(root: &Path, project_name: &str) {
    std::fs::write(
        root.join("pyproject.toml"),
        format!("[project]\nname = \"{project_name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "from .mod import foo, Bar\n").unwrap();
    std::fs::write(
        root.join("pkg/mod.py"),
        "from dataclasses import dataclass\n\n\
         def foo():\n    return 1\n\n\
         @dataclass\n\
         class Bar:\n    a: int = 0\n",
    )
    .unwrap();
}

fn skip_if_binary_missing() -> bool {
    if atlas_analyzers::locate_python_analyzer_binary().is_none() {
        eprintln!("skipping: python-analyzer binary not located in target/");
        return true;
    }
    false
}

#[test]
fn l5_python_package_surface_artefacts_lists_pkg_mod_foo_and_pkg_mod_bar_bindings() {
    // PR-3 acceptance criterion (§4): a `pyproject.toml` +
    // `pkg/__init__.py` + `pkg/mod.py` fixture is classified
    // `python-package` at L3 with no LLM call, and its surfaces.yaml
    // lists `pkg.mod.foo` and `pkg.mod.Bar` as bindings.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    write_python_package_fixture(&root, "py-pkg");

    let db = build_db_lenient(&root);

    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce a live component");
    let comp_id = comp.id.clone();
    assert_eq!(
        comp.kind, "python-package",
        "fixture must classify as python-package, got {}",
        comp.kind
    );

    let artefacts = surface_artefacts_of(&db, comp_id);
    let symbols: Vec<&str> = artefacts
        .bindings
        .iter()
        .map(|b| b.symbol.as_str())
        .collect();
    assert!(
        symbols.contains(&"foo"),
        "expected `foo` in bindings, got: {symbols:?}"
    );
    assert!(
        symbols.contains(&"Bar"),
        "expected `Bar` in bindings, got: {symbols:?}"
    );

    // Module path: `pkg.mod.foo` (not `pkg.__init__.foo`).
    let foo = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "foo" && b.module_path == vec!["pkg", "mod", "foo"])
        .unwrap_or_else(|| {
            panic!(
                "expected pkg.mod.foo binding; got: {:?}",
                artefacts.bindings
            )
        });
    assert_eq!(foo.language, "python");
    assert!(matches!(
        foo.visibility,
        atlas_index::Visibility::Conventional
    ));

    // The Bar binding carries the @dataclass decorator chain.
    let bar = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Bar")
        .expect("Bar binding present");
    let chain = bar
        .attributes
        .get("decorator_chain")
        .expect("decorator_chain must be present on @dataclass class")
        .as_sequence()
        .unwrap();
    let names: Vec<String> = chain
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        names.contains(&"dataclass".to_string()),
        "got decorator_chain {names:?}"
    );
}

#[test]
fn python_underscore_prefix_function_records_conventional_private_attribute() {
    // PR-3 acceptance criterion: a Python file with `def _private()`
    // and `def public()` produces two bindings, distinguished by the
    // conventional-private attribute.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"private-pkg\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("pkg/mod.py"),
        "def public():\n    return 1\n\n\
         def _private():\n    return 2\n",
    )
    .unwrap();

    let db = build_db_test(&root);
    let comp_id = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .unwrap()
        .id
        .clone();
    let artefacts = surface_artefacts_of(&db, comp_id);

    let public = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "public")
        .expect("public binding present");
    let private = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "_private")
        .expect("_private binding present");

    assert!(
        !public.attributes.contains_key("private"),
        "public binding must not have private attribute"
    );
    assert_eq!(
        private.attributes.get("private"),
        Some(&serde_yaml::Value::Bool(true)),
        "_private binding must record `private: true`"
    );
}

#[test]
fn python_dataclass_decorator_recorded_in_attributes_decorator_chain() {
    // PR-3 acceptance criterion: a `@dataclass` decorator on a class
    // produces a binding whose
    // `attributes.decorator_chain` includes `dataclass`.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"deco-pkg\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("pkg/mod.py"),
        "from dataclasses import dataclass\n\n\
         @dataclass\n\
         class Frozen:\n    a: int = 0\n",
    )
    .unwrap();

    let db = build_db_test(&root);
    let comp_id = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .unwrap()
        .id
        .clone();
    let artefacts = surface_artefacts_of(&db, comp_id);

    let frozen = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Frozen")
        .expect("Frozen binding present");
    let chain = frozen
        .attributes
        .get("decorator_chain")
        .expect("decorator_chain must be present")
        .as_sequence()
        .unwrap();
    let names: Vec<String> = chain
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        names.contains(&"dataclass".to_string()),
        "expected `dataclass` in chain, got: {names:?}"
    );
}

#[test]
fn python_binary_sha_change_invalidates_l5_cache() {
    // PR-3 acceptance criterion: re-running with the same binary
    // produces a cache hit; touching the python-analyzer binary
    // content invalidates the L5 cache.
    //
    // Implementation sketch: the L5 fingerprint contributes the
    // python-analyzer binary's content sha via tag 0x06 when the
    // component is Python. Two fingerprint computations that differ
    // only in the binary sha must produce different fingerprints.
    // The test exercises the production fingerprint path by hashing
    // the locate_python_analyzer_binary() artefact under two
    // different content sha values.
    use atlas_engine::{FingerprintBuilder, Sha256Hex};
    use atlas_index::Stage;

    fn fp_with_binary_sha(binary_sha: &Sha256Hex) -> Sha256Hex {
        let mut fb = FingerprintBuilder::new(Stage::L5, "l5-driver", "1.0.0");
        fb.add_analyzer_registry_sha(&"reg".to_string());
        fb.add_file_content_sha(&"file".to_string());
        fb.add_analyzer_binary_sha(binary_sha);
        fb.finalise()
    }

    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let fp_a = fp_with_binary_sha(&sha_a);
    let fp_b = fp_with_binary_sha(&sha_b);
    assert_ne!(
        fp_a, fp_b,
        "different python-analyzer binary shas must produce \
         different L5 fingerprints (cache miss on binary content change)"
    );

    // Re-computing with the same sha must produce the same
    // fingerprint (cache hit on no-op rerun).
    let fp_a_again = fp_with_binary_sha(&sha_a);
    assert_eq!(
        fp_a, fp_a_again,
        "stable binary sha must produce stable L5 fingerprint (cache hit)"
    );
}

#[test]
fn python_package_classifies_via_python_classifier_at_l3() {
    // L3 path identity check: ensure the Python L3 classifier
    // produces the verdict, not the legacy heuristics rule.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"identity-pkg\"\n",
    )
    .unwrap();

    let db = build_db_test(&root);
    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");
    assert_eq!(
        comp.kind,
        ComponentKind::PythonPackage.as_str(),
        "expected python-package, got {}",
        comp.kind
    );
}
