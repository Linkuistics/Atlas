//! WI-3 acceptance: the deterministic Cargo classifier is retired; the
//! LLM-spine agent classifies Cargo components by reading the manifest
//! (via `parse_cargo_toml`) plus a source entry-point, instead of
//! dispatching to a deterministic `classify_cargo_component` tool.
//!
//! Three tests, mapping 1:1 to the WI-3 deliverables:
//!
//!   1. Catalog absence — `default_tool_catalog` no longer contains
//!      `CargoClassifyTool`; total count drops from 22 to 21;
//!      `parse_cargo_toml` (the manifest parser, not the classifier)
//!      stays.
//!   2. Rust-library worked example — the classify prompt's worked
//!      YAML example continues to demonstrate `kind: "rust-library"`,
//!      and the rewritten `confidence_grade` rubric rewards
//!      `parse_cargo_toml` + source entry-point READ rather than a
//!      deterministic "classifier tool whose name matches the
//!      declared kind" call.
//!   3. Rust-workspace vocabulary — the prompt's canonical-vocabulary
//!      list includes `rust-workspace`, distinguishing a `Cargo.toml`
//!      with `[workspace]` (no `[lib]`/`[bin]`) from `rust-library` /
//!      `rust-binary`.

use std::path::Path;

use atlas_agents::default_tool_catalog;
use atlas_agents::runtime::build_classify_prompt;
use atlas_agents::runtime::dispatch::{ComponentFieldOverrides, ComponentPartition};

fn synthetic_workspace_root() -> &'static Path {
    Path::new("/tmp/synthetic-workspace")
}

fn synthetic_component() -> ComponentPartition {
    ComponentPartition {
        id: "mylib".to_string(),
        subsystem_id: "mylib_subsystem".to_string(),
        field_overrides: ComponentFieldOverrides::default(),
    }
}

#[test]
fn default_tool_catalog_excludes_cargo_classifier() {
    let catalog = default_tool_catalog();
    let ids: Vec<&str> = catalog.iter().map(|t| t.id()).collect();

    assert!(
        !ids.contains(&"classify_cargo_component"),
        "Cargo classifier tool must not be in the default catalog after WI-3; \
         found ids: {ids:?}"
    );
    // Sanity: parse_cargo_toml stays — it's the LLM's tool for reading
    // Cargo manifests, only the deterministic classifier is retired.
    assert!(
        ids.contains(&"parse_cargo_toml"),
        "parse_cargo_toml must remain in the catalog (manifest parser, \
         not classifier); found ids: {ids:?}"
    );
    assert_eq!(
        catalog.iter().count(),
        21,
        "default catalog should drop from 22 to 21 tools after WI-3; \
         found ids: {ids:?}"
    );
}

#[test]
fn classify_prompt_keeps_rust_library_as_worked_example() {
    // The plan keeps the rust-library worked YAML example as the
    // shape illustration (rust-workspace is added to the vocabulary
    // list, not as a second worked example). Verify the example still
    // exemplifies rust-library, AND that the rewritten rubric rewards
    // the parser-tool path rather than a deterministic classifier call.
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 6, 12);

    assert!(
        prompt.contains(r#"kind: "rust-library""#),
        "classify prompt's worked YAML example must continue to demonstrate \
         `kind: \"rust-library\"` (the shape illustration). Prompt body:\n{prompt}"
    );

    // New rubric: "strong" requires the parser tool + source-read.
    // Mention the parser tool by name so the test is anchored to the
    // post-WI-3 rubric wording.
    assert!(
        prompt.contains("parse_cargo_toml"),
        "rewritten rubric/available-tools section must name `parse_cargo_toml` \
         (the parser tool the LLM should call for Rust components). Prompt body:\n{prompt}"
    );

    // Old rubric language: the rubric used to reward "the classifier
    // tool whose name matches the declared `kind` was CALLED". That
    // wording is the deterministic-dispatch reward and must be gone.
    assert!(
        !prompt.contains("classifier tool whose name matches"),
        "rewritten rubric must NOT carry the legacy `classifier tool whose \
         name matches the declared kind` reward (that's the deterministic-\
         dispatch framing WI-3 retires). Prompt body:\n{prompt}"
    );
}

#[test]
fn classify_prompt_adds_rust_workspace_to_vocabulary() {
    // A `Cargo.toml` with `[workspace]` and no `[lib]`/`[bin]` is a
    // distinct shape from rust-library / rust-binary; WI-3 adds
    // `rust-workspace` to the canonical-vocabulary list so the LLM
    // has a kebab-case term to emit for that shape.
    let prompt = build_classify_prompt(synthetic_workspace_root(), &synthetic_component(), 6, 12);

    assert!(
        prompt.contains("rust-workspace"),
        "classify prompt's canonical-vocabulary list must include \
         `rust-workspace` after WI-3. Prompt body:\n{prompt}"
    );

    // The vocabulary clarifier should reference the `[workspace]` table
    // shape so the LLM can disambiguate from rust-library / rust-binary.
    assert!(
        prompt.contains("[workspace]"),
        "classify prompt must clarify that `rust-workspace` applies to a \
         `Cargo.toml` with a `[workspace]` table. Prompt body:\n{prompt}"
    );
}
