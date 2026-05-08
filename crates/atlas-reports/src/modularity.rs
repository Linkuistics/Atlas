//! Modularity report — per-component metric files plus a top-level
//! subsystem rollup. Schema is fixed by Phase 3 design spec §4.3.
//!
//! PR-7 ships only the report types and a stubbed [`modularity`]
//! entry-point that returns [`ReportError::NotImplemented`]; PR-10
//! lands the actual metric computation and history rotation.

use std::collections::HashMap;

use atlas_engine::Sha256Hex;
use chrono::{DateTime, Utc};
use component_ontology::ComponentId;
use serde::{Deserialize, Serialize};

use crate::types::{ReportError, ReportInputs};

/// Compute modularity metrics for every component in the workspace,
/// rotate per-component history, and emit a subsystem-level rollup.
/// PR-7 stub: always returns [`ReportError::NotImplemented`].
pub fn modularity(
    _inputs: ReportInputs,
    _prior_per_component: HashMap<ComponentId, ModularityHistory>,
) -> Result<ModularityReport, ReportError> {
    Err(ReportError::NotImplemented)
}

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
            component_id: ComponentId::parse("ravel-lite/api").unwrap(),
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
                members: vec![
                    ComponentId::parse("ravel-lite/api").unwrap(),
                    ComponentId::parse("ravel-lite/worker").unwrap(),
                ],
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
                ids: vec![ComponentId::parse("misc/scratch-tool").unwrap()],
            },
        }
    }

    fn report_fixture() -> ModularityReport {
        let mut per_component = HashMap::new();
        per_component.insert(
            ComponentId::parse("ravel-lite/api").unwrap(),
            component_payload(),
        );
        ModularityReport {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 35, 11).unwrap(),
            per_component,
            rollup: rollup_fixture(),
        }
    }

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
}
