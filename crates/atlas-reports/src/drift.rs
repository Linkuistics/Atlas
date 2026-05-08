//! Drift report — `.atlas/cache/reports/drift.yaml`.
//!
//! Schema is fixed by Phase 3 design spec §4.1. PR-8 lands the diff
//! logic — both the public [`drift`] entry point that walks an
//! [`atlas_engine::AtlasDatabase`] and the internal [`drift_pure`]
//! helper that takes flat collections (so unit tests can exercise
//! every diff scenario without building a full engine database).
//!
//! ## Pinned-binding semantic
//!
//! A binding is "pinned to the prior sha" iff
//! `binding.derived_from_contract_sha == prior_content_sha
//!  != current_content_sha` (design §4.1).
//!
//! Phase 1's surface schema (`atlas_index::Binding`) does not yet have
//! a dedicated `derived_from_contract_sha` field; PR-8 reads it from
//! the existing `Binding::attributes` map under the well-known key
//! `"derived_from_contract_sha"`. A binding that omits the attribute
//! is treated as "no recorded derivation point" and never reported as
//! pinned. Future phases (Phase 2/3 analyser updates) can populate
//! the attribute, at which point pinned-binding detection becomes
//! load-bearing without a wire-format break.

use std::collections::{BTreeMap, BTreeSet};

use atlas_engine::Sha256Hex;
use chrono::{DateTime, Utc};
use component_ontology::ComponentId;
use serde::{Deserialize, Serialize};

use crate::snapshot::{ContractShaEntry, ContractShaSnapshot};
use crate::types::{ContractId, ReportError, ReportInputs};

/// Well-known key under [`atlas_index::Binding::attributes`] that
/// records the contract `content_sha` the binding was last computed
/// against (design §4.1 — used by drift's pinned-binding detector).
pub const DERIVED_FROM_CONTRACT_SHA_ATTR: &str = "derived_from_contract_sha";

/// One contract's current state, as observed in the live engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentContract {
    /// Stable contract id.
    pub id: ContractId,
    /// `content_sha` in the engine's current state.
    pub content_sha: Sha256Hex,
}

/// One binding-consumes-contract relationship, as observed in the
/// live engine. A binding may consume multiple contracts; this struct
/// represents one (binding, contract) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentBinding {
    /// Component that owns the binding.
    pub component: ComponentId,
    /// Contract id the binding consumes.
    pub contract_id: ContractId,
    /// `content_sha` of the binding artefact bytes.
    pub binding_content_sha: Sha256Hex,
    /// Contract sha the binding was last computed against. `None`
    /// when the binding's data does not record a derivation point —
    /// the binding is then never reported as pinned.
    pub derived_from_contract_sha: Option<Sha256Hex>,
    /// Implementation language of the binding (verbatim from the
    /// engine surface schema).
    pub language: String,
}

/// Compute drift between the engine's current state and the prior
/// snapshot, returning the report alongside the new snapshot to
/// persist.
///
/// ## Inputs
///
/// `inputs.db` is walked to collect every contract defined in any
/// live component's `surfaces.yaml` (the component is a *defining*
/// component for the contract) and every binding-consumes-contract
/// relationship. The current-state collections are then handed to
/// [`drift_pure`] which performs the pure diff against `prev_snapshot`.
///
/// ## First-run semantic
///
/// When `prev_snapshot` is `None`, the returned report has empty
/// change arrays; only `summary.total_contracts` is populated.
pub fn drift(
    inputs: ReportInputs,
    prev_snapshot: Option<ContractShaSnapshot>,
) -> Result<(DriftReport, ContractShaSnapshot), ReportError> {
    let now = Utc::now();
    let (current_contracts, current_bindings) = collect_current_state(inputs);
    Ok(drift_pure(
        &current_contracts,
        &current_bindings,
        prev_snapshot.as_ref(),
        now,
        now,
    ))
}

