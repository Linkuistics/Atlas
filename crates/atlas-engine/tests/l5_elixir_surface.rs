//! L5 surface-extraction integration tests for the Elixir subprocess
//! analyser (Atlas vNext Phase 2 PR-8).
//!
//! These tests drive the Elixir classification and surface extraction
//! end-to-end. Three acceptance criteria are verified:
//!
//! 1. A `mix.exs` + `lib/foo.ex` fixture is classified `elixir-project`
//!    at L3 with no LLM call.
//! 2. `def foo/0` emits one binding; `defp bar/0` is excluded.
//! 3. `defprotocol Stringable` emits a `Contract` with `kind: behaviour`.
//!
//! Tests that drive the subprocess binary skip themselves when the
//! `elixir-analyzer` binary cannot be located in the cargo target
//! directory — defensive against environments outside a workspace
//! build. `cargo test --workspace` always builds the binary so the
//! skip path is dead code in practice.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::testing::LenientBackend;
use atlas_engine::{
    all_components, seed_filesystem, surface_artefacts_of, AtlasDatabase, ComponentKind,
};
use atlas_llm::{LlmBackend, LlmFingerprint};
use serde_json::json;
use tempfile::TempDir;

// ─── helpers ──────────────────────────────────────────────────────────────────

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
        "kind": "elixir-project",
        "language": "elixir",
        "evidence_grade": "strong",
        "evidence_fields": [],
        "rationale": "stub",
        "is_boundary": true,
    })
}

fn build_db(root: &Path) -> AtlasDatabase {
    let backend: Arc<dyn LlmBackend> =
        LenientBackend::with_classify(default_fingerprint(), lenient_classify());
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
}

fn skip_if_binary_missing() -> bool {
    if atlas_analyzers::locate_elixir_analyzer_binary().is_none() {
        eprintln!("skipping: elixir-analyzer binary not located in target/");
        return true;
    }
    false
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// Write a minimal Elixir project fixture at `dir`.
///
/// Layout:
/// ```
/// mix.exs
/// lib/<module_name>.ex   — contains `def foo` (public) and `defp bar` (private)
/// ```
fn write_elixir_fixture(dir: &Path, app_name: &str, module_name: &str) {
    write(
        &dir.join("mix.exs"),
        &format!(
            "defmodule {module_name}.MixProject do\n  use Mix.Project\n\n  def project do\n    [\n      app: :{app_name},\n      version: \"0.1.0\"\n    ]\n  end\nend\n"
        ),
    );
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    write(
        &dir.join(format!("lib/{}.ex", app_name)),
        &format!("defmodule {module_name} do\n  def foo, do: :ok\n  defp bar, do: :hidden\nend\n"),
    );
}

// ─── acceptance criterion 1 ───────────────────────────────────────────────────

#[test]
fn elixir_mix_project_classifies_as_elixir_project_no_llm() {
    // PR-8 acceptance criterion 1: a `mix.exs` + `lib/foo.ex` fixture
    // is classified `elixir-project` at L3 deterministically (no LLM
    // Classify call required).
    let td = TempDir::new().unwrap();
    write_elixir_fixture(td.path(), "my_app", "MyApp");

    let db = build_db(td.path());
    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce a live component");

    assert_eq!(
        comp.kind,
        ComponentKind::ElixirProject.as_str(),
        "mix.exs fixture must classify as elixir-project, got {}",
        comp.kind
    );
}

// ─── acceptance criterion 2 ───────────────────────────────────────────────────

#[test]
fn elixir_def_emits_binding_defp_is_excluded() {
    // PR-8 acceptance criterion 2: `def foo/0` emits one public binding;
    // `defp bar/0` is excluded from the surfaces output.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    write_elixir_fixture(td.path(), "my_app", "MyApp");

    let db = build_db(td.path());
    let comp_id = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce a live component")
        .id
        .clone();

    let artefacts = surface_artefacts_of(&db, comp_id);
    let symbols: Vec<&str> = artefacts
        .bindings
        .iter()
        .map(|b| b.symbol.as_str())
        .collect();

    assert!(
        symbols.contains(&"foo"),
        "expected `foo` in bindings (def foo is public); got {symbols:?}"
    );
    assert!(
        !symbols.contains(&"bar"),
        "expected `bar` to be absent (defp bar is private); got {symbols:?}"
    );
}

// ─── acceptance criterion 3 ───────────────────────────────────────────────────

#[test]
fn elixir_defprotocol_emits_contract_with_behaviour_kind() {
    // PR-8 acceptance criterion 3: a `defprotocol Stringable do ... end`
    // produces a Contract with `kind == ContractKind::Behaviour`.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();

    write(
        &td.path().join("mix.exs"),
        "defmodule MyApp.MixProject do\n  use Mix.Project\n\n  def project do\n    [app: :my_app, version: \"0.1.0\"]\n  end\nend\n",
    );
    std::fs::create_dir_all(td.path().join("lib")).unwrap();
    write(
        &td.path().join("lib/stringable.ex"),
        "defprotocol Stringable do\n  @doc \"Convert t to a string.\"\n  @callback to_string(t) :: String.t()\nend\n",
    );

    let db = build_db(td.path());
    let comp_id = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce a live component")
        .id
        .clone();

    let artefacts = surface_artefacts_of(&db, comp_id);

    // The surfaces artefact's contracts list must contain one entry
    // for the Stringable protocol with kind "behaviour".
    let behaviour_contract = artefacts
        .contracts
        .iter()
        .find(|c| c.kind == atlas_index::ContractKind::Behaviour)
        .unwrap_or_else(|| {
            panic!(
                "expected a ContractKind::Behaviour contract for `defprotocol Stringable`; \
                 got contracts: {:?}",
                artefacts.contracts
            )
        });

    // The contract id must reference the Stringable module.
    assert!(
        behaviour_contract.id.contains("Stringable"),
        "expected contract id to reference `Stringable`, got: {}",
        behaviour_contract.id
    );
}
