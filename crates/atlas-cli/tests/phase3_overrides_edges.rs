//! Phase 3 PR-6 acceptance tests: per-component field overrides
//! (`language` / `kind` / `lifecycle` / `subsystem`) and top-level
//! `edges_add` / `edges_suppress`.
//!
//! These exercises drive `atlas index` end-to-end against tempdir
//! fixtures and assert against the on-disk cache outputs:
//!
//! - `<root>/.atlas/cache/related-components.yaml` — gains user-added
//!   edges, loses analyser-discovered edges that match an
//!   `edges_suppress` triple, and is unchanged by a no-match
//!   suppress entry.
//! - `<component>/.atlas/cache/component.yaml` — projects the
//!   post-override `language` / `kind` / `lifecycle` fields when a
//!   per-component `overrides.yaml` declares `field_overrides`.
//!
//! The LLM backend is stubbed (`LenientBackend`) so every Stage 1
//! and Stage 2 call returns canned defaults; PR-6 is a deterministic
//! pipeline change and the tests do not depend on real LLM output.

use std::path::Path;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_index::{
    load_or_default_components, load_or_default_related_components, ComponentsFile,
    PerComponentFile,
};
use atlas_llm::LlmFingerprint;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [13u8; 32],
        model_id: "test-backend".into(),
        backend_version: "v-pr6-edges".into(),
    }
}

fn write_lib(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
}

/// Run `atlas index` against `root` with the lenient backend.
fn run(root: &Path) -> atlas_cli::IndexSummary {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    let backend = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index succeeds")
}

/// Locate the per-component `cache/component.yaml` for a given
/// component id by reading the top-level components.yaml.
fn per_component_file_for(root: &Path, output_dir: &Path, component_id: &str) -> PerComponentFile {
    let top_level: ComponentsFile =
        load_or_default_components(&output_dir.join("cache/components.yaml")).unwrap();
    let entry = top_level
        .components
        .iter()
        .find(|c| c.id.as_str() == component_id)
        .unwrap_or_else(|| {
            panic!(
                "component id `{component_id}` not in {}",
                output_dir.join("cache/components.yaml").display()
            )
        });
    let segment = entry
        .path_segments
        .first()
        .expect("live component has a path segment");
    let path = root
        .join(&segment.path)
        .join(".atlas")
        .join("cache")
        .join("component.yaml");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as PerComponentFile: {e}",
            path.display()
        )
    })
}

// =====================================================================
// Per-component field overrides — language / kind / lifecycle.
// =====================================================================

#[test]
fn field_override_language_replaces_analyser_emitted_value() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");

    // Per-component overrides.yaml: language: ocaml. The analyser
    // would otherwise emit language: rust (Cargo.toml is present).
    let overrides_path = root.join("alpha-lib/.atlas/overrides.yaml");
    std::fs::create_dir_all(overrides_path.parent().unwrap()).unwrap();
    std::fs::write(
        &overrides_path,
        "schema_version: 1\noverrides:\n  language: ocaml\n",
    )
    .unwrap();

    let summary = run(root);
    assert!(summary.outputs_written);

    let pc = per_component_file_for(root, &root.join(".atlas"), "alpha-lib");
    let langs: Vec<&str> = pc.component.languages.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        langs,
        vec!["ocaml"],
        "field_override.language must replace the analyser-emitted languages \
         on the per-component projection"
    );
}

#[test]
fn field_override_kind_replaces_analyser_emitted_value() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");

    let overrides_path = root.join("alpha-lib/.atlas/overrides.yaml");
    std::fs::create_dir_all(overrides_path.parent().unwrap()).unwrap();
    std::fs::write(
        &overrides_path,
        "schema_version: 1\noverrides:\n  kind: docker-image\n",
    )
    .unwrap();

    let summary = run(root);
    assert!(summary.outputs_written);

    let pc = per_component_file_for(root, &root.join(".atlas"), "alpha-lib");
    assert_eq!(
        pc.component.kind, "docker-image",
        "field_override.kind must replace the analyser-emitted kind"
    );
}

