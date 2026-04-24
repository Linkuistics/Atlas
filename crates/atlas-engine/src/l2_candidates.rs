//! L2 candidate generation. Walks the L1 manifest, git-boundary, and
//! documentation signals under a directory and emits one
//! [`Candidate`] per distinct candidate-dir. Overrides' `additions`
//! list contributes candidates even when no signal would otherwise
//! produce one; pinned dirs are treated the same as any other
//! candidate at this layer (the short-circuit happens in L3).
//!
//! `subcarve_plan` is consulted per candidate for extra sub-dirs to
//! recurse into. The real implementation lands in task 8; today the
//! stub in [`crate::l8_recurse`] returns the empty list.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db::Workspace;
use crate::l1_queries::{doc_headings, git_boundaries, manifests_in, shebangs};
use crate::l8_recurse::subcarve_plan;
use crate::types::{Candidate, RationaleBundle};

/// Enumerate every candidate component directory at or under `dir`.
/// Candidates come from four independent sources, deduped by
/// directory:
///
/// 1. Every directory that directly contains a manifest file.
/// 2. Every directory that carries a `.git` marker.
/// 3. Every `overrides.additions` entry whose `path_segments` fall
///    under `dir`.
/// 4. Every directory surfaced by [`subcarve_plan`] for an existing
///    confirmed component (stub today — back-edge wired in task 8).
///
/// Each emitted [`Candidate`] carries a [`RationaleBundle`] with the
/// scoped signals at its own directory.
#[salsa::tracked]
pub fn candidate_components_at<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<Candidate>> {
    let mut candidate_dirs: BTreeSet<PathBuf> = BTreeSet::new();

    // Source 1: every manifest's parent directory.
    let manifest_paths = manifests_in(db, workspace, dir.clone());
    for manifest_path in manifest_paths.iter() {
        if let Some(parent) = manifest_path.parent() {
            candidate_dirs.insert(parent.to_path_buf());
        }
    }

    // Source 2: every `.git`-carrying directory.
    let git_dirs = git_boundaries(db, workspace, dir.clone());
    for git_dir in git_dirs.iter() {
        candidate_dirs.insert(git_dir.clone());
    }

    // Source 3: overrides.additions. Use the first path_segment's path
    // as the candidate dir.  A path_segment is stored relative to the
    // workspace root, so reconstruct the absolute form.
    let overrides = workspace.components_overrides(db);
    let root = workspace.root(db);
    for addition in &overrides.additions {
        if let Some(first_segment) = addition.path_segments.first() {
            let abs_path = absolute_under_root(root, &first_segment.path);
            if path_is_inside(&abs_path, &dir) {
                candidate_dirs.insert(abs_path);
            }
        }
    }

    // Source 4: subcarve_plan sub-dirs per id listed in prior components.
    // Today the stub returns empty; keeping the back-edge wired here
    // means task 8 only needs to implement the real query.
    let prior = workspace.prior_components(db);
    for component in &prior.components {
        let sub_dirs = subcarve_plan(db, workspace, component.id.clone());
        for sub_dir in sub_dirs.iter() {
            let abs_path = absolute_under_root(root, sub_dir);
            if path_is_inside(&abs_path, &dir) {
                candidate_dirs.insert(abs_path);
            }
        }
    }

    // Build the RationaleBundle for each candidate by scoping the L1
    // queries to the candidate's own directory.
    let mut out: Vec<Candidate> = Vec::with_capacity(candidate_dirs.len());
    for candidate_dir in candidate_dirs {
        let bundle = build_rationale_bundle(db, workspace, &candidate_dir);
        out.push(Candidate {
            dir: candidate_dir,
            rationale_bundle: bundle,
        });
    }

    // BTreeSet already walked in sorted order, so `out` is
    // dir-sorted without an extra pass.
    Arc::new(out)
}

fn build_rationale_bundle(
    db: &dyn salsa::Database,
    workspace: Workspace,
    candidate_dir: &Path,
) -> RationaleBundle {
    let manifests_here: Vec<PathBuf> = manifests_in(db, workspace, candidate_dir.to_path_buf())
        .iter()
        .filter(|m| m.parent() == Some(candidate_dir))
        .cloned()
        .collect();

    let git_dirs_here = git_boundaries(db, workspace, candidate_dir.to_path_buf());
    let is_git_root = git_dirs_here.iter().any(|d| d == candidate_dir);

    let doc_headings_here = doc_headings(db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();
    let shebangs_here = shebangs(db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();

    RationaleBundle {
        manifests: manifests_here,
        is_git_root,
        doc_headings: doc_headings_here,
        shebangs: shebangs_here,
    }
}

fn absolute_under_root(root: &Path, segment_path: &Path) -> PathBuf {
    if segment_path.is_absolute() {
        segment_path.to_path_buf()
    } else {
        root.join(segment_path)
    }
}

fn path_is_inside(candidate: &Path, dir: &Path) -> bool {
    dir.as_os_str().is_empty() || candidate.starts_with(dir)
}
