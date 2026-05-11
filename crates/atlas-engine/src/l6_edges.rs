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

use atlas_index::{rewrite_participant_owner_prefix, ComponentEntry, Stage, SurfacesFile};
#[cfg(test)]
use atlas_index::{EdgeAdd, EdgeSuppress, RenameMap};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use component_ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::AtlasDatabase;
use crate::l4_tree::{all_components, merged_overrides, rename_map_after_match};
use crate::l5_surface::{surface_artefacts_of, surface_of};
use crate::l6_compose_edges::composition_edges_from_compose;
use crate::l6_composition::composition_edges_from_dockerfiles;
use crate::l9_projections::surfaces_yaml_snapshot;
use crate::surface_types::SurfaceRecord;

/// Driver version baked into the L6 stage fingerprint (PR-10). Bump
/// only on a structural change in `all_proposed_edges`'s call shape;
/// cosmetic edits do not justify a bump (per the same rule as
/// `l5_surface::L5_DRIVER_VERSION`).
pub const L6_DRIVER_VERSION: &str = "1.0.0";

/// The shipped Atlas Stage 2 prompt, embedded at compile time.
pub const EMBEDDED_STAGE2_EDGES_PROMPT: &str =
    include_str!("../../../defaults/prompts/stage2-edges.md");

/// Keys produced by [`build_inputs`]. Single source of truth for the
/// bidirectional template/builder coverage check in
/// [`crate::prompt_token_coverage`]: validated against
/// `stage2-edges.md` at compile time, and against the runtime builder
/// output by a unit test.
pub(crate) const BUILD_INPUTS_KEYS: &[&str] = &["ONTOLOGY_KINDS", "SURFACE_RECORDS_YAML"];

