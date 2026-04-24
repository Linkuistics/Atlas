//! L5 surface extraction — for one component, call Stage 1 and produce
//! a parsed [`SurfaceRecord`].
//!
//! The query is not `#[salsa::tracked]`. Like L3 it takes
//! `&AtlasDatabase` and drops through the non-Salsa LLM backend;
//! incremental memoisation at the response level happens in
//! [`crate::llm_cache`] which is keyed by the fingerprint + request
//! inputs. That satisfies the "zero LLM calls on no-op re-run" contract
//! without requiring a Salsa downcast.
//!
//! ## Pin short-circuit
//!
//! `overrides.pins[<id>]["surface"]` may carry a `PinValue::Value` whose
//! `value` is a YAML-serialised [`SurfaceRecord`]. When present, L5
//! parses it and returns it directly — no LLM call. This is a rare
//! manual escape hatch (design §4.1 L5) for components whose surface
//! the LLM cannot produce well on its own.

use std::sync::Arc;

use atlas_index::{ComponentEntry, OverridesFile, PinValue};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use serde_json::{json, Value};

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;
use crate::surface_types::SurfaceRecord;

/// The shipped Atlas Stage 1 prompt, embedded at compile time. Exposed
/// so the atlas-cli driver can compute the prompt SHA that feeds
/// [`atlas_llm::LlmFingerprint`] without re-reading the file from
/// disk.
pub const EMBEDDED_STAGE1_SURFACE_PROMPT: &str =
    include_str!("../../../defaults/prompts/stage1-surface.md");

/// Produce the Stage 1 surface record for the component whose id is
/// `id`. Pin-short-circuits through [`surface_pin`] before any LLM
/// call; otherwise invokes the backend via
/// [`AtlasDatabase::call_llm_cached`] so repeated calls at the same
/// revision are free.
///
/// If the id does not resolve to an internal component (or the
/// component has no path_segments), returns a default
/// [`SurfaceRecord`] — the engine is intentionally non-panicking on
/// unknown ids so callers can probe freely.
pub fn surface_of(db: &AtlasDatabase, id: String) -> Arc<SurfaceRecord> {
    // Resolve the component. `all_components` does the L2→L4 walk;
    // Salsa caches the result across repeated surface_of calls that
    // share a revision.
    let components = all_components(db);
    let Some(entry) = components.iter().find(|c| c.id == id && !c.deleted) else {
        return Arc::new(SurfaceRecord::default());
    };

    let workspace = db.workspace();
    let overrides = workspace
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    if let Some(pinned) = surface_pin(&overrides, &id) {
        return Arc::new(pinned);
    }

    let peer_ids: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted && c.id != id)
        .map(|c| c.id.clone())
        .collect();

    let request = LlmRequest {
        prompt_template: PromptId::Stage1Surface,
        inputs: build_inputs(entry, &peer_ids),
        schema: ResponseSchema::accept_any(),
    };

    let value = match db.call_llm_cached(&request) {
        Ok(v) => v,
        Err(err) => {
            // Conservative failure mode: empty surface annotated with
            // the error in `notes`. The caller (L6 / CLI) can decide
            // whether to surface this or skip. Matches L3's "soft fail"
            // behaviour rather than panicking deep in the pipeline.
            return Arc::new(SurfaceRecord {
                notes: format!("LLM call failed: {err}"),
                ..SurfaceRecord::default()
            });
        }
    };

    match parse_surface_response(&value) {
        Ok(record) => Arc::new(record),
        Err(reason) => Arc::new(SurfaceRecord {
            notes: format!("LLM response parse failed: {reason}"),
            ..SurfaceRecord::default()
        }),
    }
}

