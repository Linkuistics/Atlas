//! Impact query — `atlas impact <id>`, output to stdout (no file
//! written; the `--no-write` flag is intentionally rejected by the
//! CLI).
//!
//! Schema is fixed by Phase 3 design spec §4.2. PR-7 ships only the
//! report types and a stubbed [`impact`] entry-point that returns
//! [`ReportError::NotImplemented`]; PR-9 lands the actual traversal.

use std::collections::{BTreeMap, BTreeSet};

use atlas_index::ComponentEntry;
use chrono::{DateTime, Utc};
use component_ontology::{Edge, EdgeKind};
use serde::{Deserialize, Serialize};

use crate::types::{ImpactTarget, ReportError, ReportInputs};

/// Bucket key used when a transitive consumer carries no value on a
/// partition axis (no language, no deploy graph, no lifecycle role).
const UNKNOWN_BUCKET: &str = "unknown";

/// Walk downstream consumers of a contract or component, returning
/// direct + transitive consumer sets and three independent partitions
/// (by language, deploy graph, lifecycle).
///
/// Resolution order: contract namespace first, component namespace
/// second. If neither resolves, [`ReportError::TargetNotFound`] is
/// returned with the Levenshtein-1 candidates the CLI handler renders
/// to stderr.
///
/// Traversal walks `consumes-contract` edges only — `depends-on` build
/// edges are out of scope per design §4.2. Cycle safety is provided by
/// a `BTreeSet<ComponentId>` seen-set: a component already enqueued is
/// not re-walked.
///
/// **Inputs source.** Per design §3.1 / §3.3, reports observe
/// "whatever the engine has already produced" without triggering
/// recomputation. We read [`atlas_engine::Workspace::prior_components`]
/// and [`atlas_engine::Workspace::prior_related_components`] — the
/// snapshots populated by [`atlas_engine::AtlasDatabase::set_prior_components`]
/// / [`atlas_engine::AtlasDatabase::set_prior_related_components`]
/// from the most recent `atlas index` write. In Phase 5 those workspace
/// inputs become file-watcher-driven Salsa inputs; the function shape
/// here stays identical so the migration is mechanical.
pub fn impact(inputs: ReportInputs, target: ImpactTarget) -> Result<ImpactReport, ReportError> {
    let workspace = inputs.workspace;
    let db: &dyn salsa::Database = inputs.db;
    let components_file = workspace.prior_components(db);
    let related_file = workspace.prior_related_components(db);
    impact_pure(
        &components_file.components,
        &related_file.edges,
        target,
        Utc::now(),
    )
}

