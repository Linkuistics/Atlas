//! Match freshly-proposed component candidates against a prior
//! `components.yaml` by path-segment content overlap.
//!
//! Identifier stability is the central v1 requirement: a renamed or
//! relocated component should keep the id it had last run. Rename-match
//! treats every prior entry and every new candidate as a *set of
//! content SHAs* (one per `PathSegment`) and pairs them up by set
//! overlap. A threshold of 0.70 (default) is the line between "same
//! component, moved" and "genuinely different component".
//!
//! Algorithm: greedy bipartite matching. Prior entries are processed in
//! their input order; each picks the still-unmatched new candidate with
//! the highest overlap meeting the threshold. Ties break on lower
//! candidate index (the first suitable match wins). The greedy choice
//! is cheap and matches the design-doc §5.5 sketch. A maximum-weight
//! bipartite matching would be more accurate in pathological tie
//! configurations but is overkill for the sizes we handle (hundreds of
//! components per repo).

use std::collections::{BTreeMap, HashSet};

use component_ontology::ComponentId;

use super::schema::ComponentEntry;
use super::surfaces::SurfacesFile;

pub const DEFAULT_RENAME_MATCH_THRESHOLD: f32 = 0.70;

pub struct RenameMatchInput<'a> {
    pub prior: &'a [ComponentEntry],
    pub new_candidates: &'a [ComponentEntry],
    pub threshold: f32,
}