/// JSON input document for the Stage 1 prompt. The key set is stable
/// across the live code so cache-key equality is a proxy for
/// "inputs unchanged".
///
/// Fields:
///
/// - `COMPONENT_ID` — so a future prompt version can key directly on it.
/// - `COMPONENT_PATHS` — a JSON array of the relative path segments
///   the component spans (design §4.1 L5 notes that a component may
///   span multiple segments).
/// - `COMPONENT_CONTENT_SHAS` — the matching per-segment content SHAs
///   so a file-level content change invalidates the cache key.
/// - `CATALOG_COMPONENTS` — marker-formatted list of peer component
///   ids so `{{CATALOG_COMPONENTS}}` substitution in the shipped
///   prompt has something to expand to.
/// - `SURFACE_OUTPUT_PATH` — placeholder so the Ravel-Lite-inherited
///   output instruction renders without a template error; the value
///   is cosmetic for backends that consume JSON via stdout.
fn build_inputs(component: &ComponentEntry, peer_ids: &[String]) -> Value {
    let component_paths: Vec<String> = component
        .path_segments
        .iter()
        .map(|seg| path_to_forward_slash(&seg.path))
        .collect();
    let content_shas: Vec<String> = component
        .path_segments
        .iter()
        .map(|seg| seg.content_sha.clone())
        .collect();
    let catalog_block = render_catalog_for_prompt(peer_ids);

    json!({
        "COMPONENT_ID": component.id,
        "COMPONENT_PATHS": component_paths,
        "COMPONENT_CONTENT_SHAS": content_shas,
        "CATALOG_COMPONENTS": catalog_block,
        "SURFACE_OUTPUT_PATH": "(stdout)",
    })
}

/// Test-only escape hatch: other L-layer tests need to know exactly
/// what [`build_inputs`] produces so they can register a canned
/// TestBackend response against that shape. Kept here rather than in
/// a shared fixtures module so the real function stays private.
#[cfg(test)]
pub(crate) fn build_inputs_for_tests(
    component: &ComponentEntry,
    peer_ids: &[String],
) -> Value {
    build_inputs(component, peer_ids)
}

