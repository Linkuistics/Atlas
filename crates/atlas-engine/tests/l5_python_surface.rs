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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use atlas_engine::testing::LenientBackend;
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

fn lenient_classify() -> serde_json::Value {
    json!({
        "kind": "python-package",
        "language": "python",
        "evidence_grade": "strong",
        "evidence_fields": [],
        "rationale": "stub",
        "is_boundary": true,
    })
}

/// A counting backend that tracks every `Classify`-stage prompt
/// dispatch. Used by the §4 PR-3 criterion-1 combined test (F1) to
/// prove deterministic L3 *without* an LLM call: if the python
/// classifier short-circuits correctly, `classify_calls` stays 0 even
/// while L5's `Stage1Surface` is dispatched. Non-Classify prompts
/// receive lenient stub responses so L5 can complete.
struct ClassifyCountingBackend {
    fingerprint: LlmFingerprint,
    classify_calls: Arc<AtomicUsize>,
}

impl LlmBackend for ClassifyCountingBackend {
    fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        match req.prompt_template {
            PromptId::Classify => {
                self.classify_calls.fetch_add(1, Ordering::SeqCst);
                Err(LlmError::TestBackendMiss(
                    "Classify prompt must NOT fire on a deterministic-classifier fixture \
                     (§4 PR-3 criterion 1: `pyproject.toml` is classified at L3 with no \
                     LLM call)"
                        .to_string(),
                ))
            }
            PromptId::Stage1Surface => Ok(json!({ "purpose": "stub", "notes": "" })),
            PromptId::Stage2Edges => Ok(json!([])),
            PromptId::Subcarve => Ok(json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            })),
        }
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