/// L6 has no cache-only fingerprinting keys.
pub(crate) const CACHE_ONLY_KEYS: &[&str] = &[];

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

    // PR-8: deterministic contract edges from per-component
    // surfaces.yaml. Computed regardless of `live.len()` because a
    // single-component workspace that defines a contract still emits
    // its `defines-contract` / `implements-contract` edges.
    let contract_edges = contract_edges_from_surfaces(db);

    // PR-9: deterministic composition edges from Dockerfile `COPY`
    // directives. Computed regardless of `live.len()` because a
    // single-component workspace can still carry a docker-image
    // bundling external sources (none today, but the contract is
    // "composition edges flow whenever Dockerfiles exist"). The
    // result is merged with the LLM batch below before
    // canonicalisation; canonicalise_edges dedupes any overlap.
    let composition_edges = composition_edges_from_dockerfiles(db);

    // PR-11: deterministic composition edges from Docker Compose files.
    // `bundled-into` (image/build → orchestration) and `deployed-with`
    // (between co-declared services). Computed unconditionally —
    // single-component workspaces can still have a compose orchestration
    // that references external images, and those `bundled-into` edges
    // are load-bearing even without an LLM batch.
    let compose_edges = composition_edges_from_compose(db);

    if live.len() < 2 {
        // A single-component run has no pairs to consider for the
        // LLM Stage 2 batch; skip the prompt to avoid wasting tokens
        // on a no-op. Contract edges, Dockerfile composition edges, and
        // Compose edges still flow — see above.
        let mut combined: Vec<Edge> = Vec::with_capacity(
            contract_edges.len() + composition_edges.len() + compose_edges.len(),
        );
        combined.extend(contract_edges);
        combined.extend(composition_edges);
        combined.extend(compose_edges);
        apply_contract_owner_follows_to_edge_participants(db, &mut combined);
        let canonicalised = canonicalise_edges(combined);
        return Arc::new(apply_user_edge_overrides(db, canonicalised));
    }

    let surfaces: Vec<SurfaceWithId> = live
        .iter()
        .map(|c| SurfaceWithId {
            id: c.id.as_str().to_string(),
            surface: (*surface_of(db, c.id.clone())).clone(),
        })
        .collect();

    let inputs = build_inputs(&surfaces);
    let request = LlmRequest {
        prompt_template: PromptId::Stage2Edges,
        inputs: inputs.clone(),
        schema: ResponseSchema::accept_any(),
    };

    // PR-10 / PR-11: L6 stage fingerprint per design §8.1. Contributors:
    //
    // - `analyzer_registry_sha` — registry-shape change invalidates;
    // - `llm_fingerprint` — model / backend / template / ontology;
    // - `prompt_sha` — sha of the canonical-JSON inputs (the
    //   serialised surface records the backend will see);
    // - `file_content_sha` for every component path segment whose
    //   surface contributed to the batch — propagates a per-file
    //   change up to the batch;
    // - `participant_surface_sha` per participant component whose
    //   `SurfacesFile` carries contract content (PR-11, design §8.2).
    //   Components with no contract content contribute nothing so the
    //   fingerprint shape is byte-identical to PR-10 for workspaces
    //   with no contracts (no-contract stability invariant).
    let workspace = db.workspace();
    let llm_fp = workspace
        .llm_fingerprint(db as &dyn salsa::Database)
        .clone();
    let rendered_prompt_sha = sha256_hex_bytes(
        serde_json::to_string(&inputs)
            .unwrap_or_default()
            .as_bytes(),
    );
    let registry_sha = db.analyzer_registry().registry_sha();
    let l6_fingerprint =
        compute_l6_batch_fingerprint(db, &live, &rendered_prompt_sha, &registry_sha, &llm_fp);

    let value = match db.call_llm_cached_with_fp(Stage::L6, &l6_fingerprint, &request) {
        Ok(v) => v,
        Err(_) => {
            // LLM call failed — return only the deterministic edges
            // (contract + Dockerfile composition + Compose composition).
            let mut combined: Vec<Edge> = Vec::with_capacity(
                contract_edges.len() + composition_edges.len() + compose_edges.len(),
            );
            combined.extend(contract_edges);
            combined.extend(composition_edges);
            combined.extend(compose_edges);
            apply_contract_owner_follows_to_edge_participants(db, &mut combined);
            let canonicalised = canonicalise_edges(combined);
            return Arc::new(apply_user_edge_overrides(db, canonicalised));
        }
    };

    let mut parsed = parse_edges_response(&value).unwrap_or_default();
    // PR-8/PR-11: merge deterministic edges (contract, Dockerfile
    // composition, Compose composition) with the LLM batch before
    // canonicalisation.  Deterministic edges go first so that on a
    // `(kind, lifecycle, participants)` collision the deterministic edge
    // wins (`canonicalise_edges` keeps the first insertion per canonical
    // key).  Order: contract → Dockerfile composition → Compose
    // composition → LLM batch (ascending confidence precedence).
    let mut combined: Vec<Edge> = Vec::with_capacity(
        contract_edges.len() + composition_edges.len() + compose_edges.len() + parsed.len(),
    );
    combined.extend(contract_edges);
    combined.extend(composition_edges);
    combined.extend(compose_edges);
    combined.append(&mut parsed);
    apply_contract_owner_follows_to_edge_participants(db, &mut combined);
    let canonicalised = canonicalise_edges(combined);
    Arc::new(apply_user_edge_overrides(db, canonicalised))
}

/// Phase 6 PR-2: apply the contract rename-match owner-follows rewrite
/// to every edge participant. When the L4 rename-match seam produced a
/// non-identity `prior_id A → new_id B` entry, participants whose
/// string begins with `A/` are rewritten to begin with `B/`. The map is
/// empty in the common case (rename-match preserves the prior id under
/// the path-derived id allocator), so the helper exits early on the
/// `rename_map.is_empty()` check inside [`rewrite_participant_owner_prefix`].
///
/// The rewrite runs *before* [`canonicalise_edges`] so symmetric-kind
/// participants get re-sorted as part of canonicalisation. The rewrite
/// also runs before [`apply_user_edge_overrides`] so user-authored
/// `edges_add` / `edges_suppress` entries see post-rewrite ids.
fn apply_contract_owner_follows_to_edge_participants(db: &AtlasDatabase, edges: &mut Vec<Edge>) {
    let rename_map = rename_map_after_match(db);
    if rename_map.is_empty() {
        return;
    }
    for edge in edges {
        for participant in &mut edge.participants {
            rewrite_participant_owner_prefix(participant, &rename_map);
        }
    }
}

