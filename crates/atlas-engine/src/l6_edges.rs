//! L6 candidate-edge proposal — one batch Stage 2 call per run,
//! fanned out into per-component filtered lists.
//!
//! ## Batching
//!
//! Ravel-Lite's Stage 2 is a global pass over every surface record
//! (see `Ravel-Lite/src/discover/stage2.rs`). Atlas preserves the
//! batching for prompt efficiency: [`all_proposed_edges`] renders
//! every component's [`SurfaceRecord`] into one `{{SURFACE_RECORDS_YAML}}`
//! block and makes one backend call. [`candidate_edges_for`] then
//! filters the batch by participant id — cheap, since
//! [`AtlasDatabase::call_llm_cached`] returns the same
//! `Arc<Vec<Edge>>` reference across repeated calls within the same
//! revision.
//!
//! ## Canonicalisation
//!
//! Every proposed [`Edge`] is pushed through
//! [`component_ontology::Edge::validate`] before it reaches the
//! return value, so symmetric kinds land with sorted participants
//! and malformed proposals never appear in later layers.

use std::sync::Arc;

use atlas_index::ComponentEntry;
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use component_ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
use serde::Serialize;
use serde_json::{json, Value};

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;
use crate::l5_surface::surface_of;
use crate::surface_types::SurfaceRecord;

/// The shipped Atlas Stage 2 prompt, embedded at compile time.
pub const EMBEDDED_STAGE2_EDGES_PROMPT: &str =
    include_str!("../../../defaults/prompts/stage2-edges.md");

/// Edges involving the component with id `id`. Built by filtering the
/// global batch produced by [`all_proposed_edges`], so any number of
/// per-component queries within a revision cost one backend call.
pub fn candidate_edges_for(db: &AtlasDatabase, id: String) -> Arc<Vec<Edge>> {
    let all = all_proposed_edges(db);
    let mine: Vec<Edge> = all.iter().filter(|e| e.involves(&id)).cloned().collect();
    Arc::new(mine)
}

/// Batch Stage 2 pass. Produces the full canonicalised edge set,
/// memoised through [`AtlasDatabase::call_llm_cached`]. A
/// component-or-file change that invalidates any surface invalidates
/// the batch key, so this cannot silently serve stale edges.
pub fn all_proposed_edges(db: &AtlasDatabase) -> Arc<Vec<Edge>> {
    let components = all_components(db);
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();

    if live.len() < 2 {
        // A single-component run has no pairs to consider; the
        // prompt still technically runs, but we skip it to avoid
        // wasting tokens on a no-op.
        return Arc::new(Vec::new());
    }

    let surfaces: Vec<SurfaceWithId> = live
        .iter()
        .map(|c| SurfaceWithId {
            id: c.id.clone(),
            surface: (*surface_of(db, c.id.clone())).clone(),
        })
        .collect();

    let request = LlmRequest {
        prompt_template: PromptId::Stage2Edges,
        inputs: build_inputs(&surfaces),
        schema: ResponseSchema::accept_any(),
    };

    let value = match db.call_llm_cached(&request) {
        Ok(v) => v,
        Err(_) => return Arc::new(Vec::new()),
    };

    let parsed = parse_edges_response(&value).unwrap_or_default();
    let canonicalised = canonicalise_edges(parsed);
    Arc::new(canonicalised)
}

/// Surface record bundled with its component id — the shape the
/// Stage 2 prompt's `{{SURFACE_RECORDS_YAML}}` block expects.
#[derive(Debug, Clone, Serialize)]
struct SurfaceWithId {
    id: String,
    surface: SurfaceRecord,
}

fn build_inputs(surfaces: &[SurfaceWithId]) -> Value {
    // Render the surfaces as YAML because the Ravel-Lite-inherited
    // prompt's `{{SURFACE_RECORDS_YAML}}` token expects a YAML
    // fragment. ONTOLOGY_KINDS is populated from the embedded
    // ontology so the prompt's kind-list substitution renders cleanly.
    let surfaces_yaml = serde_yaml::to_string(&SurfacesWrapper { surfaces })
        .unwrap_or_else(|_| String::from("surfaces: []\n"));
    let ontology_block =
        component_ontology::render_embedded_kinds_for_prompt().unwrap_or_default();

    json!({
        "ONTOLOGY_KINDS": ontology_block,
        "CONFIG_ROOT": "(unused — Atlas does not shell out to a CLI)",
        "SURFACE_RECORDS_YAML": surfaces_yaml,
    })
}

