//! Composition divergence report —
//! `.atlas/cache/reports/composition-divergence.yaml`.
//!
//! Schema is fixed by Phase 3 design spec §4.4. PR-7 ships only the
//! report types and a stubbed [`divergence`] entry-point that returns
//! [`ReportError::NotImplemented`]; PR-11 lands the actual pair-wise
//! classification logic.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::snapshot::ContractShaSnapshot;
use crate::types::{ContractId, ReportError, ReportInputs};

/// For each unordered pair of components, classify as build-only,
/// deploy-only, both, or neither; flag pairs that are coupled in
/// exactly one of the two graphs as **divergent** and score severity
/// against the drift baseline. PR-7 stub: always returns
/// [`ReportError::NotImplemented`].
pub fn divergence(
    _inputs: ReportInputs,
    _drift_baseline: Option<&ContractShaSnapshot>,
) -> Result<DivergenceReport, ReportError> {
    Err(ReportError::NotImplemented)
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
    use chrono::TimeZone;

    fn fixture() -> DivergenceReport {
        let mut by_severity = BTreeMap::new();
        by_severity.insert(0, 1);
        by_severity.insert(2, 1);

        DivergenceReport {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 40, 11).unwrap(),
            drift_baseline_at: Some(Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap()),
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
        let original = fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: DivergenceReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn divergence_report_round_trips_json() {
        let original = fixture();
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
        let mut original = fixture();
        original.drift_baseline_at = None;
        for pair in &mut original.divergent_pairs {
            pair.severity = None;
        }
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: DivergenceReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }
}