/// Test-only pure form of the participant-rewrite pass — takes the
/// rename map directly so unit tests can exercise the rewrite without
/// a database. Kept under `cfg(test)` so it never compiles into the
/// shipped library.
#[cfg(test)]
fn apply_contract_owner_follows_to_edge_participants_for_tests(
    edges: &mut Vec<Edge>,
    rename_map: &RenameMap,
) {
    if rename_map.is_empty() {
        return;
    }
    for edge in edges {
        for participant in &mut edge.participants {
            rewrite_participant_owner_prefix(participant, rename_map);
        }
    }
}

/// Apply hand-authored `edges_add` / `edges_suppress` from the
/// merged `OverridesFile` (Phase 3 PR-6 / design §5.5) to the
/// analyser-discovered edge set.
///
/// Order of operations:
///
/// 1. **Union with `edges_add`.** Each entry is materialised as an
///    [`Edge`] and pushed to the working set. Lifecycle defaults to
///    [`LifecycleScope::Design`] (the open vocabulary's neutral
///    scope); evidence grade is `Strong` because the source is a
///    deliberate human authoring; the rationale carries the
///    user-supplied `reason` so the audit trail survives.
/// 2. **Subtract `edges_suppress`.** Each entry matches by exact
///    `(kind, from, to)` triple — kind is parsed through
///    [`EdgeKind::parse`], participants are matched as-strings.
///    Match is over the *post-add* set (so a suppress that names a
///    user-added edge drops both the analyser-discovered AND
///    user-added edge for that triple, consistent with "subtract
///    after union").
/// 3. **No-match warnings.** A suppress entry that matches no
///    analyser-or-add edge logs a single warning to stderr — the
///    operation is otherwise a no-op (the spec is explicit that
///    suppress is forgiving, not strict).
/// 4. **Suppress-after-add forensics.** When the same `(kind, from,
///    to)` appears in both add and suppress, the suppress wins (the
///    edge is dropped) AND an `info:` line records both reasons so
///    a later reader can reconstruct the intent.
///
/// `db` is passed because the merged-overrides walk re-reads the
/// per-component files from disk (see
/// [`crate::l4_tree::merged_overrides`]). The walk is cheap
/// (filesystem read + parse); a future PR can wrap it in
/// `#[salsa::tracked]` if profiling demands it.
///
/// The returned vector is re-canonicalised after suppress so that
/// user-added edges with un-sorted participants for symmetric kinds
/// land in canonical form.
fn apply_user_edge_overrides(db: &AtlasDatabase, mut edges: Vec<Edge>) -> Vec<Edge> {
    let overrides = merged_overrides(db);
    let edges_add = &overrides.edges_add;
    let edges_suppress = &overrides.edges_suppress;
    if edges_add.is_empty() && edges_suppress.is_empty() {
        return edges;
    }

    // Step 1: union with edges_add. Skip entries with an unknown
    // EdgeKind so a typo in the user's overrides surfaces as a
    // missing edge rather than a silent corruption of the edge set.
    for add in edges_add {
        let Some(kind) = EdgeKind::parse(&add.kind) else {
            eprintln!(
                "warning: edges_add entry has unknown kind `{}` for [{} -> {}]; entry dropped",
                add.kind, add.from, add.to,
            );
            continue;
        };
        let mut participants = vec![add.from.clone(), add.to.clone()];
        if !kind.is_directed() {
            participants.sort();
        }
        edges.push(Edge {
            kind,
            lifecycle: LifecycleScope::Design,
            participants,
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["overrides.yaml:edges_add".to_string()],
            rationale: format!(
                "user-authored edges_add entry: {} (reason: {})",
                add.kind, add.reason
            ),
        });
    }

    // Step 2: subtract edges_suppress. Match is exact on the
    // (kind, from, to) triple. Symmetric kinds canonicalise their
    // participants lex-asc; we apply the same rule to the suppress
    // input so a user who wrote `from: B, to: A` for a symmetric
    // kind still matches the analyser-emitted `[A, B]` participants.
    for suppress in edges_suppress {
        let Some(kind) = EdgeKind::parse(&suppress.kind) else {
            eprintln!(
                "warning: edges_suppress entry has unknown kind `{}` for [{} -> {}]; entry dropped",
                suppress.kind, suppress.from, suppress.to,
            );
            continue;
        };
        let mut want_participants = vec![suppress.from.clone(), suppress.to.clone()];
        if !kind.is_directed() {
            want_participants.sort();
        }
        let initial_len = edges.len();
        // Find any matching analyser/add-derived entries before
        // removal so the forensics log can cite their reasons.
        let mut matching_add_reasons: Vec<String> = Vec::new();
        for e in &edges {
            if e.kind == kind && e.participants == want_participants {
                // Heuristic: an add-sourced edge has the
                // `overrides.yaml:edges_add` evidence field. Use
                // that to recover the add's reason text from the
                // rationale.
                if e.evidence_fields
                    .iter()
                    .any(|f| f == "overrides.yaml:edges_add")
                {
                    matching_add_reasons.push(e.rationale.clone());
                }
            }
        }
        edges.retain(|e| !(e.kind == kind && e.participants == want_participants));
        let removed = initial_len - edges.len();
        if removed == 0 {
            eprintln!(
                "warning: edges_suppress entry [{} {} -> {}] matched no analyser-discovered edge",
                suppress.kind, suppress.from, suppress.to,
            );
        } else if !matching_add_reasons.is_empty() {
            // Suppress-after-add: log both reasons for the audit
            // trail. The matching add(s) carry their own reason in
            // the rationale; attach the suppress reason explicitly.
            for add_rationale in &matching_add_reasons {
                eprintln!(
                    "info: edges_add overridden by edges_suppress for [{} {} -> {}] \
                     (add: {}; suppress reason: {})",
                    suppress.kind, suppress.from, suppress.to, add_rationale, suppress.reason,
                );
            }
        }
    }

    // Re-canonicalise so any user-added edges that fail validation
    // get dropped and any duplicate-key entries collapse — the
    // surrounding pipeline already does this once on the analyser
    // batch, but the override pass added new edges below the
    // canonicalisation seam.
    canonicalise_edges(edges)
}

