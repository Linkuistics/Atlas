//! Composition divergence report —
//! `.atlas/cache/reports/composition-divergence.yaml`.
//!
//! Schema is fixed by Phase 3 design spec §4.4. PR-7 shipped the report
//! types and a stubbed entry-point; PR-11 lands the actual pair-wise
//! classification logic.
//!
//! ## Pair classification (design §4.4)
//!
//! For each unordered pair `{A, B}` of live components:
//!
//! - **Build-coupled:** a direct `depends-on` edge in either direction
//!   (transitive coupling intentionally not flagged).
//! - **Deploy-coupled:** any direct edge in the composition family
//!   (`bundled-into`, `published-as`, `deployed-with`, `released-with`,
//!   `bundled-from-external`).
//! - **Divergent:** `build_coupled XOR deploy_coupled`.
//!
//! ## Severity (design §4.4)
//!
//! Severity scores each divergent pair by the count of contracts the
//! pair shares whose `content_sha` changed since the drift baseline:
//!
//! - Shared contracts = `(consumes ∪ provides)_A ∩ (consumes ∪ provides)_B`.
//! - A shared contract counts toward severity when the baseline does
//!   not list it (added since baseline) **or** the baseline's
//!   `content_sha` differs from the current `content_sha`.
//! - When `drift_baseline` is `None`, every pair's `severity` is
//!   `None` and the report header notes baseline absence.
//!
//! ## Determinism
//!
//! Pairs are canonicalised as `(min(A, B), max(A, B))` lexicographic
//! tuples; divergent pairs are sorted by this tuple before emission. The
//! `drifting_contracts` list per pair is sorted lexicographically.

use std::collections::{BTreeMap, BTreeSet};

use atlas_engine::{all_components, all_proposed_edges, surface_artefacts_of, Sha256Hex};
use chrono::{DateTime, Utc};
use component_ontology::EdgeKind;
use serde::{Deserialize, Serialize};

use crate::snapshot::ContractShaSnapshot;
use crate::types::{ContractId, ReportError, ReportInputs};

/// For each unordered pair of components, classify as build-only,
/// deploy-only, both, or neither; flag pairs that are coupled in
/// exactly one of the two graphs as **divergent** and score severity
/// against the drift baseline.
///
/// `inputs` borrows the engine database and workspace.
/// `drift_baseline` is the optional snapshot read from
/// `.atlas/cache/contract-shas-snapshot.yaml`. The function never
/// mutates the snapshot.
pub fn divergence(
    inputs: ReportInputs<'_>,
    drift_baseline: Option<&ContractShaSnapshot>,
) -> Result<DivergenceReport, ReportError> {
    let db = inputs.db;

    // 1. Live component ids.
    let components = all_components(db);
    let component_ids: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str().to_string())
        .collect();

    // 2. Edge sets, keyed by canonical (min, max) participant tuples.
    //
    //    Build edges: a directed `depends-on` edge in either direction
    //    couples the pair. We collapse to the canonical (min, max)
    //    tuple at insertion time so the per-pair lookup is direction-
    //    agnostic and dedupes across direction pairs.
    //
    //    Deploy edges: any composition-family edge.
    let edges = all_proposed_edges(db);
    let mut build_edge_kinds: BTreeMap<(String, String), BTreeSet<&'static str>> = BTreeMap::new();
    let mut deploy_edge_kinds: BTreeMap<(String, String), BTreeSet<&'static str>> = BTreeMap::new();
    for edge in edges.iter() {
        if edge.participants.len() != 2 {
            continue;
        }
        let key = canonical_pair(&edge.participants[0], &edge.participants[1]);
        match edge.kind {
            EdgeKind::DependsOn => {
                build_edge_kinds
                    .entry(key)
                    .or_default()
                    .insert(edge.kind.as_str());
            }
            k if is_composition_kind(k) => {
                deploy_edge_kinds.entry(key).or_default().insert(k.as_str());
            }
            _ => {}
        }
    }

    // 3. Per-component contract sets and current-shas. Each component
    //    contributes both contracts it defines (provides) and contracts
    //    its `contracts_consumed` references — though Phase 1 leaves
    //    `contracts_consumed` empty, so this defaults to the defined
    //    set.
    let mut component_contracts: BTreeMap<String, BTreeSet<ContractId>> = BTreeMap::new();
    let mut current_contract_shas: BTreeMap<ContractId, Sha256Hex> = BTreeMap::new();
    for entry in components.iter().filter(|c| !c.deleted) {
        let id_str = entry.id.as_str().to_string();
        let artefacts = surface_artefacts_of(db, entry.id.clone());
        let mut owned: BTreeSet<ContractId> = BTreeSet::new();
        for contract in &artefacts.contracts {
            owned.insert(contract.id.clone());
            // Multiple components may "own" the same contract id (e.g.
            // bridging bindings); the canonical content sha is the
            // contract's `fingerprint`. Last-writer-wins is fine here
            // because divergence treats severity as "did the sha
            // change since baseline" — duplicate entries with the same
            // sha collapse to a no-op insert.
            current_contract_shas
                .entry(contract.id.clone())
                .or_insert_with(|| contract.fingerprint.clone());
        }
        // `contracts_consumed` is wired through the L9 projection but
        // empty in Phase 1; iterating defensively keeps the impl
        // forward-compatible for PR-9's traversal.
        component_contracts.insert(id_str, owned);
    }

    // 4. Pair classification.
    Ok(compute_divergence_report(
        &component_ids,
        &build_edge_kinds,
        &deploy_edge_kinds,
        &component_contracts,
        &current_contract_shas,
        drift_baseline,
        Utc::now(),
    ))
}

