//! PR-8 integration tests: contract edges (`defines-contract`,
//! `implements-contract`, `consumes-contract`) derived from per-component
//! `surfaces.yaml` (deterministic) and the LLM Stage 2 batch (LLM-driven).
//!
//! ## Test layout
//!
//! - [`defines_and_implements_edges_from_serde_struct`] — acceptance
//!   criterion #1 (first two sub-assertions): a component with a
//!   `#[derive(Serialize, Deserialize)]` pub struct emits both
//!   `defines-contract` and `implements-contract` edges deterministically.
//! - [`consumes_contract_edge_from_llm_stage2`] — acceptance criterion
//!   #1 (third sub-assertion): a canned Stage 2 response containing a
//!   `consumes-contract` edge surfaces in `all_proposed_edges`.
//! - [`empty_surfaces_produce_no_contract_edges`] — acceptance criterion
//!   #3: components with no serde structs produce zero contract edges.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{all_components, all_proposed_edges, seed_filesystem, AtlasDatabase};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use component_ontology::EdgeKind;
use serde_json::{json, Value};
use tempfile::TempDir;

// -----------------------------------------------------------------------
// A backend that accepts any Stage 1 call (returns a bare surface JSON),
// and any Stage 2 call (returns a pre-registered canned response).  All
// other calls (Classify, Subcarve, etc.) error so tests do not silently
// pass by swallowing unexpected LLM calls.
// -----------------------------------------------------------------------

struct AnyStage2Backend {
    stage2_response: Value,
}

impl AnyStage2Backend {
    fn new(stage2_response: Value) -> Self {
        AnyStage2Backend { stage2_response }
    }
}

impl LlmBackend for AnyStage2Backend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        match req.prompt_template {
            PromptId::Stage1Surface => {
                // Return a minimal valid surface record so `surface_of`
                // does not fall into the error-notes path.
                Ok(json!({
                    "purpose": "test component",
                    "consumes_files": [],
                    "produces_files": [],
                    "network_endpoints": [],
                    "data_formats": [],
                    "external_tools_spawned": [],
                    "explicit_cross_component_mentions": [],
                    "interaction_role_hints": [],
                    "notes": "",
                }))
            }
            PromptId::Stage2Edges => Ok(self.stage2_response.clone()),
            other => Err(LlmError::TestBackendMiss(format!(
                "AnyStage2Backend: unexpected call for prompt {:?}",
                other
            ))),
        }
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [8u8; 32],
            ontology_sha: [8u8; 32],
            model_id: "any-stage2-backend".into(),
            backend_version: "0".into(),
        }
    }
}

// -----------------------------------------------------------------------
// Fixture helpers
// -----------------------------------------------------------------------

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [8u8; 32],
        ontology_sha: [8u8; 32],
        model_id: "any-stage2-backend".into(),
        backend_version: "0".into(),
    }
}

/// Write a Rust library crate with a simple `pub fn` — no serde structs.
fn write_plain_lib_crate(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "pub fn baz() {}\n").unwrap();
}

/// Write a Rust library crate that defines a serde-derived pub struct `Foo`.
/// The analyser will detect this and emit a `data-format` contract.
fn write_serde_lib_crate(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n\
             \n[dependencies]\nserde = \"1\"\n"
        ),
    )
    .unwrap();
    // A pub struct with serde derive triggers `data-format` contract emission.
    std::fs::write(
        dir.join("src").join("lib.rs"),
        "#[derive(serde::Serialize, serde::Deserialize)]\npub struct Foo { pub a: u32 }\n",
    )
    .unwrap();
}

/// Locate the single component whose id ends with `leaf`.
fn find_by_leaf(db: &AtlasDatabase, leaf: &str) -> String {
    let components = all_components(db);
    let matches: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted)
        .filter(|c| {
            let id = c.id.as_str();
            match id.rfind('/') {
                Some(idx) => &id[idx + 1..] == leaf,
                None => id == leaf,
            }
        })
        .map(|c| c.id.as_str().to_string())
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one component with leaf `{leaf}`, got {matches:?}\n\
         all: {:?}",
        components.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
    );
    matches.into_iter().next().unwrap()
}

// -----------------------------------------------------------------------
// Acceptance criterion #1 (part A): defines-contract + implements-contract
// from the deterministic path (no LLM Stage 2 needed).
// -----------------------------------------------------------------------

