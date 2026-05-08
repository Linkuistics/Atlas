//! PR-7 acceptance test: `atlas index` emits a per-component
//! `surfaces.yaml` for every Rust component, populating
//! `contracts_defined`, `contracts_implemented`, `library_apis`, and
//! the aggregate `fingerprint`.
//!
//! ## What this test asserts
//!
//! 1. **Schema version.** Each emitted `surfaces.yaml` reads as
//!    `schema_version: 1` (per [`atlas_index::SURFACES_SCHEMA_VERSION`]).
//! 2. **Code-derived contract.** A multi-crate fixture where
//!    `crate-a/src/lib.rs` defines `pub struct Foo {...}` with
//!    `#[derive(Serialize, Deserialize)]` produces a `data-format`
//!    contract whose `definition_binding.span` matches the byte
//!    positions of the `pub struct Foo` definition (start-of-`pub`
//!    to byte-after-`}`).
//! 3. **Aggregate fingerprint.** The top-level `fingerprint` field is
//!    populated (non-empty) and stable across no-op re-runs.
//! 4. **Cross-whitespace stability.** Whitespace edits *outside* the
//!    binding span (a doc-comment moved or rephrased) leave the
//!    binding `content_sha` unchanged — the spec §2.1 invariant
//!    other components' caches rely on.
//! 5. **Library API.** Every Rust component emits a `library-api`
//!    entry under `library_apis` listing its top-level `pub` items.

use std::path::Path;
use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_index::{ContractKind, PubItemKind, SurfacesFile, SURFACES_SCHEMA_VERSION};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [13u8; 32],
        ontology_sha: [17u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-pr7".into(),
    }
}

/// Returns canned defaults so the pipeline can complete without the
/// surface emission depending on a real LLM. Mirrors
/// `scattered_atlas_layout::LenientBackend` in shape.
struct LenientBackend {
    fingerprint: LlmFingerprint,
    call_log: Mutex<Vec<PromptId>>,
}

impl LenientBackend {
    fn new() -> Arc<Self> {
        Arc::new(LenientBackend {
            fingerprint: fingerprint(),
            call_log: Mutex::new(Vec::new()),
        })
    }
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call_log.lock().unwrap().push(req.prompt_template);
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "default lenient",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({"purpose": "stub", "notes": ""}),
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

fn write_lib_with_serde_struct(root: &Path, name: &str, lib_body: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n\n[dependencies]\nserde = {{ version = \"1\", features = [\"derive\"] }}\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), lib_body).unwrap();
}

#[test]
fn rust_component_surfaces_yaml_carries_data_format_contract_with_correct_span() {
    let tmp = TempDir::new().unwrap();
    let lib_body = "use serde::{Serialize, Deserialize};\n\n\
                    #[derive(Serialize, Deserialize)]\n\
                    pub struct Foo { pub a: u32 }\n\n\
                    pub fn helper() {}\n";
    write_lib_with_serde_struct(tmp.path(), "alpha", lib_body);

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();

    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");
    assert!(summary.outputs_written, "outputs must be written");

    // -- The component-side cache/surfaces.yaml (PR-2) -----------------
    let surfaces_path = tmp.path().join("alpha/.atlas/cache/surfaces.yaml");
    assert!(
        surfaces_path.exists(),
        "expected cache/surfaces.yaml at {}",
        surfaces_path.display()
    );

    let bytes = std::fs::read(&surfaces_path).unwrap();
    let parsed: SurfacesFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as SurfacesFile: {e}",
            surfaces_path.display()
        )
    });
    assert_eq!(parsed.schema_version, SURFACES_SCHEMA_VERSION);
    assert_eq!(parsed.schema_version, 1);
    assert!(
        !parsed.fingerprint.is_empty(),
        "aggregate fingerprint must be populated"
    );
    assert_eq!(parsed.fingerprint.len(), 64);

    // -- The data-format contract for `Foo` ----------------------------
    let foo_contract = parsed
        .contracts_defined
        .iter()
        .find(|c| c.definition_binding.symbol == "Foo")
        .expect("expected a `Foo` contract in contracts_defined");
    assert_eq!(foo_contract.kind, ContractKind::DataFormat);
    assert_eq!(foo_contract.definition_binding.language, "rust");
    assert_eq!(
        foo_contract.definition_binding.file.to_string_lossy(),
        "src/lib.rs"
    );
    // Span points at the bytes covered by `pub struct Foo { pub a: u32 }`.
    let expected_pub_start = lib_body.find("pub struct Foo").unwrap();
    // The struct's body ends at the closing brace of the struct
    // declaration, not the next pub helper. Find the FIRST `}` after
    // the struct-body opening `{`.
    let after_open_brace = lib_body[expected_pub_start..].find('{').unwrap();
    let close_brace_offset = lib_body[expected_pub_start + after_open_brace..]
        .find('}')
        .unwrap();
    let expected_after_close_brace = expected_pub_start + after_open_brace + close_brace_offset + 1;
    assert_eq!(
        foo_contract.definition_binding.span,
        (expected_pub_start, expected_after_close_brace),
        "binding span must cover `pub struct Foo {{ pub a: u32 }}`"
    );