/// Test-only pure form of [`apply_user_edge_overrides`] — takes
/// the merged overrides directly so tests can exercise the
/// union/subtract semantics without a database. Kept under
/// `cfg(test)` so it never compiles into the shipped library.
#[cfg(test)]
pub(crate) fn apply_user_edge_overrides_for_tests(
    mut edges: Vec<Edge>,
    edges_add: &[EdgeAdd],
    edges_suppress: &[EdgeSuppress],
) -> Vec<Edge> {
    if edges_add.is_empty() && edges_suppress.is_empty() {
        return edges;
    }
    for add in edges_add {
        let Some(kind) = EdgeKind::parse(&add.kind) else {
            continue;
        };
        let mut participants = vec![add.from.clone(), add.to.clone()];
        if !kind.is_directed() {
            participants.sort();
        }
        edges.push(Edge {
            kind,
            lifecycle: LifecycleScope::Design,
            participants,
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["overrides.yaml:edges_add".to_string()],
            rationale: format!(
                "user-authored edges_add entry: {} (reason: {})",
                add.kind, add.reason
            ),
        });
    }
    for suppress in edges_suppress {
        let Some(kind) = EdgeKind::parse(&suppress.kind) else {
            continue;
        };
        let mut want_participants = vec![suppress.from.clone(), suppress.to.clone()];
        if !kind.is_directed() {
            want_participants.sort();
        }
        edges.retain(|e| !(e.kind == kind && e.participants == want_participants));
    }
    canonicalise_edges(edges)
}