/// Pure helper that does the impact traversal over already-extracted
/// components and edges. Split out from [`impact`] so unit tests can
/// drive the algorithm with hand-built fixtures (no `AtlasDatabase`,
/// no filesystem, no LLM backend).
///
/// `now` is injected so tests can pin `generated_at`.
fn impact_pure(
    components: &[ComponentEntry],
    edges: &[Edge],
    target: ImpactTarget,
    now: DateTime<Utc>,
) -> Result<ImpactReport, ReportError> {
    // Build the live (non-deleted) component slice once; every later
    // step iterates this view.
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();

    // ── 1. Build look-up tables we need repeatedly ───────────────────
    // `defines-contract` edges encode "this component defines this
    // contract". Participants are `[component_id, contract_id]`.
    let mut component_to_provided_contracts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut all_contract_ids: BTreeSet<String> = BTreeSet::new();
    for e in edges {
        if e.kind == EdgeKind::DefinesContract && e.participants.len() == 2 {
            let component = e.participants[0].clone();
            let contract = e.participants[1].clone();
            component_to_provided_contracts
                .entry(component)
                .or_default()
                .push(contract.clone());
            all_contract_ids.insert(contract);
        }
    }
    // `consumes-contract` edges encode "this component consumes this
    // contract". Participants are `[component_id, contract_id]`.
    let mut contract_to_consumers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in edges {
        if e.kind == EdgeKind::ConsumesContract && e.participants.len() == 2 {
            let component = e.participants[0].clone();
            let contract = e.participants[1].clone();
            contract_to_consumers
                .entry(contract)
                .or_default()
                .push(component);
        }
    }

    let live_component_ids: BTreeSet<String> =
        live.iter().map(|c| c.id.as_str().to_string()).collect();

    // ── 2. Resolve the target ────────────────────────────────────────
    let (target_kind, target_id) = match target {
        ImpactTarget::Contract(id) => {
            if !all_contract_ids.contains(&id) {
                return Err(ReportError::TargetNotFound {
                    needle: id.clone(),
                    candidates: levenshtein_distance_1_candidates(
                        &id,
                        &all_ids_for_suggestions(&live_component_ids, &all_contract_ids),
                    ),
                });
            }
            (ImpactReportTargetKind::Contract, id)
        }
        ImpactTarget::Component(id) => {
            let id_str = id.as_str().to_string();
            if !live_component_ids.contains(&id_str) {
                return Err(ReportError::TargetNotFound {
                    needle: id_str.clone(),
                    candidates: levenshtein_distance_1_candidates(
                        &id_str,
                        &all_ids_for_suggestions(&live_component_ids, &all_contract_ids),
                    ),
                });
            }
            (ImpactReportTargetKind::Component, id_str)
        }
    };

    // ── 3. Determine the seed contracts to walk consumers of ─────────
    // Contract input: a single contract; the seed is `[id]`.
    // Component input: every contract the component provides.
    let seed_contracts: Vec<String> = match target_kind {
        ImpactReportTargetKind::Contract => vec![target_id.clone()],
        ImpactReportTargetKind::Component => component_to_provided_contracts
            .get(&target_id)
            .cloned()
            .unwrap_or_default(),
    };

    // ── 4. Direct consumers: union of `contract_to_consumers` over
    //      `seed_contracts`, deduped, in stable lex order. ────────────
    let mut direct_set: BTreeSet<String> = BTreeSet::new();
    for c in &seed_contracts {
        if let Some(consumers) = contract_to_consumers.get(c) {
            for cid in consumers {
                direct_set.insert(cid.clone());
            }
        }
    }
    let direct_consumers: Vec<String> = direct_set.iter().cloned().collect();

    // ── 5. Transitive walk. BFS with a seen-set keyed on component id.
    //      For each visited component, collect every contract it
    //      provides and enqueue their consumers. ──────────────────────
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = direct_consumers.clone();
    // Seed the seen-set with direct consumers up front so we do not
    // re-enqueue them via another contract path.
    for c in &queue {
        seen.insert(c.clone());
    }
    while let Some(component_id) = queue.pop() {
        // For every contract this component defines, walk its consumers.
        let provided = match component_to_provided_contracts.get(&component_id) {
            Some(v) => v,
            None => continue,
        };
        for contract in provided {
            let Some(consumers) = contract_to_consumers.get(contract) else {
                continue;
            };
            for next in consumers {
                if seen.insert(next.clone()) {
                    queue.push(next.clone());
                }
            }
        }
    }
    let transitive_consumers: Vec<String> = seen.iter().cloned().collect();

    // ── 6. Partitions over `transitive_consumers`. ───────────────────
    let deploy_graph_membership = compute_deploy_graph_membership(edges);
    let component_lookup: BTreeMap<String, &ComponentEntry> = live
        .iter()
        .map(|c| (c.id.as_str().to_string(), *c))
        .collect();

    let mut by_language: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_deploy_graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_lifecycle: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for cid in &transitive_consumers {
        let entry = component_lookup.get(cid);

        // Language axis: a component appears under each of its
        // declared languages; empty `languages` falls into the
        // `unknown` bucket.
        match entry {
            Some(e) if !e.languages.is_empty() => {
                for lang in &e.languages {
                    by_language
                        .entry(lang.clone())
                        .or_default()
                        .push(cid.clone());
                }
            }
            _ => {
                by_language
                    .entry(UNKNOWN_BUCKET.to_string())
                    .or_default()
                    .push(cid.clone());
            }
        }

        // Lifecycle axis: a component appears under each of its
        // declared lifecycle roles (kebab-cased via
        // `LifecycleScope::as_str`); empty roles → `unknown`.
        match entry {
            Some(e) if !e.lifecycle_roles.is_empty() => {
                for role in &e.lifecycle_roles {
                    by_lifecycle
                        .entry(role.as_str().to_string())
                        .or_default()
                        .push(cid.clone());
                }
            }
            _ => {
                by_lifecycle
                    .entry(UNKNOWN_BUCKET.to_string())
                    .or_default()
                    .push(cid.clone());
            }
        }

        // Deploy-graph axis: a component appears under every
        // compose-orchestration component whose graph it participates
        // in (via `bundled-into` or `deployed-with`); none → `unknown`.
        let memberships = deploy_graph_membership.get(cid);
        match memberships {
            Some(set) if !set.is_empty() => {
                for graph in set {
                    by_deploy_graph
                        .entry(graph.clone())
                        .or_default()
                        .push(cid.clone());
                }
            }
            _ => {
                by_deploy_graph
                    .entry(UNKNOWN_BUCKET.to_string())
                    .or_default()
                    .push(cid.clone());
            }
        }
    }
    // BTreeMap iteration order is already stable; the per-bucket
    // vectors are insertion-ordered against the BTreeSet-sorted
    // `transitive_consumers`, so the on-the-wire YAML is byte-stable.
    sort_partition(&mut by_language);
    sort_partition(&mut by_deploy_graph);
    sort_partition(&mut by_lifecycle);

    // ── 7. Assemble the report. ──────────────────────────────────────
    let summary = ImpactSummary {
        direct_count: direct_consumers.len() as u32,
        transitive_count: transitive_consumers.len() as u32,
    };
    Ok(ImpactReport {
        schema_version: 1,
        generated_at: now,
        target: ImpactReportTarget {
            kind: target_kind,
            id: target_id,
        },
        direct_consumers,
        transitive_consumers,
        partitions: ImpactPartitions {
            by_language,
            by_deploy_graph,
            by_lifecycle,
        },
        summary,
    })
}

