//! Drift baseline snapshot — `.atlas/cache/contract-shas-snapshot.yaml`.
//!
//! Schema is fixed by Phase 3 design spec §4.1. The on-disk form is a
//! sorted-by-id sequence of `{id, content_sha}` entries; this matches
//! the YAML exemplar in the design spec exactly. The in-memory form is
//! a `Vec<ContractShaEntry>` so the on-disk ordering is preserved
//! without an additional conversion step.

use std::collections::BTreeMap;

use atlas_engine::Sha256Hex;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::ContractId;

/// One row of the snapshot: a contract id and the `content_sha` it had
/// when the baseline was captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractShaEntry {
    /// Stable contract id (e.g. `atlas-contracts/index-schema/v1`).
    pub id: ContractId,
    /// `content_sha` at capture time (e.g. `sha256:abc...`).
    pub content_sha: Sha256Hex,
}

/// Captured baseline of every contract's `content_sha`, used by
/// `atlas drift` to detect changes since the last run.
///
/// Sorted by [`ContractShaEntry::id`] for deterministic file content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractShaSnapshot {
    /// Schema version (always `1` for Phase 3).
    pub schema_version: u32,
    /// Wall-clock time when the snapshot was captured.
    pub captured_at: DateTime<Utc>,
    /// Sorted-by-id sequence of contract ids and their captured shas.
    pub contract_shas: Vec<ContractShaEntry>,
}

impl ContractShaSnapshot {
    /// Convenience: convert the entry sequence into a map for
    /// look-up-by-id during drift comparisons.
    pub fn as_map(&self) -> BTreeMap<&ContractId, &Sha256Hex> {
        self.contract_shas
            .iter()
            .map(|e| (&e.id, &e.content_sha))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> ContractShaSnapshot {
        ContractShaSnapshot {
            schema_version: 1,
            captured_at: Utc.with_ymd_and_hms(2026, 5, 8, 14, 23, 1).unwrap(),
            contract_shas: vec![
                ContractShaEntry {
                    id: "atlas-contracts/eval-schema/v1".to_string(),
                    content_sha: "sha256:def456".to_string(),
                },
                ContractShaEntry {
                    id: "atlas-contracts/index-schema/v1".to_string(),
                    content_sha: "sha256:abc123".to_string(),
                },
            ],
        }
    }

    #[test]
    fn snapshot_round_trips_through_yaml() {
        let original = fixture();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let round_tripped: ContractShaSnapshot = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let original = fixture();
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: ContractShaSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped);
    }

    /// Acceptance criterion 6: parse a hand-written exemplar matching
    /// the design §4.1 schema, re-serialise, parse again, and assert
    /// the two parsed instances are deep-equal. We compare via parse
    /// round-trips rather than byte-equal strings because chrono's
    /// default RFC3339 format may add or strip fractional seconds vs
    /// the hand-written input.
    #[test]
    fn snapshot_matches_design_spec_exemplar() {
        let exemplar = r#"schema_version: 1
captured_at: 2026-05-08T14:23:01Z
contract_shas:
  - id: "atlas-contracts/index-schema/v1"
    content_sha: "sha256:abc123"
  - id: "atlas-contracts/eval-schema/v1"
    content_sha: "sha256:def456"
"#;
        let parsed: ContractShaSnapshot = serde_yaml::from_str(exemplar).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.contract_shas.len(), 2);
        assert_eq!(
            parsed.contract_shas[0].id,
            "atlas-contracts/index-schema/v1"
        );
        assert_eq!(parsed.contract_shas[0].content_sha, "sha256:abc123");

        // Re-serialise and re-parse; the second parse must equal the first.
        let re_serialised = serde_yaml::to_string(&parsed).unwrap();
        let re_parsed: ContractShaSnapshot = serde_yaml::from_str(&re_serialised).unwrap();
        assert_eq!(parsed, re_parsed);
    }

    #[test]
    fn as_map_indexes_by_id() {
        let snap = fixture();
        let map = snap.as_map();
        assert_eq!(
            map.get(&"atlas-contracts/index-schema/v1".to_string())
                .map(|s| s.as_str()),
            Some("sha256:abc123"),
        );
    }
}