/// Pure-function diff over flat current-state collections. Bypasses
/// the engine database so unit tests can exercise every scenario
/// (added / removed / changed / pinned / first-run) by hand-rolling
/// fixture inputs.
///
/// `generated_at` stamps the returned [`DriftReport::generated_at`];
/// `captured_at` stamps the new snapshot's
/// [`ContractShaSnapshot::captured_at`]. Callers normally pass the
/// same `Utc::now()` value for both.
pub fn drift_pure(
    current_contracts: &[CurrentContract],
    current_bindings: &[CurrentBinding],
    prev_snapshot: Option<&ContractShaSnapshot>,
    generated_at: DateTime<Utc>,
    captured_at: DateTime<Utc>,
) -> (DriftReport, ContractShaSnapshot) {
    // Build the new snapshot first — it is captured from the current
    // state irrespective of whether a prior baseline existed.
    let mut current_by_id: BTreeMap<ContractId, Sha256Hex> = BTreeMap::new();
    for c in current_contracts {
        // If the same contract id appears in multiple components
        // (shouldn't happen by construction, but defensively), the
        // last writer wins — sorted iteration guarantees determinism
        // when callers feed sorted input.
        current_by_id.insert(c.id.clone(), c.content_sha.clone());
    }

    let new_snapshot = ContractShaSnapshot {
        schema_version: 1,
        captured_at,
        contract_shas: current_by_id
            .iter()
            .map(|(id, sha)| ContractShaEntry {
                id: id.clone(),
                content_sha: sha.clone(),
            })
            .collect(),
    };

    let total_contracts = current_by_id.len() as u32;

    // First run — no prior baseline. Empty change arrays; only
    // `summary.total_contracts` is non-zero.
    let Some(prev) = prev_snapshot else {
        let report = DriftReport {
            schema_version: 1,
            generated_at,
            baseline_captured_at: None,
            contracts_changed: Vec::new(),
            contracts_added: Vec::new(),
            contracts_removed: Vec::new(),
            summary: DriftSummary {
                total_contracts,
                changed: 0,
                added: 0,
                removed: 0,
                pinned_bindings_count: 0,
            },
        };
        return (report, new_snapshot);
    };

    // Index the prior snapshot for O(log n) lookups.
    let prev_by_id: BTreeMap<ContractId, Sha256Hex> = prev
        .contract_shas
        .iter()
        .map(|e| (e.id.clone(), e.content_sha.clone()))
        .collect();

    // Pre-bucket bindings by contract_id so the pinned-binding walk
    // is O(b) rather than O(b · |contracts_changed|).
    let mut bindings_by_contract: BTreeMap<&ContractId, Vec<&CurrentBinding>> = BTreeMap::new();
    for b in current_bindings {
        bindings_by_contract
            .entry(&b.contract_id)
            .or_default()
            .push(b);
    }

    // Build the union of contract ids so we visit every (added /
    // removed / changed / unchanged) bucket exactly once. BTreeSet
    // gives us deterministic iteration order keyed by id.
    let all_ids: BTreeSet<&ContractId> = prev_by_id.keys().chain(current_by_id.keys()).collect();

    let mut contracts_changed: Vec<ContractChange> = Vec::new();
    let mut contracts_added: Vec<ContractAdded> = Vec::new();
    let mut contracts_removed: Vec<ContractRemoved> = Vec::new();
    let mut pinned_bindings_count: u32 = 0;

    for id in all_ids {
        match (prev_by_id.get(id), current_by_id.get(id)) {
            (Some(prior_sha), Some(current_sha)) if prior_sha == current_sha => {
                // Unchanged — drop from the report.
            }
            (Some(prior_sha), Some(current_sha)) => {
                // Changed: walk every binding that consumes this
                // contract; report any whose recorded
                // derivation-point still equals the prior sha.
                let mut pinned: Vec<PinnedBinding> = Vec::new();
                if let Some(consumers) = bindings_by_contract.get(id) {
                    for b in consumers {
                        let Some(derived_from) = b.derived_from_contract_sha.as_ref() else {
                            continue;
                        };
                        if derived_from == prior_sha {
                            pinned.push(PinnedBinding {
                                component: b.component.clone(),
                                binding_content_sha: b.binding_content_sha.clone(),
                                pinned_to: prior_sha.clone(),
                                language: b.language.clone(),
                            });
                        }
                    }
                }
                pinned.sort_by(|a, b| a.component.as_str().cmp(b.component.as_str()));
                pinned_bindings_count += pinned.len() as u32;
                contracts_changed.push(ContractChange {
                    id: id.clone(),
                    prior_content_sha: prior_sha.clone(),
                    current_content_sha: current_sha.clone(),
                    pinned_bindings: pinned,
                });
            }
            (None, Some(current_sha)) => {
                contracts_added.push(ContractAdded {
                    id: id.clone(),
                    current_content_sha: current_sha.clone(),
                });
            }
            (Some(prior_sha), None) => {
                contracts_removed.push(ContractRemoved {
                    id: id.clone(),
                    prior_content_sha: prior_sha.clone(),
                });
            }
            (None, None) => unreachable!(
                "all_ids is the union of prev_by_id and current_by_id keys; \
                 at least one branch is Some for every id"
            ),
        }
    }

    let report = DriftReport {
        schema_version: 1,
        generated_at,
        baseline_captured_at: Some(prev.captured_at),
        summary: DriftSummary {
            total_contracts,
            changed: contracts_changed.len() as u32,
            added: contracts_added.len() as u32,
            removed: contracts_removed.len() as u32,
            pinned_bindings_count,
        },
        contracts_changed,
        contracts_added,
        contracts_removed,
    };
    (report, new_snapshot)
}

