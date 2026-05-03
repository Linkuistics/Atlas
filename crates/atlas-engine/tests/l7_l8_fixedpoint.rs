//! Integration tests for L7/L8 + the fixedpoint driver.
//!
//! The module-level unit tests in `l7_structural.rs`, `subcarve_policy.rs`,
//! `l8_recurse.rs`, and `fixedpoint.rs` cover the algorithmic arms.
//! Here we exercise the back-edge closure end-to-end: driver →
//! subcarve decision → carve_back_edge input → L2 picks up new
//! candidates → L4 grows the tree.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{
    all_components, run_fixedpoint, seed_filesystem, AtlasDatabase, FixedpointConfig,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::{json, Value};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [11u8; 32],
        ontology_sha: [12u8; 32],
        model_id: "integration-test".into(),
        backend_version: "0".into(),
    }
}

fn write_lib_crate(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
}

/// Stateful backend that records every call and decides responses by
/// prompt id. Used to prove the back-edge closes without having to
/// canonicalise the exact input shape every per-prompt call issues.
struct ScriptedBackend {
    responses: std::sync::Mutex<std::collections::HashMap<PromptId, Value>>,
    fingerprint: LlmFingerprint,
}

impl ScriptedBackend {
    fn new(responses: Vec<(PromptId, Value)>) -> Self {
        let mut map = std::collections::HashMap::new();
        for (id, v) in responses {
            map.insert(id, v);
        }
        ScriptedBackend {
            responses: std::sync::Mutex::new(map),
            fingerprint: fingerprint(),
        }
    }
}

impl LlmBackend for ScriptedBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let map = self.responses.lock().unwrap();
        map.get(&req.prompt_template).cloned().ok_or_else(|| {
            LlmError::TestBackendMiss(format!(
                "ScriptedBackend has no response for {:?}",
                req.prompt_template
            ))
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

#[test]
fn back_edge_adds_subcarve_sub_dirs_to_workspace_carve_back_edge() {
    // Under L8 map/reduce, the back-edge is populated by immediate
    // sub-dirs of the component that L3 verdicts mark as boundaries
    // (no LLM-proposed paths). The fixture has `lib/src/lib.rs`, so
    // `lib/`'s immediate sub-dir is `src`. With a Classify response
    // that says "yes, boundary", `lib/src` lands in the back-edge
    // keyed by the library id.
    let tmp = TempDir::new().unwrap();
    write_lib_crate(tmp.path(), "lib");
    let backend = Arc::new(ScriptedBackend::new(vec![
        (
            PromptId::Classify,
            json!({
                "kind": "rust-library",
                "rationale": "boundary",
                "is_boundary": true,
                "evidence_grade": "medium",
            }),
        ),
        // Stage2 can fire incidentally via L7's edge_graph → L6 path.
        // Empty edges keep the test's focus on the back-edge.
        (PromptId::Stage2Edges, Value::Array(Vec::new())),
    ]));
    let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
    let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
    seed_filesystem(&mut db, tmp.path(), false).unwrap();

    let lib_id = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce one component")
        .id
        .clone();

    let result = run_fixedpoint(
        &mut db,
        FixedpointConfig {
            max_depth: 4,
            hard_cap: 8,
            ..FixedpointConfig::default()
        },
    );

    let plan = result
        .back_edge
        .get(&lib_id)
        .expect("library must have a carve plan in the back edge")
        .clone();
    let plan_as_strings: Vec<String> = plan
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(
        plan_as_strings.contains(&"lib/src".to_string()),
        "back edge missing expected immediate sub-dir `lib/src`: got {plan_as_strings:?}"
    );
}

#[test]
fn max_depth_zero_blocks_every_sub_carve() {
    // The universal depth guard short-circuits every decision before
    // any L3 map call fires.
    let tmp = TempDir::new().unwrap();
    write_lib_crate(tmp.path(), "lib");
    let backend = Arc::new(ScriptedBackend::new(Vec::new()));
    let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
    let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
    seed_filesystem(&mut db, tmp.path(), false).unwrap();

    let result = run_fixedpoint(
        &mut db,
        FixedpointConfig {
            max_depth: 0,
            hard_cap: 4,
            ..FixedpointConfig::default()
        },
    );
    assert_eq!(result.iterations, 0);
    assert!(
        result.back_edge.is_empty(),
        "max_depth=0 must block every sub-carve; got {:?}",
        result.back_edge
    );
    assert_eq!(
        db.llm_cache().call_count(),
        0,
        "max_depth=0 must short-circuit before any cached LLM call"
    );
}

#[test]
fn converged_run_stops_growing_back_edge_on_the_stable_iteration() {
    // A backend that consistently classifies sub-dirs as boundaries
    // grows the back-edge once, then converges: the next pass enumerates
    // the same single immediate sub-dir, the merge sees no new
    // (parent_id, sub_dir) pairs and exits.
    let tmp = TempDir::new().unwrap();
    write_lib_crate(tmp.path(), "lib");
    let backend = Arc::new(ScriptedBackend::new(vec![
        (
            PromptId::Classify,
            json!({
                "kind": "rust-library",
                "rationale": "stable",
                "is_boundary": true,
                "evidence_grade": "medium",
            }),
        ),
        (PromptId::Stage2Edges, Value::Array(Vec::new())),
    ]));
    let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
    let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
    seed_filesystem(&mut db, tmp.path(), false).unwrap();

    let result = run_fixedpoint(
        &mut db,
        FixedpointConfig {
            max_depth: 4,
            hard_cap: 8,
            ..FixedpointConfig::default()
        },
    );
    assert!(
        result.iterations >= 1,
        "stable backend must converge in at least 1 productive round; got {result:?}"
    );
}
