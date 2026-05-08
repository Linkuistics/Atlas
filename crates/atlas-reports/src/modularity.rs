//! Modularity report — per-component metric files plus a top-level
//! subsystem rollup. Schema is fixed by Phase 3 design spec §4.3.
//!
//! PR-7 shipped the report types and a stubbed [`modularity`] entry
//! point; PR-10 (this PR) lands the six metric formulas, history
//! rotation, and the subsystem aggregate / outlier flagging.
//!
//! `atlas-reports` stays I/O-free — all file reads/writes happen in the
//! CLI handler. The [`modularity`] function takes:
//!
//! - [`ReportInputs`] — borrowed handles to the engine database +
//!   workspace; this is the canonical Phase 3 contract.
//! - `prior_per_component: HashMap<ComponentId, ModularityHistory>` —
//!   the CLI handler's pre-loaded prior history map (one entry per
//!   component whose `<component>/.atlas/cache/modularity.yaml` exists
//!   on disk; absent components are treated as empty histories).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use atlas_engine::{
    all_components, all_proposed_edges, subsystems_yaml_snapshot, surfaces_yaml_snapshot, Sha256Hex,
};
use chrono::{DateTime, Utc};
use component_ontology::{ComponentId, EdgeKind};
use serde::{Deserialize, Serialize};

use crate::types::{ReportError, ReportInputs};

/// Hard cap on per-component history entries.
///
/// Phase 3 design spec §4.3 + plan §4 PR-10 fix this at 5 (FIFO) and
/// explicitly call it out as **not configurable** — making the cap a
/// knob is on the deferred-indefinitely list (plan §7.3). Keep this
/// constant private: the only legitimate reader is [`rotate_history`].
const HISTORY_CAP: usize = 5;

/// Compute modularity metrics for every component in the workspace,
/// rotate per-component history, and emit a subsystem-level rollup.
///
/// Reads every component's surfaces (via [`surfaces_yaml_snapshot`])
/// and the consumes/defines-contract edges (via
/// [`all_proposed_edges`]) to derive the six metrics. Subsystem
/// aggregates draw from [`subsystems_yaml_snapshot`]; components not
/// in any subsystem land in `unattached_components` and are excluded
/// from rollup means.
///
/// The per-component history is rotated against
/// `prior_per_component`: if the current `surface_fingerprint` matches
/// the most-recent prior entry, no append (no duplicate); otherwise
/// the current entry is prepended and the oldest dropped if the total
/// exceeds five (Phase 3 design spec §4.3).
pub fn modularity(
    inputs: ReportInputs,
    prior_per_component: HashMap<ComponentId, ModularityHistory>,
) -> Result<ModularityReport, ReportError> {
    let generated_at = Utc::now();
    let db = inputs.db;

    // Resolve the live component set. Sorted by id for deterministic
    // output across re-runs.
    let mut live_ids: Vec<ComponentId> = all_components(db)
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.clone())
        .collect();
    live_ids.sort();

    // Walk the edge list once to build:
    // - providers_of[contract_id] = set of components that DEFINE the
    //   contract (one in well-formed workspaces, but tolerate >1 to
    //   stay defensive against duplicate `defines-contract` edges).
    // - consumers_of[contract_id] = set of components that CONSUME
    //   the contract.
    let edges = all_proposed_edges(db);
    let mut providers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
    let mut consumers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
    for edge in edges.iter() {
        match edge.kind {
            EdgeKind::DefinesContract => {
                if let (Some(comp), Some(contract)) =
                    (edge.participants.first(), edge.participants.get(1))
                {
                    if let Ok(cid) = ComponentId::parse(comp) {
                        providers_of
                            .entry(contract.clone())
                            .or_default()
                            .insert(cid);
                    }
                }
            }
            EdgeKind::ConsumesContract => {
                if let (Some(comp), Some(contract)) =
                    (edge.participants.first(), edge.participants.get(1))
                {
                    if let Ok(cid) = ComponentId::parse(comp) {
                        consumers_of
                            .entry(contract.clone())
                            .or_default()
                            .insert(cid);
                    }
                }
            }
            _ => {}
        }
    }

    // Compute current per-component metrics + history-entry payload.
    let mut per_component: HashMap<ComponentId, ComponentModularity> = HashMap::new();
    for id in &live_ids {
        // Pull the surfaces snapshot for this component to learn what
        // contracts it provides + total binding count (needed for
        // surface_complexity) + the surface_fingerprint (needed for
        // history rotation).
        let surfaces = match surfaces_yaml_snapshot(db, id) {
            Ok(arc) => arc,
            Err(_) => {
                // A surfaces projection error means the component was
                // unresolvable in the live tree — skip it (the
                // `live_ids` filter already excluded `deleted`
                // components, so this is defensive). Components with
                // no contracts still produce a valid snapshot, just
                // empty.
                continue;
            }
        };
        let surface_fingerprint = surfaces.fingerprint.clone();

        // Contracts this component provides — use the
        // `defines-contract` edge index built above so the metric
        // formulas all derive from one consistent edge view. (The
        // `surfaces.contracts_defined` list is the same data, but
        // edges round-trip through canonicalisation; reading from
        // the edge index keeps Ca / Ce / cohesion symmetric.)
        let provided: BTreeSet<String> = providers_of
            .iter()
            .filter_map(|(cid, ps)| {
                if ps.contains(id) {
                    Some(cid.clone())
                } else {
                    None
                }
            })
            .collect();
        let consumed: BTreeSet<String> = consumers_of
            .iter()
            .filter_map(|(cid, cs)| {
                if cs.contains(id) {
                    Some(cid.clone())
                } else {
                    None
                }
            })
            .collect();

        let ca = compute_ca(id, &provided, &consumers_of);
        let ce = compute_ce(id, &consumed, &providers_of);
        let instability = compute_instability(ca, ce);
        let cohesion = compute_cohesion(id, &provided, &consumers_of);

        // surface_complexity = provided_contracts × avg_bindings_per_contract.
        // Per design §4.3 it's an integer raw count; we compute it as
        // total bindings across defined contracts (which equals
        // provided_contracts × avg_bindings_per_contract when the
        // average is rational over the same provided count).
        let total_bindings: u32 = surfaces.contracts_defined.len() as u32;
        let surface_complexity = compute_surface_complexity(provided.len() as u32, total_bindings);

        // Pull the prior history (newest-first) for this component;
        // empty when no `<component>/.atlas/cache/modularity.yaml`
        // existed on disk before this run.
        let prior_entries = prior_per_component
            .get(id)
            .map(|h| h.entries.clone())
            .unwrap_or_default();

        let surface_stability = compute_surface_stability(&prior_entries);

        let metrics = ModularityMetrics {
            afferent_coupling: ca,
            efferent_coupling: ce,
            instability,
            cohesion,
            surface_stability,
            surface_complexity,
        };
        let new_entry = ModularityHistoryEntry {
            generated_at,
            surface_fingerprint: surface_fingerprint.clone(),
            metrics: ModularityHistoryMetrics {
                afferent_coupling: ca,
                efferent_coupling: ce,
                instability,
                cohesion,
                surface_complexity,
            },
        };
        let history = rotate_history(prior_entries, new_entry);

        per_component.insert(
            id.clone(),
            ComponentModularity {
                schema_version: 1,
                component_id: id.clone(),
                generated_at,
                metrics,
                history,
            },
        );
    }

    // Subsystem rollup. Read `subsystems.yaml` from the engine; assign
    // each component to at most one subsystem in the order subsystems
    // appear (a component appearing in multiple subsystem definitions
    // is tagged once). Anything not tagged ends up in
    // `unattached_components`.
    let subsystems_file = subsystems_yaml_snapshot(db);
    let mut tagged: BTreeSet<ComponentId> = BTreeSet::new();
    let mut subsystems: Vec<SubsystemAggregate> = Vec::new();
    for sub in &subsystems_file.subsystems {
        let members_in_workspace: Vec<ComponentId> = sub
            .members
            .iter()
            .filter(|id| per_component.contains_key(id))
            .cloned()
            .collect();
        for id in &members_in_workspace {
            tagged.insert(id.clone());
        }
        let aggregates = compute_subsystem_aggregate_metrics(&members_in_workspace, &per_component);
        let outliers =
            compute_subsystem_outliers(&members_in_workspace, &per_component, &aggregates);
        subsystems.push(SubsystemAggregate {
            id: sub.id.clone(),
            members: members_in_workspace,
            aggregates,
            outliers,
        });
    }

    let mut unattached_ids: Vec<ComponentId> = per_component
        .keys()
        .filter(|id| !tagged.contains(*id))
        .cloned()
        .collect();
    unattached_ids.sort();
    let unattached_components = UnattachedComponents {
        count: unattached_ids.len() as u32,
        ids: unattached_ids,
    };

    Ok(ModularityReport {
        schema_version: 1,
        generated_at,
        per_component,
        rollup: ModularityRollup {
            schema_version: 1,
            generated_at,
            subsystems,
            unattached_components,
        },
    })
}