/// Pure inner function: classify every unordered pair, score severity,
/// and assemble the report. Factored out so unit tests can drive every
/// AC against in-memory fixtures without building a real engine.
fn compute_divergence_report(
    component_ids: &[String],
    build_edge_kinds: &BTreeMap<(String, String), BTreeSet<&'static str>>,
    deploy_edge_kinds: &BTreeMap<(String, String), BTreeSet<&'static str>>,
    component_contracts: &BTreeMap<String, BTreeSet<ContractId>>,
    current_contract_shas: &BTreeMap<ContractId, Sha256Hex>,
    drift_baseline: Option<&ContractShaSnapshot>,
    generated_at: DateTime<Utc>,
) -> DivergenceReport {
    let baseline_map: BTreeMap<&ContractId, &Sha256Hex> =
        drift_baseline.map(|b| b.as_map()).unwrap_or_default();
    let baseline_present = drift_baseline.is_some();

    // Sort component ids so the pair iteration is deterministic.
    let mut ids: Vec<&String> = component_ids.iter().collect();
    ids.sort();

    let n = ids.len();
    let total_pairs_examined = if n >= 2 {
        (n as u64 * (n as u64 - 1) / 2) as u32
    } else {
        0
    };

    let mut divergent_pairs: Vec<DivergencePair> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let a = ids[i];
            let b = ids[j];
            let key = canonical_pair(a, b);

            let build_kinds = build_edge_kinds.get(&key);
            let deploy_kinds = deploy_edge_kinds.get(&key);
            let build_coupled = build_kinds.is_some();
            let deploy_coupled = deploy_kinds.is_some();
            // XOR: divergent iff exactly one is true.
            if build_coupled == deploy_coupled {
                continue;
            }

            let coupling = if build_coupled {
                DivergenceCoupling::BuildOnly
            } else {
                DivergenceCoupling::DeployOnly
            };
            let build_edges: Vec<String> = build_kinds
                .map(|s| s.iter().map(|k| (*k).to_string()).collect())
                .unwrap_or_default();
            let deploy_edges: Vec<String> = deploy_kinds
                .map(|s| s.iter().map(|k| (*k).to_string()).collect())
                .unwrap_or_default();

            // Shared contracts and severity.
            let empty = BTreeSet::new();
            let contracts_a = component_contracts.get(a).unwrap_or(&empty);
            let contracts_b = component_contracts.get(b).unwrap_or(&empty);
            let shared: Vec<&ContractId> = contracts_a.intersection(contracts_b).collect();

            let (severity, drifting_contracts) = if !baseline_present {
                (None, Vec::new())
            } else {
                let mut drifting: Vec<ContractId> = Vec::new();
                for cid in &shared {
                    let current = match current_contract_shas.get(*cid) {
                        Some(s) => s,
                        // Shared contract is somehow not in the current
                        // sha map (defensive — should not happen). Skip.
                        None => continue,
                    };
                    let baseline_sha = baseline_map.get(cid);
                    let drifted = match baseline_sha {
                        // Missing in baseline → added since baseline →
                        // counts toward drift.
                        None => true,
                        // Present and equal → unchanged.
                        Some(b) => *b != current,
                    };
                    if drifted {
                        drifting.push((*cid).clone());
                    }
                }
                drifting.sort();
                let severity = drifting.len() as u32;
                (Some(severity), drifting)
            };

            divergent_pairs.push(DivergencePair {
                components: [key.0.clone(), key.1.clone()],
                coupling,
                build_edges,
                deploy_edges,
                severity,
                drifting_contracts,
            });
        }
    }

    // Sort divergent pairs by (min, max) lex order. The components
    // field is already canonicalised to (min, max); a straight Vec
    // sort suffices.
    divergent_pairs.sort_by(|x, y| {
        let xa = (&x.components[0], &x.components[1]);
        let ya = (&y.components[0], &y.components[1]);
        xa.cmp(&ya)
    });

    let mut by_severity: BTreeMap<u32, u32> = BTreeMap::new();
    if baseline_present {
        for pair in &divergent_pairs {
            if let Some(s) = pair.severity {
                *by_severity.entry(s).or_insert(0) += 1;
            }
        }
    }

    let divergent_count = divergent_pairs.len() as u32;
    DivergenceReport {
        schema_version: 1,
        generated_at,
        drift_baseline_at: drift_baseline.map(|b| b.captured_at),
        divergent_pairs,
        summary: DivergenceSummary {
            total_pairs_examined,
            divergent_count,
            by_severity,
        },
    }
}

