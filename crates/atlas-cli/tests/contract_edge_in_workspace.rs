//! Contract-edge family in a single-root workspace.
//!
//! End-to-end smoke test that exercises the Phase 1 PR-12 acceptance
//! flow against the post-Phase-5 single-root model: a consumer crate
//! has a Cargo path-dep on a sibling crate (both workspace members of
//! the same root). The rust-surface analyser detects the sibling's
//! `pub struct Foo` → emits a `data-format` contract under
//! `<sibling-id>/foo`. L6's contract-edge batch emits a
//! `consumes-contract` edge from the consumer to that contract id.
//!
//! ## Acceptance criteria (salvaged from the deleted Phase 1 PR-12
//! test atlas_contracts_in_ravel_lite.rs lines 21-36)
//!
//! 1. `components.yaml` lists both crates' components.
//! 2. `related-components.yaml` carries a `consumes-contract` edge
//!    from the consumer to the schema crate's contract id.
//! 3. The schema crate's `surfaces.yaml` lists that contract id under
//!    `contracts_defined`.
//! 4. A no-op re-run makes zero LLM calls (persistent cache hit).
//! 5. Editing the defining binding's source invalidates only the L6
//!    batch and the schema crate's L5 — the consumer's L5 entry still
//!    hits the persistent cache, and the deterministic Cargo-registry
//!    classifier short-circuits L3 so no Classify call fires.
//!
//! Two `#[test]` functions split the work: the first verifies AC#1+2+3
//! against a single cold run; the second walks AC#4+5 via cold → no-op
//! → edit → re-run.

use std::path::Path;
use std::sync::{Arc, Mutex};

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_index::{ComponentsFile, ContractKind, SurfacesFile};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use component_ontology::{EdgeKind, RelatedComponentsFile};
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

const CONSUMER_ID: &str = "consumer";
const SCHEMA_ID: &str = "schema-crate";
const CONTRACT_ID: &str = "schema-crate/foo";

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [31u8; 32],
        ontology_sha: [32u8; 32],
        model_id: "pr4-salvage-test-backend".into(),
        backend_version: "v-pr4-salvage".into(),
    }
}

/// Backend that returns valid canned responses for every prompt and
/// logs every call so tests can assert per-PromptId counts.
///
/// `Stage2Edges` returns a single `consumes-contract` edge from the
/// consumer to the schema-crate/foo contract — the canonical edge this
/// test is built around. Because `Edge::validate` requires the two
/// participants to be distinct and the participants come back unchanged
/// to the validator, the canned response also satisfies §9.5.
struct PR4Backend {
    fingerprint: LlmFingerprint,
    /// `(PromptId, canonical inputs JSON)` per call. The inputs are
    /// retained so AC#5 can attribute Stage1Surface calls to the
    /// affected component via the embedded `COMPONENT_ID` field.
    call_log: Mutex<Vec<(PromptId, String)>>,
}

impl PR4Backend {
    fn new() -> Arc<Self> {
        Arc::new(PR4Backend {
            fingerprint: fingerprint(),
            call_log: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(PromptId, String)> {
        self.call_log.lock().unwrap().clone()
    }

    fn count(&self, p: PromptId) -> usize {
        self.call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(prompt, _)| *prompt == p)
            .count()
    }

    fn total(&self) -> usize {
        self.call_log.lock().unwrap().len()
    }

    /// Number of Stage1Surface calls whose canonical inputs JSON
    /// has a `COMPONENT_ID` field equal to `component_id`. Used by AC#5
    /// to prove that editing schema-crate's source did NOT invalidate
    /// the consumer's L5 entry.
    ///
    /// Parses the canonical inputs JSON via `serde_json::from_str` and
    /// looks up the structured `COMPONENT_ID` field — robust against
    /// key-ordering / whitespace changes in Stage 1's input
    /// canonicaliser. Calls whose inputs are not parseable JSON or
    /// whose top level is not an object are skipped (they cannot match
    /// any component id by construction).
    fn surface_calls_for(&self, component_id: &str) -> usize {
        self.call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, inputs)| {
                if *p != PromptId::Stage1Surface {
                    return false;
                }
                let parsed: Value = match serde_json::from_str(inputs) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                parsed.get("COMPONENT_ID").and_then(Value::as_str) == Some(component_id)
            })
            .count()
    }
}