/// Build every deterministic contract edge implied by each live
/// component's surface artefacts.
///
/// For each component:
///
/// - **`defines-contract`** — one edge per entry in the
///   component's `SurfaceArtefacts.contracts` list (code-derived
///   contracts the component defines).
/// - **`implements-contract`** — one edge per defined contract (the
///   defining component is also the defining-binding implementer of
///   its own contracts, per design §6.3).
///
/// `consumes-contract` edges are *not* emitted here; they flow
/// through the LLM Stage 2 batch (the model already knows the kind
/// via `{{ONTOLOGY_KINDS}}`). See the PR-8 architectural context for
/// why.
///
/// Components that have no code-derived contracts contribute nothing.
/// The result is **not** canonicalised — the caller feeds it through
/// [`canonicalise_edges`] alongside the LLM batch.
pub fn contract_edges_from_surfaces(db: &AtlasDatabase) -> Vec<Edge> {
    let components = all_components(db);
    let live = components.iter().filter(|c| !c.deleted);
    let mut out: Vec<Edge> = Vec::new();

    for entry in live {
        let component_id_str = entry.id.as_str().to_string();
        // `surface_artefacts_of` is not Salsa-tracked, so each call
        // re-walks the component's path segments and re-runs the
        // deterministic Rust-surface analyser. The cost is bounded by
        // the number of live components × per-component source size and
        // is acceptable for Phase 1 workspaces. A future PR can wrap it
        // in `#[salsa::tracked]` if profiling demands it; that change is
        // outside PR-8's scope.
        let artefacts = surface_artefacts_of(db, entry.id.clone());

        // defines-contract edges: one per code-derived contract.
        for contract in &artefacts.contracts {
            out.push(Edge {
                kind: EdgeKind::DefinesContract,
                lifecycle: LifecycleScope::Design,
                participants: vec![component_id_str.clone(), contract.id.clone()],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["surfaces.yaml:contracts_defined".to_string()],
                rationale: format!(
                    "component `{}` lists contract `{}` under contracts_defined",
                    component_id_str, contract.id
                ),
            });
            // The defining component is also the defining-binding
            // implementer (design §6.3).
            out.push(Edge {
                kind: EdgeKind::ImplementsContract,
                lifecycle: LifecycleScope::Design,
                participants: vec![component_id_str.clone(), contract.id.clone()],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["surfaces.yaml:contracts_implemented".to_string()],
                rationale: format!(
                    "component `{}` lists contract `{}` under contracts_implemented",
                    component_id_str, contract.id
                ),
            });
        }
    }

    out
}

/// Surface record bundled with its component id — the shape the
/// Stage 2 prompt's `{{SURFACE_RECORDS_YAML}}` block expects.
#[derive(Debug, Clone, Serialize)]
struct SurfaceWithId {
    id: String,
    surface: SurfaceRecord,
}

fn build_inputs(surfaces: &[SurfaceWithId]) -> Value {
    // Render the surfaces as a YAML fragment for the
    // `{{SURFACE_RECORDS_YAML}}` token; `{{ONTOLOGY_KINDS}}` comes from
    // the embedded ontology so the model sees the same vocabulary the
    // parser validates against.
    let surfaces_yaml = serde_yaml::to_string(&SurfacesWrapper { surfaces })
        .unwrap_or_else(|_| String::from("surfaces: []\n"));
    let ontology_block = component_ontology::render_embedded_kinds_for_prompt().unwrap_or_default();

    json!({
        "ONTOLOGY_KINDS": ontology_block,
        "SURFACE_RECORDS_YAML": surfaces_yaml,
    })
}

/// Parameterless wrapper for the unified prompt/builder coverage
/// matrix in [`crate::prompt_token_coverage`]. Constructs a minimal
/// pair of stub surfaces so the matrix can call all four builders
/// uniformly.
#[cfg(test)]
pub(crate) fn build_inputs_with_stubs_for_tests() -> Value {
    let surfaces = vec![
        SurfaceWithId {
            id: "alpha".into(),
            surface: crate::surface_types::SurfaceRecord::default(),
        },
        SurfaceWithId {
            id: "beta".into(),
            surface: crate::surface_types::SurfaceRecord::default(),
        },
    ];
    build_inputs(&surfaces)
}

#[derive(Debug, Serialize)]
struct SurfacesWrapper<'a> {
    surfaces: &'a [SurfaceWithId],
}

/// SHA-256 of `bytes` rendered as 64-character lowercase hex. Used by
/// the L6 fingerprint construction to derive a `prompt_sha` from the
/// canonical-JSON inputs (the serialised surface records).
fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut out = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut out, "{b:02x}").expect("writing to String never fails");
    }
    out
}

/// Returns `true` when a `SurfacesFile` carries at least one contract
/// or library-API entry — i.e. when its surface fingerprint is
/// load-bearing for cross-component cache invalidation.
///
/// Components whose surfaces carry **no** contract content contribute
/// nothing to the L6 batch fingerprint via
/// `add_participant_surface_sha`. This preserves the no-contract
/// stability invariant (acceptance criterion #3 of PR-11): workspaces
/// with no contracts produce the same L6 fingerprint before and after
/// PR-11 because no `participant_surface_sha` calls fire.
fn surface_has_contract_content(sf: &SurfacesFile) -> bool {
    !sf.contracts_defined.is_empty()
        || !sf.contracts_implemented.is_empty()
        || !sf.contracts_consumed.is_empty()
        || !sf.library_apis.is_empty()
}