#[derive(Debug, Serialize)]
struct SurfacesWrapper<'a> {
    surfaces: &'a [SurfaceWithId],
}

/// Parse the Stage 2 response into a raw edge list. Accepts two
/// shapes:
///
/// 1. A JSON array of edge objects.
/// 2. A JSON object with an `edges` key whose value is an array.
///
/// Unknown fields on individual edges are tolerated — Atlas only
/// extracts the fields it needs.
fn parse_edges_response(value: &Value) -> Result<Vec<Edge>, String> {
    let array = match value {
        Value::Array(a) => a,
        Value::Object(o) => {
            let Some(inner) = o.get("edges") else {
                return Err(
                    "expected top-level array or object with `edges` key".to_string(),
                );
            };
            inner
                .as_array()
                .ok_or_else(|| "`edges` field must be an array".to_string())?
        }
        _ => return Err(format!("expected array or object, got {value}")),
    };

    let mut out = Vec::with_capacity(array.len());
    for item in array {
        if let Some(edge) = parse_one_edge(item) {
            out.push(edge);
        }
    }
    Ok(out)
}

fn parse_one_edge(value: &Value) -> Option<Edge> {
    let obj = value.as_object()?;
    let kind = EdgeKind::parse(obj.get("kind")?.as_str()?)?;
    let lifecycle = LifecycleScope::parse(obj.get("lifecycle")?.as_str()?)?;
    let participants: Vec<String> = obj
        .get("participants")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if participants.len() != 2 {
        return None;
    }
    let evidence_grade = obj
        .get("evidence_grade")
        .and_then(|v| v.as_str())
        .and_then(EvidenceGrade::parse)
        .unwrap_or(EvidenceGrade::Medium);
    let evidence_fields: Vec<String> = obj
        .get("evidence_fields")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let rationale = obj
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("LLM did not supply a rationale")
        .to_string();

    Some(Edge {
        kind,
        lifecycle,
        participants,
        evidence_grade,
        evidence_fields,
        rationale,
    })
}