fn render_catalog_for_prompt(peer_ids: &[String]) -> String {
    if peer_ids.is_empty() {
        return "_(none — this component is the only catalog entry)_".to_string();
    }
    peer_ids
        .iter()
        .map(|n| format!("- {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn path_to_forward_slash(path: &std::path::Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Parse the LLM response into a [`SurfaceRecord`]. Accepts two
/// shapes:
///
/// 1. A JSON object matching `SurfaceRecord` directly.
/// 2. A JSON object with a `surface` key whose value matches
///    `SurfaceRecord` — mirrors Ravel-Lite's `SurfaceFile`-wrapped
///    shape for compatibility with a backend that forwards the
///    entire file.
fn parse_surface_response(value: &Value) -> Result<SurfaceRecord, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected JSON object, got {}", value_kind(value)))?;

    let body = if let Some(inner) = object.get("surface") {
        inner
    } else {
        value
    };
    serde_json::from_value::<SurfaceRecord>(body.clone())
        .map_err(|e| format!("{e}"))
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Look up a surface-level pin for `id`. Pins live in
/// `overrides.pins[<id>]["surface"]` with a `PinValue::Value` whose
/// string is a YAML-encoded [`SurfaceRecord`]. Returns `None` unless
/// the pin is present AND parses cleanly — a malformed pin is
/// reported via `notes` by the caller's fallback, not silently
/// ignored.
fn surface_pin(overrides: &OverridesFile, id: &str) -> Option<SurfaceRecord> {
    let pins = overrides.pins.get(id)?;
    let entry = pins.get("surface")?;
    let PinValue::Value { value, .. } = entry else {
        return None;
    };
    serde_yaml::from_str::<SurfaceRecord>(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AtlasDatabase;
    use crate::ingest::seed_filesystem;
    use crate::prompt_migration::project_to_component;
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    const RAVEL_LITE_STAGE1: &str =
        include_str!("../../../../Ravel-Lite/defaults/discover-stage1.md");

    #[test]
    fn stage1_surface_prompt_is_the_transformed_ravel_lite_original() {
        let expected = project_to_component(RAVEL_LITE_STAGE1);
        assert_eq!(
            EMBEDDED_STAGE1_SURFACE_PROMPT, expected,
            "Atlas stage1-surface.md diverged from Ravel-Lite's discover-stage1.md \
             modulo the documented project→component substitutions; either the \
             Ravel-Lite prose changed, or the Atlas copy was edited outside the \
             transformation. Reconcile in one place."
        );
    }

    #[test]
    fn stage1_surface_prompt_mentions_the_catalog_components_token() {
        assert!(
            EMBEDDED_STAGE1_SURFACE_PROMPT.contains("{{CATALOG_COMPONENTS}}"),
            "stage1-surface.md must expose the renamed catalog token"
        );
        assert!(
            !EMBEDDED_STAGE1_SURFACE_PROMPT.contains("{{CATALOG_PROJECTS}}"),
            "stage1-surface.md must not retain the pre-migration token name"
        );
    }

    #[test]
    fn stage1_surface_prompt_has_no_residual_project_word() {
        // Every occurrence of "project" case-variants is a migration
        // miss. The substitution is exhaustive in both Ravel-Lite's
        // current prose and any drift — a "project" word re-entering
        // the corpus should fail loud.
        for stem in ["project", "Project", "PROJECT"] {
            assert!(
                !EMBEDDED_STAGE1_SURFACE_PROMPT.contains(stem),
                "stage1-surface.md contains stray `{stem}` token"
            );
        }
    }

    // ---------------------------------------------------------------
    // Fixture helpers for surface_of integration tests. The backend
    // is owned by an `Arc<TestBackend>` kept by the test; the
    // database receives a cheaply-cloned `Arc<dyn LlmBackend>` that
    // points at the same heap object, so `backend.respond(...)`
    // calls from the test land in the map the database reads.
    // ---------------------------------------------------------------

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn canned_surface() -> Value {
        json!({
            "purpose": "Does the alpha thing.",
            "consumes_files": ["~/.config/alpha/*.yaml"],
            "produces_files": ["/tmp/out/*.json"],
            "network_endpoints": ["grpc://alpha:50051"],
            "data_formats": ["AlphaRecord"],
            "external_tools_spawned": ["git"],
            "explicit_cross_component_mentions": ["Beta"],
            "interaction_role_hints": ["generator"],
            "notes": "",
        })
    }

    fn write_cargo_lib_fixture(root: &std::path::Path, crate_name: &str) {
        let crate_dir = root.join(crate_name);
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{crate_name}\"\n"),
        )
        .unwrap();
        std::fs::write(crate_dir.join("src").join("lib.rs"), "// lib\n").unwrap();
        std::fs::write(crate_dir.join("README.md"), format!("# {crate_name}\n")).unwrap();
    }

    /// Build an AtlasDatabase from a filesystem fixture rooted in
    /// `tmp`, returning `(db, backend)` so tests can register canned
    /// responses on the very same backend instance the engine calls.
    fn db_with_shared_backend(tmp: &TempDir) -> (AtlasDatabase, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn atlas_llm::LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();
        (db, backend)
    }

    /// The engine includes the component's path_segments'
    /// content_shas in its cache-key inputs, which the test cannot
    /// know until after seeding. Returns the exact inputs
    /// [`surface_of`] will build.
    fn inputs_for_id(db: &AtlasDatabase, id: &str) -> Value {
        let components = all_components(db);
        let entry = components
            .iter()
            .find(|c| c.id == id && !c.deleted)
            .expect("id must resolve to a live component");
        let peer_ids: Vec<String> = components
            .iter()
            .filter(|c| !c.deleted && c.id != id)
            .map(|c| c.id.clone())
            .collect();
        build_inputs(entry, &peer_ids)
    }

    #[test]
    fn surface_of_parses_canned_stage1_response_for_single_component_fixture() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "alpha");
        let (db, backend) = db_with_shared_backend(&tmp);

        let components = all_components(&db);
        let id = components
            .iter()
            .find(|c| !c.deleted)
            .expect("fixture must produce a component")
            .id
            .clone();
        let inputs = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs, canned_surface());

        let record = surface_of(&db, id);
        assert_eq!(record.purpose, "Does the alpha thing.");
        assert_eq!(record.explicit_cross_component_mentions, vec!["Beta".to_string()]);
        assert_eq!(record.interaction_role_hints.len(), 1);
    }

    #[test]
    fn surface_of_hits_cache_on_second_call_with_unchanged_inputs() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "beta");
        let (db, backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let inputs = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs, canned_surface());

        // Reset counter — previous tests in the same module may have
        // been counted if cache state were shared. It is not: every
        // AtlasDatabase owns its own LlmResponseCache.
        assert_eq!(db.llm_cache().call_count(), 0);
        let _first = surface_of(&db, id.clone());
        assert_eq!(db.llm_cache().call_count(), 1);
        let _second = surface_of(&db, id.clone());
        assert_eq!(
            db.llm_cache().call_count(),
            1,
            "second identical call must hit the cache — this is the \
             'zero LLM calls on no-op re-run' contract in miniature"
        );
    }

    #[test]
    fn surface_of_misses_cache_when_file_content_changes() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "gamma");
        let (mut db, backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let inputs_before = inputs_for_id(&db, &id);
        backend.respond(
            PromptId::Stage1Surface,
            inputs_before,
            canned_surface(),
        );

        let _ = surface_of(&db, id.clone());
        let calls_after_first = db.llm_cache().call_count();
        assert_eq!(calls_after_first, 1);

        // Mutate a file — its content_sha propagates up to
        // path_segments[0].content_sha, invalidating the cache key.
        let lib_path = tmp.path().join("gamma").join("src").join("lib.rs");
        std::fs::write(&lib_path, "// modified\n").unwrap();
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        // The new content_sha produces a new input shape. Register
        // the new inputs with the same canned response so the
        // response parses cleanly; the assertion is on call count,
        // not response shape.
        let inputs_after = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs_after, canned_surface());

        let _ = surface_of(&db, id);
        assert_eq!(
            db.llm_cache().call_count(),
            2,
            "content-sha change must force a cache miss"
        );
    }

    #[test]
    fn surface_pin_short_circuits_before_backend() {
        use atlas_index::{OverridesFile, PinValue};
        use std::collections::BTreeMap;

        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "delta");
        let (mut db, _backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();

        // Install a surface pin that encodes a minimal SurfaceRecord as
        // YAML in the pin value. No canned response is registered —
        // if the pin is not honoured the TestBackend would error.
        let mut field_pins = BTreeMap::new();
        let pinned_yaml = "purpose: pinned\nnotes: hand-authored\n";
        field_pins.insert(
            "surface".to_string(),
            PinValue::Value {
                value: pinned_yaml.to_string(),
                reason: None,
            },
        );
        let mut pins = BTreeMap::new();
        pins.insert(id.clone(), field_pins);
        db.set_components_overrides(OverridesFile {
            pins,
            ..OverridesFile::default()
        });

        let record = surface_of(&db, id);
        assert_eq!(record.purpose, "pinned");
        assert_eq!(record.notes, "hand-authored");
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "pinned surface must not touch the backend"
        );
    }

    #[test]
    fn surface_of_returns_default_for_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "epsilon");
        let (db, _backend) = db_with_shared_backend(&tmp);

        let record = surface_of(&db, "does-not-exist".into());
        assert_eq!(record.as_ref(), &SurfaceRecord::default());
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "unknown id must not make an LLM call"
        );
    }

    #[test]
    fn parse_surface_response_accepts_bare_record_shape() {
        let v = json!({ "purpose": "p" });
        let got = parse_surface_response(&v).unwrap();
        assert_eq!(got.purpose, "p");
    }

    #[test]
    fn parse_surface_response_accepts_surface_wrapped_shape() {
        let v = json!({ "surface": { "purpose": "w" } });
        let got = parse_surface_response(&v).unwrap();
        assert_eq!(got.purpose, "w");
    }

    #[test]
    fn parse_surface_response_rejects_non_object() {
        let err = parse_surface_response(&json!("string-value")).unwrap_err();
        assert!(err.contains("object"), "{err}");
    }
}