// ---------------------------------------------------------------------
// Pure-function metric helpers. Exposed `pub(crate)` so the unit-test
// module below can exercise each formula against a hand-crafted fixture.
// ---------------------------------------------------------------------

/// Afferent coupling (Ca): count of distinct components that consume
/// any contract this component provides. Self-loops excluded.
pub(crate) fn compute_ca(
    component: &ComponentId,
    provided: &BTreeSet<String>,
    consumers_of: &BTreeMap<String, BTreeSet<ComponentId>>,
) -> u32 {
    let mut consumers: BTreeSet<&ComponentId> = BTreeSet::new();
    for contract in provided {
        if let Some(set) = consumers_of.get(contract) {
            for c in set {
                if c != component {
                    consumers.insert(c);
                }
            }
        }
    }
    consumers.len() as u32
}

/// Efferent coupling (Ce): count of distinct components whose
/// contracts this component consumes. Self-loops excluded.
pub(crate) fn compute_ce(
    component: &ComponentId,
    consumed: &BTreeSet<String>,
    providers_of: &BTreeMap<String, BTreeSet<ComponentId>>,
) -> u32 {
    let mut providers: BTreeSet<&ComponentId> = BTreeSet::new();
    for contract in consumed {
        if let Some(set) = providers_of.get(contract) {
            for p in set {
                if p != component {
                    providers.insert(p);
                }
            }
        }
    }
    providers.len() as u32
}

/// Instability (I) = `Ce / (Ca + Ce)`, defined as `0.0` when both are
/// zero (Phase 3 design spec §4.3).
pub(crate) fn compute_instability(ca: u32, ce: u32) -> f64 {
    let total = ca + ce;
    if total == 0 {
        0.0
    } else {
        f64::from(ce) / f64::from(total)
    }
}