/// Enforce §9.5 canonicalisation on every proposal. Symmetric kinds
/// have their participants sorted; anything that fails
/// [`Edge::validate`] after that adjustment is dropped. Duplicate
/// edges (equal canonical keys) are collapsed in insertion order —
/// the first wins so an earlier, typically more-confident proposal
/// is preferred over a later restatement.
fn canonicalise_edges(edges: Vec<Edge>) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::with_capacity(edges.len());
    let mut seen: std::collections::HashSet<(EdgeKind, LifecycleScope, Vec<String>)> =
        std::collections::HashSet::new();

    for mut edge in edges {
        if !edge.kind.is_directed() {
            edge.participants.sort();
        }
        if edge.validate().is_err() {
            continue;
        }
        let key = edge.canonical_key();
        if seen.insert(key) {
            out.push(edge);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AtlasDatabase;
    use crate::ingest::seed_filesystem;
    use crate::l5_surface::EMBEDDED_STAGE1_SURFACE_PROMPT;
    use crate::prompt_migration::project_to_component;
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    const RAVEL_LITE_STAGE2: &str =
        include_str!("../../../../Ravel-Lite/defaults/discover-stage2.md");

    #[test]
    fn stage2_edges_prompt_is_the_transformed_ravel_lite_original() {
        let expected = project_to_component(RAVEL_LITE_STAGE2);
        assert_eq!(
            EMBEDDED_STAGE2_EDGES_PROMPT, expected,
            "Atlas stage2-edges.md diverged from Ravel-Lite's discover-stage2.md \
             modulo the documented project→component substitutions"
        );
    }

    #[test]
    fn stage2_edges_prompt_has_no_residual_project_word() {
        for stem in ["project", "Project", "PROJECT"] {
            assert!(
                !EMBEDDED_STAGE2_EDGES_PROMPT.contains(stem),
                "stage2-edges.md contains stray `{stem}` token"
            );
        }
    }

    #[test]
    fn both_prompts_are_non_empty() {
        assert!(!EMBEDDED_STAGE1_SURFACE_PROMPT.is_empty());
        assert!(!EMBEDDED_STAGE2_EDGES_PROMPT.is_empty());
    }

    // ---------------------------------------------------------------
    // Fixtures: build a two-crate workspace so L4 produces two live
    // components, and drive surface_of + L6 end-to-end with canned
    // responses on a shared TestBackend.
    // ---------------------------------------------------------------

    fn fp() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn write_lib_crate(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
        std::fs::write(dir.join("README.md"), format!("# {name}\n")).unwrap();
    }

    fn two_component_setup() -> (AtlasDatabase, Arc<TestBackend>, TempDir) {
        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "alpha");
        write_lib_crate(tmp.path(), "beta");
        let backend = Arc::new(TestBackend::with_fingerprint(fp()));
        let backend_dyn: Arc<dyn atlas_llm::LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fp());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();
        (db, backend, tmp)
    }

    fn minimal_surface(tag: &str) -> serde_json::Value {
        json!({
            "purpose": format!("{tag} component"),
            "consumes_files": [],
            "produces_files": [],
            "network_endpoints": [],
            "data_formats": [],
            "external_tools_spawned": [],
            "explicit_cross_component_mentions": [],
            "interaction_role_hints": [],
            "notes": "",
        })
    }

    /// Register canned Stage 1 responses for every live component so
    /// `surface_of` walks do not error. Returns the ordered ids.
    fn prime_surfaces(db: &AtlasDatabase, backend: &TestBackend) -> Vec<String> {
        let components = all_components(db);
        let ids: Vec<String> = components
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| c.id.clone())
            .collect();
        for id in &ids {
            let peer_ids: Vec<String> = ids.iter().filter(|p| *p != id).cloned().collect();
            let entry = components.iter().find(|c| &c.id == id).unwrap();
            let inputs = crate::l5_surface::build_inputs_for_tests(entry, &peer_ids);
            backend.respond(PromptId::Stage1Surface, inputs, minimal_surface(id));
        }
        ids
    }

    #[test]
    fn all_proposed_edges_parses_canned_stage2_response() {
        let (db, backend, _tmp) = two_component_setup();
        let ids = prime_surfaces(&db, &backend);
        assert_eq!(ids.len(), 2, "fixture must produce exactly two components");

        // Build the exact Stage 2 inputs the engine will use, then
        // register a canned response proposing one edge.
        let surfaces: Vec<SurfaceWithId> = ids
            .iter()
            .map(|id| SurfaceWithId {
                id: id.clone(),
                surface: (*surface_of(&db, id.clone())).clone(),
            })
            .collect();
        let inputs = build_inputs(&surfaces);

        let edge_response = json!([
            {
                "kind": "depends-on",
                "lifecycle": "build",
                "participants": [ids[0], ids[1]],
                "evidence_grade": "strong",
                "evidence_fields": [
                    format!("{}.produces_files", ids[0]),
                    format!("{}.consumes_files", ids[1]),
                ],
                "rationale": "synthetic fixture edge",
            }
        ]);
        backend.respond(PromptId::Stage2Edges, inputs, edge_response);

        let edges = all_proposed_edges(&db);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::DependsOn);
        assert_eq!(edges[0].lifecycle, LifecycleScope::Build);
        assert_eq!(edges[0].participants, vec![ids[0].clone(), ids[1].clone()]);

        // And candidate_edges_for should filter by participant.
        let a_edges = candidate_edges_for(&db, ids[0].clone());
        assert_eq!(a_edges.len(), 1);
        let unrelated = candidate_edges_for(&db, "nonexistent".into());
        assert!(unrelated.is_empty());
    }

    #[test]
    fn symmetric_edge_participants_get_sorted_during_canonicalisation() {
        let (db, backend, _tmp) = two_component_setup();
        let ids = prime_surfaces(&db, &backend);
        let surfaces: Vec<SurfaceWithId> = ids
            .iter()
            .map(|id| SurfaceWithId {
                id: id.clone(),
                surface: (*surface_of(&db, id.clone())).clone(),
            })
            .collect();
        let inputs = build_inputs(&surfaces);

        // Canonical order is alphabetical; deliberately reverse the
        // participants in the canned response to prove canonicalisation
        // sorts them before returning.
        let (first, second) = {
            let mut sorted = ids.clone();
            sorted.sort();
            (sorted[0].clone(), sorted[1].clone())
        };
        let response = json!([
            {
                "kind": "co-implements",
                "lifecycle": "design",
                "participants": [second.clone(), first.clone()], // reversed
                "evidence_grade": "medium",
                "evidence_fields": [format!("{first}.purpose")],
                "rationale": "same spec",
            }
        ]);
        backend.respond(PromptId::Stage2Edges, inputs, response);

        let edges = all_proposed_edges(&db);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0].participants,
            vec![first, second],
            "symmetric kind must have sorted participants after canonicalisation"
        );
    }

    #[test]
    fn directed_edge_participants_preserve_callers_order() {
        let (db, backend, _tmp) = two_component_setup();
        let ids = prime_surfaces(&db, &backend);
        let surfaces: Vec<SurfaceWithId> = ids
            .iter()
            .map(|id| SurfaceWithId {
                id: id.clone(),
                surface: (*surface_of(&db, id.clone())).clone(),
            })
            .collect();
        let inputs = build_inputs(&surfaces);

        // Generates is directed — whatever order we feed in is what
        // canonicalise_edges must preserve (Gen → Out).
        let (from_id, to_id) = (ids[1].clone(), ids[0].clone());
        let response = json!([
            {
                "kind": "generates",
                "lifecycle": "codegen",
                "participants": [from_id.clone(), to_id.clone()],
                "evidence_grade": "strong",
                "evidence_fields": [
                    format!("{from_id}.produces_files"),
                    format!("{to_id}.consumes_files"),
                ],
                "rationale": "A generates outputs B consumes",
            }
        ]);
        backend.respond(PromptId::Stage2Edges, inputs, response);

        let edges = all_proposed_edges(&db);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].participants, vec![from_id, to_id]);
    }

    #[test]
    fn empty_batch_when_fewer_than_two_components() {
        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "solo");
        let backend = Arc::new(TestBackend::with_fingerprint(fp()));
        let backend_dyn: Arc<dyn atlas_llm::LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fp());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let edges = all_proposed_edges(&db);
        assert!(edges.is_empty());
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "single-component run must not call the backend for Stage 2"
        );
    }

    #[test]
    fn parse_edges_response_accepts_top_level_array() {
        let v = json!([
            {
                "kind": "depends-on",
                "lifecycle": "build",
                "participants": ["A", "B"],
                "evidence_grade": "strong",
                "evidence_fields": ["A.x"],
                "rationale": "x",
            }
        ]);
        let got = parse_edges_response(&v).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn parse_edges_response_accepts_edges_wrapped_object() {
        let v = json!({ "edges": [
            {
                "kind": "depends-on",
                "lifecycle": "build",
                "participants": ["A", "B"],
                "evidence_grade": "strong",
                "evidence_fields": ["A.x"],
                "rationale": "x",
            }
        ]});
        let got = parse_edges_response(&v).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn parse_edges_response_skips_malformed_entries_silently() {
        let v = json!([
            { "kind": "depends-on" }, // missing fields — dropped
            {
                "kind": "depends-on",
                "lifecycle": "build",
                "participants": ["A", "B"],
                "rationale": "x",
            }
        ]);
        let got = parse_edges_response(&v).unwrap();
        assert_eq!(got.len(), 1, "malformed entries are dropped, not propagated");
    }

    #[test]
    fn canonicalise_edges_dedupes_within_a_single_batch() {
        let twice = vec![
            Edge {
                kind: EdgeKind::DependsOn,
                lifecycle: LifecycleScope::Build,
                participants: vec!["A".into(), "B".into()],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["A.x".into()],
                rationale: "once".into(),
            },
            Edge {
                kind: EdgeKind::DependsOn,
                lifecycle: LifecycleScope::Build,
                participants: vec!["A".into(), "B".into()],
                evidence_grade: EvidenceGrade::Medium,
                evidence_fields: vec!["A.y".into()],
                rationale: "twice".into(),
            },
        ];
        let out = canonicalise_edges(twice);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rationale, "once", "first wins on duplicate key");
    }
}