#[async_trait::async_trait]
impl LlmBackend for PR4Backend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let inputs_canonical = serde_json::to_string(&req.inputs).unwrap_or_default();
        self.call_log.lock().unwrap().push((
            req.prompt_template
                .expect("test backend services templated requests"),
            inputs_canonical,
        ));

        Ok(
            match req
                .prompt_template
                .expect("test backend services templated requests")
            {
                PromptId::Classify => json!({
                    "kind": "rust-library",
                    "language": "rust",
                    "build_system": "cargo",
                    "evidence_grade": "medium",
                    "evidence_fields": [],
                    "rationale": "pr4-salvage backend default",
                    "is_boundary": true,
                }),
                PromptId::Stage1Surface => json!({
                    "purpose": "pr4-salvage backend stage-1 stub",
                    "notes": "",
                }),
                PromptId::Stage2Edges => json!([{
                    "kind": "consumes-contract",
                    "lifecycle": "design",
                    "participants": [CONSUMER_ID, CONTRACT_ID],
                    "evidence_grade": "strong",
                    "evidence_fields": ["consumer.uses-foo"],
                    "rationale": "consumer references schema_crate::Foo",
                }]),
                PromptId::Subcarve => json!({
                    "should_subcarve": false,
                    "sub_dirs": [],
                    "rationale": "policy declined",
                }),
            },
        )
    }

    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        self.call(req)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Materialise a single-root two-crate workspace under `tmp`:
///
/// ```text
/// <tmp>/                        <- workspace root (no top-level Cargo.toml)
///   consumer/
///     Cargo.toml                (path-dep on ../schema-crate)
///     src/lib.rs                (uses schema_crate::Foo)
///   schema-crate/
///     Cargo.toml                ([package] + serde dep)
///     src/lib.rs                (#[derive(Serialize, Deserialize)] pub struct Foo)
/// ```
///
/// Both crates live under the same root — the single-root model
/// introduced in Phase 5. No peer-root discovery (the old `expand_roots`
/// path) is needed; the engine discovers both crates by scanning for
/// `Cargo.toml` files under the root. No workspace-level `Cargo.toml`
/// is written at the root (adding one would generate a spurious
/// `kind: workspace` component for the tempdir itself).
fn write_fixture(tmp: &Path) {
    // consumer crate.
    let consumer_dir = tmp.join("consumer");
    std::fs::create_dir_all(consumer_dir.join("src")).unwrap();
    std::fs::write(
        consumer_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"consumer\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         name = \"consumer\"\n\
         path = \"src/lib.rs\"\n\
         \n\
         [dependencies]\n\
         schema-crate = { path = \"../schema-crate\" }\n",
    )
    .unwrap();
    std::fs::write(
        consumer_dir.join("src/lib.rs"),
        "//! Consumer of schema_crate::Foo.\n\
         use schema_crate::Foo;\n\
         \n\
         /// Sample function that touches the contract type so the\n\
         /// reference is real, even though Stage 2 is canned.\n\
         pub fn read_foo(f: &Foo) -> u32 { f.x }\n",
    )
    .unwrap();

    // schema-crate (the contract-defining crate).
    let schema_dir = tmp.join("schema-crate");
    std::fs::create_dir_all(schema_dir.join("src")).unwrap();
    std::fs::write(
        schema_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"schema-crate\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         name = \"schema_crate\"\n\
         path = \"src/lib.rs\"\n\
         \n\
         [dependencies]\n\
         serde = { version = \"1\", features = [\"derive\"] }\n",
    )
    .unwrap();
    std::fs::write(
        schema_dir.join("src/lib.rs"),
        "//! Phase-1 schema crate.\n\
         #[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct Foo { pub x: u32 }\n",
    )
    .unwrap();
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

fn run_with(config: &IndexConfig, backend: Arc<PR4Backend>) {
    run_index(
        config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed");
}

// ---------------------------------------------------------------------------
// AC #1, #2, #3 — structural assertions on the output artefacts
// ---------------------------------------------------------------------------

#[test]
fn ac_1_2_3_components_edge_and_surfaces() {
    let tmp = TempDir::new().unwrap();
    write_fixture(tmp.path());
    let config = base_config(tmp.path());

    // Cold run — exercises the full pipeline; one round of Classify /
    // Stage1Surface / Stage2Edges / Subcarve calls per component (the
    // precise count depends on which deterministic short-circuits fire,
    // which is not load-bearing for this test).
    let backend = PR4Backend::new();
    run_with(&config, backend);

    // ---- AC#1 — components.yaml lists both crates' components ----
    let components_path = tmp.path().join(".atlas/cache/components.yaml");
    assert!(
        components_path.exists(),
        "components.yaml must exist at {}",
        components_path.display()
    );
    let bytes = std::fs::read(&components_path).unwrap();
    let parsed: ComponentsFile = serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as ComponentsFile: {e}",
            components_path.display()
        )
    });

    let consumer = parsed
        .components
        .iter()
        .find(|c| c.id.as_str() == CONSUMER_ID)
        .unwrap_or_else(|| {
            panic!(
                "expected a `{CONSUMER_ID}` component in components.yaml; got ids: {:?}",
                parsed
                    .components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    let schema_component = parsed
        .components
        .iter()
        .find(|c| c.id.as_str() == SCHEMA_ID)
        .unwrap_or_else(|| {
            panic!(
                "expected a `{SCHEMA_ID}` component in components.yaml; got ids: {:?}",
                parsed
                    .components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert!(!consumer.deleted, "consumer must be live");
    assert!(!schema_component.deleted, "schema-crate must be live");

    // ---- AC#2 — related-components.yaml carries the consumes-contract edge ---
    let related_path = tmp.path().join(".atlas/cache/related-components.yaml");
    assert!(
        related_path.exists(),
        "related-components.yaml must exist at {}",
        related_path.display()
    );
    let related_bytes = std::fs::read(&related_path).unwrap();
    let related: RelatedComponentsFile =
        serde_yaml::from_slice(&related_bytes).unwrap_or_else(|e| {
            panic!(
                "failed to parse {} as RelatedComponentsFile: {e}",
                related_path.display()
            )
        });
    let consumes: Vec<_> = related
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::ConsumesContract)
        .collect();
    assert!(
        !consumes.is_empty(),
        "related-components.yaml must carry at least one consumes-contract edge; \
         got edges: {:?}",
        related
            .edges
            .iter()
            .map(|e| (e.kind.as_str(), &e.participants))
            .collect::<Vec<_>>()
    );
    let edge_to_foo = consumes
        .iter()
        .find(|e| {
            e.participants.first().map(String::as_str) == Some(CONSUMER_ID)
                && e.participants
                    .get(1)
                    .map(|s| s.starts_with(&format!("{SCHEMA_ID}/")))
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a consumes-contract edge from `{CONSUMER_ID}` to a contract under \
                 `{SCHEMA_ID}/`; got: {:?}",
                consumes.iter().map(|e| &e.participants).collect::<Vec<_>>()
            )
        });
    let contract_id_in_edge = edge_to_foo.participants[1].clone();
    assert_eq!(
        contract_id_in_edge, CONTRACT_ID,
        "the consumes-contract edge's contract participant must be `{CONTRACT_ID}`"
    );

    // ---- AC#3 — schema-crate/.atlas/cache/surfaces.yaml lists the contract ----
    let surfaces_path = tmp.path().join("schema-crate/.atlas/cache/surfaces.yaml");
    assert!(
        surfaces_path.exists(),
        "schema-crate must have a per-component cache/surfaces.yaml at {}",
        surfaces_path.display()
    );
    let surfaces_bytes = std::fs::read(&surfaces_path).unwrap();
    let surfaces: SurfacesFile = serde_yaml::from_slice(&surfaces_bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as SurfacesFile: {e}",
            surfaces_path.display()
        )
    });
    assert_eq!(
        surfaces.component_id.as_str(),
        SCHEMA_ID,
        "surfaces.yaml component_id must match the schema-crate component"
    );
    assert!(
        !surfaces.contracts_defined.is_empty(),
        "schema-crate surfaces.yaml must list at least one defined contract; got empty"
    );
    let foo_contract = surfaces
        .contracts_defined
        .iter()
        .find(|c| c.id == contract_id_in_edge)
        .unwrap_or_else(|| {
            panic!(
                "schema-crate/contracts_defined must include `{contract_id_in_edge}` \
                 (the same id as the consumes-contract edge's contract participant); \
                 got: {:?}",
                surfaces
                    .contracts_defined
                    .iter()
                    .map(|c| &c.id)
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        foo_contract.kind,
        ContractKind::DataFormat,
        "the `Foo` contract must be `data-format`"
    );
    assert_eq!(
        foo_contract.definition_binding.symbol, "Foo",
        "the defining binding's symbol must be `Foo`"
    );
}

// ---------------------------------------------------------------------------
// AC #4, #5 — persistent cache hit and partial invalidation
// ---------------------------------------------------------------------------

#[test]
fn ac_4_5_cache_hit_on_no_op_rerun_and_invalidates_on_defining_binding_edit() {
    let tmp = TempDir::new().unwrap();
    write_fixture(tmp.path());
    let config = base_config(tmp.path());

    // ---- Cold run — populates the persistent cache ----
    let cold = PR4Backend::new();
    run_with(&config, cold.clone());
    let cold_total = cold.total();
    assert!(
        cold_total > 0,
        "cold run must exercise the backend at least once for the test to be meaningful; \
         got {cold_total} calls"
    );
    let cold_stage2 = cold.count(PromptId::Stage2Edges);
    assert!(
        cold_stage2 >= 1,
        "cold run must invoke Stage2Edges at least once (the L6 batch was a miss); \
         got {cold_stage2}"
    );

    // ---- AC#4 — no-op re-run hits the persistent cache for every entry ----
    let warm = PR4Backend::new();
    run_with(&config, warm.clone());
    assert_eq!(
        warm.total(),
        0,
        "no-op re-run must hit the persistent cache for every entry; \
         actual calls: {:?}",
        warm.calls()
    );

    // The on-disk persistent cache must exist by now (PR-10 design §5.4).
    assert!(
        config.output_dir.join("cache").exists(),
        "persistent cache root must exist after a successful run"
    );

    // ---- AC#5 — edit defining binding → only the consumer's L5 survives ----
    //
    // Editing schema-crate's source changes the file bytes, so:
    //  - schema-crate's L5 fingerprint cites the new file shas →
    //    Stage1Surface for schema-crate must miss.
    //  - The L6 batch fingerprint cites every component's segment shas
    //    AND PR-11's participant_surface_sha → Stage2Edges must miss.
    //  - consumer's source did NOT change, its
    //    `path_segments[0].content_sha` is unchanged, and its L5
    //    fingerprint is per-component → Stage1Surface for the consumer
    //    must HIT (this is the load-bearing partial-invalidation claim).
    //  - Cargo's deterministic registry classifier short-circuits L3 →
    //    no Classify calls fire (PR-10's note).
    //
    // We assert the precise per-PromptId shape; the brittle
    // alternative (assert "fewer total calls than cold") would silently
    // accept a regression where the consumer's L5 is also re-run.
    std::fs::write(
        tmp.path().join("schema-crate/src/lib.rs"),
        "//! Phase-1 schema crate (edited by test).\n\
         #[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct Foo { pub x: u32, pub y: u64 }\n",
    )
    .unwrap();

    let edited = PR4Backend::new();
    run_with(&config, edited.clone());

    // The L6 batch must miss because PR-11's
    // `add_participant_surface_sha` cites schema-crate's surface
    // sha, which changed when `Foo`'s body changed.
    assert!(
        edited.count(PromptId::Stage2Edges) >= 1,
        "after editing schema-crate/src/lib.rs, the L6 batch must miss the persistent \
         cache and Stage2Edges must be invoked at least once; got {} Stage2Edges calls. \
         Full call log: {:?}",
        edited.count(PromptId::Stage2Edges),
        edited.calls()
    );

    // schema-crate's L5 entry must miss (its content_sha changed).
    let schema_l5 = edited.surface_calls_for(SCHEMA_ID);
    assert!(
        schema_l5 >= 1,
        "schema-crate's Stage1Surface entry must miss after its source file changed; \
         got {schema_l5} calls for `{SCHEMA_ID}`. Full call log: {:?}",
        edited.calls()
    );

    // consumer's L5 entry must HIT (its content_sha is
    // unaffected by the schema-crate edit). This is the
    // load-bearing partial-invalidation claim of AC#5.
    let consumer_l5 = edited.surface_calls_for(CONSUMER_ID);
    assert_eq!(
        consumer_l5,
        0,
        "consumer's Stage1Surface entry must hit the persistent cache (its content \
         sha is unaffected by the schema-crate edit); got {consumer_l5} calls for \
         `{CONSUMER_ID}`. Full call log: {:?}",
        edited.calls()
    );

    // L3 must not re-run: deterministic Cargo classifier short-circuits.
    assert_eq!(
        edited.count(PromptId::Classify),
        0,
        "Classify must not fire on the post-edit run — the deterministic Cargo registry \
         classifier short-circuits L3 (PR-10's note); got {} Classify calls",
        edited.count(PromptId::Classify)
    );

    // The post-edit run must do strictly less work than the cold run.
    assert!(
        edited.total() < cold_total,
        "post-edit run must do strictly less LLM work than the cold run; \
         cold={cold_total}, post-edit={}",
        edited.total()
    );
}