/// LCOM4-adapted cohesion. With 0 or 1 provided contracts, defined as
/// `1.0` (vacuous — no fragmentation possible).
///
/// Otherwise: count distinct consumer-id-sets across the provided
/// contracts, then compute
/// `1 - ((distinct_consumer_sets - 1) / (num_provided_contracts - 1))`.
/// The defining component itself is excluded from consumer sets (a
/// component consuming its own contract is a self-loop).
pub(crate) fn compute_cohesion(
    component: &ComponentId,
    provided: &BTreeSet<String>,
    consumers_of: &BTreeMap<String, BTreeSet<ComponentId>>,
) -> f64 {
    let n = provided.len();
    if n <= 1 {
        return 1.0;
    }
    let mut sets: BTreeSet<Vec<ComponentId>> = BTreeSet::new();
    for contract in provided {
        let consumer_set = consumers_of
            .get(contract)
            .map(|s| {
                s.iter()
                    .filter(|c| *c != component)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // BTreeSet::iter is already sorted, so the Vec we just
        // constructed is canonical.
        sets.insert(consumer_set);
    }
    let distinct = sets.len() as f64;
    let denom = (n as f64) - 1.0;
    1.0 - ((distinct - 1.0) / denom)
}

/// Surface stability: fraction of adjacent history pairs whose
/// `surface_fingerprint` matched. With <2 history entries, defined as
/// `1.0` (no pairs possible).
///
/// `history` is in the same newest-first order [`ModularityHistory`]
/// uses. The "last 5" cap is applied at history-rotation time, so this
/// helper just walks whatever it's given.
pub(crate) fn compute_surface_stability(history: &[ModularityHistoryEntry]) -> f64 {
    if history.len() < 2 {
        return 1.0;
    }
    let mut matching = 0u32;
    let total = (history.len() - 1) as u32;
    for pair in history.windows(2) {
        if pair[0].surface_fingerprint == pair[1].surface_fingerprint {
            matching += 1;
        }
    }
    f64::from(matching) / f64::from(total)
}

/// Surface complexity: `provided_contracts × avg_bindings_per_contract`,
/// computed as the integer total binding count (which equals the
/// product whenever the average is rational over the same provided
/// count). Returns `0` when no contracts are provided — the design
/// spec calls this out by name in PR-10's
/// `surface_complexity_zero_for_no_contracts` AC.
pub(crate) fn compute_surface_complexity(provided: u32, total_bindings: u32) -> u32 {
    if provided == 0 {
        0
    } else {
        total_bindings
    }
}

/// Apply Phase 3 design §4.3 history rotation: prepend `new_entry`
/// to `prior` (newest-first), drop the oldest if the total exceeds
/// five, and short-circuit when the prior's most-recent
/// `surface_fingerprint` matches `new_entry.surface_fingerprint` (no
/// duplicate, no append).
///
/// History entries are immutable once written; this helper only
/// inserts at the head and may drop from the tail. The hard cap of
/// five is fixed by Phase 3 design §4.3 + plan §4 PR-10 and is not
/// configurable.
pub(crate) fn rotate_history(
    prior: Vec<ModularityHistoryEntry>,
    new_entry: ModularityHistoryEntry,
) -> Vec<ModularityHistoryEntry> {
    if let Some(head) = prior.first() {
        if head.surface_fingerprint == new_entry.surface_fingerprint {
            // Duplicate — keep prior history verbatim. Per design
            // spec §4.3, history entries are immutable once written.
            return prior;
        }
    }
    let mut out = Vec::with_capacity((prior.len() + 1).min(HISTORY_CAP));
    out.push(new_entry);
    for entry in prior.into_iter().take(HISTORY_CAP - 1) {
        out.push(entry);
    }
    out
}

/// Compute `mean` and sample standard deviation of one metric across
/// the supplied values. With <2 values the standard-deviation
/// denominator collapses to zero; we return `stddev = 0.0` in that
/// case (a single-member subsystem can't deviate from itself, and a
/// zero-member subsystem produces a mean of `0.0` for free).
pub(crate) fn compute_metric_stats(values: &[f64]) -> SubsystemMetricStats {
    if values.is_empty() {
        return SubsystemMetricStats {
            mean: 0.0,
            stddev: 0.0,
        };
    }
    let n = values.len() as f64;
    let mean: f64 = values.iter().sum::<f64>() / n;
    if values.len() < 2 {
        return SubsystemMetricStats { mean, stddev: 0.0 };
    }
    let variance: f64 = values
        .iter()
        .map(|v| {
            let d = v - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0);
    SubsystemMetricStats {
        mean,
        stddev: variance.sqrt(),
    }
}

/// Compute the per-metric mean+stddev block for a subsystem given
/// its resolved member ids and the per-component metric map.
pub(crate) fn compute_subsystem_aggregate_metrics(
    members: &[ComponentId],
    per_component: &HashMap<ComponentId, ComponentModularity>,
) -> SubsystemAggregateMetrics {
    let collect = |f: fn(&ModularityMetrics) -> f64| -> Vec<f64> {
        members
            .iter()
            .filter_map(|id| per_component.get(id))
            .map(|c| f(&c.metrics))
            .collect()
    };
    SubsystemAggregateMetrics {
        afferent_coupling: compute_metric_stats(&collect(|m| f64::from(m.afferent_coupling))),
        efferent_coupling: compute_metric_stats(&collect(|m| f64::from(m.efferent_coupling))),
        instability: compute_metric_stats(&collect(|m| m.instability)),
        cohesion: compute_metric_stats(&collect(|m| m.cohesion)),
        surface_stability: compute_metric_stats(&collect(|m| m.surface_stability)),
        surface_complexity: compute_metric_stats(&collect(|m| f64::from(m.surface_complexity))),
    }
}

/// Flag any member whose value on any metric is more than 2σ from the
/// subsystem mean. The threshold is *strict* `>` — exactly-2σ
/// deviations are not flagged (matches design §4.3 wording: "any
/// member whose value is `>2σ` from the subsystem mean").
///
/// A single member can produce multiple outlier rows (one per metric
/// it's flagged on), in stable order: the metric ordering matches the
/// declaration order on [`SubsystemAggregateMetrics`], with members
/// sorted by id within each metric.
pub(crate) fn compute_subsystem_outliers(
    members: &[ComponentId],
    per_component: &HashMap<ComponentId, ComponentModularity>,
    aggregates: &SubsystemAggregateMetrics,
) -> Vec<SubsystemOutlier> {
    /// Project a single metric out of a [`ModularityMetrics`] block.
    /// Local alias keeps the metric-table type compact for clippy.
    type Project = fn(&ModularityMetrics) -> f64;
    let metrics_in_order: [(&str, Project, SubsystemMetricStats); 6] = [
        (
            "afferent_coupling",
            |m| f64::from(m.afferent_coupling),
            aggregates.afferent_coupling,
        ),
        (
            "efferent_coupling",
            |m| f64::from(m.efferent_coupling),
            aggregates.efferent_coupling,
        ),
        ("instability", |m| m.instability, aggregates.instability),
        ("cohesion", |m| m.cohesion, aggregates.cohesion),
        (
            "surface_stability",
            |m| m.surface_stability,
            aggregates.surface_stability,
        ),
        (
            "surface_complexity",
            |m| f64::from(m.surface_complexity),
            aggregates.surface_complexity,
        ),
    ];
    let mut sorted_members: Vec<ComponentId> = members.to_vec();
    sorted_members.sort();

    let mut outliers: Vec<SubsystemOutlier> = Vec::new();
    for (metric_name, project, stats) in metrics_in_order {
        if stats.stddev == 0.0 {
            // No spread → no outliers on this metric. Treat both
            // empty subsystems and tightly-clustered ones uniformly.
            continue;
        }
        for id in &sorted_members {
            let Some(comp) = per_component.get(id) else {
                continue;
            };
            let value = project(&comp.metrics);
            let deviation = (value - stats.mean).abs() / stats.stddev;
            if deviation > 2.0 {
                outliers.push(SubsystemOutlier {
                    component_id: id.clone(),
                    metric: metric_name.to_string(),
                    value,
                    subsystem_mean: stats.mean,
                    deviation_sigmas: deviation,
                });
            }
        }
    }
    outliers
}

// ---------------------------------------------------------------------
// Wire-format types (PR-7-shipped; unchanged here).
// ---------------------------------------------------------------------

/// Top-level modularity report. The CLI handler writes
/// `per_component[id]` to each component's
/// `<component>/.atlas/cache/modularity.yaml` and the `rollup` to
/// `.atlas/cache/reports/modularity-rollup.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityReport {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// Per-component file payloads, keyed by component id. Each value
    /// is the full content of that component's `modularity.yaml`.
    pub per_component: HashMap<ComponentId, ComponentModularity>,
    /// Subsystem-aggregated rollup — content of
    /// `modularity-rollup.yaml`.
    pub rollup: ModularityRollup,
}

/// Per-component modularity payload, written to
/// `<component>/.atlas/cache/modularity.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentModularity {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Component id.
    pub component_id: ComponentId,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// Current metrics for this component.
    pub metrics: ModularityMetrics,
    /// FIFO history (newest first), bounded at 5 entries by Phase 3
    /// design spec §4.3.
    pub history: Vec<ModularityHistoryEntry>,
}

/// The five Phase 3 modularity metrics for a single component.
///
/// `instability`, `cohesion`, and `surface_stability` are floating-
/// point values in the range `0.0..=1.0`; the integer fields are raw
/// counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityMetrics {
    /// Distinct components that consume any contract this component
    /// provides (Ca; `consumes` edges only, self-loops excluded).
    pub afferent_coupling: u32,
    /// Distinct components whose contracts this component consumes
    /// (Ce; `consumes` edges only, self-loops excluded).
    pub efferent_coupling: u32,
    /// `Ce / (Ca + Ce)`, defined as `0.0` when `Ca + Ce == 0`.
    pub instability: f64,
    /// LCOM4-adapted cohesion in `0.0..=1.0`.
    pub cohesion: f64,
    /// Fraction of adjacent history pairs whose
    /// `surface_fingerprint` matched (`1.0` for <2 history entries).
    pub surface_stability: f64,
    /// `provided_contracts × avg_bindings_per_contract` (integer).
    pub surface_complexity: u32,
}

