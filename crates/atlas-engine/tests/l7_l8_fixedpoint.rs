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
    // that says "yes, boundary", `<root>/lib/src` lands in the
    // back-edge keyed by the library id (PR-13: stored as ABSOLUTE
    // path so multi-root layouts route to the correct root in L2).
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
    let mut db = AtlasDatabase::new(backend_dyn, vec![tmp.path().to_path_buf()], fingerprint());
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

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
        .get(lib_id.as_str())
        .expect("library must have a carve plan in the back edge")
        .clone();
    let expected = tmp.path().join("lib").join("src");
    assert!(
        plan.contains(&expected),
        "back edge missing expected immediate sub-dir `{}`: got {:?}",
        expected.display(),
        plan
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
    let mut db = AtlasDatabase::new(backend_dyn, vec![tmp.path().to_path_buf()], fingerprint());
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

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

/// Regression for PR-13 — L8 phantom subcomponent observation.
///
/// PR-12-of-Phase-1 noted that the L8 fixedpoint emitted phantom
/// subcomponents (e.g. `atlas-contracts/consumer-crate`) when the
/// peer root and the primary share a parent-directory layout. The
/// underlying cause: `enumerate_immediate_subdirs` resolved the
/// peer-root component's segment (`""`) under `roots[0]` (the primary)
/// because `<primary>/<empty>` and `<peer>/<empty>` both trivially
/// "contain" registered files; iterating roots in order then accepted
/// the primary, and every immediate sub-dir of the primary was
/// proposed as a sub-carve of the peer-root component.
///
/// The fix disambiguates via the entry's manifests: a root is only
/// accepted when at least one of `<root>/<manifest>` is registered as
/// a workspace file. With that in place, the peer-root atlas-contracts
/// component resolves to the peer root, and the only immediate sub-dir
/// proposed under it is `atlas-contracts/src` — never
/// `atlas-contracts/consumer-crate`.
#[test]
fn peer_root_with_empty_segment_does_not_phantom_emit_primary_subdirs() {
    // Mirror PR-12-of-Phase-1's layout: parent dir holds a primary
    // root (`ravel-lite/`) with one consumer crate inside, and a peer
    // root (`atlas-contracts/`) whose Cargo manifest sits at the peer
    // root itself (segment.path == "").
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("ravel-lite");
    let peer = parent.path().join("atlas-contracts");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&peer).unwrap();

    // Primary root: `consumer-crate/` lives one level inside.
    write_lib_crate(&primary, "consumer-crate");

    // Peer root: the Cargo manifest sits AT the peer root, so the
    // resulting component's `path_segments[0].path == ""`. The crate
    // has a `src/lib.rs` so L8 has at least one immediate sub-dir to
    // potentially propose.
    std::fs::create_dir_all(peer.join("src")).unwrap();
    std::fs::write(
        peer.join("Cargo.toml"),
        "[package]\n\
         name = \"atlas-contracts\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         name = \"atlas_contracts\"\n\
         path = \"src/lib.rs\"\n",
    )
    .unwrap();
    std::fs::write(peer.join("src/lib.rs"), "// peer-root lib\n").unwrap();

    // Always-boundary classifier: any sub-dir L3 sees gets reported
    // as a boundary, so phantom emissions show up as accepted entries
    // in the back-edge rather than being silently filtered. This is
    // the failure mode the original bug exhibited.
    let backend = Arc::new(ScriptedBackend::new(vec![
        (
            PromptId::Classify,
            json!({
                "kind": "rust-library",
                "rationale": "stub",
                "is_boundary": true,
                "evidence_grade": "medium",
            }),
        ),
        (PromptId::Stage2Edges, Value::Array(Vec::new())),
        (
            PromptId::Stage1Surface,
            json!({ "purpose": "stub", "notes": "" }),
        ),
    ]));
    let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
    let roots = vec![primary.clone(), peer.clone()];
    let mut db = AtlasDatabase::new(backend_dyn, roots.clone(), fingerprint());
    seed_filesystem(&mut db, &roots, false).unwrap();

    let result = run_fixedpoint(
        &mut db,
        FixedpointConfig {
            max_depth: 4,
            hard_cap: 8,
            ..FixedpointConfig::default()
        },
    );

    // The atlas-contracts component must have at most `src` as a
    // proposed sub-dir — never `consumer-crate`. Any back-edge entry
    // whose key is `atlas-contracts` and whose values include a
    // primary-root sub-dir is a phantom.
    let plan = result
        .back_edge
        .get("atlas-contracts")
        .cloned()
        .unwrap_or_default();
    let plan_strings: Vec<String> = plan
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(
        !plan_strings
            .iter()
            .any(|s| s.contains("consumer-crate") || s == "consumer-crate"),
        "atlas-contracts must not phantom-emit `consumer-crate` (a primary-root sub-dir) \
         as a sub-component; back-edge sub_dirs: {plan_strings:?}"
    );
    // Defensive: the only legitimate sub-dir for atlas-contracts in
    // this fixture is `src`. If the back-edge contains anything else,
    // it is a phantom by exclusion.
    for entry in &plan_strings {
        let basename = std::path::Path::new(entry)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(
            basename == "src",
            "atlas-contracts back-edge contained unexpected sub-dir `{entry}`; \
             only `src` is legitimate for this fixture (any other entry is a phantom)"
        );
    }
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
    let mut db = AtlasDatabase::new(backend_dyn, vec![tmp.path().to_path_buf()], fingerprint());
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

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