/// Build the L6 batch fingerprint from its constituent inputs.
///
/// This is a pure function (no side effects; takes the pre-computed
/// `prompt_sha`, `registry_sha`, and `llm_fp` as arguments) so that
/// the stability acceptance test can call it in isolation without
/// wiring a full `AtlasDatabase`.
///
/// Contributors (design §8.1 and §8.2):
/// - `analyzer_registry_sha` — registry-shape change invalidates all
///   downstream cache entries.
/// - `llm_fingerprint` — model / backend / template / ontology drift.
/// - `prompt_sha` — sha of the canonical-JSON inputs (the serialised
///   surface records the backend will see).
/// - `file_content_sha` per component path segment — propagates a
///   per-file change up to the batch.
/// - `participant_surface_sha` per live component whose `SurfacesFile`
///   carries contract content (PR-11, design §8.2). Components with
///   empty surfaces contribute nothing (no-contract stability
///   invariant).
pub fn compute_l6_batch_fingerprint(
    db: &AtlasDatabase,
    live: &[&ComponentEntry],
    prompt_sha: &str,
    registry_sha: &str,
    llm_fp: &atlas_llm::LlmFingerprint,
) -> crate::Sha256Hex {
    let mut fb = crate::FingerprintBuilder::new(Stage::L6, "l6-driver", L6_DRIVER_VERSION);
    fb.add_analyzer_registry_sha(&registry_sha.to_string());
    fb.add_llm_fingerprint(llm_fp);
    fb.add_prompt_sha(&prompt_sha.to_string());
    for c in live {
        for seg in &c.path_segments {
            fb.add_file_content_sha(&seg.content_sha);
        }
    }
    // PR-11: contribute the surfaces fingerprint of every participant
    // component that carries contract content. Skip components whose
    // surfaces are empty — that preserves the no-contract stability
    // invariant per acceptance criterion #3.
    for c in live {
        if let Ok(sf) = surfaces_yaml_snapshot(db, &c.id) {
            if surface_has_contract_content(&sf) {
                fb.add_participant_surface_sha(&sf.fingerprint);
            }
        }
    }
    fb.finalise()
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
                return Err("expected top-level array or object with `edges` key".to_string());
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
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
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
    // §4 PR-11: Dockerfile-derived and Compose-derived composition edges
    // must be interleaved in lexicographic order.  A stable sort on the
    // canonical key (kind, lifecycle, participants) achieves this while
    // preserving the relative order of edges that share the same key —
    // so deterministic edges that were inserted before LLM edges continue
    // to win the "first insertion wins" dedup below.
    let mut edges = edges;
    edges.sort_by(|a, b| {
        let ka = a.canonical_key();
        let kb = b.canonical_key();
        (ka.0.as_str(), ka.1.as_str(), ka.2).cmp(&(kb.0.as_str(), kb.1.as_str(), kb.2))
    });

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
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn stage2_edges_prompt_exposes_required_substitution_tokens() {
        for token in ["{{ONTOLOGY_KINDS}}", "{{SURFACE_RECORDS_YAML}}"] {
            assert!(
                EMBEDDED_STAGE2_EDGES_PROMPT.contains(token),
                "stage2-edges.md must expose `{token}` — the engine's \
                 `build_inputs` populates it and the prompt is expected \
                 to reference it"
            );
        }
    }

    #[test]
    fn stage2_edges_prompt_has_no_ravel_lite_protocol_remnants() {
        // The Ravel-Lite-era prompt told the model to shell out to
        // `ravel-lite state discover-proposals add-proposal` once per
        // edge. Atlas's `ClaudeCodeBackend` expects a JSON array on
        // stdout; those instructions are now wrong and must stay gone.
        for forbidden in ["CONFIG_ROOT", "ravel-lite", "Ravel-Lite", "add-proposal"] {
            assert!(
                !EMBEDDED_STAGE2_EDGES_PROMPT.contains(forbidden),
                "stage2-edges.md still contains `{forbidden}` — this is \
                 a leftover Ravel-Lite protocol reference that conflicts \
                 with Atlas's JSON-on-stdout contract"
            );
        }
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

    // Bidirectional token coverage between stage2-edges.md and
    // build_inputs is enforced at compile time by
    // `prompt_token_coverage.rs`.

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
        seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();
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
            .map(|c| c.id.as_str().to_string())
            .collect();
        for id in &ids {
            let peer_ids: Vec<String> = ids.iter().filter(|p| *p != id).cloned().collect();
            let entry = components.iter().find(|c| c.id.as_str() == id).unwrap();
            let inputs = crate::l5_surface::build_inputs_for_tests(entry, &peer_ids);
            backend.respond(PromptId::Stage1Surface, inputs, minimal_surface(id));
        }
        ids
    }

    fn cid(s: &str) -> component_ontology::ComponentId {
        component_ontology::ComponentId::parse(s).unwrap()
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
                surface: (*surface_of(&db, cid(id))).clone(),
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
                surface: (*surface_of(&db, cid(id))).clone(),
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
                surface: (*surface_of(&db, cid(id))).clone(),
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
        seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

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
        assert_eq!(
            got.len(),
            1,
            "malformed entries are dropped, not propagated"
        );
    }

    fn defines_contract_edge(component: &str, contract: &str) -> Edge {
        Edge {
            kind: EdgeKind::DefinesContract,
            lifecycle: LifecycleScope::Design,
            participants: vec![component.to_string(), contract.to_string()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["surfaces.yaml:contracts_defined".to_string()],
            rationale: format!("test fixture: {component} defines {contract}"),
        }
    }

    #[test]
    fn participant_rewrite_for_tests_noop_under_empty_map() {
        let mut edges = vec![defines_contract_edge("a", "a/c1")];
        let map: RenameMap = RenameMap::new();
        apply_contract_owner_follows_to_edge_participants_for_tests(&mut edges, &map);
        assert_eq!(
            edges[0].participants,
            vec!["a".to_string(), "a/c1".to_string()]
        );
    }

    #[test]
    fn participant_rewrite_for_tests_rewrites_contract_id_prefix_only() {
        // The owner-follows rewrite cascades only into participants
        // whose string starts with `<prior_id>/`. Bare component-id
        // participants (e.g. the component participant of a
        // defines-contract edge) are NOT rewritten by this pass —
        // component-id stabilisation is the rename-match's own job in
        // L4, not the owner-follows pass.
        let mut edges = vec![defines_contract_edge("a", "a/c1")];
        let mut map: RenameMap = RenameMap::new();
        map.insert(cid("a"), cid("b"));
        apply_contract_owner_follows_to_edge_participants_for_tests(&mut edges, &map);
        assert_eq!(
            edges[0].participants,
            vec!["a".to_string(), "b/c1".to_string()],
            "the contract participant rewrites to the new owner-prefix; \
             the bare component participant is left alone because its \
             string does not match `<prior_id>/` (it has no trailing slash)"
        );
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

    // ---------------------------------------------------------------
    // PR-6: edges_add / edges_suppress unit tests on the pure helper.
    // The db-driven entry point is exercised by the engine
    // integration tests under `crates/atlas-engine/tests/`.
    // ---------------------------------------------------------------

    fn add(kind: &str, from: &str, to: &str, reason: &str) -> EdgeAdd {
        EdgeAdd {
            kind: kind.into(),
            from: from.into(),
            to: to.into(),
            reason: reason.into(),
        }
    }

    fn suppress(kind: &str, from: &str, to: &str, reason: &str) -> EdgeSuppress {
        EdgeSuppress {
            kind: kind.into(),
            from: from.into(),
            to: to.into(),
            reason: reason.into(),
        }
    }

    fn analyser_edge(kind: EdgeKind, from: &str, to: &str) -> Edge {
        let mut participants = vec![from.to_string(), to.to_string()];
        if !kind.is_directed() {
            participants.sort();
        }
        Edge {
            kind,
            lifecycle: LifecycleScope::Design,
            participants,
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["analyser.x".into()],
            rationale: "analyser-derived".into(),
        }
    }

    #[test]
    fn edges_add_inserts_a_new_edge() {
        let analyser = vec![analyser_edge(EdgeKind::DependsOn, "alpha", "beta")];
        let out = apply_user_edge_overrides_for_tests(
            analyser,
            &[add("bundled-into", "alpha", "gamma", "manual")],
            &[],
        );
        // analyser depends-on + user-added bundled-into = 2 edges.
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|e| e.kind == EdgeKind::BundledInto
            && e.participants == vec!["alpha".to_string(), "gamma".to_string()]));
    }

    #[test]
    fn edges_suppress_removes_analyser_edge() {
        let analyser = vec![
            analyser_edge(EdgeKind::DependsOn, "alpha", "beta"),
            analyser_edge(EdgeKind::DependsOn, "alpha", "gamma"),
        ];
        let out = apply_user_edge_overrides_for_tests(
            analyser,
            &[],
            &[suppress("depends-on", "alpha", "beta", "false-positive")],
        );
        // beta edge dropped; gamma edge survives.
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].participants,
            vec!["alpha".to_string(), "gamma".into()]
        );
    }

    #[test]
    fn edges_suppress_after_add_drops_both_user_and_analyser_edges() {
        let analyser = vec![analyser_edge(EdgeKind::DependsOn, "alpha", "beta")];
        let out = apply_user_edge_overrides_for_tests(
            analyser,
            &[add("depends-on", "alpha", "beta", "ensure")],
            &[suppress("depends-on", "alpha", "beta", "actually no")],
        );
        // Subtract-after-union: the suppress drops every (kind,
        // from, to)-matching edge regardless of source.
        assert!(
            out.is_empty(),
            "suppress-after-add must drop both user-added and analyser edges, got {out:?}"
        );
    }

    #[test]
    fn edges_suppress_no_match_leaves_set_unchanged() {
        let analyser = vec![analyser_edge(EdgeKind::DependsOn, "alpha", "beta")];
        let out = apply_user_edge_overrides_for_tests(
            analyser.clone(),
            &[],
            &[suppress("depends-on", "no-such-from", "no-such-to", "x")],
        );
        // The non-matching suppress is a no-op — the analyser edge
        // survives. (The pure helper does not emit warnings; the
        // db-driven entry point logs to stderr instead.)
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].participants, analyser[0].participants);
    }

    #[test]
    fn edges_add_with_unknown_kind_is_dropped_silently() {
        let analyser = vec![analyser_edge(EdgeKind::DependsOn, "alpha", "beta")];
        let out = apply_user_edge_overrides_for_tests(
            analyser,
            &[add("not-a-real-kind", "alpha", "gamma", "x")],
            &[],
        );
        // Unknown kind dropped; analyser set untouched.
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn symmetric_edge_add_canonicalises_participants() {
        // `co-implements` is a symmetric kind. The user authored
        // `from: B, to: A`; the canonicalised form sorts to `[A, B]`.
        let out = apply_user_edge_overrides_for_tests(
            vec![],
            &[add("co-implements", "beta", "alpha", "x")],
            &[],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EdgeKind::CoImplements);
        assert_eq!(
            out[0].participants,
            vec!["alpha".to_string(), "beta".to_string()],
            "symmetric kind must have sorted participants after the override pass"
        );
    }

    #[test]
    fn symmetric_edge_suppress_canonicalises_participants() {
        // Mirror of `symmetric_edge_add_canonicalises_participants`
        // for the suppress side. The analyser emits a `co-implements`
        // edge with already-canonicalised participants `[alpha, beta]`
        // (because non-directed kinds are sorted at construction time
        // — see `analyser_edge`). The user authors `edges_suppress`
        // with `from: beta, to: alpha` (reversed). The suppress must
        // canonicalise its own participants before the
        // `(kind, participants)` match runs, otherwise it would fail
        // to remove the analyser edge. This is the regression guard
        // for the `want_participants.sort()` call in the production
        // path at `l6_edges.rs:280-281`.
        let analyser = vec![analyser_edge(EdgeKind::CoImplements, "alpha", "beta")];
        let out = apply_user_edge_overrides_for_tests(
            analyser,
            &[],
            &[suppress("co-implements", "beta", "alpha", "false-positive")],
        );
        assert!(
            out.is_empty(),
            "symmetric-kind suppress with reversed participants must canonicalise \
             before matching and remove the analyser edge; got {out:?}"
        );
    }

    #[test]
    fn empty_overrides_return_input_unchanged() {
        let analyser = vec![
            analyser_edge(EdgeKind::DependsOn, "alpha", "beta"),
            analyser_edge(EdgeKind::DependsOn, "gamma", "delta"),
        ];
        let out = apply_user_edge_overrides_for_tests(analyser.clone(), &[], &[]);
        assert_eq!(out, analyser);
    }
}