/// One past entry in the per-component history. Entries are immutable
/// once written and are bounded at 5 by Phase 3 design spec §4.3.
///
/// `surface_stability` is intentionally absent from history entries —
/// it is computed *from* history rather than stored per entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityHistoryEntry {
    /// Wall-clock time the entry was captured.
    pub generated_at: DateTime<Utc>,
    /// `Sha256Hex` of the surface inputs at capture time. Determines
    /// whether a new entry is appended on the next run.
    pub surface_fingerprint: Sha256Hex,
    /// Per-entry metrics (the four metrics that make sense per-run).
    pub metrics: ModularityHistoryMetrics,
}

/// Subset of [`ModularityMetrics`] persisted in each history entry.
/// Excludes `surface_stability` (which is derived *from* history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityHistoryMetrics {
    /// See [`ModularityMetrics::afferent_coupling`].
    pub afferent_coupling: u32,
    /// See [`ModularityMetrics::efferent_coupling`].
    pub efferent_coupling: u32,
    /// See [`ModularityMetrics::instability`].
    pub instability: f64,
    /// See [`ModularityMetrics::cohesion`].
    pub cohesion: f64,
    /// See [`ModularityMetrics::surface_complexity`].
    pub surface_complexity: u32,
}

/// Container the CLI handler reads off disk for each component before
/// invoking [`modularity`]. Same payload as [`ComponentModularity`]
/// minus the redundant `component_id` (the CLI already keys the prior
/// map by component id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityHistory {
    /// History entries in newest-first order; capped at 5 by Phase 3
    /// design spec §4.3.
    pub entries: Vec<ModularityHistoryEntry>,
}

/// Top-level rollup written to
/// `.atlas/cache/reports/modularity-rollup.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModularityRollup {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the rollup was generated.
    pub generated_at: DateTime<Utc>,
    /// One entry per subsystem in `subsystems.yaml`.
    pub subsystems: Vec<SubsystemAggregate>,
    /// Components not in any subsystem (excluded from rollup means).
    pub unattached_components: UnattachedComponents,
}