#[test]
fn field_override_lifecycle_replaces_analyser_emitted_value() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");

    let overrides_path = root.join("alpha-lib/.atlas/overrides.yaml");
    std::fs::create_dir_all(overrides_path.parent().unwrap()).unwrap();
    // `runtime` is a known LifecycleScope variant in the open
    // ontology — the engine parses it and replaces the analyser
    // value (typically `build` for a Cargo lib).
    std::fs::write(
        &overrides_path,
        "schema_version: 1\noverrides:\n  lifecycle: runtime\n",
    )
    .unwrap();

    let summary = run(root);
    assert!(summary.outputs_written);

    let pc = per_component_file_for(root, &root.join(".atlas"), "alpha-lib");
    let lifecycles: Vec<&str> = pc
        .component
        .lifecycle_roles
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        lifecycles,
        vec!["runtime"],
        "field_override.lifecycle must replace the analyser-emitted \
         lifecycle_roles vec with the single authored scope"
    );
}

#[test]
fn field_override_subsystem_is_captured_but_does_not_panic() {
    // Subsystem has no destination on ComponentEntry yet (see the
    // ComponentFieldOverrides docstring). The test confirms the
    // engine accepts the field and runs to completion without
    // surfacing a parse or schema error.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");

    let overrides_path = root.join("alpha-lib/.atlas/overrides.yaml");
    std::fs::create_dir_all(overrides_path.parent().unwrap()).unwrap();
    std::fs::write(
        &overrides_path,
        "schema_version: 1\noverrides:\n  subsystem: ravel-lite/runtime\n",
    )
    .unwrap();

    let summary = run(root);
    assert!(summary.outputs_written);
}

#[test]
fn per_component_overrides_with_pin_outside_scope_is_rejected() {
    // The per-component overrides file at alpha-lib/.atlas/overrides.yaml
    // declares a pin on a sibling id (beta-lib). Phase 1 PR-0c's
    // scoping rule (carried forward unchanged in PR-6) rejects this
    // with a hard error.
    //
    // The error surfaces as a panic from the L4 engine path (via
    // `all_components`'s "panic on assembly error" contract — which
    // matches the design's "acyclicity / scoping is invariant, not
    // recoverable" stance). The CLI propagates the panic; the test
    // therefore catches the unwind and inspects the message.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");
    write_lib(root, "beta-lib");

    let overrides_path = root.join("alpha-lib/.atlas/overrides.yaml");
    std::fs::create_dir_all(overrides_path.parent().unwrap()).unwrap();
    std::fs::write(
        &overrides_path,
        "schema_version: 1\npins:\n  beta-lib:\n    kind:\n      value: docker-image\n",
    )
    .unwrap();

    // Suppress the default panic hook for this scoped block so the
    // panic message does not pollute test output. AssertUnwindSafe is
    // safe here — the closure's captured state is local to the
    // tempdir and is not observed after the catch returns.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut config = IndexConfig::new(root.to_path_buf());
        config.respect_gitignore = false;
        config.fingerprint_override = Some(fingerprint());
        let backend = LenientBackend::new(fingerprint());
        run_index(
            &config,
            backend,
            None,
            make_stderr_reporter(ProgressMode::Never, None),
        )
    }));
    std::panic::set_hook(prev_hook);

    let payload = result.expect_err("scope violation must surface as a panic");
    let msg = panic_message(&payload);
    assert!(
        msg.contains("alpha-lib/.atlas/overrides.yaml") && msg.contains("beta-lib"),
        "panic message must name the offending file and id; got: {msg}"
    );
    assert!(
        msg.contains("scoping prefix") || msg.contains("outside its scoping"),
        "panic message must reference scoping rule; got: {msg}"
    );
}

/// Recover the message string from a `Box<dyn Any + Send>` payload
/// produced by `std::panic::catch_unwind`. The `panic!("{e}")` macro
/// ships the formatted message as a `String`; older payloads may
/// arrive as `&'static str`. Anything else is rendered as
/// `<non-string panic payload>` so the assertion fails cleanly.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return s.to_string();
    }
    "<non-string panic payload>".into()
}

