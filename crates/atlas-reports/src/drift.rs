//! Drift report — `.atlas/cache/reports/drift.yaml`.
//!
//! Schema is fixed by Phase 3 design spec §4.1. PR-7 ships only the
//! report types and a stubbed [`drift`] entry-point that returns
//! [`ReportError::NotImplemented`]; PR-8 lands the actual diff logic.

use atlas_engine::Sha256Hex;
use chrono::{DateTime, Utc};
use component_ontology::ComponentId;
use serde::{Deserialize, Serialize};

use crate::snapshot::ContractShaSnapshot;
use crate::types::{ContractId, ReportError, ReportInputs};

/// Compute drift between the engine's current state and the prior
/// snapshot, returning the report alongside the new snapshot to
/// persist. PR-7 stub: always returns
/// [`ReportError::NotImplemented`].
pub fn drift(
    _inputs: ReportInputs,
    _prev_snapshot: Option<ContractShaSnapshot>,
) -> Result<(DriftReport, ContractShaSnapshot), ReportError> {
    Err(ReportError::NotImplemented)
}

/// Top-level drift report (Phase 3 design spec §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftReport {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time the report was generated.
    pub generated_at: DateTime<Utc>,
    /// `captured_at` of the prior snapshot, or `None` on first run.
    pub baseline_captured_at: Option<DateTime<Utc>>,
    /// Contracts whose `content_sha` differs from the baseline.
    pub contracts_changed: Vec<ContractChange>,
    /// Contracts present today that were absent in the baseline.
    pub contracts_added: Vec<ContractAdded>,
    /// Contracts present in the baseline that are absent today.
    /// (Phase 3 reports renames as removed; rename-match is Phase 4.)
    pub contracts_removed: Vec<ContractRemoved>,
    /// Aggregate counts.
    pub summary: DriftSummary,
}

/// One row of `contracts_changed`: a contract whose sha shifted, plus
/// any bindings still pinned to the prior sha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractChange {
    /// Stable contract id.
    pub id: ContractId,
    /// `content_sha` from the baseline snapshot.
    pub prior_content_sha: Sha256Hex,
    /// `content_sha` in the engine's current state.
    pub current_content_sha: Sha256Hex,
    /// Bindings whose `derived_from_contract_sha` still equals
    /// `prior_content_sha`. Sorted by component id.
    pub pinned_bindings: Vec<PinnedBinding>,
}

/// One binding still pinned to the prior contract sha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedBinding {
    /// Component that owns the binding.
    pub component: ComponentId,
    /// `binding_content_sha` of the binding artefact.
    pub binding_content_sha: Sha256Hex,
    /// The prior contract sha this binding was last computed against.
    pub pinned_to: Sha256Hex,
    /// Implementation language of the binding (free-form string —
    /// engine surface schema records this verbatim).
    pub language: String,
}

/// One row of `contracts_added`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractAdded {
    /// Stable contract id.
    pub id: ContractId,
    /// `content_sha` in the engine's current state.
    pub current_content_sha: Sha256Hex,
}

/// One row of `contracts_removed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRemoved {
    /// Stable contract id (as recorded in the baseline).
    pub id: ContractId,
    /// `content_sha` from the baseline snapshot.
    pub prior_content_sha: Sha256Hex,
}

/// Aggregate counts at the bottom of the drift report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftSummary {
    /// Contracts in the engine's current state.
    pub total_contracts: u32,
    /// Length of `contracts_changed`.
    pub changed: u32,
    /// Length of `contracts_added`.
    pub added: u32,
    /// Length of `contracts_removed`.
    pub removed: u32,
    /// Sum of `pinned_bindings.len()` across `contracts_changed`.
    pub pinned_bindings_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use component_ontology::ComponentId;

    fn fixture() -> DriftReport {
        DriftReport {
            schema_version: 1,
            generated_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 25, 1).unwrap(),
            baseline_captured_at: Some(Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap()),
            contracts_changed: vec![ContractChange {
                id: "atlas-contracts/index-schema/v1".to_string(),
                prior_content_sha: "sha256:abc123".to_string(),
                current_content_sha: "sha256:abc999".to_string(),
                pinned_bindings: vec![PinnedBinding {
                    component: ComponentId::parse("ravel-lite/api").unwrap(),
                    binding_content_sha: "sha256:bind7".to_string(),
                    pinned_to: "sha256:abc123".to_string(),
                    language: "typescript".to_string(),
                }],
            }],
            contracts_added: vec![ContractAdded {
                id: "atlas-contracts/new-schema/v1".to_string(),
                current_content_sha: "sha256:new111".to_string(),
            }],
            contracts_removed: vec![ContractRemoved {
                id: "atlas-contracts/dead-schema/v1".to_string(),
                prior_content_sha: "sha256:dead222".to_string(),
            }],
            summary: DriftSummary {
                total_contracts: 47,
                changed: 1,
                added: 1,
                removed: 1,
                pinned_bindings_count: 1,
            },
        }
    }

    #[test]
    fn drift_report_round_trips_yaml() {
        let original = fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: DriftReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn drift_report_round_trips_json() {
        let original = fixture();
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: DriftReport = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn drift_stub_returns_not_implemented() {
        // Constructing real ReportInputs requires an AtlasDatabase + Workspace,
        // which is non-trivial in a unit test. Instead, we assert the error
        // shape via the public ReportError type so the discriminant is locked.
        let err = ReportError::NotImplemented;
        match err {
            ReportError::NotImplemented => {}
            _ => panic!("expected NotImplemented"),
        }
    }
}