#[test]
fn defines_and_implements_edges_from_serde_struct() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Crate A: has a serde-derived struct → deterministic contract detection.
    write_serde_lib_crate(root, "crate-a");
    // Crate B: plain lib — no serde structs.
    write_plain_lib_crate(root, "crate-b");

    // Use a backend that always accepts Stage 1 and returns an empty
    // Stage 2 response (no LLM edges). Deterministic edges still flow.
    let backend: Arc<dyn LlmBackend> = Arc::new(AnyStage2Backend::new(json!([])));
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).unwrap();

    let a_id = find_by_leaf(&db, "crate-a");

    let edges = all_proposed_edges(&db);

    // The contract id is `{component_id}/{kebabified_struct_name}`.
    // For struct `Foo` in component `{root_prefix}/crate-a`, the id is
    // `{root_prefix}/crate-a/foo`.
    let contract_id = format!("{a_id}/foo");

    let defines: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DefinesContract)
        .collect();
    assert_eq!(
        defines.len(),
        1,
        "expected exactly one defines-contract edge, got {edges:?}"
    );
    assert_eq!(
        defines[0].participants,
        vec![a_id.clone(), contract_id.clone()],
        "defines-contract participants must be [component, contract]"
    );

    let implements: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::ImplementsContract)
        .collect();
    assert_eq!(
        implements.len(),
        1,
        "expected exactly one implements-contract edge, got {edges:?}"
    );
    assert_eq!(
        implements[0].participants,
        vec![a_id.clone(), contract_id.clone()],
        "implements-contract participants must be [component, contract]"
    );
}

// -----------------------------------------------------------------------
// Acceptance criterion #1 (part B): consumes-contract from canned Stage 2.
// -----------------------------------------------------------------------

#[test]
fn consumes_contract_edge_from_llm_stage2() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    write_serde_lib_crate(root, "crate-a");
    write_plain_lib_crate(root, "crate-b");

    // We need to know the ids BEFORE building the database so we can
    // form the canned response. The contract id will be
    // `{a_id}/foo` where `a_id` comes from the L4 tree. Build the db
    // first (seed_filesystem), grab the ids, then rebuild with the
    // canned backend.
    //
    // Because the contract id depends on `a_id` (which depends on the
    // workspace root prefix), we build the database twice: first pass
    // to discover the ids, second pass with the correct canned response.
    let probe_backend: Arc<dyn LlmBackend> = Arc::new(AnyStage2Backend::new(json!([])));
    let mut probe_db = AtlasDatabase::new(probe_backend, root.to_path_buf(), fingerprint());
    seed_filesystem(&mut probe_db, &[root.to_path_buf()], false).unwrap();
    let a_id = find_by_leaf(&probe_db, "crate-a");
    let b_id = find_by_leaf(&probe_db, "crate-b");
    let contract_id = format!("{a_id}/foo");
    drop(probe_db);

    // Build the real database with the correct canned Stage 2 response.
    let stage2_response = json!([{
        "kind": "consumes-contract",
        "lifecycle": "design",
        "participants": [b_id.clone(), contract_id.clone()],
        "evidence_grade": "medium",
        "evidence_fields": ["crate-b.consumes"],
        "rationale": "crate-b reads crate-a/foo data format",
    }]);
    let backend: Arc<dyn LlmBackend> = Arc::new(AnyStage2Backend::new(stage2_response));
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).unwrap();

    let edges = all_proposed_edges(&db);

    // Deterministic edges from crate-a's serde struct.
    assert!(
        edges.iter().any(|e| e.kind == EdgeKind::DefinesContract
            && e.participants == [a_id.clone(), contract_id.clone()]),
        "expected defines-contract edge, got {edges:?}"
    );
    assert!(
        edges.iter().any(|e| e.kind == EdgeKind::ImplementsContract
            && e.participants == [a_id.clone(), contract_id.clone()]),
        "expected implements-contract edge, got {edges:?}"
    );

    // LLM-sourced consumes-contract edge.
    assert!(
        edges.iter().any(|e| e.kind == EdgeKind::ConsumesContract
            && e.participants == [b_id.clone(), contract_id.clone()]),
        "expected consumes-contract edge from Stage 2, got {edges:?}"
    );
}

// -----------------------------------------------------------------------
// Acceptance criterion #3: no serde structs → no contract edges.
// -----------------------------------------------------------------------

#[test]
fn empty_surfaces_produce_no_contract_edges() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Both crates have no serde-derived structs.
    write_plain_lib_crate(root, "alpha");
    write_plain_lib_crate(root, "beta");

    // TestBackend::new() errors on any LLM call → Stage 2 fails → only
    // deterministic edges survive. Since no serde structs exist,
    // `contract_edges_from_surfaces` returns empty.
    let backend: Arc<dyn LlmBackend> = Arc::new(AnyStage2Backend::new(json!([])));
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).unwrap();

    let edges = all_proposed_edges(&db);

    let contract_edges: Vec<_> = edges
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                EdgeKind::DefinesContract
                    | EdgeKind::ImplementsContract
                    | EdgeKind::ConsumesContract
            )
        })
        .collect();

    assert!(
        contract_edges.is_empty(),
        "expected no contract edges when surfaces.yaml is empty, got {contract_edges:?}"
    );
}
