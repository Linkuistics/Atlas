//! Integration tests for the LispKit surface analyser (Atlas vNext
//! Phase 2 PR-10).
//!
//! Tests run the `extract_lispkit_surface` function against realistic
//! `*.sld` fixtures and the `lispkit-analyzer` subprocess binary
//! (for the wire-protocol round-trip test) to verify:
//!
//! 1. Exported symbols from `(export ...)` → non-private bindings.
//! 2. Non-exported `(define name ...)` inside `(begin ...)` → private.
//! 3. Library identifier `(define-library (lib name) ...)` → module_path.
//! 4. `language` is always `"scheme"`.
//! 5. `library_apis` contains only the exported symbols.

use std::path::PathBuf;

use atlas_index::{Visibility, ATTR_PRIVATE};
use atlas_lispkit_analyzer::{
    extract_lispkit_surface, LispKitSourceInputs, ANALYZER_ID, ANALYZER_VERSION,
};
use serde_yaml::Value as YamlValue;

/// Load the simple-lib fixture and return it as inputs.
fn simple_lib_inputs() -> LispKitSourceInputs {
    let fixture_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple-lib/core.sld");
    let bytes = std::fs::read(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {fixture_path:?}: {e}"));
    LispKitSourceInputs {
        sources: vec![(PathBuf::from("core.sld"), bytes)],
    }
}

// ---------------------------------------------------------------------------
// Acceptance criterion 1: exported symbols → non-private bindings
// ---------------------------------------------------------------------------

#[test]
fn exported_symbols_are_not_private() {
    let inputs = simple_lib_inputs();
    let out = extract_lispkit_surface("simple-lib", &inputs);

    let greet = out
        .bindings
        .iter()
        .find(|b| b.symbol == "greet")
        .expect("`greet` binding must be present");
    assert!(
        !greet.attributes.contains_key("private"),
        "`greet` is exported and must not be private, got attrs: {:?}",
        greet.attributes
    );
    assert!(
        matches!(greet.visibility, Visibility::Conventional),
        "expected Conventional visibility"
    );

    let farewell = out
        .bindings
        .iter()
        .find(|b| b.symbol == "farewell")
        .expect("`farewell` binding must be present");
    assert!(
        !farewell.attributes.contains_key("private"),
        "`farewell` is exported and must not be private"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criterion 2: non-exported define → private: true
// ---------------------------------------------------------------------------

#[test]
fn non_exported_define_is_flagged_private() {
    let inputs = simple_lib_inputs();
    let out = extract_lispkit_surface("simple-lib", &inputs);

    let helper = out
        .bindings
        .iter()
        .find(|b| b.symbol == "format-greeting")
        .expect("`format-greeting` binding must be present");
    assert_eq!(
        helper.attributes.get(ATTR_PRIVATE),
        Some(&YamlValue::Bool(true)),
        "`format-greeting` is not exported and must have private: true, \
         got attrs: {:?}",
        helper.attributes
    );
    // Visibility is still Conventional (no Rust-style `pub`/`priv`).
    assert!(matches!(helper.visibility, Visibility::Conventional));
}

// ---------------------------------------------------------------------------
// Acceptance criterion 3: library identifier → module_path
// ---------------------------------------------------------------------------

#[test]
fn library_identifier_becomes_module_path() {
    let inputs = simple_lib_inputs();
    let out = extract_lispkit_surface("simple-lib", &inputs);

    for b in &out.bindings {
        assert_eq!(
            b.module_path,
            vec!["simple-lib".to_string(), "core".to_string()],
            "module_path should reflect the (define-library (simple-lib core) ...) identifier; \
             got {:?} for symbol `{}`",
            b.module_path,
            b.symbol
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance criterion 4: language is always "scheme"
// ---------------------------------------------------------------------------

#[test]
fn all_bindings_have_scheme_language() {
    let inputs = simple_lib_inputs();
    let out = extract_lispkit_surface("simple-lib", &inputs);
    assert!(!out.bindings.is_empty(), "fixture must produce bindings");
    for b in &out.bindings {
        assert_eq!(
            b.language, "scheme",
            "expected language=scheme, got `{}`",
            b.language
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance criterion 5: library_apis contains only exported symbols
// ---------------------------------------------------------------------------

#[test]
fn library_api_contains_only_exported_symbols() {
    let inputs = simple_lib_inputs();
    let out = extract_lispkit_surface("simple-lib", &inputs);

    assert_eq!(out.library_apis.len(), 1, "exactly one LibraryApi expected");
    let api = &out.library_apis[0];
    assert_eq!(api.id, "simple-lib/public-api");
    assert_eq!(api.language, "scheme");

    let pub_names: Vec<&str> = api.pub_items.iter().map(|p| p.name.as_str()).collect();
    assert!(
        pub_names.contains(&"greet"),
        "pub_items must contain `greet`; got {pub_names:?}"
    );
    assert!(
        pub_names.contains(&"farewell"),
        "pub_items must contain `farewell`; got {pub_names:?}"
    );
    assert!(
        !pub_names.contains(&"format-greeting"),
        "pub_items must NOT contain `format-greeting` (private); got {pub_names:?}"
    );
}

// ---------------------------------------------------------------------------
// Component classified as lispkit-package
// ---------------------------------------------------------------------------

#[test]
fn fixture_classified_as_lispkit_package_by_classifier() {
    use atlas_analyzers::{
        AnalysisContext, Analyzer, AnalyzerResult, LispKitClassifier, TargetFile,
    };
    use std::collections::BTreeSet;

    let fixture_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple-lib/core.sld");
    let bytes = std::fs::read(&fixture_path).unwrap();
    let classifier = LispKitClassifier::new();
    let target = atlas_analyzers::Target {
        dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple-lib"),
        languages: BTreeSet::new(),
        manifests: vec![TargetFile {
            name: "core.sld".into(),
            relpath: PathBuf::from("core.sld"),
            content_sha: "test-sha".into(),
            bytes,
        }],
        top_level_files: vec!["core.sld".into()],
    };
    assert!(
        classifier.applies(&target),
        "classifier must apply to *.sld target"
    );
    let ctx = AnalysisContext::deterministic_only();
    match classifier.analyse(&ctx, &target) {
        AnalyzerResult::Confident(out) => {
            let lk = out
                .as_any()
                .downcast_ref::<atlas_analyzers::LispKitClassificationOutput>()
                .expect("output is LispKitClassificationOutput");
            assert_eq!(lk.kind, "lispkit-package");
            assert_eq!(lk.language, "scheme");
        }
        other => panic!("expected Confident, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Constants stability
// ---------------------------------------------------------------------------

#[test]
fn analyzer_constants_are_stable() {
    assert_eq!(ANALYZER_ID, "lispkit-surface-analyzer");
    assert_eq!(ANALYZER_VERSION, "1.0.0");
}