/// Build the (min, max) canonical pair key for two component ids.
fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// True iff the edge kind is in the composition family
/// (design §4.4 deploy graph).
fn is_composition_kind(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::BundledInto
            | EdgeKind::PublishedAs
            | EdgeKind::DeployedWith
            | EdgeKind::ReleasedWith
            | EdgeKind::BundledFromExternal
    )
}

/// Top-level composition divergence report (Phase 3 design spec §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceReport {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// `captured_at` of the drift baseline used for severity scoring,
    /// or `None` if no baseline exists yet.
    pub drift_baseline_at: Option<DateTime<Utc>>,
    /// Pairs that are coupled in exactly one of the build/deploy
    /// graphs.
    pub divergent_pairs: Vec<DivergencePair>,
    /// Aggregate counts.
    pub summary: DivergenceSummary,
}

/// One divergent pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergencePair {
    /// The two component ids, sorted lexicographically for
    /// determinism.
    pub components: [String; 2],
    /// Whether the coupling is build-only or deploy-only.
    pub coupling: DivergenceCoupling,
    /// Build-graph edges between the pair (only present for
    /// `coupling: build_only`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_edges: Vec<String>,
    /// Deploy-graph edges between the pair (only present for
    /// `coupling: deploy_only`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deploy_edges: Vec<String>,
    /// Number of shared contracts that drifted since the baseline,
    /// or `None` if no baseline exists.
    pub severity: Option<u32>,
    /// Shared contracts that drifted (count == `severity`).
    pub drifting_contracts: Vec<ContractId>,
}

/// Which graph coupled the pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceCoupling {
    /// Edge in `depends-on` only.
    BuildOnly,
    /// Edge in a composition edge type only.
    DeployOnly,
}