/// Walk every component's `surfaces.yaml` projection and lift out the
/// per-contract / per-binding state needed by [`drift_pure`].
fn collect_current_state(inputs: ReportInputs) -> (Vec<CurrentContract>, Vec<CurrentBinding>) {
    use atlas_engine::{all_components, surfaces_yaml_snapshot};

    let mut contracts: Vec<CurrentContract> = Vec::new();
    let mut bindings: Vec<CurrentBinding> = Vec::new();

    let components = all_components(inputs.db);
    for entry in components.iter() {
        if entry.deleted {
            continue;
        }
        let Ok(surfaces) = surfaces_yaml_snapshot(inputs.db, &entry.id) else {
            continue;
        };

        // Each `Contract` in `contracts_defined` is a contract this
        // component owns; its `fingerprint` is the canonical
        // `content_sha` per spec §2 (canonicalisation).
        for c in &surfaces.contracts_defined {
            contracts.push(CurrentContract {
                id: c.id.clone(),
                content_sha: c.fingerprint.clone(),
            });
        }

        // `contracts_consumed` lists contracts this component
        // consumes; each entry's `binding` is the consuming binding
        // (the artefact in this component that depends on the
        // contract). Phase 1 always emits this list as empty
        // (PR-7 ships the field but no analyser populates it yet);
        // Phase 2/3 analyser updates land the data, at which point
        // pinned-binding detection becomes load-bearing.
        for ic in &surfaces.contracts_consumed {
            let derived_from = ic
                .binding
                .attributes
                .get(DERIVED_FROM_CONTRACT_SHA_ATTR)
                .and_then(|v| v.as_str())
                .map(str::to_string);
            bindings.push(CurrentBinding {
                component: surfaces.component_id.clone(),
                contract_id: ic.contract_id.clone(),
                binding_content_sha: ic.binding.content_sha.clone(),
                derived_from_contract_sha: derived_from,
                language: ic.binding.language.clone(),
            });
        }
    }

    contracts.sort_by(|a, b| a.id.cmp(&b.id));
    bindings.sort_by(|a, b| {
        a.contract_id
            .cmp(&b.contract_id)
            .then_with(|| a.component.as_str().cmp(b.component.as_str()))
    });
    (contracts, bindings)
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

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 14, 25, 1).unwrap()
    }

    fn baseline_ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 7, 9, 11, 42).unwrap()
    }

    fn contract(id: &str, sha: &str) -> CurrentContract {
        CurrentContract {
            id: id.to_string(),
            content_sha: sha.to_string(),
        }
    }

    fn binding(
        component: &str,
        contract_id: &str,
        binding_sha: &str,
        derived_from: Option<&str>,
        language: &str,
    ) -> CurrentBinding {
        CurrentBinding {
            component: ComponentId::parse(component).unwrap(),
            contract_id: contract_id.to_string(),
            binding_content_sha: binding_sha.to_string(),
            derived_from_contract_sha: derived_from.map(str::to_string),
            language: language.to_string(),
        }
    }

    fn snapshot(captured_at: DateTime<Utc>, entries: Vec<(&str, &str)>) -> ContractShaSnapshot {
        ContractShaSnapshot {
            schema_version: 1,
            captured_at,
            contract_shas: entries
                .into_iter()
                .map(|(id, sha)| ContractShaEntry {
                    id: id.to_string(),
                    content_sha: sha.to_string(),
                })
                .collect(),
        }
    }

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

    /// AC: `drift_first_run_no_baseline` — `prev_snapshot = None`,
    /// fixture with 3 contracts → report has empty change arrays,
    /// snapshot captures all 3.
    #[test]
    fn drift_first_run_no_baseline() {
        let contracts = vec![
            contract("a/c1", "sha:1"),
            contract("a/c2", "sha:2"),
            contract("a/c3", "sha:3"),
        ];
        let bindings: Vec<CurrentBinding> = Vec::new();
        let (report, new_snapshot) = drift_pure(&contracts, &bindings, None, ts(), ts());

        assert!(report.contracts_changed.is_empty());
        assert!(report.contracts_added.is_empty());
        assert!(report.contracts_removed.is_empty());
        assert_eq!(report.baseline_captured_at, None);
        assert_eq!(report.summary.total_contracts, 3);
        assert_eq!(report.summary.changed, 0);
        assert_eq!(report.summary.added, 0);
        assert_eq!(report.summary.removed, 0);
        assert_eq!(report.summary.pinned_bindings_count, 0);

        assert_eq!(new_snapshot.contract_shas.len(), 3);
        // Sorted by id (the snapshot's deterministic-content rule).
        let ids: Vec<&str> = new_snapshot
            .contract_shas
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a/c1", "a/c2", "a/c3"]);
    }

    /// AC: `drift_baseline_unchanged` — `prev_snapshot` matches
    /// current → empty change arrays.
    #[test]
    fn drift_baseline_unchanged() {
        let contracts = vec![contract("a/c1", "sha:1"), contract("a/c2", "sha:2")];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:1"), ("a/c2", "sha:2")]);
        let (report, _new) = drift_pure(&contracts, &[], Some(&prev), ts(), ts());

        assert!(report.contracts_changed.is_empty());
        assert!(report.contracts_added.is_empty());
        assert!(report.contracts_removed.is_empty());
        assert_eq!(report.baseline_captured_at, Some(baseline_ts()));
        assert_eq!(report.summary.total_contracts, 2);
        assert_eq!(report.summary.changed, 0);
    }

    /// AC: `drift_baseline_changed` — one contract's `content_sha`
    /// differs → that contract is in `contracts_changed` with the
    /// prior and current shas.
    #[test]
    fn drift_baseline_changed() {
        let contracts = vec![contract("a/c1", "sha:1-NEW"), contract("a/c2", "sha:2")];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:1"), ("a/c2", "sha:2")]);
        let (report, _new) = drift_pure(&contracts, &[], Some(&prev), ts(), ts());

        assert_eq!(report.contracts_changed.len(), 1);
        let change = &report.contracts_changed[0];
        assert_eq!(change.id, "a/c1");
        assert_eq!(change.prior_content_sha, "sha:1");
        assert_eq!(change.current_content_sha, "sha:1-NEW");
        assert!(change.pinned_bindings.is_empty());
        assert!(report.contracts_added.is_empty());
        assert!(report.contracts_removed.is_empty());
        assert_eq!(report.summary.changed, 1);
    }

    /// AC: `drift_contract_added` — fixture adds a contract → it's
    /// in `contracts_added`.
    #[test]
    fn drift_contract_added() {
        let contracts = vec![
            contract("a/c1", "sha:1"),
            contract("a/c2", "sha:2"),
            contract("a/c3-new", "sha:3"),
        ];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:1"), ("a/c2", "sha:2")]);
        let (report, _new) = drift_pure(&contracts, &[], Some(&prev), ts(), ts());

        assert_eq!(report.contracts_added.len(), 1);
        assert_eq!(report.contracts_added[0].id, "a/c3-new");
        assert_eq!(report.contracts_added[0].current_content_sha, "sha:3");
        assert!(report.contracts_changed.is_empty());
        assert!(report.contracts_removed.is_empty());
        assert_eq!(report.summary.added, 1);
    }

    /// AC: `drift_contract_removed` — fixture removes a contract →
    /// it's in `contracts_removed`.
    #[test]
    fn drift_contract_removed() {
        let contracts = vec![contract("a/c1", "sha:1")];
        let prev = snapshot(
            baseline_ts(),
            vec![("a/c1", "sha:1"), ("a/c2-gone", "sha:2")],
        );
        let (report, _new) = drift_pure(&contracts, &[], Some(&prev), ts(), ts());

        assert_eq!(report.contracts_removed.len(), 1);
        assert_eq!(report.contracts_removed[0].id, "a/c2-gone");
        assert_eq!(report.contracts_removed[0].prior_content_sha, "sha:2");
        assert!(report.contracts_changed.is_empty());
        assert!(report.contracts_added.is_empty());
        assert_eq!(report.summary.removed, 1);
    }

    /// AC: `drift_pinned_binding_detected` — a binding whose
    /// `derived_from_contract_sha == prior` and the contract changed
    /// → binding appears under `pinned_bindings` for that contract.
    #[test]
    fn drift_pinned_binding_detected() {
        let contracts = vec![contract("a/c1", "sha:NEW")];
        let bindings = vec![binding(
            "ravel-lite/api",
            "a/c1",
            "binding-sha:7",
            Some("sha:OLD"),
            "typescript",
        )];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:OLD")]);
        let (report, _new) = drift_pure(&contracts, &bindings, Some(&prev), ts(), ts());

        assert_eq!(report.contracts_changed.len(), 1);
        let change = &report.contracts_changed[0];
        assert_eq!(change.id, "a/c1");
        assert_eq!(change.pinned_bindings.len(), 1);
        let pin = &change.pinned_bindings[0];
        assert_eq!(pin.component.as_str(), "ravel-lite/api");
        assert_eq!(pin.binding_content_sha, "binding-sha:7");
        assert_eq!(pin.pinned_to, "sha:OLD");
        assert_eq!(pin.language, "typescript");
        assert_eq!(report.summary.pinned_bindings_count, 1);
    }

    /// AC: `drift_pinned_binding_up_to_date` — a binding whose
    /// `derived_from_contract_sha == current` → NOT in
    /// `pinned_bindings`.
    #[test]
    fn drift_pinned_binding_up_to_date() {
        let contracts = vec![contract("a/c1", "sha:NEW")];
        let bindings = vec![binding(
            "ravel-lite/api",
            "a/c1",
            "binding-sha:7",
            // Up-to-date: derived against the current sha, not the
            // prior. Must not appear in pinned_bindings.
            Some("sha:NEW"),
            "typescript",
        )];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:OLD")]);
        let (report, _new) = drift_pure(&contracts, &bindings, Some(&prev), ts(), ts());

        assert_eq!(report.contracts_changed.len(), 1);
        let change = &report.contracts_changed[0];
        assert!(
            change.pinned_bindings.is_empty(),
            "up-to-date bindings must not be reported as pinned: got {:?}",
            change.pinned_bindings
        );
        assert_eq!(report.summary.pinned_bindings_count, 0);
    }

    /// Defensive: a binding without a recorded
    /// `derived_from_contract_sha` is never reported as pinned.
    /// Phase 1 analyser data does not yet carry the attribute, so
    /// this is the common case until Phase 2/3 analyser updates land.
    #[test]
    fn drift_binding_without_derived_from_is_never_pinned() {
        let contracts = vec![contract("a/c1", "sha:NEW")];
        let bindings = vec![binding(
            "ravel-lite/api",
            "a/c1",
            "binding-sha:7",
            None, // No recorded derivation point.
            "typescript",
        )];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:OLD")]);
        let (report, _new) = drift_pure(&contracts, &bindings, Some(&prev), ts(), ts());

        assert_eq!(report.contracts_changed.len(), 1);
        assert!(report.contracts_changed[0].pinned_bindings.is_empty());
        assert_eq!(report.summary.pinned_bindings_count, 0);
    }

    /// Defensive: pinned-binding entries are sorted by component id
    /// for deterministic on-disk output.
    #[test]
    fn drift_pinned_bindings_are_sorted_by_component() {
        let contracts = vec![contract("a/c1", "sha:NEW")];
        let bindings = vec![
            binding("zeta/api", "a/c1", "b:z", Some("sha:OLD"), "rust"),
            binding("alpha/api", "a/c1", "b:a", Some("sha:OLD"), "typescript"),
            binding("middle/api", "a/c1", "b:m", Some("sha:OLD"), "python"),
        ];
        let prev = snapshot(baseline_ts(), vec![("a/c1", "sha:OLD")]);
        let (report, _new) = drift_pure(&contracts, &bindings, Some(&prev), ts(), ts());

        let comps: Vec<&str> = report.contracts_changed[0]
            .pinned_bindings
            .iter()
            .map(|p| p.component.as_str())
            .collect();
        assert_eq!(comps, vec!["alpha/api", "middle/api", "zeta/api"]);
    }

    /// Defensive: snapshot entries are sorted by id for byte-stable
    /// on-disk content.
    #[test]
    fn drift_snapshot_is_sorted_by_id() {
        let contracts = vec![
            contract("zeta/c", "sha:z"),
            contract("alpha/c", "sha:a"),
            contract("middle/c", "sha:m"),
        ];
        let (_report, new_snapshot) = drift_pure(&contracts, &[], None, ts(), ts());
        let ids: Vec<&str> = new_snapshot
            .contract_shas
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["alpha/c", "middle/c", "zeta/c"]);
    }

    /// Defensive: `pinned_bindings_count` aggregates across multiple
    /// changed contracts, not just one.
    #[test]
    fn drift_pinned_bindings_count_aggregates_across_changed_contracts() {
        let contracts = vec![contract("a/c1", "sha:1-NEW"), contract("a/c2", "sha:2-NEW")];
        let bindings = vec![
            binding("comp/a", "a/c1", "b:1", Some("sha:1-OLD"), "rust"),
            binding("comp/b", "a/c1", "b:2", Some("sha:1-OLD"), "rust"),
            binding("comp/c", "a/c2", "b:3", Some("sha:2-OLD"), "rust"),
        ];
        let prev = snapshot(
            baseline_ts(),
            vec![("a/c1", "sha:1-OLD"), ("a/c2", "sha:2-OLD")],
        );
        let (report, _) = drift_pure(&contracts, &bindings, Some(&prev), ts(), ts());

        assert_eq!(report.summary.changed, 2);
        assert_eq!(report.summary.pinned_bindings_count, 3);
    }
}