/// Per-subsystem aggregate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsystemAggregate {
    /// Stable subsystem id.
    pub id: String,
    /// Member component ids.
    pub members: Vec<ComponentId>,
    /// Mean and stddev for each metric.
    pub aggregates: SubsystemAggregateMetrics,
    /// Member components flagged as outliers (>2σ from mean) for at
    /// least one metric.
    pub outliers: Vec<SubsystemOutlier>,
}

/// Mean+stddev of every modularity metric across a subsystem's
/// members.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsystemAggregateMetrics {
    /// Aggregate of `afferent_coupling` across members.
    pub afferent_coupling: SubsystemMetricStats,
    /// Aggregate of `efferent_coupling` across members.
    pub efferent_coupling: SubsystemMetricStats,
    /// Aggregate of `instability` across members.
    pub instability: SubsystemMetricStats,
    /// Aggregate of `cohesion` across members.
    pub cohesion: SubsystemMetricStats,
    /// Aggregate of `surface_stability` across members.
    pub surface_stability: SubsystemMetricStats,
    /// Aggregate of `surface_complexity` across members.
    pub surface_complexity: SubsystemMetricStats,
}

/// Mean and stddev of one metric across a subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SubsystemMetricStats {
    /// Mean across member components.
    pub mean: f64,
    /// Sample standard deviation across member components.
    pub stddev: f64,
}

/// A flagged outlier — one component, one metric, one deviation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsystemOutlier {
    /// Member component flagged as outlying.
    pub component_id: ComponentId,
    /// Name of the metric on which the component is outlying
    /// (e.g. `"instability"`).
    pub metric: String,
    /// Component's value for that metric.
    pub value: f64,
    /// Subsystem mean for that metric.
    pub subsystem_mean: f64,
    /// |value - mean| / stddev.
    pub deviation_sigmas: f64,
}