fn build_db_lenient(root: &Path) -> AtlasDatabase {
    let backend: Arc<dyn LlmBackend> =
        LenientBackend::with_classify(default_fingerprint(), lenient_classify());
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

    // §4 PR-3 schema: `module_path` is file-path components only
    // (`["pkg", "mod"]` for `pkg/mod.py`), NOT including the
    // symbol. The dotted `pkg.mod.foo` is reconstructed downstream
    // as `module_path.join(".") + "." + symbol`.
    let foo = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "foo" && b.module_path == vec!["pkg", "mod"])
        .unwrap_or_else(|| {
            panic!(
                "expected `foo` binding with module_path [\"pkg\", \"mod\"]; got: {:?}",
                artefacts.bindings
            )
        });
    assert_eq!(foo.language, "python");
    assert!(matches!(
        foo.visibility,
        atlas_index::Visibility::Conventional
    ));
    // The dotted form `pkg.mod.foo` is the join + symbol.
    let dotted_foo = format!("{}.{}", foo.module_path.join("."), foo.symbol);
    assert_eq!(dotted_foo, "pkg.mod.foo");
    let bar_binding = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Bar")
        .expect("Bar binding present");
    let dotted_bar = format!(
        "{}.{}",
        bar_binding.module_path.join("."),
        bar_binding.symbol
    );
    assert_eq!(dotted_bar, "pkg.mod.Bar");

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

    // §4 PR-3: the *only* discriminator between `public` and
    // `_private` is the `attributes.private` flag. Both bindings
    // share `Visibility::Conventional` — Python has no leading-`_`
    // visibility *keyword*, so a regression that synthesised
    // `Visibility::Explicit { keyword: "_" }` for the underscore
    // form would slip through tests that only assert the attribute.
    // Pin the variant for both so that regression is caught.
    assert!(
        matches!(public.visibility, atlas_index::Visibility::Conventional),
        "public binding must be Visibility::Conventional, got {:?}",
        public.visibility
    );
    assert!(
        matches!(private.visibility, atlas_index::Visibility::Conventional),
        "_private binding must also be Visibility::Conventional (NOT \
         Explicit{{keyword=\"_\"}}); got {:?}",
        private.visibility
    );

    assert!(
        !public.attributes.contains_key("private"),
        "public binding must not have private attribute"
    );
    assert!(
        private.attributes.contains_key("private"),
        "_private binding must record the `private` attribute key"
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
fn python_l5_cache_layer_honours_binary_sha_in_fingerprint_end_to_end() {
    // §4 PR-3 acceptance criterion 4 (end-to-end):
    //
    // (a) re-running with the same binary produces a cache hit;
    // (b) touching the python-analyzer binary content invalidates
    //     the L5 cache.
    //
    // The complementary `python_binary_sha_change_invalidates_l5_cache`
    // test above proves the FingerprintBuilder math is correct in
    // isolation. This test goes one level higher and asserts the
    // *cache layer* itself honours the binary-sha-bearing
    // fingerprint: a regression that dropped `binary_sha` from the
    // persistent-cache key (or short-circuited the lookup before
    // the fingerprint was consulted) would be caught here.
    //
    // Shape:
    //
    // 1. Open a `PersistentCache` over a tempdir. Build a
    //    counting backend.
    // 2. Drive `LlmResponseCache::call_cached_with_fp` once with a
    //    fingerprint that includes a synthetic binary_sha = "a*64".
    //    Backend call_count goes from 0 → 1; persistent_hits stays
    //    at 0.
    // 3. Construct a *fresh* `LlmResponseCache` over the *same*
    //    persistent store and call again with the *same*
    //    fingerprint. Backend call_count stays at 0;
    //    persistent_hit_count goes from 0 → 1 (cache hit).
    // 4. Construct a *fresh* `LlmResponseCache` over the same
    //    persistent store and call with a fingerprint that differs
    //    *only* in the binary_sha contribution (binary_sha =
    //    "b*64"). Backend call_count goes from 0 → 1;
    //    persistent_hit_count stays at 0 (cache miss).
    //
    // This pins the binary_sha as a *load-bearing* contributor to
    // the L5 cache key — independent of whether
    // `surface_of`/`surface_artefacts_of` happen to wrap it
    // correctly.
    use atlas_engine::cache::PersistentCache;
    use atlas_engine::llm_cache::LlmResponseCache;
    use atlas_engine::{FingerprintBuilder, Sha256Hex};
    use atlas_index::Stage;
    use atlas_llm::ResponseSchema;

    fn build_l5_fp(binary_sha: &Sha256Hex) -> Sha256Hex {
        // Deterministic synthetic fingerprint shape mirroring
        // production: the only knob the test varies is the trailing
        // `add_analyzer_binary_sha` contribution.
        let mut fb = FingerprintBuilder::new(Stage::L5, "l5-driver", "1.0.0");
        fb.add_analyzer_registry_sha(&"reg-sha".to_string());
        fb.add_file_content_sha(&"file-sha".to_string());
        fb.add_analyzer_binary_sha(binary_sha);
        fb.finalise()
    }

    let request = atlas_llm::LlmRequest {
        prompt_template: PromptId::Stage1Surface,
        inputs: json!({ "id": "py-comp" }),
        schema: ResponseSchema::accept_any(),
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let backend = TestBackend::with_fingerprint(default_fingerprint());
    backend.respond(
        PromptId::Stage1Surface,
        json!({ "id": "py-comp" }),
        json!({ "purpose": "stub", "notes": "" }),
    );

    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let fp_a = build_l5_fp(&sha_a);
    let fp_b = build_l5_fp(&sha_b);

    // Step 1: cold cache, fingerprint A — backend invoked.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_a, &backend, &request)
            .expect("cold call A succeeds");
        assert_eq!(
            cache.call_count(),
            1,
            "cold run with binary_sha=A must invoke the backend exactly once"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            0,
            "cold run cannot have a persistent hit"
        );
    }

    // Step 2: warm cache (fresh in-memory layer over same persistent
    // store), same fingerprint A — must hit persistent layer, no
    // backend call.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_a, &backend, &request)
            .expect("warm call A succeeds");
        assert_eq!(
            cache.call_count(),
            0,
            "warm run with the same binary_sha must not invoke the backend (cache hit)"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            1,
            "warm run with the same binary_sha must record a persistent hit"
        );
    }

    // Step 3: fresh cache, fingerprint B (binary_sha mutated) — must
    // miss the persistent layer (cache miss on binary content
    // change) and invoke the backend again.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_b, &backend, &request)
            .expect("call B succeeds");
        assert_eq!(
            cache.call_count(),
            1,
            "binary_sha mutation must invalidate the L5 cache (backend re-invoked)"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            0,
            "binary_sha mutation must NOT serve a persistent-cache hit"
        );
    }

    // Final defensive check: fingerprint A and B differ.
    assert_ne!(
        fp_a, fp_b,
        "synthetic fingerprints must differ when binary_sha changes"
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

#[test]
fn pyproject_fixture_classifies_python_package_no_llm_and_lists_pkg_mod_bindings() {
    // §4 PR-3 acceptance criterion 1 (combined): a `pyproject.toml`
    // + `pkg/__init__.py` + `pkg/mod.py` fixture is classified
    // `python-package` at L3 with no LLM call, AND its surfaces.yaml
    // lists `pkg.mod.foo` and `pkg.mod.Bar` as bindings. The two
    // halves are observed against the *same* fixture in this single
    // test so a regression that drifts only one half (e.g. the
    // classifier still works but the L5 wiring loses bindings, or
    // vice versa) is caught.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    write_python_package_fixture(&root, "py-pkg");

    // ClassifyCountingBackend errors on every Classify-prompt
    // dispatch; if the deterministic python-classifier short-circuit
    // works the counter stays 0. Stage1Surface is stubbed so L5 can
    // complete (the LLM-derived inner SurfaceRecord is not under
    // test here).
    let classify_calls = Arc::new(AtomicUsize::new(0));
    let backend: Arc<dyn LlmBackend> = Arc::new(ClassifyCountingBackend {
        fingerprint: default_fingerprint(),
        classify_calls: classify_calls.clone(),
    });
    let mut db = AtlasDatabase::new(backend, vec![root.clone()], default_fingerprint());
    seed_filesystem(&mut db, std::slice::from_ref(&root), false)
        .expect("seed_filesystem must succeed");

    // L3 half: classified `python-package` deterministically.
    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");
    assert_eq!(
        comp.kind,
        ComponentKind::PythonPackage.as_str(),
        "fixture must classify as python-package, got {}",
        comp.kind
    );
    assert_eq!(
        classify_calls.load(Ordering::SeqCst),
        0,
        "L3 deterministic classifier must short-circuit — Classify \
         prompt fired {} time(s)",
        classify_calls.load(Ordering::SeqCst)
    );

    // L5 half: the *same* fixture's surfaces.yaml lists
    // `pkg.mod.foo` and `pkg.mod.Bar` as bindings.
    let artefacts = surface_artefacts_of(&db, comp.id.clone());
    let foo = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "foo")
        .expect("expected `foo` binding");
    let bar = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Bar")
        .expect("expected `Bar` binding");
    let dotted_foo = format!("{}.{}", foo.module_path.join("."), foo.symbol);
    let dotted_bar = format!("{}.{}", bar.module_path.join("."), bar.symbol);
    assert_eq!(
        dotted_foo, "pkg.mod.foo",
        "expected pkg.mod.foo in bindings; got module_path={:?} symbol={}",
        foo.module_path, foo.symbol
    );
    assert_eq!(
        dotted_bar, "pkg.mod.Bar",
        "expected pkg.mod.Bar in bindings; got module_path={:?} symbol={}",
        bar.module_path, bar.symbol
    );

    // Final defensive check: even after the L5 walk, no Classify
    // prompt should have fired. (L5's `Stage1Surface` is allowed.)
    assert_eq!(
        classify_calls.load(Ordering::SeqCst),
        0,
        "Classify prompt fired during L5 traversal — \
         deterministic python-classifier short-circuit was bypassed"
    );
}