/// Build the per-component deploy-graph membership map. A component is
/// "in" a deploy graph identified by an orchestration component's id
/// when there is a `bundled-into` edge from the component to the
/// orchestration, or when a `deployed-with` edge connects it to a peer
/// that bundles into the same orchestration.
///
/// We model it as: for every `bundled-into` edge `[source, orch]`,
/// `source` is a member of `orch`'s graph. This captures the canonical
/// case (Compose `image:` / `build:`); `deployed-with` membership falls
/// out transitively because every `deployed-with` peer also has its own
/// `bundled-into` edge into the same orchestration (per
/// `composition_edges_from_compose`).
fn compute_deploy_graph_membership(edges: &[Edge]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in edges {
        if e.kind == EdgeKind::BundledInto && e.participants.len() == 2 {
            let source = e.participants[0].clone();
            let orchestration = e.participants[1].clone();
            out.entry(source).or_default().insert(orchestration);
        }
    }
    out
}

/// Sort each bucket's vector lex-ascending so duplicates collapse and
/// output is deterministic across runs.
fn sort_partition(map: &mut BTreeMap<String, Vec<String>>) {
    for v in map.values_mut() {
        v.sort();
        v.dedup();
    }
}

/// Build the suggestion-candidate id pool: live component ids plus
/// known contract ids, lex-sorted, deduped.
fn all_ids_for_suggestions(
    live_component_ids: &BTreeSet<String>,
    all_contract_ids: &BTreeSet<String>,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for id in live_component_ids {
        out.insert(id.clone());
    }
    for id in all_contract_ids {
        out.insert(id.clone());
    }
    out.into_iter().collect()
}