    // The contract id is namespaced under the component id with the
    // struct name kebab-cased.
    assert!(
        foo_contract.id.ends_with("/foo"),
        "contract id must end with the kebab struct name; got `{}`",
        foo_contract.id
    );

    // -- contracts_implemented mirrors contracts_defined ---------------
    let implemented_for_foo = parsed
        .contracts_implemented
        .iter()
        .find(|i| i.contract_id == foo_contract.id)
        .expect("contracts_implemented must list the defining-binding");
    assert_eq!(
        implemented_for_foo.role,
        atlas_index::BindingRole::DefiningBinding
    );
    assert_eq!(implemented_for_foo.binding.symbol, "Foo");

    // -- contracts_consumed is empty in PR-7 ---------------------------
    assert!(parsed.contracts_consumed.is_empty());

    // -- library_apis carries every pub item ---------------------------
    assert_eq!(parsed.library_apis.len(), 1);
    let api = &parsed.library_apis[0];
    assert_eq!(api.kind, ContractKind::LibraryApi);
    assert_eq!(api.language, "rust");
    let names: Vec<&str> = api.pub_items.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Foo"), "got pub_items: {names:?}");
    assert!(names.contains(&"helper"), "got pub_items: {names:?}");
    let foo_kind = api
        .pub_items
        .iter()
        .find(|p| p.name == "Foo")
        .map(|p| p.kind);
    assert_eq!(foo_kind, Some(PubItemKind::Struct));
    let helper_kind = api
        .pub_items
        .iter()
        .find(|p| p.name == "helper")
        .map(|p| p.kind);
    assert_eq!(helper_kind, Some(PubItemKind::Fn));
}

#[test]
fn whitespace_outside_span_does_not_change_binding_content_sha() {
    // Spec §2.1: whitespace **outside** the binding span does not affect
    // the binding sha (whitespace **inside** the span does — proven in
    // unit tests in the analyser crate). This integration test proves
    // the invariant survives a full `atlas index` run, which is the
    // load-bearing surface for PR-11's cross-component cache key.
    fn run_once(source: &str) -> String {
        let tmp = TempDir::new().unwrap();
        write_lib_with_serde_struct(tmp.path(), "alpha", source);
        let mut config = IndexConfig::new(tmp.path().to_path_buf());
        config.respect_gitignore = false;
        config.fingerprint_override = Some(fingerprint());
        let backend = LenientBackend::new();
        run_index(
            &config,
            backend,
            None,
            make_stderr_reporter(ProgressMode::Never, None),
        )
        .expect("run_index succeeds");

        let surfaces: SurfacesFile = serde_yaml::from_slice(
            &std::fs::read(tmp.path().join("alpha/.atlas/cache/surfaces.yaml")).unwrap(),
        )
        .unwrap();
        let foo = surfaces
            .contracts_defined
            .iter()
            .find(|c| c.definition_binding.symbol == "Foo")
            .expect("Foo contract must be present");
        foo.definition_binding.content_sha.clone()
    }

    let body_a = "use serde::{Serialize, Deserialize};\n\n\
                  #[derive(Serialize, Deserialize)]\n\
                  pub struct Foo { pub a: u32 }\n";
    let body_b = "use serde::{Serialize, Deserialize};\n\n\
                  /// A doc comment that did not exist before.\n\
                  /// Multiple lines, even.\n\
                  #[derive(Serialize, Deserialize)]\n\
                  pub struct Foo { pub a: u32 }\n";
    let sha_a = run_once(body_a);
    let sha_b = run_once(body_b);
    assert_eq!(
        sha_a, sha_b,
        "whitespace / doc-comment changes outside the binding span must not change the content sha (spec §2.1)"
    );
}

