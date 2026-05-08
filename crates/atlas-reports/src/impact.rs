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
}