/// Return ids in `all_ids` that are exactly Levenshtein-distance-1 from
/// `needle` (one insertion, one deletion, or one substitution). Result
/// is lex-sorted, deduped.
///
/// The Phase 3 plan limits this to distance-1 deliberately — distance-2
/// suggestions add noise without much pay-off when the user mistyped a
/// component / contract id with a long path prefix. No external crate
/// is pulled in for this helper; the inline implementation is O(n × m)
/// where n is the candidate-pool size and m is the average id length,
/// which is fine at typical workspace sizes.
fn levenshtein_distance_1_candidates(needle: &str, all_ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in all_ids {
        if levenshtein_distance_one(needle, id) {
            out.push(id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `true` iff `a` and `b` are exactly one edit (insert / delete /
/// substitute) apart. Walks the strings as `char` slices so multi-byte
/// UTF-8 ids (theoretically allowed, even if Atlas's id grammar is
/// ASCII-only) are handled correctly.
fn levenshtein_distance_one(a: &str, b: &str) -> bool {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let (longer, shorter) = if av.len() >= bv.len() {
        (&av, &bv)
    } else {
        (&bv, &av)
    };
    let len_diff = longer.len() - shorter.len();
    if len_diff > 1 {
        return false;
    }

    if len_diff == 0 {
        // Same length → exactly one substitution.
        let mut diffs = 0;
        for (x, y) in av.iter().zip(bv.iter()) {
            if x != y {
                diffs += 1;
                if diffs > 1 {
                    return false;
                }
            }
        }
        diffs == 1
    } else {
        // Lengths differ by one → exactly one insertion in `longer`
        // (or one deletion from `longer` to get `shorter`).
        let mut i = 0usize;
        let mut j = 0usize;
        let mut found_skip = false;
        while i < longer.len() && j < shorter.len() {
            if longer[i] == shorter[j] {
                i += 1;
                j += 1;
            } else if found_skip {
                return false;
            } else {
                found_skip = true;
                i += 1;
            }
        }
        // Trailing char in `longer`: skipped at the end is fine.
        true
    }
}

/// Top-level impact report (Phase 3 design spec §4.2).
///
/// Wire format mirrors the design-spec YAML byte-for-byte:
/// [`Self::direct_consumers`] and [`Self::transitive_consumers`] are
/// bare component-id strings (not records), and [`Self::partitions`]
/// carries three independent axes that each map every transitive
/// consumer to its value on that axis ("three independent partitions,
/// not a 3D grid").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// The query target, echoed in the report.
    pub target: ImpactReportTarget,
    /// Direct consumers of the target (one hop on `consumes` edges),
    /// as bare component-id strings.
    pub direct_consumers: Vec<String>,
    /// Transitive closure of consumers (includes direct), as bare
    /// component-id strings.
    pub transitive_consumers: Vec<String>,
    /// Three independent partitions over `transitive_consumers`.
    pub partitions: ImpactPartitions,
    /// Aggregate counts.
    pub summary: ImpactSummary,
}

/// Echoed view of the query target. Distinct from
/// [`crate::types::ImpactTarget`] (the function input is an enum;
/// the rendered output is a `{kind, id}` struct per design §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReportTarget {
    /// `"contract"` or `"component"`.
    pub kind: ImpactReportTargetKind,
    /// The id the user passed (verbatim).
    pub id: String,
}

/// Tag for whether the target of an impact query is a contract or a
/// component. Serialises lowercase to match design §4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImpactReportTargetKind {
    /// A contract (a versioned data-format / API surface).
    Contract,
    /// A component (a unit of code that consumes/provides contracts).
    Component,
}

/// Three independent partitions over `transitive_consumers`. Each
/// partition maps every transitive consumer to its value on that axis.
/// `BTreeMap` preserves a stable serialisation order across runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactPartitions {
    /// Component ids grouped by implementation language.
    pub by_language: BTreeMap<String, Vec<String>>,
    /// Component ids grouped by deploy graph (e.g. `compose:dev`).
    pub by_deploy_graph: BTreeMap<String, Vec<String>>,
    /// Component ids grouped by lifecycle (`runtime`, `build-time`,
    /// `test-only`).
    pub by_lifecycle: BTreeMap<String, Vec<String>>,
}

/// Aggregate counts at the bottom of the impact report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactSummary {
    /// `direct_consumers.len()`.
    pub direct_count: u32,
    /// `transitive_consumers.len()`.
    pub transitive_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::{ComponentEntry, PathSegment};
    use chrono::TimeZone;
    use component_ontology::{ComponentId, Edge, EdgeKind, EvidenceGrade, LifecycleScope};

    fn fixture() -> ImpactReport {
        ImpactReport {
            schema_version: 1,
            generated_at: DateTime::parse_from_rfc3339("2026-05-08T14:30:11Z")
                .unwrap()
                .with_timezone(&Utc),
            target: ImpactReportTarget {
                kind: ImpactReportTargetKind::Contract,
                id: "atlas-contracts/index-schema/v1".into(),
            },
            direct_consumers: vec!["ravel-lite/api".into(), "ravel-lite/worker".into()],
            transitive_consumers: vec![
                "ravel-lite/api".into(),
                "ravel-lite/worker".into(),
                "ravel-lite/dashboard".into(),
                "ops/observability-shipper".into(),
            ],
            partitions: ImpactPartitions {
                by_language: [
                    (
                        "typescript".into(),
                        vec!["ravel-lite/api".into(), "ravel-lite/dashboard".into()],
                    ),
                    ("rust".into(), vec!["ravel-lite/worker".into()]),
                    ("elixir".into(), vec!["ops/observability-shipper".into()]),
                ]
                .into_iter()
                .collect(),
                by_deploy_graph: [
                    (
                        "compose:dev".into(),
                        vec![
                            "ravel-lite/api".into(),
                            "ravel-lite/worker".into(),
                            "ravel-lite/dashboard".into(),
                        ],
                    ),
                    (
                        "compose:ops".into(),
                        vec!["ops/observability-shipper".into()],
                    ),
                ]
                .into_iter()
                .collect(),
                by_lifecycle: [
                    (
                        "runtime".into(),
                        vec![
                            "ravel-lite/api".into(),
                            "ravel-lite/worker".into(),
                            "ravel-lite/dashboard".into(),
                            "ops/observability-shipper".into(),
                        ],
                    ),
                    ("build-time".into(), vec![]),
                    ("test-only".into(), vec![]),
                ]
                .into_iter()
                .collect(),
            },
            summary: ImpactSummary {
                direct_count: 2,
                transitive_count: 4,
            },
        }
    }

    #[test]
    fn impact_report_round_trips_yaml() {
        let report = fixture();
        let yaml = serde_yaml::to_string(&report).expect("serialise");
        let parsed: ImpactReport = serde_yaml::from_str(&yaml).expect("parse");
        assert_eq!(report, parsed);
    }

    #[test]
    fn impact_report_round_trips_json() {
        let report = fixture();
        let json = serde_json::to_string(&report).expect("serialise");
        let parsed: ImpactReport = serde_json::from_str(&json).expect("parse");
        assert_eq!(report, parsed);
    }

    #[test]
    fn impact_report_target_kind_serialises_lowercase() {
        let yaml = serde_yaml::to_string(&ImpactReportTargetKind::Contract).unwrap();
        assert!(yaml.contains("contract"));
        let yaml = serde_yaml::to_string(&ImpactReportTargetKind::Component).unwrap();
        assert!(yaml.contains("component"));
    }

    /// Parse the design §4.2 YAML exemplar verbatim and confirm it
    /// deserialises into the in-memory shape the rest of the crate
    /// expects. This pins the wire format against drift in either
    /// direction (struct rename or spec rewrite).
    #[test]
    fn impact_report_matches_design_spec_exemplar() {
        let yaml = r#"schema_version: 1
generated_at: 2026-05-08T14:30:11Z
target:
  kind: contract
  id: "atlas-contracts/index-schema/v1"
direct_consumers:
  - "ravel-lite/api"
  - "ravel-lite/worker"
transitive_consumers:
  - "ravel-lite/api"
  - "ravel-lite/worker"
  - "ravel-lite/dashboard"
  - "ops/observability-shipper"
partitions:
  by_language:
    typescript: ["ravel-lite/api", "ravel-lite/dashboard"]
    rust: ["ravel-lite/worker"]
    elixir: ["ops/observability-shipper"]
  by_deploy_graph:
    "compose:dev": ["ravel-lite/api", "ravel-lite/worker", "ravel-lite/dashboard"]
    "compose:ops": ["ops/observability-shipper"]
  by_lifecycle:
    runtime: ["ravel-lite/api", "ravel-lite/worker", "ravel-lite/dashboard", "ops/observability-shipper"]
    build-time: []
    test-only: []
summary:
  direct_count: 2
  transitive_count: 4
"#;

        let parsed: ImpactReport = serde_yaml::from_str(yaml).expect("parse design exemplar");
        assert_eq!(parsed, fixture());
    }

    // ── Test fixtures for impact_pure ────────────────────────────────

    /// Build a minimal `ComponentEntry` for unit-test fixtures. The
    /// fields not exercised by impact-traversal logic (path_segments,
    /// manifests, evidence) are stubbed.
    fn make_component(
        id: &str,
        languages: &[&str],
        lifecycle_roles: &[LifecycleScope],
    ) -> ComponentEntry {
        ComponentEntry {
            id: ComponentId::parse(id).expect("parse id"),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: lifecycle_roles.to_vec(),
            languages: languages.iter().map(|s| (*s).to_string()).collect(),
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: std::path::PathBuf::from(id),
                content_sha: "sha256:0".into(),
            }],
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["test-fixture".into()],
            rationale: "test fixture".into(),
            deleted: false,
        }
    }

    fn defines_contract(component: &str, contract: &str) -> Edge {
        Edge {
            kind: EdgeKind::DefinesContract,
            lifecycle: LifecycleScope::Design,
            participants: vec![component.into(), contract.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["surfaces.yaml:contracts_defined".into()],
            rationale: "test fixture".into(),
        }
    }

    fn consumes_contract(component: &str, contract: &str) -> Edge {
        Edge {
            kind: EdgeKind::ConsumesContract,
            lifecycle: LifecycleScope::Runtime,
            participants: vec![component.into(), contract.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["surfaces.yaml:contracts_consumed".into()],
            rationale: "test fixture".into(),
        }
    }

    fn bundled_into(source: &str, orchestration: &str) -> Edge {
        Edge {
            kind: EdgeKind::BundledInto,
            lifecycle: LifecycleScope::Deploy,
            participants: vec![source.into(), orchestration.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["docker-compose.yml:services.x.image".into()],
            rationale: "test fixture".into(),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 14, 30, 11).unwrap()
    }

    // ── 9 acceptance-criteria unit tests ─────────────────────────────

    #[test]
    fn impact_direct_only_consumer_returned() {
        // One contract consumed by exactly one component → that component
        // is the only consumer.
        let provider = make_component("atlas-contracts/owner", &["rust"], &[]);
        let consumer = make_component("ravel-lite/api", &["rust"], &[LifecycleScope::Runtime]);
        let components = vec![provider, consumer];
        let edges = vec![
            defines_contract("atlas-contracts/owner", "atlas-contracts/index-schema/v1"),
            consumes_contract("ravel-lite/api", "atlas-contracts/index-schema/v1"),
        ];
        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("atlas-contracts/index-schema/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        assert_eq!(report.direct_consumers, vec!["ravel-lite/api"]);
        assert_eq!(report.transitive_consumers, vec!["ravel-lite/api"]);
        assert_eq!(report.summary.direct_count, 1);
        assert_eq!(report.summary.transitive_count, 1);
    }

    #[test]
    fn impact_transitive_consumer_returned() {
        // A consumes B's contract; B consumes C's contract; impact on
        // C → both A and B in `transitive_consumers`.
        let a = make_component("a", &["rust"], &[LifecycleScope::Runtime]);
        let b = make_component("b", &["rust"], &[LifecycleScope::Runtime]);
        let c = make_component("c", &["rust"], &[LifecycleScope::Runtime]);
        let components = vec![a, b, c];
        let edges = vec![
            defines_contract("c", "c/contract/v1"),
            defines_contract("b", "b/contract/v1"),
            // B consumes C.
            consumes_contract("b", "c/contract/v1"),
            // A consumes B.
            consumes_contract("a", "b/contract/v1"),
        ];
        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("c/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        assert_eq!(report.direct_consumers, vec!["b"]);
        assert_eq!(report.transitive_consumers, vec!["a", "b"]);
        assert_eq!(report.summary.transitive_count, 2);
    }

    #[test]
    fn impact_cycle_safe() {
        // A consumes B; B consumes A → impact on either returns the
        // cycle members exactly once each (no infinite loop).
        let a = make_component("a", &["rust"], &[LifecycleScope::Runtime]);
        let b = make_component("b", &["rust"], &[LifecycleScope::Runtime]);
        let components = vec![a, b];
        let edges = vec![
            defines_contract("a", "a/contract/v1"),
            defines_contract("b", "b/contract/v1"),
            consumes_contract("b", "a/contract/v1"),
            consumes_contract("a", "b/contract/v1"),
        ];

        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("a/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");
        // Direct consumer of A's contract is B; transitively, A also
        // consumes its own contract via B's contract, so the cycle
        // closes back through A. Both members appear exactly once.
        assert_eq!(report.transitive_consumers, vec!["a", "b"]);
        assert_eq!(report.summary.transitive_count, 2);

        // Symmetric: target B's contract → same membership.
        let report_b = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("b/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");
        assert_eq!(report_b.transitive_consumers, vec!["a", "b"]);
    }

    #[test]
    fn impact_partition_by_language_correct() {
        // Three consumers with mixed languages → each lists under its
        // language partition.
        let owner = make_component("owner", &["rust"], &[LifecycleScope::Runtime]);
        let api = make_component(
            "ravel-lite/api",
            &["typescript"],
            &[LifecycleScope::Runtime],
        );
        let dash = make_component(
            "ravel-lite/dashboard",
            &["typescript"],
            &[LifecycleScope::Runtime],
        );
        let worker = make_component("ravel-lite/worker", &["rust"], &[LifecycleScope::Runtime]);
        let components = vec![owner, api, dash, worker];
        let edges = vec![
            defines_contract("owner", "atlas-contracts/index/v1"),
            consumes_contract("ravel-lite/api", "atlas-contracts/index/v1"),
            consumes_contract("ravel-lite/dashboard", "atlas-contracts/index/v1"),
            consumes_contract("ravel-lite/worker", "atlas-contracts/index/v1"),
        ];
        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("atlas-contracts/index/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        let ts = report
            .partitions
            .by_language
            .get("typescript")
            .expect("typescript bucket present");
        assert_eq!(
            ts,
            &vec![
                "ravel-lite/api".to_string(),
                "ravel-lite/dashboard".to_string()
            ]
        );
        let rust = report
            .partitions
            .by_language
            .get("rust")
            .expect("rust bucket present");
        assert_eq!(rust, &vec!["ravel-lite/worker".to_string()]);
    }

    #[test]
    fn impact_partition_by_deploy_graph_correct() {
        // Two compose orchestrations covering different consumers →
        // partition reflects deploy-graph membership.
        let owner = make_component("owner", &["rust"], &[LifecycleScope::Runtime]);
        let api = make_component(
            "ravel-lite/api",
            &["typescript"],
            &[LifecycleScope::Runtime],
        );
        let worker = make_component("ravel-lite/worker", &["rust"], &[LifecycleScope::Runtime]);
        let dev_orch = make_component("ravel-lite/compose-dev", &[], &[LifecycleScope::Deploy]);
        let ops_orch = make_component("ops/compose-ops", &[], &[LifecycleScope::Deploy]);
        let components = vec![owner, api, worker, dev_orch, ops_orch];
        let edges = vec![
            defines_contract("owner", "atlas-contracts/index/v1"),
            consumes_contract("ravel-lite/api", "atlas-contracts/index/v1"),
            consumes_contract("ravel-lite/worker", "atlas-contracts/index/v1"),
            // api lives in the dev compose graph; worker in the ops graph.
            bundled_into("ravel-lite/api", "ravel-lite/compose-dev"),
            bundled_into("ravel-lite/worker", "ops/compose-ops"),
        ];
        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("atlas-contracts/index/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        let dev = report
            .partitions
            .by_deploy_graph
            .get("ravel-lite/compose-dev")
            .expect("dev bucket present");
        assert_eq!(dev, &vec!["ravel-lite/api".to_string()]);
        let ops = report
            .partitions
            .by_deploy_graph
            .get("ops/compose-ops")
            .expect("ops bucket present");
        assert_eq!(ops, &vec!["ravel-lite/worker".to_string()]);
    }

    #[test]
    fn impact_partition_by_lifecycle_correct() {
        // runtime / build-time / test-only consumers each in their
        // bucket. We use the canonical kebab-case names emitted by
        // `LifecycleScope::as_str`: `runtime`, `build`, `test`.
        let owner = make_component("owner", &["rust"], &[LifecycleScope::Runtime]);
        let runtime_consumer =
            make_component("runtime-consumer", &["rust"], &[LifecycleScope::Runtime]);
        let build_consumer = make_component("build-consumer", &["rust"], &[LifecycleScope::Build]);
        let test_consumer = make_component("test-consumer", &["rust"], &[LifecycleScope::Test]);
        let components = vec![owner, runtime_consumer, build_consumer, test_consumer];
        let edges = vec![
            defines_contract("owner", "owner/contract/v1"),
            consumes_contract("runtime-consumer", "owner/contract/v1"),
            consumes_contract("build-consumer", "owner/contract/v1"),
            consumes_contract("test-consumer", "owner/contract/v1"),
        ];
        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("owner/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        assert_eq!(
            report.partitions.by_lifecycle.get("runtime"),
            Some(&vec!["runtime-consumer".to_string()])
        );
        assert_eq!(
            report.partitions.by_lifecycle.get("build"),
            Some(&vec!["build-consumer".to_string()])
        );
        assert_eq!(
            report.partitions.by_lifecycle.get("test"),
            Some(&vec!["test-consumer".to_string()])
        );
    }

    #[test]
    fn impact_target_not_found_returns_levenshtein_candidates() {
        // Query "ravel-lit" against a fixture containing
        // "ravel-lite/api" → candidates include "ravel-lite/api".
        // (One deletion of `e/api` is distance 4, not 1; we use a
        // closer mistype: omit one char from a contract id.)
        let api = make_component(
            "ravel-lite/api",
            &["typescript"],
            &[LifecycleScope::Runtime],
        );
        let components = vec![api];
        let edges: Vec<Edge> = vec![];

        // The CLI takes a free-text id and tries contract-namespace
        // first, then component-namespace. Querying as a Component
        // means the resolver searches in the component-id pool;
        // "ravel-lite/ap" is exactly one deletion from
        // "ravel-lite/api" so the suggestion fires.
        let mistyped = ComponentId::parse("ravel-lite/ap").expect("parse mistype");
        let err = impact_pure(
            &components,
            &edges,
            ImpactTarget::Component(mistyped),
            now(),
        )
        .expect_err("expected target-not-found");
        match err {
            ReportError::TargetNotFound { needle, candidates } => {
                assert_eq!(needle, "ravel-lite/ap");
                assert!(
                    candidates.iter().any(|c| c == "ravel-lite/api"),
                    "expected `ravel-lite/api` in candidates, got {candidates:?}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn impact_empty_result_for_unconsumed_contract() {
        // A contract no one consumes → empty `direct_consumers` and
        // `transitive_consumers`.
        let owner = make_component("owner", &["rust"], &[LifecycleScope::Runtime]);
        let components = vec![owner];
        let edges = vec![defines_contract("owner", "owner/contract/v1")];

        let report = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("owner/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");

        assert!(report.direct_consumers.is_empty());
        assert!(report.transitive_consumers.is_empty());
        assert_eq!(report.summary.direct_count, 0);
        assert_eq!(report.summary.transitive_count, 0);
    }

    #[test]
    fn impact_contract_input_vs_component_input() {
        // Same contract via contract id and via providing-component id
        // produce the same consumer set (assuming the component
        // provides only that contract).
        let owner = make_component("owner", &["rust"], &[LifecycleScope::Runtime]);
        let api = make_component(
            "ravel-lite/api",
            &["typescript"],
            &[LifecycleScope::Runtime],
        );
        let components = vec![owner, api];
        let edges = vec![
            defines_contract("owner", "owner/contract/v1"),
            consumes_contract("ravel-lite/api", "owner/contract/v1"),
        ];

        let via_contract = impact_pure(
            &components,
            &edges,
            ImpactTarget::Contract("owner/contract/v1".into()),
            now(),
        )
        .expect("impact succeeds");
        let via_component = impact_pure(
            &components,
            &edges,
            ImpactTarget::Component(ComponentId::parse("owner").unwrap()),
            now(),
        )
        .expect("impact succeeds");

        assert_eq!(
            via_contract.transitive_consumers, via_component.transitive_consumers,
            "contract-input and component-input should produce the same consumer set",
        );
        assert_eq!(
            via_contract.direct_consumers,
            via_component.direct_consumers
        );
    }

    // ── Helpers ──────────────────────────────────────────────────────

    #[test]
    fn levenshtein_distance_one_basic() {
        assert!(levenshtein_distance_one("abc", "abd")); // sub
        assert!(levenshtein_distance_one("abc", "ab")); // delete
        assert!(levenshtein_distance_one("abc", "abcd")); // insert at end
        assert!(levenshtein_distance_one("abc", "xabc")); // insert at start
        assert!(!levenshtein_distance_one("abc", "abc")); // identical = 0
        assert!(!levenshtein_distance_one("abc", "xyz")); // 3 subs
        assert!(!levenshtein_distance_one("abc", "ax")); // delete + sub
    }
}