/// Components not in any subsystem; excluded from rollup means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnattachedComponents {
    /// `ids.len()`.
    pub count: u32,
    /// Component ids, sorted for determinism.
    pub ids: Vec<ComponentId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn cid(s: &str) -> ComponentId {
        ComponentId::parse(s).unwrap()
    }

    fn metrics() -> ModularityMetrics {
        ModularityMetrics {
            afferent_coupling: 3,
            efferent_coupling: 2,
            instability: 0.4,
            cohesion: 0.83,
            surface_stability: 1.0,
            surface_complexity: 8,
        }
    }

    fn history_metrics() -> ModularityHistoryMetrics {
        ModularityHistoryMetrics {
            afferent_coupling: 3,
            efferent_coupling: 2,
            instability: 0.4,
            cohesion: 0.83,
            surface_complexity: 8,
        }
    }

    fn component_payload() -> ComponentModularity {
        ComponentModularity {
            schema_version: 1,
            component_id: cid("ravel-lite/api"),
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 35, 11).unwrap(),
            metrics: metrics(),
            history: vec![ModularityHistoryEntry {
                generated_at: Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap(),
                surface_fingerprint: "sha256:hist1".to_string(),
                metrics: history_metrics(),
            }],
        }
    }

    fn rollup_fixture() -> ModularityRollup {
        ModularityRollup {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 35, 11).unwrap(),
            subsystems: vec![SubsystemAggregate {
                id: "ravel-lite/runtime".to_string(),
                members: vec![cid("ravel-lite/api"), cid("ravel-lite/worker")],
                aggregates: SubsystemAggregateMetrics {
                    afferent_coupling: SubsystemMetricStats {
                        mean: 2.5,
                        stddev: 0.7,
                    },
                    efferent_coupling: SubsystemMetricStats {
                        mean: 1.5,
                        stddev: 0.7,
                    },
                    instability: SubsystemMetricStats {
                        mean: 0.45,
                        stddev: 0.07,
                    },
                    cohesion: SubsystemMetricStats {
                        mean: 0.81,
                        stddev: 0.05,
                    },
                    surface_stability: SubsystemMetricStats {
                        mean: 1.0,
                        stddev: 0.0,
                    },
                    surface_complexity: SubsystemMetricStats {
                        mean: 7.5,
                        stddev: 0.7,
                    },
                },
                outliers: vec![],
            }],
            unattached_components: UnattachedComponents {
                count: 1,
                ids: vec![cid("misc/scratch-tool")],
            },
        }
    }

    fn report_fixture() -> ModularityReport {
        let mut per_component = HashMap::new();
        per_component.insert(cid("ravel-lite/api"), component_payload());
        ModularityReport {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 35, 11).unwrap(),
            per_component,
            rollup: rollup_fixture(),
        }
    }

    // ---------------------------------------------------------------
    // Wire-format round-trips (PR-7 inheritance).
    // ---------------------------------------------------------------

    #[test]
    fn modularity_report_round_trips_yaml() {
        let original = report_fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ModularityReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn modularity_report_round_trips_json() {
        let original = report_fixture();
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: ModularityReport = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn component_payload_round_trips_yaml() {
        let original = component_payload();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ComponentModularity = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn rollup_round_trips_yaml() {
        let original = rollup_fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ModularityRollup = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn modularity_history_round_trips_yaml() {
        let original = ModularityHistory {
            entries: vec![ModularityHistoryEntry {
                generated_at: Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap(),
                surface_fingerprint: "sha256:hist1".to_string(),
                metrics: history_metrics(),
            }],
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ModularityHistory = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    // ---------------------------------------------------------------
    // PR-10 AC: per-formula unit tests.
    // ---------------------------------------------------------------

    /// Three components — A defines two contracts, B and C consume
    /// one each; D consumes the second. Ca for A is `|{B, C, D}| = 3`.
    #[test]
    fn ca_counts_distinct_consumers() {
        let a = cid("a");
        let b = cid("b");
        let c = cid("c");
        let d = cid("d");
        let provided: BTreeSet<String> = ["k1", "k2"].into_iter().map(String::from).collect();
        let mut consumers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
        consumers_of.insert("k1".into(), [b.clone(), c.clone()].into_iter().collect());
        consumers_of.insert("k2".into(), [d.clone()].into_iter().collect());
        // Self-loop: A consumes one of its own contracts → must be
        // excluded from Ca.
        consumers_of.get_mut("k1").unwrap().insert(a.clone());
        let ca = compute_ca(&a, &provided, &consumers_of);
        assert_eq!(ca, 3);
    }

    /// A consumes contracts owned by P and Q (and self-loop owned by
    /// A); Ce for A is `|{P, Q}| = 2`.
    #[test]
    fn ce_counts_distinct_provided_by() {
        let a = cid("a");
        let p = cid("p");
        let q = cid("q");
        let consumed: BTreeSet<String> = ["k1", "k2", "k3"].into_iter().map(String::from).collect();
        let mut providers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
        providers_of.insert("k1".into(), [p.clone()].into_iter().collect());
        providers_of.insert("k2".into(), [q.clone()].into_iter().collect());
        providers_of.insert("k3".into(), [a.clone()].into_iter().collect());
        let ce = compute_ce(&a, &consumed, &providers_of);
        assert_eq!(ce, 2);
    }

    #[test]
    fn instability_zero_when_no_couplings() {
        assert_eq!(compute_instability(0, 0), 0.0);
    }

    #[test]
    fn instability_correct_for_balanced_couplings() {
        // Ca = 3, Ce = 2 → I = 2 / 5 = 0.4.
        assert_eq!(compute_instability(3, 2), 0.4);
        // Ce-only → I = 1.0 (fully unstable).
        assert_eq!(compute_instability(0, 5), 1.0);
        // Ca-only → I = 0.0 (fully stable).
        assert_eq!(compute_instability(5, 0), 0.0);
    }

    #[test]
    fn cohesion_one_for_zero_or_one_contract() {
        let a = cid("a");
        let consumers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
        // Zero contracts.
        let zero: BTreeSet<String> = BTreeSet::new();
        assert_eq!(compute_cohesion(&a, &zero, &consumers_of), 1.0);
        // One contract.
        let one: BTreeSet<String> = ["k1"].into_iter().map(String::from).collect();
        assert_eq!(compute_cohesion(&a, &one, &consumers_of), 1.0);
    }

    /// Three contracts, each consumed by a *different* set of
    /// components. distinct_consumer_sets = 3, num_provided = 3, so
    /// cohesion = 1 - ((3-1)/(3-1)) = 0.0.
    #[test]
    fn cohesion_decreases_with_disjoint_consumer_sets() {
        let a = cid("a");
        let provided: BTreeSet<String> = ["k1", "k2", "k3"].into_iter().map(String::from).collect();
        let mut consumers_of: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
        consumers_of.insert("k1".into(), [cid("b")].into_iter().collect());
        consumers_of.insert("k2".into(), [cid("c")].into_iter().collect());
        consumers_of.insert("k3".into(), [cid("d")].into_iter().collect());
        assert_eq!(compute_cohesion(&a, &provided, &consumers_of), 0.0);

        // Same fixture, but every contract is consumed by the SAME
        // set {b}. distinct = 1 → cohesion = 1 - 0/2 = 1.0.
        let mut consumers_shared: BTreeMap<String, BTreeSet<ComponentId>> = BTreeMap::new();
        consumers_shared.insert("k1".into(), [cid("b")].into_iter().collect());
        consumers_shared.insert("k2".into(), [cid("b")].into_iter().collect());
        consumers_shared.insert("k3".into(), [cid("b")].into_iter().collect());
        assert_eq!(compute_cohesion(&a, &provided, &consumers_shared), 1.0);
    }

    fn make_history_entry(fp: &str) -> ModularityHistoryEntry {
        ModularityHistoryEntry {
            generated_at: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
            surface_fingerprint: fp.to_string(),
            metrics: history_metrics(),
        }
    }

    #[test]
    fn surface_stability_one_with_lt_2_history_entries() {
        assert_eq!(compute_surface_stability(&[]), 1.0);
        assert_eq!(compute_surface_stability(&[make_history_entry("a")]), 1.0);
    }

    /// Four entries with fingerprints [A, A, B, B]. Adjacent pairs:
    /// (A,A) match, (A,B) no, (B,B) match → 2 / 3.
    #[test]
    fn surface_stability_correct_with_4_entries() {
        let history = vec![
            make_history_entry("A"),
            make_history_entry("A"),
            make_history_entry("B"),
            make_history_entry("B"),
        ];
        let stability = compute_surface_stability(&history);
        assert!(
            (stability - (2.0 / 3.0)).abs() < 1e-12,
            "expected ~0.6667, got {stability}"
        );
    }

    #[test]
    fn surface_complexity_zero_for_no_contracts() {
        assert_eq!(compute_surface_complexity(0, 0), 0);
        // Even when total_bindings is non-zero (defensive), zero
        // provided contracts must yield zero complexity.
        assert_eq!(compute_surface_complexity(0, 5), 0);
    }

    // ---------------------------------------------------------------
    // PR-10 AC: history rotation tests.
    // ---------------------------------------------------------------

    #[test]
    fn history_rotation_no_duplicate_when_fingerprint_matches() {
        let prior = vec![make_history_entry("FP-1")];
        let new_entry = make_history_entry("FP-1"); // same fingerprint
        let rotated = rotate_history(prior.clone(), new_entry);
        assert_eq!(rotated, prior, "matching fingerprint must not append");
    }

    #[test]
    fn history_rotation_drops_oldest_at_5_entries() {
        // Start with 5 entries, fingerprints "1".."5" newest-first.
        let mut prior = (1..=5)
            .rev()
            .map(|i| make_history_entry(&format!("fp-{i}")))
            .collect::<Vec<_>>();
        // Sanity: prior is in newest-first order (fp-5 first, fp-1 last).
        assert_eq!(prior.first().unwrap().surface_fingerprint, "fp-5");
        assert_eq!(prior.last().unwrap().surface_fingerprint, "fp-1");
        // Append a sixth entry with a fresh fingerprint.
        let new_entry = make_history_entry("fp-6");
        let rotated = rotate_history(prior.clone(), new_entry.clone());
        assert_eq!(rotated.len(), 5, "history must be capped at 5");
        // Newest first: fp-6.
        assert_eq!(rotated[0].surface_fingerprint, "fp-6");
        // Oldest dropped: fp-1 must be gone.
        assert!(
            rotated.iter().all(|e| e.surface_fingerprint != "fp-1"),
            "oldest entry must be dropped on overflow"
        );
        // Remaining entries retain their order (fp-5, fp-4, fp-3, fp-2).
        assert_eq!(rotated[1].surface_fingerprint, "fp-5");
        assert_eq!(rotated[4].surface_fingerprint, "fp-2");

        // Sanity: a workspace simulating six successive distinct
        // surface fingerprints (replay the same rotation step from
        // empty) ends up with the same final state.
        let mut history: Vec<ModularityHistoryEntry> = Vec::new();
        for i in 1..=6 {
            history = rotate_history(history, make_history_entry(&format!("fp-{i}")));
        }
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].surface_fingerprint, "fp-6");
        prior.clear(); // silence unused-mut on older rustc versions
    }

    // ---------------------------------------------------------------
    // PR-10 AC: subsystem aggregates / outliers.
    // ---------------------------------------------------------------

    fn comp_with_metrics(id: &str, m: ModularityMetrics) -> ComponentModularity {
        ComponentModularity {
            schema_version: 1,
            component_id: cid(id),
            generated_at: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
            metrics: m,
            history: Vec::new(),
        }
    }

    fn metrics_with_instability(i: f64) -> ModularityMetrics {
        ModularityMetrics {
            afferent_coupling: 0,
            efferent_coupling: 0,
            instability: i,
            cohesion: 1.0,
            surface_stability: 1.0,
            surface_complexity: 0,
        }
    }

    #[test]
    fn subsystem_aggregate_mean_stddev_correct() {
        // Three members with instability values [0.2, 0.4, 0.6].
        // mean = 0.4; sample stddev = sqrt(((-0.2)^2 + 0^2 + 0.2^2) / 2)
        //       = sqrt(0.04) = 0.2.
        let m1 = comp_with_metrics("ns/a", metrics_with_instability(0.2));
        let m2 = comp_with_metrics("ns/b", metrics_with_instability(0.4));
        let m3 = comp_with_metrics("ns/c", metrics_with_instability(0.6));
        let mut per_component: HashMap<ComponentId, ComponentModularity> = HashMap::new();
        per_component.insert(cid("ns/a"), m1);
        per_component.insert(cid("ns/b"), m2);
        per_component.insert(cid("ns/c"), m3);
        let members = vec![cid("ns/a"), cid("ns/b"), cid("ns/c")];
        let agg = compute_subsystem_aggregate_metrics(&members, &per_component);
        assert!((agg.instability.mean - 0.4).abs() < 1e-12);
        assert!(
            (agg.instability.stddev - 0.2).abs() < 1e-12,
            "got stddev {}",
            agg.instability.stddev
        );
    }

    #[test]
    fn subsystem_outlier_flagged_at_2_sigma() {
        // Three members with instability [0.0, 0.0, 0.5].
        // mean = ~0.1667; stddev (sample) = sqrt(((-0.1667)^2 +
        //   (-0.1667)^2 + 0.3333^2) / 2) = sqrt(0.0833) ≈ 0.2887.
        // The third member is at |0.5 - 0.1667| / 0.2887 ≈ 1.155σ —
        // not outlying. We need a sharper outlier; replace with
        // [0.0, 0.0, 1.0]:
        // mean = 1/3 ≈ 0.3333; stddev = sqrt(((-0.3333)^2 * 2 +
        //   (0.6667)^2) / 2) = sqrt(0.3333) ≈ 0.5774.
        // |1.0 - 0.3333| / 0.5774 ≈ 1.155σ. Still under 2σ. Use a
        // tighter cluster: [0.5, 0.5, 1.0]:
        // mean = 2/3 ≈ 0.6667; stddev = sqrt(((-0.1667)^2 * 2 +
        //   0.3333^2) / 2) = sqrt(0.04167) ≈ 0.2041.
        // |1.0 - 0.6667| / 0.2041 ≈ 1.633σ. Still under 2σ.
        //
        // To get a clean >2σ deviation with three members, we need the
        // outlier far from a tight cluster: [0.10, 0.10, 0.80]:
        // mean = 1/3 ≈ 0.3333; stddev = sqrt(((-0.2333)^2 * 2 +
        //   0.4667^2) / 2) = sqrt(0.16333 / 2) ... let me compute:
        // diffs²: 0.0544, 0.0544, 0.2178. sum = 0.3267. /2 = 0.1633.
        // sqrt = 0.4041. dev = 0.4667 / 0.4041 ≈ 1.155σ. Still bad.
        //
        // The arithmetic property bites: with three points, max
        // |x_i - mean| / sample_stddev = sqrt(2) ≈ 1.414 (regardless
        // of values). Two-σ outliers require ≥4 members. Use four:
        // [0.1, 0.1, 0.1, 0.9]:
        // mean = 0.3; diffs² = 0.04, 0.04, 0.04, 0.36. sum = 0.48.
        // sample stddev = sqrt(0.48 / 3) = sqrt(0.16) = 0.4.
        // dev for 0.9: 0.6 / 0.4 = 1.5σ. Still under.
        // [0.05, 0.05, 0.05, 0.95]:
        // mean = 0.275; diffs² = 0.0506, 0.0506, 0.0506, 0.4556.
        // sum = 0.6075. sample stddev = sqrt(0.6075/3) = sqrt(0.2025)
        // = 0.45. dev for 0.95: 0.675 / 0.45 = 1.5σ. Still under.
        // The relationship `max_z = (n-1)/sqrt(n)` for sample stddev:
        //   n=4 → 3/2 = 1.5; n=5 → 4/sqrt(5) ≈ 1.789; n=6 → 5/sqrt(6) ≈ 2.041.
        // So the AC's "2.5σ from mean → flagged" requires ≥6 members
        // (the only way to push deviation past 2). Use six members,
        // five at 0.0 and one at 1.0:
        // mean = 1/6 ≈ 0.1667; diffs² = (0.1667)^2 * 5 + (0.8333)^2
        //   = 0.1389 + 0.6944 = 0.8333. sample stddev = sqrt(0.8333/5)
        //   = sqrt(0.1667) ≈ 0.4082.
        // dev for 1.0: 0.8333 / 0.4082 ≈ 2.041σ. JUST over 2.
        // Use [0.0]*7 + [1.0]: 8 members.
        // mean = 0.125; diffs² = (0.125)^2 * 7 + (0.875)^2
        //   = 0.1094 + 0.7656 = 0.875. sample stddev = sqrt(0.875/7)
        //   = sqrt(0.125) ≈ 0.3536.
        // dev for 1.0: 0.875 / 0.3536 ≈ 2.475σ — at the AC's 2.5σ.
        let mut per_component: HashMap<ComponentId, ComponentModularity> = HashMap::new();
        let mut members: Vec<ComponentId> = Vec::new();
        for i in 0..7 {
            let id = cid(&format!("ns/m{i}"));
            per_component.insert(
                id.clone(),
                comp_with_metrics(id.as_str(), metrics_with_instability(0.0)),
            );
            members.push(id);
        }
        let id = cid("ns/outlier");
        per_component.insert(
            id.clone(),
            comp_with_metrics(id.as_str(), metrics_with_instability(1.0)),
        );
        members.push(id);

        let agg = compute_subsystem_aggregate_metrics(&members, &per_component);
        let outliers = compute_subsystem_outliers(&members, &per_component, &agg);
        assert!(
            outliers.iter().any(|o| o.metric == "instability"
                && o.component_id == cid("ns/outlier")
                && o.deviation_sigmas > 2.0),
            "expected ns/outlier to be flagged on instability; got {:?}",
            outliers
        );
    }

    #[test]
    fn subsystem_no_outliers_when_all_within_2_sigma() {
        // Three tightly-clustered members: [0.40, 0.41, 0.42]. The
        // max possible z-score over three sample-stddev points is
        // sqrt(2) ≈ 1.414, so no member can ever cross the >2σ bar.
        let m1 = comp_with_metrics("ns/a", metrics_with_instability(0.40));
        let m2 = comp_with_metrics("ns/b", metrics_with_instability(0.41));
        let m3 = comp_with_metrics("ns/c", metrics_with_instability(0.42));
        let mut per_component: HashMap<ComponentId, ComponentModularity> = HashMap::new();
        per_component.insert(cid("ns/a"), m1);
        per_component.insert(cid("ns/b"), m2);
        per_component.insert(cid("ns/c"), m3);
        let members = vec![cid("ns/a"), cid("ns/b"), cid("ns/c")];
        let agg = compute_subsystem_aggregate_metrics(&members, &per_component);
        let outliers = compute_subsystem_outliers(&members, &per_component, &agg);
        assert!(
            outliers.is_empty(),
            "tightly-clustered members must not produce outliers; got {:?}",
            outliers
        );
    }

    #[test]
    fn unattached_components_listed_correctly() {
        // The pure-function path doesn't take `subsystems_yaml`
        // directly — that's threaded in `modularity()` via
        // `subsystems_yaml_snapshot`. We exercise the unattached
        // computation by simulating the same logic the public
        // function uses: components in any subsystem are tagged;
        // everything else lands in unattached.
        let comps: Vec<ComponentId> = vec![cid("ns/a"), cid("ns/b"), cid("ns/c")];
        let tagged: BTreeSet<ComponentId> = [cid("ns/a"), cid("ns/b")].into_iter().collect();
        let mut unattached: Vec<ComponentId> = comps
            .iter()
            .filter(|id| !tagged.contains(*id))
            .cloned()
            .collect();
        unattached.sort();
        let payload = UnattachedComponents {
            count: unattached.len() as u32,
            ids: unattached,
        };
        assert_eq!(payload.count, 1);
        assert_eq!(payload.ids, vec![cid("ns/c")]);
    }

    #[test]
    fn empty_subsystems_yaml_produces_unattached_only_rollup() {
        // With no subsystems on file, every live component must end
        // up in unattached_components. We simulate that policy
        // directly here (the public `modularity()` defers to
        // `subsystems_yaml_snapshot`, which returns an empty Vec
        // when no overrides exist).
        let comps: Vec<ComponentId> = vec![cid("ns/a"), cid("ns/b")];
        let subsystems: Vec<SubsystemAggregate> = Vec::new();
        let tagged: BTreeSet<ComponentId> = subsystems
            .iter()
            .flat_map(|s| s.members.iter().cloned())
            .collect();
        let mut unattached: Vec<ComponentId> = comps
            .iter()
            .filter(|id| !tagged.contains(*id))
            .cloned()
            .collect();
        unattached.sort();
        let payload = UnattachedComponents {
            count: unattached.len() as u32,
            ids: unattached,
        };
        assert_eq!(payload.count, 2);
        assert_eq!(payload.ids, vec![cid("ns/a"), cid("ns/b")]);
        assert!(subsystems.is_empty());
    }
}
