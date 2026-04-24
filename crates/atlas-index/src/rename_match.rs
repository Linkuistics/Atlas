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

use std::collections::HashSet;

use super::schema::ComponentEntry;

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
        .map(|e| e.path_segments.iter().map(|p| p.content_sha.as_str()).collect())
        .collect();
    let candidate_sha_sets: Vec<HashSet<&str>> = input
        .new_candidates
        .iter()
        .map(|e| e.path_segments.iter().map(|p| p.content_sha.as_str()).collect())
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use component_ontology::EvidenceGrade;

    use super::super::schema::{ComponentEntry, PathSegment};
    use super::*;

    fn entry_with_shas(id: &str, shas: &[&str]) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: None,
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
        let out = rename_match(
            RenameMatchInput::new(&prior, &new).with_threshold(0.69),
        );
        assert!(out.matches.is_empty());
        assert_eq!(out.orphans, vec![0]);
        assert_eq!(out.fresh, vec![0]);
    }

    #[test]
    fn threshold_above_succeeds() {
        // Prior {a,b,c} ∩ cand {a,b,c,x,y,z} = 3/3 = 1.0 ≥ 0.71.
        let prior = vec![entry_with_shas("p", &["a", "b", "c"])];
        let new = vec![entry_with_shas("c", &["a", "b", "c", "x", "y", "z"])];
        let out = rename_match(
            RenameMatchInput::new(&prior, &new).with_threshold(0.71),
        );
        assert_eq!(out.matches, vec![(0, 0)]);
    }

    #[test]
    fn boundary_exact_threshold_matches() {
        // Overlap must be *at least* the threshold: 0.70 exactly matches.
        let prior = vec![entry_with_shas("p", &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"])];
        // 7/10 = 0.70 exactly.
        let new = vec![entry_with_shas("c", &["a", "b", "c", "d", "e", "f", "g"])];
        let out = rename_match(
            RenameMatchInput::new(&prior, &new).with_threshold(0.70),
        );
        assert_eq!(out.matches, vec![(0, 0)], "0.70 must match at threshold 0.70");
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
        let new = vec![
            entry_with_shas("c0", &["a"]),
            entry_with_shas("c1", &["b"]),
        ];
        let out = rename_match(RenameMatchInput::new(&prior, &new));
        assert!(out.matches.is_empty());
        assert!(out.orphans.is_empty());
        assert_eq!(out.fresh, vec![0, 1]);
    }

    #[test]
    fn empty_candidates_yields_all_orphans() {
        let prior = vec![
            entry_with_shas("p0", &["a"]),
            entry_with_shas("p1", &["b"]),
        ];
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
