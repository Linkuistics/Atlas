//! PR-12 acceptance: atlas-contracts visible in Ravel-Lite.
//!
//! End-to-end smoke test that exercises the full Phase 1 seam in one
//! flow: multi-root via Cargo path-dep walking (PR-4), scattered
//! `.atlas/` (PR-2), per-component `surfaces.yaml` (PR-7), the
//! contract-edge family (PR-8 + L6), the persistent content-addressed
//! cache (PR-10), and L6's participant-surface-sha cache key (PR-11).
//!
//! The fixture mirrors the real Ravel-Lite + atlas-contracts layout:
//! a primary root (`ravel-lite`) with a single consumer crate that
//! `path = "../../atlas-contracts"`-deps a sibling repository whose
//! crate defines a serde-derived `pub struct Foo`. PR-4's
//! `expand_roots` discovers the peer root via canonicalised path-dep
//! walking; PR-7's rust-surface analyser detects `Foo` and emits an
//! `atlas-contracts/foo` `data-format` contract; the canned LLM
//! Stage 2 response synthesises a `consumes-contract` edge from the
//! consumer to that contract id; PR-8's validator round-trips the id
//! against `contracts_defined`.
//!
//! ## Acceptance criteria (plan §4 PR-12)
//!
//! 1. `components.yaml` lists components from both roots.
//! 2. `related-components.yaml` carries a `consumes-contract` edge
//!    from the consumer to a contract id under the
//!    `atlas-contracts/` namespace.
//! 3. The atlas-contracts component's `surfaces.yaml` lists that
//!    contract id under `contracts_defined`.
//! 4. A no-op re-run makes zero LLM calls (persistent cache hit).
//! 5. Editing the defining binding's source invalidates only the L6
//!    batch and atlas-contracts's L5 — the consumer's L5 entry still
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

const CONSUMER_ID: &str = "consumer-crate";
const CONTRACTS_ID: &str = "atlas-contracts";
const CONTRACT_ID: &str = "atlas-contracts/foo";

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [21u8; 32],
        ontology_sha: [22u8; 32],
        model_id: "pr12-test-backend".into(),
        backend_version: "v-pr12".into(),
    }
}

/// Backend that returns valid canned responses for every prompt and
/// logs every call so tests can assert per-PromptId counts.
///
/// `Stage2Edges` returns a single `consumes-contract` edge from the
/// consumer to the atlas-contracts/foo contract — the canonical edge
/// PR-12 is built around.  Because `Edge::validate` requires the two
/// participants to be distinct and the participants come back unchanged
/// to the validator, the canned response also satisfies §9.5.
struct PR12Backend {
    fingerprint: LlmFingerprint,
    /// `(PromptId, canonical inputs JSON)` per call.  The inputs are
    /// retained so AC#5 can attribute Stage1Surface calls to the
    /// affected component via the embedded `COMPONENT_ID` field
    /// (the same surface used by `persistent_cache_lifecycle.rs`).
    call_log: Mutex<Vec<(PromptId, String)>>,
}