#[test]
fn whitespace_inside_span_changes_binding_content_sha() {
    // Mirror of the above — whitespace **inside** the span does change
    // the sha. Together they pin both directions of spec §2.1.
    fn run_once(source: &str) -> String {
        let tmp = TempDir::new().unwrap();
        write_lib_with_serde_struct(tmp.path(), "alpha", source);
        let mut config = IndexConfig::new(tmp.path().to_path_buf());
        config.respect_gitignore = false;
        config.fingerprint_override = Some(fingerprint());
        let backend = LenientBackend::new();
        run_index(
            &config,
            backend,
            None,
            make_stderr_reporter(ProgressMode::Never, None),
        )
        .expect("run_index succeeds");
        let surfaces: SurfacesFile = serde_yaml::from_slice(
            &std::fs::read(tmp.path().join("alpha/.atlas/cache/surfaces.yaml")).unwrap(),
        )
        .unwrap();
        surfaces
            .contracts_defined
            .iter()
            .find(|c| c.definition_binding.symbol == "Foo")
            .expect("Foo contract must be present")
            .definition_binding
            .content_sha
            .clone()
    }

    let body_a = "use serde::{Serialize, Deserialize};\n\n\
                  #[derive(Serialize, Deserialize)]\n\
                  pub struct Foo { pub a: u32 }\n";
    // Extra space inside the struct body — same logical structure,
    // different bytes inside the span.
    let body_b = "use serde::{Serialize, Deserialize};\n\n\
                  #[derive(Serialize, Deserialize)]\n\
                  pub struct Foo {  pub a: u32 }\n";
    let sha_a = run_once(body_a);
    let sha_b = run_once(body_b);
    assert_ne!(
        sha_a, sha_b,
        "whitespace inside the binding span must change the content sha (spec §2.1)"
    );
}

#[test]
fn surfaces_yaml_aggregate_fingerprint_is_stable_across_no_op_reruns() {
    let tmp = TempDir::new().unwrap();
    let lib_body = "use serde::{Serialize, Deserialize};\n\
                    #[derive(Serialize, Deserialize)]\n\
                    pub struct Foo { pub a: u32 }\n";
    write_lib_with_serde_struct(tmp.path(), "alpha", lib_body);

    fn run_and_read(root: &Path) -> SurfacesFile {
        let mut config = IndexConfig::new(root.to_path_buf());
        config.respect_gitignore = false;
        config.fingerprint_override = Some(fingerprint());
        let backend = LenientBackend::new();
        run_index(
            &config,
            backend,
            None,
            make_stderr_reporter(ProgressMode::Never, None),
        )
        .expect("run_index succeeds");
        serde_yaml::from_slice(
            &std::fs::read(root.join("alpha/.atlas/cache/surfaces.yaml")).unwrap(),
        )
        .unwrap()
    }

    let first = run_and_read(tmp.path());
    let second = run_and_read(tmp.path());
    assert_eq!(
        first.fingerprint, second.fingerprint,
        "no-op re-run must leave the surfaces.yaml fingerprint unchanged"
    );
    assert!(!first.fingerprint.is_empty());
}

#[test]
fn surfaces_yaml_emitted_for_every_live_component_in_multi_crate_fixture() {
    let tmp = TempDir::new().unwrap();
    let body = "pub fn alpha() {}\n";
    write_lib_with_serde_struct(tmp.path(), "alpha", body);
    write_lib_with_serde_struct(tmp.path(), "beta", "pub fn beta() {}\n");
    write_lib_with_serde_struct(
        tmp.path(),
        "gamma",
        "use serde::{Serialize, Deserialize};\n#[derive(Serialize, Deserialize)]\npub struct Gamma { x: u32 }\n",
    );

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new();
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds");

    for name in ["alpha", "beta", "gamma"] {
        let path = tmp.path().join(name).join(".atlas/cache/surfaces.yaml");
        assert!(
            path.exists(),
            "expected cache/surfaces.yaml for `{name}` at {}",
            path.display()
        );
        let parsed: SurfacesFile = serde_yaml::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed.schema_version, 1);
    }

    // Only `gamma` declared a serde-derived struct — only its
    // cache/surfaces.yaml carries a contract.
    let gamma: SurfacesFile = serde_yaml::from_slice(
        &std::fs::read(tmp.path().join("gamma/.atlas/cache/surfaces.yaml")).unwrap(),
    )
    .unwrap();
    assert!(
        gamma
            .contracts_defined
            .iter()
            .any(|c| c.definition_binding.symbol == "Gamma"),
        "gamma must define a Gamma contract"
    );
    let alpha: SurfacesFile = serde_yaml::from_slice(
        &std::fs::read(tmp.path().join("alpha/.atlas/cache/surfaces.yaml")).unwrap(),
    )
    .unwrap();
    assert!(
        alpha.contracts_defined.is_empty(),
        "alpha defines no serde-derived contracts"
    );
}

#[test]
fn dry_run_skips_surfaces_yaml_writes() {
    let tmp = TempDir::new().unwrap();
    write_lib_with_serde_struct(tmp.path(), "dry", "pub fn d() {}\n");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config.dry_run = true;
    let backend = LenientBackend::new();
    let summary = run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("dry-run run_index succeeds");
    assert!(!summary.outputs_written);
    let path = tmp.path().join("dry/.atlas/cache/surfaces.yaml");
    assert!(
        !path.exists(),
        "dry-run must not write cache/surfaces.yaml; found {}",
        path.display()
    );
}
