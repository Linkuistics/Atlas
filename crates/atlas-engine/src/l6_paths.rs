//! Shared path-resolution utilities used by both `l6_composition` and
//! `l6_compose_edges`.
//!
//! All helpers are `pub(crate)` — they are an internal implementation
//! detail of the L6 layer and carry no public API commitment.

use std::path::{Component as PathComponent, Path, PathBuf};

use atlas_index::ComponentEntry;

use crate::roots::best_root_for;

// ─── component-id helpers ─────────────────────────────────────────────────────

/// Extract the leaf segment of a slash-delimited component id (the
/// segment after the final `/`, or the whole id if it has no `/`).
pub(crate) fn component_id_leaf(id: &str) -> &str {
    match id.rfind('/') {
        Some(idx) => &id[idx + 1..],
        None => id,
    }
}

// ─── segment-dir table ───────────────────────────────────────────────────────

/// Build a `(component_id, absolute_segment_dir)` table for every live
/// component. Each path segment contributes one row; components with
/// empty `path_segments` contribute none.
pub(crate) fn build_component_segment_dirs(
    components: &[&ComponentEntry],
    roots: &[PathBuf],
) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for c in components {
        for seg in &c.path_segments {
            let abs = absolute_under_any_root(&seg.path, roots);
            out.push((c.id.as_str().to_string(), abs));
        }
    }
    out
}

// ─── path resolution helpers ─────────────────────────────────────────────────

/// Resolve a relative path-segment to an absolute path by joining with
/// the longest-matching root.  If the segment is already absolute,
/// return it unchanged.  If no root contains it, fall back to the first
/// root; if there are no roots at all, return the path as-is.
pub(crate) fn absolute_under_any_root(rel: &Path, roots: &[PathBuf]) -> PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }
    if let Some(root) = best_root_for(roots, rel) {
        return root.join(rel);
    }
    if let Some(first) = roots.first() {
        return first.join(rel);
    }
    rel.to_path_buf()
}

/// Pick a representative absolute dir for a component.  Falls back to
/// the first segment when the component has multiple segments.
pub(crate) fn absolute_component_dir(
    component: &ComponentEntry,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    let seg = component.path_segments.first()?;
    Some(absolute_under_any_root(&seg.path, roots))
}

/// Longest-prefix lookup: find the component whose segment dir is the
/// longest prefix of `abs_path`.  Returns `None` when no segment dir
/// is a prefix.
pub(crate) fn path_prefix_lookup(
    abs_path: &Path,
    segment_dirs: &[(String, PathBuf)],
) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for (id, dir) in segment_dirs {
        if abs_path.starts_with(dir) {
            let depth = dir.components().count();
            match best {
                Some((_, best_depth)) if best_depth >= depth => {}
                _ => best = Some((id.as_str(), depth)),
            }
        }
    }
    best.map(|(id, _)| id.to_string())
}

/// Resolve `..` and `.` components in a path without touching the
/// filesystem.  Used to canonicalise relative `COPY` sources (and
/// compose `build:` contexts) before the longest-prefix match.
pub(crate) fn normalise_path(path: &Path) -> PathBuf {
    let mut out: Vec<PathComponent> = Vec::new();
    for c in path.components() {
        match c {
            PathComponent::ParentDir => {
                let popped = out
                    .last()
                    .map(|c| matches!(c, PathComponent::Normal(_)))
                    .unwrap_or(false);
                if popped {
                    out.pop();
                } else {
                    out.push(c);
                }
            }
            PathComponent::CurDir => {}
            other => out.push(other),
        }
    }
    let mut buf = PathBuf::new();
    for c in out {
        buf.push(c.as_os_str());
    }
    buf
}