/// Aggregate counts at the bottom of the divergence report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceSummary {
    /// `n*(n-1)/2` pairs over the workspace's components.
    pub total_pairs_examined: u32,
    /// `divergent_pairs.len()`.
    pub divergent_count: u32,
    /// Histogram of `severity` values (`null` severity is excluded —
    /// rendered separately in the `human` format).
    pub by_severity: BTreeMap<u32, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{ContractShaEntry, ContractShaSnapshot};
    use chrono::TimeZone;

    /// Wall-clock value used by every test below — keeps the
    /// `generated_at` field stable so individual ACs can compare
    /// reports byte-for-byte if they want to.
    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 14, 40, 11).unwrap()
    }

    fn baseline_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap()
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Build a `(min, max)` edge entry with the given kind labels.
    fn pair_with_kinds(
        a: &str,
        b: &str,
        kinds: &[&'static str],
    ) -> ((String, String), BTreeSet<&'static str>) {
        let key = canonical_pair(a, b);
        let mut set: BTreeSet<&'static str> = BTreeSet::new();
        for k in kinds {
            set.insert(*k);
        }
        (key, set)
    }

    /// Convenience: produce a baseline snapshot containing the given
    /// `(id, sha)` pairs, captured at `baseline_at()`.
    fn make_baseline(entries: &[(&str, &str)]) -> ContractShaSnapshot {
        ContractShaSnapshot {
            schema_version: 1,
            captured_at: baseline_at(),
            contract_shas: entries
                .iter()
                .map(|(id, sha)| ContractShaEntry {
                    id: (*id).to_string(),
                    content_sha: (*sha).to_string(),
                })
                .collect(),
        }
    }

    fn fixture_report() -> DivergenceReport {
        let mut by_severity = BTreeMap::new();
        by_severity.insert(0, 1);
        by_severity.insert(2, 1);

        DivergenceReport {
            schema_version: 1,
            generated_at: fixed_now(),
            drift_baseline_at: Some(baseline_at()),
            divergent_pairs: vec![
                DivergencePair {
                    components: [
                        "ops/observability-shipper".to_string(),
                        "ravel-lite/api".to_string(),
                    ],
                    coupling: DivergenceCoupling::DeployOnly,
                    build_edges: vec![],
                    deploy_edges: vec!["co-deployed-with".to_string()],
                    severity: Some(2),
                    drifting_contracts: vec![
                        "atlas-contracts/log-schema/v1".to_string(),
                        "atlas-contracts/metric-schema/v1".to_string(),
                    ],
                },
                DivergencePair {
                    components: [
                        "ravel-lite/dashboard".to_string(),
                        "ravel-lite/worker".to_string(),
                    ],
                    coupling: DivergenceCoupling::BuildOnly,
                    build_edges: vec!["depends-on".to_string()],
                    deploy_edges: vec![],
                    severity: Some(0),
                    drifting_contracts: vec![],
                },
            ],
            summary: DivergenceSummary {
                total_pairs_examined: 187,
                divergent_count: 2,
                by_severity,
            },
        }
    }

    #[test]
    fn divergence_report_round_trips_yaml() {
        let original = fixture_report();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: DivergenceReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn divergence_report_round_trips_json() {
        let original = fixture_report();
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: DivergenceReport = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn divergence_coupling_serialises_snake_case() {
        let yaml = serde_yaml::to_string(&DivergenceCoupling::BuildOnly).unwrap();
        assert!(yaml.contains("build_only"));
        let yaml = serde_yaml::to_string(&DivergenceCoupling::DeployOnly).unwrap();
        assert!(yaml.contains("deploy_only"));
    }

    #[test]
    fn divergence_with_null_baseline_round_trips() {
        let mut original = fixture_report();
        original.drift_baseline_at = None;
        for pair in &mut original.divergent_pairs {
            pair.severity = None;
        }
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: DivergenceReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    // -----------------------------------------------------------------
    // PR-11 acceptance criteria — pair classification
    // -----------------------------------------------------------------

    /// AC: fixture with `depends-on` but no composition → divergent,
    /// coupling `build_only`.
    #[test]
    fn divergence_pair_classification_build_only() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let contracts = BTreeMap::new();
        let shas = BTreeMap::new();
        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert_eq!(report.divergent_pairs.len(), 1);
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.coupling, DivergenceCoupling::BuildOnly);
        assert_eq!(pair.build_edges, vec!["depends-on".to_string()]);
        assert!(pair.deploy_edges.is_empty());
    }

    /// AC: composition edge but no `depends-on` → divergent, coupling
    /// `deploy_only`.
    #[test]
    fn divergence_pair_classification_deploy_only() {
        let cids = ids(&["a/x", "a/y"]);
        let build = BTreeMap::new();
        let mut deploy = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["deployed-with"]);
        deploy.insert(k, v);
        let contracts = BTreeMap::new();
        let shas = BTreeMap::new();
        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert_eq!(report.divergent_pairs.len(), 1);
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.coupling, DivergenceCoupling::DeployOnly);
        assert_eq!(pair.deploy_edges, vec!["deployed-with".to_string()]);
        assert!(pair.build_edges.is_empty());
    }

    /// AC: both edges present → NOT divergent.
    #[test]
    fn divergence_pair_classification_both() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (kb, vb) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(kb, vb);
        let mut deploy = BTreeMap::new();
        let (kd, vd) = pair_with_kinds("a/x", "a/y", &["deployed-with"]);
        deploy.insert(kd, vd);
        let contracts = BTreeMap::new();
        let shas = BTreeMap::new();
        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert!(report.divergent_pairs.is_empty());
        assert_eq!(report.summary.divergent_count, 0);
        assert_eq!(report.summary.total_pairs_examined, 1);
    }

    /// AC: no edges → NOT divergent.
    #[test]
    fn divergence_pair_classification_neither() {
        let cids = ids(&["a/x", "a/y"]);
        let build = BTreeMap::new();
        let deploy = BTreeMap::new();
        let contracts = BTreeMap::new();
        let shas = BTreeMap::new();
        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert!(report.divergent_pairs.is_empty());
        assert_eq!(report.summary.total_pairs_examined, 1);
    }

    // -----------------------------------------------------------------
    // PR-11 acceptance criteria — severity
    // -----------------------------------------------------------------

    /// AC: divergent pair, shared contracts all unchanged since
    /// baseline → severity 0.
    #[test]
    fn divergence_severity_zero_when_no_shared_contracts_drifted() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let mut contracts: BTreeMap<String, BTreeSet<ContractId>> = BTreeMap::new();
        let shared = ["c/one".to_string(), "c/two".to_string()];
        contracts.insert("a/x".into(), shared.iter().cloned().collect());
        contracts.insert("a/y".into(), shared.iter().cloned().collect());
        let mut shas = BTreeMap::new();
        shas.insert("c/one".to_string(), "sha256:11".to_string());
        shas.insert("c/two".to_string(), "sha256:22".to_string());
        let baseline = make_baseline(&[("c/one", "sha256:11"), ("c/two", "sha256:22")]);

        let report = compute_divergence_report(
            &cids,
            &build,
            &deploy,
            &contracts,
            &shas,
            Some(&baseline),
            fixed_now(),
        );
        assert_eq!(report.divergent_pairs.len(), 1);
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.severity, Some(0));
        assert!(pair.drifting_contracts.is_empty());
        assert_eq!(report.summary.by_severity.get(&0), Some(&1));
    }

    /// AC: divergent pair, two shared contracts changed since baseline
    /// → severity 2.
    #[test]
    fn divergence_severity_counts_drifted_shared_contracts() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let mut contracts: BTreeMap<String, BTreeSet<ContractId>> = BTreeMap::new();
        let shared = ["c/one".to_string(), "c/two".to_string()];
        contracts.insert("a/x".into(), shared.iter().cloned().collect());
        contracts.insert("a/y".into(), shared.iter().cloned().collect());
        let mut shas = BTreeMap::new();
        shas.insert("c/one".to_string(), "sha256:11-NEW".to_string());
        shas.insert("c/two".to_string(), "sha256:22-NEW".to_string());
        let baseline = make_baseline(&[("c/one", "sha256:11"), ("c/two", "sha256:22")]);

        let report = compute_divergence_report(
            &cids,
            &build,
            &deploy,
            &contracts,
            &shas,
            Some(&baseline),
            fixed_now(),
        );
        assert_eq!(report.divergent_pairs.len(), 1);
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.severity, Some(2));
        assert_eq!(
            pair.drifting_contracts,
            vec!["c/one".to_string(), "c/two".to_string()]
        );
        assert_eq!(report.summary.by_severity.get(&2), Some(&1));
    }

    /// AC: contract added since baseline → counts toward severity.
    #[test]
    fn divergence_severity_counts_added_shared_contracts() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let mut contracts: BTreeMap<String, BTreeSet<ContractId>> = BTreeMap::new();
        // Two shared contracts, one of them missing from baseline.
        let shared = ["c/old".to_string(), "c/new".to_string()];
        contracts.insert("a/x".into(), shared.iter().cloned().collect());
        contracts.insert("a/y".into(), shared.iter().cloned().collect());
        let mut shas = BTreeMap::new();
        shas.insert("c/old".to_string(), "sha256:OLD".to_string());
        shas.insert("c/new".to_string(), "sha256:NEW".to_string());
        // Baseline contains only `c/old`. `c/new` is new since baseline.
        let baseline = make_baseline(&[("c/old", "sha256:OLD")]);

        let report = compute_divergence_report(
            &cids,
            &build,
            &deploy,
            &contracts,
            &shas,
            Some(&baseline),
            fixed_now(),
        );
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.severity, Some(1));
        assert_eq!(pair.drifting_contracts, vec!["c/new".to_string()]);
    }

    /// AC: `drift_baseline = None` → severity is `None` for all pairs;
    /// report header reflects baseline absence.
    #[test]
    fn divergence_severity_null_without_baseline() {
        let cids = ids(&["a/x", "a/y"]);
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let mut contracts: BTreeMap<String, BTreeSet<ContractId>> = BTreeMap::new();
        contracts.insert("a/x".into(), ["c/one".to_string()].into_iter().collect());
        contracts.insert("a/y".into(), ["c/one".to_string()].into_iter().collect());
        let mut shas = BTreeMap::new();
        shas.insert("c/one".to_string(), "sha256:11".to_string());

        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert_eq!(report.drift_baseline_at, None);
        assert_eq!(report.divergent_pairs.len(), 1);
        for pair in &report.divergent_pairs {
            assert_eq!(pair.severity, None);
            assert!(pair.drifting_contracts.is_empty());
        }
        assert!(
            report.summary.by_severity.is_empty(),
            "by_severity histogram is empty when no baseline is present"
        );
    }

    /// AC: fixture with consistent build+deploy coupling → empty
    /// `divergent_pairs`.
    #[test]
    fn divergence_empty_when_no_divergent_pairs() {
        // Three components: every pair is either coupled in BOTH
        // graphs or NEITHER — none diverge.
        let cids = ids(&["a/x", "a/y", "a/z"]);
        let mut build = BTreeMap::new();
        let (kxy_b, vxy_b) = pair_with_kinds("a/x", "a/y", &["depends-on"]);
        build.insert(kxy_b, vxy_b);
        let mut deploy = BTreeMap::new();
        let (kxy_d, vxy_d) = pair_with_kinds("a/x", "a/y", &["deployed-with"]);
        deploy.insert(kxy_d, vxy_d);
        // (a/x, a/z) and (a/y, a/z) have no edges in either graph.
        let contracts = BTreeMap::new();
        let shas = BTreeMap::new();
        let report =
            compute_divergence_report(&cids, &build, &deploy, &contracts, &shas, None, fixed_now());
        assert!(report.divergent_pairs.is_empty());
        assert_eq!(report.summary.divergent_count, 0);
        // C(3,2) = 3 pairs.
        assert_eq!(report.summary.total_pairs_examined, 3);
    }

    /// Lexicographic-canonicalisation regression: an edge declared in
    /// reverse order (B → A) still couples the (min, max) canonical
    /// pair, and the rendered `components` field is in (min, max)
    /// order regardless of how the edge was declared.
    #[test]
    fn divergence_pair_canonicalises_reverse_order_edges() {
        let cids = ids(&["a/x", "a/y"]);
        // Edge declared (a/y, a/x) — reverse alphabetical.
        let mut build = BTreeMap::new();
        let (k, v) = pair_with_kinds("a/y", "a/x", &["depends-on"]);
        build.insert(k, v);
        let deploy = BTreeMap::new();
        let report = compute_divergence_report(
            &cids,
            &build,
            &deploy,
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            fixed_now(),
        );
        assert_eq!(report.divergent_pairs.len(), 1);
        let pair = &report.divergent_pairs[0];
        assert_eq!(pair.components[0], "a/x");
        assert_eq!(pair.components[1], "a/y");
    }

    #[test]
    fn is_composition_kind_covers_design_spec_family() {
        // Composition family per `component_ontology` — must include
        // every member design §4.4 names as a deploy edge.
        for k in [
            EdgeKind::BundledInto,
            EdgeKind::PublishedAs,
            EdgeKind::DeployedWith,
            EdgeKind::ReleasedWith,
            EdgeKind::BundledFromExternal,
        ] {
            assert!(is_composition_kind(k), "{:?} must be composition", k);
        }
        // Spot-check non-composition kinds.
        assert!(!is_composition_kind(EdgeKind::DependsOn));
        assert!(!is_composition_kind(EdgeKind::Calls));
        assert!(!is_composition_kind(EdgeKind::DefinesContract));
    }
}
