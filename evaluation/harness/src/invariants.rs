//! Structural invariants that hold for any well-formed Atlas output.
//!
//! Each invariant is a pure function; the `runner` module collects the
//! results into an `InvariantReport`. Failures carry both a message and
//! a machine-readable `InvariantFailure` so downstream code (HTML
//! report, diagnostics) can group or filter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use atlas_index::{
    rename_match, ComponentEntry, ComponentsFile, ExternalsFile, RelatedComponentsFile,
    RenameMatchInput,
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hand-pickable manifest file basenames. Kept deliberately small — the
/// harness uses this list for the `manifest_coverage` invariant only, and
/// we accept that the real pipeline (atlas-engine L1) has a richer
/// ontology-driven definition. Drift is acceptable: the invariant is a
/// conservative floor, not a ceiling.
pub const CONSERVATIVE_MANIFESTS: &[&str] =
    &["Cargo.toml", "package.json", "pyproject.toml", "go.mod"];

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("invariant `{invariant}` failed: {message}")]
pub struct InvariantFailure {
    pub invariant: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InvariantOutcome {
    Pass,
    Fail { message: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvariantReport {
    pub outcomes: BTreeMap<String, InvariantOutcome>,
}

impl InvariantReport {
    pub fn record(&mut self, name: impl Into<String>, outcome: InvariantOutcome) {
        self.outcomes.insert(name.into(), outcome);
    }

    pub fn all_passed(&self) -> bool {
        self.outcomes
            .values()
            .all(|o| matches!(o, InvariantOutcome::Pass | InvariantOutcome::Skipped { .. }))
    }

    pub fn failures(&self) -> impl Iterator<Item = (&String, &String)> {
        self.outcomes.iter().filter_map(|(k, v)| match v {
            InvariantOutcome::Fail { message } => Some((k, message)),
            _ => None,
        })
    }
}

fn fail(invariant: &str, message: impl Into<String>) -> InvariantFailure {
    InvariantFailure {
        invariant: invariant.into(),
        message: message.into(),
    }
}

/// Every internal component must have at least one non-empty
/// `path_segment`. Components without path_segments cannot be located
/// on disk and are meaningless to downstream consumers.
pub fn path_coverage(file: &ComponentsFile) -> Result<(), InvariantFailure> {
    for component in &file.components {
        if component.deleted {
            continue;
        }
        if component.path_segments.is_empty() {
            return Err(fail(
                "path_coverage",
                format!("component `{}` has no path_segments", component.id),
            ));
        }
        if component
            .path_segments
            .iter()
            .any(|s| s.path.as_os_str().is_empty())
        {
            return Err(fail(
                "path_coverage",
                format!("component `{}` has an empty path in path_segments", component.id),
            ));
        }
    }
    Ok(())
}

/// No two non-ancestor/non-descendant components may own paths that
/// overlap (one a path-component prefix of the other). Parent/child
/// nesting is allowed — that's the whole point of a component tree.
pub fn no_path_overlap(file: &ComponentsFile) -> Result<(), InvariantFailure> {
    let parents = parent_index(file);
    let components: Vec<&ComponentEntry> =
        file.components.iter().filter(|c| !c.deleted).collect();

    for (i, a) in components.iter().enumerate() {
        for b in &components[i + 1..] {
            if is_ancestor(&parents, &a.id, &b.id) || is_ancestor(&parents, &b.id, &a.id) {
                continue;
            }
            for seg_a in &a.path_segments {
                for seg_b in &b.path_segments {
                    if paths_overlap(&seg_a.path, &seg_b.path) {
                        return Err(fail(
                            "no_path_overlap",
                            format!(
                                "components `{}` and `{}` have overlapping paths `{}` and `{}` \
                                 but neither is an ancestor of the other",
                                a.id,
                                b.id,
                                seg_a.path.display(),
                                seg_b.path.display()
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn parent_index(file: &ComponentsFile) -> BTreeMap<&str, &str> {
    file.components
        .iter()
        .filter_map(|c| c.parent.as_deref().map(|p| (c.id.as_str(), p)))
        .collect()
}

fn is_ancestor(parents: &BTreeMap<&str, &str>, ancestor: &str, descendant: &str) -> bool {
    let mut cursor = parents.get(descendant).copied();
    while let Some(p) = cursor {
        if p == ancestor {
            return true;
        }
        cursor = parents.get(p).copied();
    }
    false
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    a.starts_with(b) || b.starts_with(a)
}

/// Every manifest found by the conservative walker must be covered by
/// exactly one component (via `component.manifests`). "Conservative"
/// means only the small, well-known set in `CONSERVATIVE_MANIFESTS` —
/// Atlas's L1 may detect more, in which case this invariant is a floor
/// rather than an exact reconciliation.
pub fn manifest_coverage(
    file: &ComponentsFile,
    target_root: &Path,
) -> Result<(), InvariantFailure> {
    let found = collect_manifests_conservative(target_root);
    let covered: BTreeSet<PathBuf> = file
        .components
        .iter()
        .filter(|c| !c.deleted)
        .flat_map(|c| c.manifests.iter().cloned())
        .collect();

    for manifest in &found {
        let rel = manifest
            .strip_prefix(target_root)
            .unwrap_or(manifest)
            .to_path_buf();
        if !covered.contains(&rel) && !covered.contains(manifest) {
            return Err(fail(
                "manifest_coverage",
                format!(
                    "manifest `{}` is not covered by any component",
                    rel.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Walks `root` looking for the small conservative manifest set. Skips
/// gitignored and hidden directories, matching what Atlas's L1 would do.
pub fn collect_manifests_conservative(root: &Path) -> Vec<PathBuf> {
    let mut manifests = Vec::new();
    for entry in WalkBuilder::new(root).hidden(true).build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(s) => s,
            None => continue,
        };
        if CONSERVATIVE_MANIFESTS.contains(&name) {
            manifests.push(entry.into_path());
        }
    }
    manifests.sort();
    manifests
}

/// Every edge participant must name a component that exists in the
/// union of internal components and externals. Unknown names usually
/// indicate stale edges or LLM hallucinations leaking through.
pub fn edge_participant_existence(
    components: &ComponentsFile,
    externals: &ExternalsFile,
    related: &RelatedComponentsFile,
) -> Result<(), InvariantFailure> {
    let mut known: BTreeSet<&str> = BTreeSet::new();
    for c in &components.components {
        if !c.deleted {
            known.insert(&c.id);
        }
    }
    for e in &externals.externals {
        known.insert(&e.id);
    }

    for edge in &related.edges {
        for participant in &edge.participants {
            if !known.contains(participant.as_str()) {
                return Err(fail(
                    "edge_participant_existence",
                    format!(
                        "edge participant `{}` is not present in components.yaml or \
                         external-components.yaml (kind={}, participants={:?})",
                        participant,
                        edge.kind.as_str(),
                        edge.participants
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Components whose path_segments cross a git boundary (i.e. contain a
/// nested `.git` entry not at the segment root) must mention that fact
/// in their rationale. The rationale check is loose — any of "git
/// boundary", "repo boundary", "subtree", or "submodule" counts — so
/// hand-authored goldens have some linguistic latitude.
pub fn git_boundary_rationale(
    file: &ComponentsFile,
    target_root: &Path,
) -> Result<(), InvariantFailure> {
    const MARKERS: &[&str] = &["git boundary", "repo boundary", "subtree", "submodule"];
    for component in &file.components {
        if component.deleted {
            continue;
        }
        for seg in &component.path_segments {
            let full = target_root.join(&seg.path);
            if contains_nested_git(&full, &seg.path) {
                let rationale_lc = component.rationale.to_lowercase();
                if !MARKERS.iter().any(|m| rationale_lc.contains(m)) {
                    return Err(fail(
                        "git_boundary_rationale",
                        format!(
                            "component `{}` owns path `{}` that contains a nested `.git` \
                             but its rationale does not mention a boundary or subtree",
                            component.id,
                            seg.path.display()
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn contains_nested_git(full_segment_root: &Path, segment_rel: &Path) -> bool {
    // Only treat `.git` entries strictly below segment root as nested.
    // A `.git` at the segment root itself is the component's own repo.
    if !full_segment_root.exists() {
        return false;
    }
    for entry in WalkBuilder::new(full_segment_root)
        .hidden(false)
        .git_ignore(false)
        .build()
        .flatten()
    {
        let p = entry.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            let rel_to_segment = p.strip_prefix(full_segment_root).unwrap_or(p);
            // strictly nested: `.git` is not the segment root's own
            // immediate `.git` — it appears at least one directory
            // deeper than the segment itself.
            if rel_to_segment.components().count() > 1 {
                let _ = segment_rel; // retained for future diagnostics
                return true;
            }
        }
    }
    false
}

/// Fixedpoint convergence check. Atlas's §8.2 caps the L4 fixedpoint at
/// 8 iterations. If the tool exposes an iteration count, assert
/// `<= limit` (default 8); otherwise the caller passes `None` and the
/// invariant is skipped.
pub fn fixedpoint_termination(
    iterations: Option<u32>,
    limit: u32,
) -> Result<(), InvariantFailure> {
    match iterations {
        None => Ok(()),
        Some(n) if n <= limit => Ok(()),
        Some(n) => Err(fail(
            "fixedpoint_termination",
            format!("fixedpoint ran for {n} iterations, exceeding the {limit}-iteration limit"),
        )),
    }
}

/// Round-trip prior `ComponentsFile` through `atlas_index::rename_match`
/// against a later one and assert the prior identifiers are preserved for
/// matched entries. Used as an invariant on a pair of consecutive runs.
pub fn rename_round_trip_holds(
    prior: &ComponentsFile,
    new: &ComponentsFile,
) -> Result<(), InvariantFailure> {
    let prior_entries: Vec<ComponentEntry> = prior
        .components
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();
    let new_entries: Vec<ComponentEntry> = new
        .components
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();

    let result = rename_match(RenameMatchInput::new(&prior_entries, &new_entries));

    for (prior_idx, new_idx) in &result.matches {
        let prior_id = &prior_entries[*prior_idx].id;
        let new_id = &new_entries[*new_idx].id;
        if prior_id != new_id {
            return Err(fail(
                "rename_round_trip",
                format!(
                    "rename_match paired prior `{prior_id}` with new `{new_id}`; \
                     identifier was not preserved across the run pair"
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use atlas_index::{
        CacheFingerprints, ComponentEntry, ComponentsFile, ExternalEntry, ExternalsFile,
        PathSegment, COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION,
    };
    use component_ontology::{
        Edge, EdgeKind, EvidenceGrade, LifecycleScope, RelatedComponentsFile,
    };
    use tempfile::TempDir;

    use super::*;

    fn minimal_component(id: &str, path: &str) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![LifecycleScope::Build],
            language: Some("rust".into()),
            build_system: Some("cargo".into()),
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from(path),
                content_sha: "dead".into(),
            }],
            manifests: vec![PathBuf::from(format!("{path}/Cargo.toml"))],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Cargo.toml:[package]".into()],
            rationale: "self-contained crate".into(),
            deleted: false,
        }
    }

    fn file_of(components: Vec<ComponentEntry>) -> ComponentsFile {
        ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            root: PathBuf::from("."),
            generated_at: "2026-04-24T00:00:00Z".into(),
            cache_fingerprints: CacheFingerprints::default(),
            components,
        }
    }

    #[test]
    fn path_coverage_accepts_valid_components() {
        let f = file_of(vec![minimal_component("a", "a"), minimal_component("b", "b")]);
        assert!(path_coverage(&f).is_ok());
    }

    #[test]
    fn path_coverage_rejects_component_with_empty_segments() {
        let mut c = minimal_component("a", "a");
        c.path_segments.clear();
        let f = file_of(vec![c]);
        let err = path_coverage(&f).unwrap_err();
        assert!(err.message.contains("`a`"), "{}", err.message);
    }

    #[test]
    fn path_coverage_skips_deleted_components() {
        let mut c = minimal_component("a", "a");
        c.path_segments.clear();
        c.deleted = true;
        let f = file_of(vec![c]);
        assert!(path_coverage(&f).is_ok());
    }

    #[test]
    fn no_path_overlap_accepts_parent_child_nesting() {
        let parent = minimal_component("root", "pkg");
        let mut child = minimal_component("root/inner", "pkg/inner");
        child.parent = Some("root".into());
        let f = file_of(vec![parent, child]);
        assert!(no_path_overlap(&f).is_ok());
    }

    #[test]
    fn no_path_overlap_rejects_sibling_overlap() {
        // Two components with no parent link but one path is a prefix
        // of the other — that is the violation no_path_overlap is for.
        let a = minimal_component("a", "pkg");
        let b = minimal_component("b", "pkg/sub");
        let f = file_of(vec![a, b]);
        let err = no_path_overlap(&f).unwrap_err();
        assert!(err.message.contains("overlapping"), "{}", err.message);
    }

    #[test]
    fn no_path_overlap_allows_disjoint_paths() {
        let a = minimal_component("a", "pkg-a");
        let b = minimal_component("b", "pkg-b");
        let f = file_of(vec![a, b]);
        assert!(no_path_overlap(&f).is_ok());
    }

    #[test]
    fn manifest_coverage_accepts_covered_manifests() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("a")).unwrap();
        std::fs::write(tmp.path().join("a/Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();

        let mut c = minimal_component("a", "a");
        c.manifests = vec![PathBuf::from("a/Cargo.toml")];
        let f = file_of(vec![c]);
        assert!(manifest_coverage(&f, tmp.path()).is_ok());
    }

    #[test]
    fn manifest_coverage_rejects_uncovered_manifest() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("orphan")).unwrap();
        std::fs::write(tmp.path().join("orphan/Cargo.toml"), "[package]\n").unwrap();

        let mut c = minimal_component("a", "a");
        c.manifests = vec![PathBuf::from("a/Cargo.toml")];
        let f = file_of(vec![c]);
        let err = manifest_coverage(&f, tmp.path()).unwrap_err();
        assert!(err.message.contains("orphan"), "{}", err.message);
    }

    fn edge(kind: EdgeKind, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle: LifecycleScope::Build,
            participants: vec![a.into(), b.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["manifest".into()],
            rationale: "example".into(),
        }
    }

    #[test]
    fn edge_participant_existence_accepts_known_participants() {
        let components = file_of(vec![minimal_component("a", "a")]);
        let externals = ExternalsFile {
            schema_version: EXTERNALS_SCHEMA_VERSION,
            externals: vec![ExternalEntry {
                id: "crate:serde".into(),
                kind: "external".into(),
                language: Some("rust".into()),
                purl: None,
                homepage: None,
                url: None,
                discovered_from: vec!["Cargo.toml".into()],
                evidence_grade: EvidenceGrade::Strong,
            }],
        };
        let mut related = RelatedComponentsFile::default();
        related
            .add_edge(edge(EdgeKind::DependsOn, "a", "crate:serde"))
            .unwrap();
        assert!(edge_participant_existence(&components, &externals, &related).is_ok());
    }

    #[test]
    fn edge_participant_existence_rejects_unknown_participant() {
        let components = file_of(vec![minimal_component("a", "a")]);
        let externals = ExternalsFile {
            schema_version: EXTERNALS_SCHEMA_VERSION,
            externals: vec![],
        };
        let mut related = RelatedComponentsFile::default();
        related
            .add_edge(edge(EdgeKind::DependsOn, "a", "ghost"))
            .unwrap();
        let err = edge_participant_existence(&components, &externals, &related).unwrap_err();
        assert!(err.message.contains("ghost"), "{}", err.message);
    }

    #[test]
    fn fixedpoint_termination_skips_when_iterations_unknown() {
        assert!(fixedpoint_termination(None, 8).is_ok());
    }

    #[test]
    fn fixedpoint_termination_accepts_at_limit() {
        assert!(fixedpoint_termination(Some(8), 8).is_ok());
    }

    #[test]
    fn fixedpoint_termination_rejects_over_limit() {
        let err = fixedpoint_termination(Some(9), 8).unwrap_err();
        assert!(err.message.contains("exceeding"), "{}", err.message);
    }

    #[test]
    fn rename_round_trip_holds_when_ids_preserved() {
        // prior and new have the same path + content_sha → rename_match
        // pairs them and their ids must match.
        let prior = file_of(vec![minimal_component("stable-id", "pkg")]);
        let new = file_of(vec![minimal_component("stable-id", "pkg")]);
        assert!(rename_round_trip_holds(&prior, &new).is_ok());
    }

    #[test]
    fn git_boundary_rationale_accepts_missing_git() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("pkg")).unwrap();
        let c = minimal_component("pkg", "pkg");
        let f = file_of(vec![c]);
        assert!(git_boundary_rationale(&f, tmp.path()).is_ok());
    }

    #[test]
    fn git_boundary_rationale_accepts_rationale_with_boundary_mention() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("pkg/nested/.git")).unwrap();
        let mut c = minimal_component("pkg", "pkg");
        c.rationale = "intentionally bundles a nested git subtree".into();
        let f = file_of(vec![c]);
        assert!(git_boundary_rationale(&f, tmp.path()).is_ok());
    }

    #[test]
    fn git_boundary_rationale_rejects_missing_mention() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("pkg/nested/.git")).unwrap();
        let mut c = minimal_component("pkg", "pkg");
        c.rationale = "a crate".into();
        let f = file_of(vec![c]);
        let err = git_boundary_rationale(&f, tmp.path()).unwrap_err();
        assert!(err.message.contains("boundary"), "{}", err.message);
    }
}
