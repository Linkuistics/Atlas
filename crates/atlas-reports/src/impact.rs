//! Impact query — `atlas impact <id>`, output to stdout (no file
//! written; the `--no-write` flag is intentionally rejected by the
//! CLI).
//!
//! Schema is fixed by Phase 3 design spec §4.2. PR-7 ships only the
//! report types and a stubbed [`impact`] entry-point that returns
//! [`ReportError::NotImplemented`]; PR-9 lands the actual traversal.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{ImpactTarget, ReportError, ReportInputs};

/// Walk downstream consumers of a contract or component, returning
/// direct + transitive consumer sets and three independent partitions
/// (by language, deploy graph, lifecycle). PR-7 stub: always returns
/// [`ReportError::NotImplemented`].
pub fn impact(_inputs: ReportInputs, _target: ImpactTarget) -> Result<ImpactReport, ReportError> {
    Err(ReportError::NotImplemented)
}

/// Top-level impact report (Phase 3 design spec §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// The query target, echoed in the report.
    pub target: ImpactTargetView,
    /// Direct consumers of the target (one hop on `consumes` edges).
    pub direct: Vec<ImpactNode>,
    /// Transitive closure (includes direct).
    pub transitive: Vec<ImpactNode>,
    /// Three independent partitions over `transitive`.
    pub partitions: ImpactPartitions,
    /// Aggregate counts.
    pub summary: ImpactSummary,
}

/// Echoed view of the query target. Kept distinct from [`ImpactTarget`]
/// so the on-disk schema can grow without touching the CLI's input
/// type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactTargetView {
    /// `"contract"` or `"component"`.
    pub kind: ImpactNodeKind,
    /// The id the user passed (verbatim).
    pub id: String,
}

/// Node in the consumer set: a component plus the metadata needed to
/// render it inside `human` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactNode {
    /// Stable id (component id, since impact walks downstream to
    /// consumers).
    pub id: String,
    /// `"contract"` or `"component"`.
    pub kind: ImpactNodeKind,
    /// Filesystem path of the consumer (engine-relative).
    pub path: String,
}

/// Tag for whether a node is a contract or a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactNodeKind {
    /// A contract (a versioned data-format / API surface).
    Contract,
    /// A component (a unit of code that consumes/provides contracts).
    Component,
}

/// Three independent partitions over `transitive`. Each partition maps
/// every transitive consumer to its value on that axis.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactSummary {
    /// `direct.len()`.
    pub direct_count: u32,
    /// `transitive.len()`.
    pub transitive_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> ImpactReport {
        let mut by_language = BTreeMap::new();
        by_language.insert(
            "typescript".to_string(),
            vec![
                "ravel-lite/api".to_string(),
                "ravel-lite/dashboard".to_string(),
            ],
        );
        by_language.insert("rust".to_string(), vec!["ravel-lite/worker".to_string()]);

        let mut by_deploy_graph = BTreeMap::new();
        by_deploy_graph.insert(
            "compose:dev".to_string(),
            vec![
                "ravel-lite/api".to_string(),
                "ravel-lite/worker".to_string(),
                "ravel-lite/dashboard".to_string(),
            ],
        );

        let mut by_lifecycle = BTreeMap::new();
        by_lifecycle.insert(
            "runtime".to_string(),
            vec![
                "ravel-lite/api".to_string(),
                "ravel-lite/worker".to_string(),
                "ravel-lite/dashboard".to_string(),
            ],
        );

        ImpactReport {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 30, 11).unwrap(),
            target: ImpactTargetView {
                kind: ImpactNodeKind::Contract,
                id: "atlas-contracts/index-schema/v1".to_string(),
            },
            direct: vec![ImpactNode {
                id: "ravel-lite/api".to_string(),
                kind: ImpactNodeKind::Component,
                path: "ravel-lite/api".to_string(),
            }],
            transitive: vec![
                ImpactNode {
                    id: "ravel-lite/api".to_string(),
                    kind: ImpactNodeKind::Component,
                    path: "ravel-lite/api".to_string(),
                },
                ImpactNode {
                    id: "ravel-lite/worker".to_string(),
                    kind: ImpactNodeKind::Component,
                    path: "ravel-lite/worker".to_string(),
                },
            ],
            partitions: ImpactPartitions {
                by_language,
                by_deploy_graph,
                by_lifecycle,
            },
            summary: ImpactSummary {
                direct_count: 1,
                transitive_count: 2,
            },
        }
    }

    #[test]
    fn impact_report_round_trips_yaml() {
        let original = fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ImpactReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn impact_report_round_trips_json() {
        let original = fixture();
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: ImpactReport = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn impact_node_kind_serialises_snake_case() {
        let yaml = serde_yaml::to_string(&ImpactNodeKind::Contract).unwrap();
        assert!(yaml.contains("contract"));
        let yaml = serde_yaml::to_string(&ImpactNodeKind::Component).unwrap();
        assert!(yaml.contains("component"));
    }
}