impl<'a> RenameMatchInput<'a> {
    pub fn new(prior: &'a [ComponentEntry], new_candidates: &'a [ComponentEntry]) -> Self {
        RenameMatchInput {
            prior,
            new_candidates,
            threshold: DEFAULT_RENAME_MATCH_THRESHOLD,
        }
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameMatchOutput {
    /// `(prior_idx, new_idx)` pairs; a prior entry appears at most once,
    /// as does a new candidate.
    pub matches: Vec<(usize, usize)>,
    /// Prior indices with no match above threshold — candidates for
    /// emission as `deleted: true` tombstones.
    pub orphans: Vec<usize>,
    /// New-candidate indices with no match above threshold — need fresh
    /// identifier allocation.
    pub fresh: Vec<usize>,
}

pub fn rename_match(input: RenameMatchInput<'_>) -> RenameMatchOutput {
    let prior_sha_sets: Vec<HashSet<&str>> = input
        .prior
        .iter()
        .map(|e| {
            e.path_segments
                .iter()
                .map(|p| p.content_sha.as_str())
                .collect()
        })
        .collect();
    let candidate_sha_sets: Vec<HashSet<&str>> = input
        .new_candidates
        .iter()
        .map(|e| {
            e.path_segments
                .iter()
                .map(|p| p.content_sha.as_str())
                .collect()
        })
        .collect();

    let mut matches: Vec<(usize, usize)> = Vec::new();
    let mut matched_candidates: HashSet<usize> = HashSet::new();
    let mut orphans: Vec<usize> = Vec::new();

    for (prior_idx, prior_set) in prior_sha_sets.iter().enumerate() {
        let mut best: Option<(usize, f32)> = None;
        for (cand_idx, cand_set) in candidate_sha_sets.iter().enumerate() {
            if matched_candidates.contains(&cand_idx) {
                continue;
            }
            let overlap = overlap_fraction(prior_set, cand_set);
            if overlap < input.threshold {
                continue;
            }
            if best.map(|(_, prev)| overlap > prev).unwrap_or(true) {
                best = Some((cand_idx, overlap));
            }
        }
        match best {
            Some((cand_idx, _)) => {
                matches.push((prior_idx, cand_idx));
                matched_candidates.insert(cand_idx);
            }
            None => orphans.push(prior_idx),
        }
    }

    let fresh: Vec<usize> = (0..input.new_candidates.len())
        .filter(|idx| !matched_candidates.contains(idx))
        .collect();

    RenameMatchOutput {
        matches,
        orphans,
        fresh,
    }
}

/// Fraction of `prior` that appears in `candidate` — the asymmetric
/// overlap described in §5.5. Empty `prior` is a degenerate case: a
/// prior component with no path_segments has no evidence by which to
/// match, so we return 0.0 (it will be orphaned).
fn overlap_fraction(prior: &HashSet<&str>, candidate: &HashSet<&str>) -> f32 {
    if prior.is_empty() {
        return 0.0;
    }
    let intersection = prior.iter().filter(|sha| candidate.contains(*sha)).count();
    intersection as f32 / prior.len() as f32
}

/// Phase 6 PR-2: prior-id → new-id map produced when applying rename-match
/// matches to component-id allocation. An entry appears in the map only
/// when `prior_id != new_id` — equal-id matches are filtered out at
/// construction so consumers can iterate without the per-entry guard.
///
/// The map is consumed by [`rewrite_contract_owner_prefix`] (surfaces.yaml
/// contract ids) and by edge-participant rewriting in the L6 stage
/// (related-components.yaml). α (id-embeds-owner) implementation; β
/// (content-sha-stable, owner-invariant contract ids) is deferred to
/// Phase 10 fuzzy contract matching per LLM-spine recast spec §11.4.
pub type RenameMap = BTreeMap<ComponentId, ComponentId>;

/// Phase 6 PR-2: apply the owner-follows rule to every contract whose
/// id begins with `<prior_id>/...`. Rewrites the prefix to
/// `<new_id>/...` in-place on every [`Contract::id`] (and, by
/// transitive consequence, on every `ImplementedContract::contract_id`
/// / `ConsumedContract::contract_id` that embeds the same prefix).
///
/// α implementation: contract ids embed the owner component id. β
/// (content-sha-stable) is deferred to Phase 10. Independent fuzzy
/// contract matching (a contract whose owner did *not* rename but whose
/// content moved or split) is also out of scope.
///
/// No-op when `prior_id == new_id`. The caller can leave the guard out
/// at the loop level — [`RenameMap`] construction filters identity
/// entries out — but the function itself is idempotent under identity
/// so a defensive caller pays only one pointer comparison.
pub fn rewrite_contract_owner_prefix(
    surfaces: &mut SurfacesFile,
    prior_id: &ComponentId,
    new_id: &ComponentId,
) {
    if prior_id == new_id {
        return;
    }
    let old_prefix = format!("{}/", prior_id.as_str());
    let new_prefix = format!("{}/", new_id.as_str());
    for contract in &mut surfaces.contracts_defined {
        if let Some(suffix) = contract.id.strip_prefix(&old_prefix) {
            contract.id = format!("{new_prefix}{suffix}");
        }
    }
    for implemented in &mut surfaces.contracts_implemented {
        if let Some(suffix) = implemented.contract_id.strip_prefix(&old_prefix) {
            implemented.contract_id = format!("{new_prefix}{suffix}");
        }
    }
    for consumed in &mut surfaces.contracts_consumed {
        if let Some(suffix) = consumed.contract_id.strip_prefix(&old_prefix) {
            consumed.contract_id = format!("{new_prefix}{suffix}");
        }
    }
    for api in &mut surfaces.library_apis {
        if let Some(suffix) = api.id.strip_prefix(&old_prefix) {
            api.id = format!("{new_prefix}{suffix}");
        }
    }
}

/// Phase 6 PR-2: rewrite a single edge-participant string in-place if it
/// has any prior owner-prefix recorded in `rename_map`. Returns `true`
/// when a rewrite occurred. The lookup is `O(rename_map.len())` per call;
/// the common case (no entries, identity rename) is one map-empty check.
pub fn rewrite_participant_owner_prefix(participant: &mut String, rename_map: &RenameMap) -> bool {
    if rename_map.is_empty() {
        return false;
    }
    for (prior_id, new_id) in rename_map {
        let old_prefix = format!("{}/", prior_id.as_str());
        if let Some(suffix) = participant.strip_prefix(&old_prefix) {
            let new_prefix = format!("{}/", new_id.as_str());
            *participant = format!("{new_prefix}{suffix}");
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use component_ontology::EvidenceGrade;

    use super::super::schema::{ComponentEntry, PathSegment};
    use super::super::surfaces::{
        Binding, BindingRole, Contract, ContractKind, ImplementedContract, LibraryApi,
        SurfacesFile, Visibility, SURFACES_SCHEMA_VERSION,
    };
    use super::*;

    fn cid(s: &str) -> ComponentId {
        ComponentId::parse(s).unwrap()
    }

    fn dummy_binding() -> Binding {
        Binding {
            language: "rust".into(),
            symbol: "Sym".into(),
            file: PathBuf::from("src/lib.rs"),
            span: (0, 10),
            content_sha: "0".repeat(64),
            visibility: Visibility::pub_keyword(),
            module_path: Vec::new(),
            attributes: std::collections::BTreeMap::new(),
        }
    }

    fn surfaces_with_contracts(component_id: &str, contract_ids: &[&str]) -> SurfacesFile {
        SurfacesFile {
            schema_version: SURFACES_SCHEMA_VERSION,
            component_id: cid(component_id),
            fingerprint: "0".repeat(64),
            contracts_defined: contract_ids
                .iter()
                .map(|id| Contract {
                    id: (*id).into(),
                    kind: ContractKind::DataFormat,
                    fingerprint: "f".repeat(64),
                    definition_binding: dummy_binding(),
                    description: String::new(),
                })
                .collect(),
            contracts_implemented: contract_ids
                .iter()
                .map(|id| ImplementedContract {
                    contract_id: (*id).into(),
                    role: BindingRole::DefiningBinding,
                    binding: dummy_binding(),
                })
                .collect(),
            contracts_consumed: Vec::new(),
            library_apis: Vec::new(),
        }
    }

    #[test]
    fn rewrite_contract_owner_prefix_rewrites_matching_prefix() {
        let mut surfaces =
            surfaces_with_contracts("old-name", &["old-name/c1", "old-name/c2", "unrelated/c3"]);
        rewrite_contract_owner_prefix(&mut surfaces, &cid("old-name"), &cid("new-name"));
        assert_eq!(surfaces.contracts_defined[0].id, "new-name/c1");
        assert_eq!(surfaces.contracts_defined[1].id, "new-name/c2");
        assert_eq!(
            surfaces.contracts_defined[2].id, "unrelated/c3",
            "unrelated prefix must not be touched"
        );
        // contracts_implemented carries the same ids; verify they
        // followed the rewrite.
        assert_eq!(surfaces.contracts_implemented[0].contract_id, "new-name/c1");
        assert_eq!(surfaces.contracts_implemented[1].contract_id, "new-name/c2");
        assert_eq!(
            surfaces.contracts_implemented[2].contract_id,
            "unrelated/c3"
        );
    }

    #[test]
    fn rewrite_contract_owner_prefix_no_op_when_prior_equals_new() {
        let mut surfaces = surfaces_with_contracts("same", &["same/c1"]);
        rewrite_contract_owner_prefix(&mut surfaces, &cid("same"), &cid("same"));
        assert_eq!(surfaces.contracts_defined[0].id, "same/c1");
        assert_eq!(surfaces.contracts_implemented[0].contract_id, "same/c1");
    }

    #[test]
    fn rewrite_contract_owner_prefix_handles_nested_owner_id() {
        // A nested-component owner-id like `parent/child` must rewrite
        // ONLY when the id starts with the full nested prefix --
        // `parent` alone is not a match.
        let mut surfaces =
            surfaces_with_contracts("parent/child", &["parent/child/c1", "parent/c2"]);
        rewrite_contract_owner_prefix(
            &mut surfaces,
            &cid("parent/child"),
            &cid("parent/grandchild"),
        );
        assert_eq!(
            surfaces.contracts_defined[0].id, "parent/grandchild/c1",
            "exact owner prefix must rewrite"
        );
        assert_eq!(
            surfaces.contracts_defined[1].id, "parent/c2",
            "a shorter ancestor prefix must NOT rewrite -- the helper \
             matches the full owner-id followed by `/`"
        );
    }

    #[test]
    fn rewrite_contract_owner_prefix_rewrites_library_api_ids() {
        let mut surfaces = surfaces_with_contracts("a", &[]);
        surfaces.library_apis.push(LibraryApi {
            id: "a/public-api".into(),
            kind: ContractKind::LibraryApi,
            language: "rust".into(),
            fingerprint: "0".repeat(64),
            pub_items: Vec::new(),
        });
        surfaces.library_apis.push(LibraryApi {
            id: "other/public-api".into(),
            kind: ContractKind::LibraryApi,
            language: "rust".into(),
            fingerprint: "0".repeat(64),
            pub_items: Vec::new(),
        });
        rewrite_contract_owner_prefix(&mut surfaces, &cid("a"), &cid("b"));
        assert_eq!(surfaces.library_apis[0].id, "b/public-api");
        assert_eq!(surfaces.library_apis[1].id, "other/public-api");
    }

    #[test]
    fn rewrite_participant_owner_prefix_empty_map_is_noop() {
        let mut p = String::from("a/c1");
        let map: RenameMap = RenameMap::new();
        assert!(!rewrite_participant_owner_prefix(&mut p, &map));
        assert_eq!(p, "a/c1");
    }

    #[test]
    fn rewrite_participant_owner_prefix_rewrites_matching_prefix() {
        let mut p = String::from("a/c1");
        let mut map: RenameMap = RenameMap::new();
        map.insert(cid("a"), cid("b"));
        assert!(rewrite_participant_owner_prefix(&mut p, &map));
        assert_eq!(p, "b/c1");
    }

    #[test]
    fn rewrite_participant_owner_prefix_preserves_non_matching_strings() {
        // A component-id participant (no `/` suffix anywhere in the
        // rename-map prefix space) must be left alone.
        let mut p = String::from("standalone-component");
        let mut map: RenameMap = RenameMap::new();
        map.insert(cid("a"), cid("b"));
        assert!(!rewrite_participant_owner_prefix(&mut p, &map));
        assert_eq!(p, "standalone-component");
    }

    fn entry_with_shas(id: &str, shas: &[&str]) -> ComponentEntry {
        ComponentEntry {
            id: ComponentId::parse(id).unwrap(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            languages: std::collections::BTreeSet::new(),
            build_system: None,
            role: None,
            path_segments: shas
                .iter()
                .enumerate()
                .map(|(i, sha)| PathSegment {
                    path: PathBuf::from(format!("seg-{i}")),
                    content_sha: (*sha).into(),
                })
                .collect(),
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec![],
            rationale: "test".into(),
            deleted: false,
        }
    }

    #[test]
    fn identical_entries_match_at_overlap_one() {
        let prior = vec![entry_with_shas("prior-a", &["sha1", "sha2", "sha3"])];
        let new = vec![entry_with_shas("cand-a", &["sha1", "sha2", "sha3"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert_eq!(out.matches, vec![(0, 0)]);
        assert!(out.orphans.is_empty());
        assert!(out.fresh.is_empty());
    }

    #[test]
    fn threshold_below_fails() {
        // Prior {a,b,c,d} ∩ cand {a,b,x} = {a,b} → 2/4 = 0.5 < 0.69.
        let prior = vec![entry_with_shas("p", &["a", "b", "c", "d"])];
        let new = vec![entry_with_shas("c", &["a", "b", "x"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new).with_threshold(0.69));
        assert!(out.matches.is_empty());
        assert_eq!(out.orphans, vec![0]);
        assert_eq!(out.fresh, vec![0]);
    }

    #[test]
    fn threshold_above_succeeds() {
        // Prior {a,b,c} ∩ cand {a,b,c,x,y,z} = 3/3 = 1.0 ≥ 0.71.
        let prior = vec![entry_with_shas("p", &["a", "b", "c"])];
        let new = vec![entry_with_shas("c", &["a", "b", "c", "x", "y", "z"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new).with_threshold(0.71));
        assert_eq!(out.matches, vec![(0, 0)]);
    }

    #[test]
    fn boundary_exact_threshold_matches() {
        // Overlap must be *at least* the threshold: 0.70 exactly matches.
        let prior = vec![entry_with_shas(
            "p",
            &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
        )];
        // 7/10 = 0.70 exactly.
        let new = vec![entry_with_shas("c", &["a", "b", "c", "d", "e", "f", "g"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new).with_threshold(0.70));
        assert_eq!(
            out.matches,
            vec![(0, 0)],
            "0.70 must match at threshold 0.70"
        );
    }

    #[test]
    fn greedy_picks_highest_overlap_candidate() {
        // Prior {a,b,c,d,e} overlaps:
        //   cand0 {a,b,c,d}     = 4/5 = 0.8
        //   cand1 {a,b,c,d,e}   = 5/5 = 1.0
        // Greedy must pick cand1, not cand0.
        let prior = vec![entry_with_shas("p", &["a", "b", "c", "d", "e"])];
        let new = vec![
            entry_with_shas("c0", &["a", "b", "c", "d"]),
            entry_with_shas("c1", &["a", "b", "c", "d", "e"]),
        ];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert_eq!(out.matches, vec![(0, 1)]);
        assert_eq!(out.fresh, vec![0]);
    }

    #[test]
    fn greedy_matches_two_priors_to_two_best_candidates() {
        // prior0 best matches cand1 (1.0); prior1 best matches cand0 (1.0).
        let prior = vec![
            entry_with_shas("p0", &["a", "b"]),
            entry_with_shas("p1", &["x", "y"]),
        ];
        let new = vec![
            entry_with_shas("c0", &["x", "y"]),
            entry_with_shas("c1", &["a", "b"]),
        ];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        // Greedy processes priors in order. prior0 picks cand1 (overlap 1.0)
        // because it beats cand0 (overlap 0.0). prior1 then picks cand0.
        assert_eq!(out.matches, vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn greedy_does_not_backtrack_when_first_pick_blocks_second_prior() {
        // prior0's only viable candidate is cand0 (1.0). prior1 also
        // overlaps cand0 perfectly but its only alternative is below
        // threshold. Greedy leaves prior1 as an orphan rather than
        // stealing prior0's match — documents the design trade-off.
        let prior = vec![
            entry_with_shas("p0", &["a", "b"]),
            entry_with_shas("p1", &["a", "b"]),
        ];
        let new = vec![entry_with_shas("c0", &["a", "b"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert_eq!(out.matches, vec![(0, 0)]);
        assert_eq!(out.orphans, vec![1]);
        assert!(out.fresh.is_empty());
    }

    #[test]
    fn prior_with_zero_overlap_becomes_orphan() {
        let prior = vec![entry_with_shas("p", &["a", "b", "c"])];
        let new = vec![entry_with_shas("c", &["x", "y", "z"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert!(out.matches.is_empty());
        assert_eq!(out.orphans, vec![0]);
        assert_eq!(out.fresh, vec![0]);
    }

    #[test]
    fn new_candidate_with_zero_overlap_becomes_fresh() {
        let prior = vec![entry_with_shas("p", &["a", "b"])];
        let new = vec![
            entry_with_shas("c0", &["a", "b"]),
            entry_with_shas("c1", &["x", "y"]),
        ];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert_eq!(out.matches, vec![(0, 0)]);
        assert_eq!(out.fresh, vec![1]);
    }

    #[test]
    fn empty_prior_yields_all_fresh() {
        let prior: Vec<ComponentEntry> = vec![];
        let new = vec![entry_with_shas("c0", &["a"]), entry_with_shas("c1", &["b"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert!(out.matches.is_empty());
        assert!(out.orphans.is_empty());
        assert_eq!(out.fresh, vec![0, 1]);
    }

    #[test]
    fn empty_candidates_yields_all_orphans() {
        let prior = vec![entry_with_shas("p0", &["a"]), entry_with_shas("p1", &["b"])];
        let new: Vec<ComponentEntry> = vec![];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert!(out.matches.is_empty());
        assert_eq!(out.orphans, vec![0, 1]);
        assert!(out.fresh.is_empty());
    }

    #[test]
    fn prior_with_no_path_segments_is_orphaned_not_matched() {
        // A prior entry with zero segments has no evidence by which to
        // match; overlap_fraction returns 0 for an empty prior set.
        let prior = vec![entry_with_shas("p", &[])];
        let new = vec![entry_with_shas("c", &["a", "b"])];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert!(out.matches.is_empty());
        assert_eq!(out.orphans, vec![0]);
        assert_eq!(out.fresh, vec![0]);
    }
}