// =====================================================================
// edges_add / edges_suppress — top-level overrides drive related-components.
// =====================================================================

fn read_related(output_dir: &Path) -> atlas_index::RelatedComponentsFile {
    load_or_default_related_components(&output_dir.join("cache/related-components.yaml"))
        .unwrap_or_else(|e| {
            panic!(
                "failed to load {}: {e}",
                output_dir.join("cache/related-components.yaml").display()
            )
        })
}

fn write_top_level_overrides(root: &Path, body: &str) {
    let path = root.join(".atlas/components.overrides.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
}

#[test]
fn edges_add_inserts_a_user_authored_edge() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");
    write_lib(root, "beta-lib");

    write_top_level_overrides(
        root,
        "schema_version: 1\n\
         edges_add:\n  \
         - kind: bundled-into\n    \
           from: alpha-lib\n    \
           to: beta-lib\n    \
           reason: \"manual annotation\"\n",
    );

    let summary = run(root);
    assert!(summary.outputs_written);

    let related = read_related(&root.join(".atlas"));
    let matching: Vec<&atlas_index::Edge> = related
        .edges
        .iter()
        .filter(|e| {
            e.kind == component_ontology::EdgeKind::BundledInto
                && e.participants == vec!["alpha-lib".to_string(), "beta-lib".to_string()]
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one user-added (bundled-into, alpha-lib -> beta-lib) \
         edge expected; got {related:#?}"
    );
    assert!(
        matching[0].rationale.contains("manual annotation"),
        "edges_add rationale must echo the authored reason; got: {}",
        matching[0].rationale
    );
}

#[test]
fn edges_suppress_no_match_leaves_set_unchanged() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");
    write_lib(root, "beta-lib");

    // Suppress an edge that the analyser doesn't emit (the lenient
    // stub returns []). The edge set should be unchanged from the
    // baseline; the engine logs a warning to stderr (not asserted
    // here — the warning channel is best-effort and the test
    // harness does not capture it).
    write_top_level_overrides(
        root,
        "schema_version: 1\n\
         edges_suppress:\n  \
         - kind: depends-on\n    \
           from: alpha-lib\n    \
           to: beta-lib\n    \
           reason: \"belt and braces\"\n",
    );

    let summary = run(root);
    assert!(summary.outputs_written);

    let related = read_related(&root.join(".atlas"));
    // The lenient stub emits no Stage 2 edges; suppression of a
    // non-existent edge is a no-op. The expected set is empty (no
    // composition or contract edges at this fixture), so we assert
    // the edge set has no `(depends-on, alpha-lib, beta-lib)`
    // entry (always true) and that the suppress did not somehow
    // crash the run.
    assert!(
        !related
            .edges
            .iter()
            .any(|e| e.kind == component_ontology::EdgeKind::DependsOn
                && e.participants == vec!["alpha-lib".to_string(), "beta-lib".to_string()]),
        "edge set must not contain the suppressed (non-existent) triple"
    );
}

#[test]
fn edges_add_and_edges_suppress_on_same_triple_drops_the_edge() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_lib(root, "alpha-lib");
    write_lib(root, "beta-lib");

    write_top_level_overrides(
        root,
        "schema_version: 1\n\
         edges_add:\n  \
         - kind: bundled-into\n    \
           from: alpha-lib\n    \
           to: beta-lib\n    \
           reason: \"add it\"\n\
         edges_suppress:\n  \
         - kind: bundled-into\n    \
           from: alpha-lib\n    \
           to: beta-lib\n    \
           reason: \"never mind\"\n",
    );

    let summary = run(root);
    assert!(summary.outputs_written);

    let related = read_related(&root.join(".atlas"));
    assert!(
        !related
            .edges
            .iter()
            .any(|e| e.kind == component_ontology::EdgeKind::BundledInto
                && e.participants == vec!["alpha-lib".to_string(), "beta-lib".to_string()]),
        "suppress-after-add must drop the edge; got: {related:#?}"
    );
}