impl PR12Backend {
    fn new() -> Arc<Self> {
        Arc::new(PR12Backend {
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
    /// contains `"COMPONENT_ID":"<id>"`.  Used by AC#5 to prove that
    /// editing atlas-contracts's source did NOT invalidate the
    /// consumer's L5 entry.
    fn surface_calls_for(&self, component_id: &str) -> usize {
        let needle = format!("\"COMPONENT_ID\":\"{component_id}\"");
        self.call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, inputs)| *p == PromptId::Stage1Surface && inputs.contains(&needle))
            .count()
    }
}

impl LlmBackend for PR12Backend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let inputs_canonical = serde_json::to_string(&req.inputs).unwrap_or_default();
        self.call_log
            .lock()
            .unwrap()
            .push((req.prompt_template, inputs_canonical));

        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "rust-library",
                "language": "rust",
                "build_system": "cargo",
                "evidence_grade": "medium",
                "evidence_fields": [],
                "rationale": "pr12 backend default",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({
                "purpose": "pr12 backend stage-1 stub",
                "notes": "",
            }),
            PromptId::Stage2Edges => json!([{
                "kind": "consumes-contract",
                "lifecycle": "design",
                "participants": [CONSUMER_ID, CONTRACT_ID],
                "evidence_grade": "strong",
                "evidence_fields": ["consumer-crate.uses-foo"],
                "rationale": "consumer-crate references atlas_contracts::Foo",
            }]),
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

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Materialise the Ravel-Lite + atlas-contracts layout under `parent`:
///
/// ```text
/// <parent>/
///   ravel-lite/                         <- primary root
///     consumer-crate/
///       Cargo.toml                      (path-deps ../../atlas-contracts)
///       src/lib.rs                      (uses atlas_contracts::Foo)
///   atlas-contracts/                    <- peer root, discovered by PR-4
///     Cargo.toml                        ([package] + serde dep)
///     src/lib.rs                        (#[derive(Serialize, Deserialize)] pub struct Foo)
/// ```
///
/// The path-dep walk in `expand_roots` canonicalises
/// `<ravel-lite>/consumer-crate/../../atlas-contracts` to
/// `<parent>/atlas-contracts`, finds it outside the primary root,
/// and adds it as a peer — the integration point PR-4 was designed
/// for.  No manual `--additional-root` is required.
fn write_fixture(parent: &Path) {
    let consumer_dir = parent.join("ravel-lite/consumer-crate");
    std::fs::create_dir_all(consumer_dir.join("src")).unwrap();
    std::fs::write(
        consumer_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"consumer-crate\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         name = \"consumer_crate\"\n\
         path = \"src/lib.rs\"\n\
         \n\
         [dependencies]\n\
         atlas-contracts = { path = \"../../atlas-contracts\" }\n",
    )
    .unwrap();
    std::fs::write(
        consumer_dir.join("src/lib.rs"),
        "//! Consumer of atlas_contracts::Foo.\n\
         use atlas_contracts::Foo;\n\
         \n\
         /// Sample function that touches the contract type so the\n\
         /// reference is real, even though Stage 2 is canned.\n\
         pub fn read_foo(f: &Foo) -> u32 { f.x }\n",
    )
    .unwrap();

    let contracts_dir = parent.join("atlas-contracts");
    std::fs::create_dir_all(contracts_dir.join("src")).unwrap();
    std::fs::write(
        contracts_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"atlas-contracts\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         name = \"atlas_contracts\"\n\
         path = \"src/lib.rs\"\n\
         \n\
         [dependencies]\n\
         serde = { version = \"1\", features = [\"derive\"] }\n",
    )
    .unwrap();
    std::fs::write(
        contracts_dir.join("src/lib.rs"),
        "//! Phase-1 schema crate.\n\
         #[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct Foo { pub x: u32 }\n",
    )
    .unwrap();
}

fn base_config(primary: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(primary.to_path_buf());
    config.output_dir = primary.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

fn run_with(config: &IndexConfig, backend: Arc<PR12Backend>) {
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
fn fixture_produces_expected_components_edges_and_surfaces() {
    let parent = TempDir::new().unwrap();
    write_fixture(parent.path());
    let primary = parent.path().join("ravel-lite");
    let config = base_config(&primary);

    // Cold run — exercises the full pipeline; one round of Classify /
    // Stage1Surface / Stage2Edges / Subcarve calls per component (the
    // precise count depends on which deterministic short-circuits fire,
    // which is not load-bearing for this test).
    let backend = PR12Backend::new();
    run_with(&config, backend);

    // ---- AC#1 — components.yaml lists components from both roots ----
    let components_path = primary.join(".atlas/components.yaml");
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

    // The `roots` field must list both directories; PR-4's expand_roots
    // canonicalises so we compare canonicalised paths.
    let roots_canonical: Vec<_> = parsed
        .roots
        .iter()
        .map(|r| r.canonicalize().unwrap_or_else(|_| r.clone()))
        .collect();
    let primary_canonical = primary.canonicalize().unwrap();
    let peer_canonical = parent
        .path()
        .join("atlas-contracts")
        .canonicalize()
        .unwrap();
    assert!(
        roots_canonical.contains(&primary_canonical),
        "components.yaml roots must include the primary root; got {:?}",
        parsed.roots
    );
    assert!(
        roots_canonical.contains(&peer_canonical),
        "components.yaml roots must include the atlas-contracts peer root \
         discovered via path-dep walking; got {:?}",
        parsed.roots
    );

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
    let contracts_component = parsed
        .components
        .iter()
        .find(|c| c.id.as_str() == CONTRACTS_ID)
        .unwrap_or_else(|| {
            panic!(
                "expected an `{CONTRACTS_ID}` component in components.yaml; got ids: {:?}",
                parsed
                    .components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert!(!consumer.deleted, "consumer must be live");
    assert!(!contracts_component.deleted, "atlas-contracts must be live");

    // ---- AC#2 — related-components.yaml carries the consumes-contract edge ---
    let related_path = primary.join(".atlas/related-components.yaml");
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
                    .map(|s| s.starts_with(&format!("{CONTRACTS_ID}/")))
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a consumes-contract edge from `{CONSUMER_ID}` to a contract under \
                 `{CONTRACTS_ID}/`; got: {:?}",
                consumes.iter().map(|e| &e.participants).collect::<Vec<_>>()
            )
        });
    let contract_id_in_edge = edge_to_foo.participants[1].clone();
    assert_eq!(
        contract_id_in_edge, CONTRACT_ID,
        "the consumes-contract edge's contract participant must be `{CONTRACT_ID}`"
    );

    // ---- AC#3 — atlas-contracts/.atlas/surfaces.yaml lists the contract ----
    let surfaces_path = parent.path().join("atlas-contracts/.atlas/surfaces.yaml");
    assert!(
        surfaces_path.exists(),
        "atlas-contracts must have a per-component surfaces.yaml at {}",
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
        CONTRACTS_ID,
        "surfaces.yaml component_id must match the atlas-contracts component"
    );
    assert!(
        !surfaces.contracts_defined.is_empty(),
        "atlas-contracts surfaces.yaml must list at least one defined contract; got empty"
    );
    let foo_contract = surfaces
        .contracts_defined
        .iter()
        .find(|c| c.id == contract_id_in_edge)
        .unwrap_or_else(|| {
            panic!(
                "atlas-contracts/contracts_defined must include `{contract_id_in_edge}` \
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
fn cache_hit_on_no_op_rerun_and_invalidates_on_defining_binding_edit() {
    let parent = TempDir::new().unwrap();
    write_fixture(parent.path());
    let primary = parent.path().join("ravel-lite");
    let config = base_config(&primary);

    // ---- Cold run — populates the persistent cache ----
    let cold = PR12Backend::new();
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
    let warm = PR12Backend::new();
    run_with(&config, warm.clone());
    assert_eq!(
        warm.total(),
        0,
        "no-op re-run must hit the persistent cache for every entry; \
         actual calls: {:?}",
        warm.calls()
    );

    // The on-disk persistent cache must exist by now (PR-10 design §5.5).
    assert!(
        config.output_dir.join("cache").exists(),
        "persistent cache root must exist after a successful run"
    );

    // ---- AC#5 — edit defining binding → only the consumer's L5 survives ----
    //
    // Editing atlas-contracts's source changes the file bytes, so:
    //  - atlas-contracts's L5 fingerprint cites the new file shas →
    //    Stage1Surface for atlas-contracts must miss.
    //  - The L6 batch fingerprint cites every component's segment shas
    //    AND PR-11's participant_surface_sha → Stage2Edges must miss.
    //  - consumer-crate's source did NOT change, its
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
        parent.path().join("atlas-contracts/src/lib.rs"),
        "//! Phase-1 schema crate (edited by test).\n\
         #[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct Foo { pub x: u32, pub y: u64 }\n",
    )
    .unwrap();

    let edited = PR12Backend::new();
    run_with(&config, edited.clone());

    // The L6 batch must miss because PR-11's
    // `add_participant_surface_sha` cites atlas-contracts's surface
    // sha, which changed when `Foo`'s body changed.
    assert!(
        edited.count(PromptId::Stage2Edges) >= 1,
        "after editing atlas-contracts/src/lib.rs, the L6 batch must miss the persistent \
         cache and Stage2Edges must be invoked at least once; got {} Stage2Edges calls. \
         Full call log: {:?}",
        edited.count(PromptId::Stage2Edges),
        edited.calls()
    );

    // atlas-contracts's L5 entry must miss (its content_sha changed).
    let contracts_l5 = edited.surface_calls_for(CONTRACTS_ID);
    assert!(
        contracts_l5 >= 1,
        "atlas-contracts's Stage1Surface entry must miss after its source file changed; \
         got {contracts_l5} calls for `{CONTRACTS_ID}`. Full call log: {:?}",
        edited.calls()
    );

    // consumer-crate's L5 entry must HIT (its content_sha is
    // unaffected by the atlas-contracts edit).  This is the
    // load-bearing partial-invalidation claim of AC#5.
    let consumer_l5 = edited.surface_calls_for(CONSUMER_ID);
    assert_eq!(
        consumer_l5,
        0,
        "consumer-crate's Stage1Surface entry must hit the persistent cache (its content \
         sha is unaffected by the atlas-contracts edit); got {consumer_l5} calls for \
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
