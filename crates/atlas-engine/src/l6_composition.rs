//! L6 composition edges (PR-9) — `bundled-into` and `deployed-with`
//! edges derived deterministically from Dockerfile `COPY` directives.
//!
//! ## Why this lives next to L6
//!
//! Composition edges are an L6-stage projection: they answer the same
//! "how do components relate?" question as the LLM Stage 2 batch in
//! [`crate::l6_edges`], but they are derived purely from
//! deterministic structural data (Dockerfile parses + workspace path
//! prefixes). The two edge sources are merged before
//! [`crate::l6_edges::canonicalise_edges`] dedupes — see
//! [`crate::l6_edges::all_proposed_edges`].
//!
//! ## Algorithm (per design §3.5 + §6.4)
//!
//! For every live component classified as `kind: docker-image`:
//!
//! 1. Find the enclosing-component dir by reading the path segment
//!    relative to the owning workspace root. The Dockerfile lives
//!    directly inside this dir (the L3 driver only marks a dir as
//!    `docker-image` when a Dockerfile exists there).
//! 2. Re-parse the Dockerfile via
//!    [`atlas_analyzers::dockerfile_classifier::parse_dockerfile`]. The
//!    parser is pure and the cost is negligible — re-parsing is
//!    materially simpler than threading the structured shape through
//!    the L3 Salsa output (the brief explicitly endorses option (1)).
//! 3. For each `COPY` directive: skip when `from_stage` is `Some` (an
//!    intra-image stage copy, no host-repo source); otherwise iterate
//!    `sources` and resolve each to a source component using the
//!    two-tier strategy described on
//!    [`resolve_source_to_component`].
//! 4. Emit `bundled-into` (source → image, lifecycle `deploy`) per
//!    successfully-resolved pair.
//! 5. Per docker-image, emit `deployed-with` for every distinct
//!    unordered pair of resolved source components (lifecycle
//!    `runtime`, lex-sorted participants — symmetric per
//!    [`component_ontology::EdgeKind::is_directed`]).
//!
//! Resolution is best-effort: an unresolvable source path (e.g. a base
//! image's filesystem path, an absolute path outside the workspace)
//! contributes no edge, no warning, and no error. The acceptance
//! criterion is that the deterministic edges that *can* be derived
//! always appear; spurious edges are the failure mode this module
//! avoids.
//!
//! ## Determinism
//!
//! `composition_edges_from_dockerfiles` walks components in their
//! [`crate::l4_tree::all_components`] order and emits edges in source
//! order within each docker-image. Final ordering is enforced by
//! [`crate::l6_edges::canonicalise_edges`] (dedup + symmetric-sort) and
//! by [`atlas_index::RelatedComponentsFile::add_edge`]'s canonical-key
//! dedup at L9. Re-runs over identical inputs produce byte-identical
//! YAML.

use std::collections::BTreeSet;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::Arc;

use atlas_analyzers::dockerfile_classifier::parse_dockerfile;
use atlas_index::ComponentEntry;
use component_ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};

use crate::db::AtlasDatabase;
use crate::l1_queries::file_content;
use crate::l4_tree::all_components;
use crate::roots::best_root_for;

/// Build every deterministic composition edge implied by Dockerfiles
/// in the current workspace. Returns an empty `Vec` when there are no
/// `kind: docker-image` components.
///
/// The result is **not** canonicalised — the caller (L6) feeds it
/// through [`crate::l6_edges::canonicalise_edges`] alongside the LLM
/// batch.
pub fn composition_edges_from_dockerfiles(db: &AtlasDatabase) -> Vec<Edge> {
    let components = all_components(db);
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();
    let workspace = db.workspace();
    let roots = workspace.roots(db as &dyn salsa::Database).clone();

    // Pre-compute the absolute path-segment dirs for every live
    // component, paired with the component id. This is the lookup
    // table the path-prefix tier of `resolve_source_to_component`
    // walks; precomputing once amortises the per-COPY cost from
    // O(components × segments) to O(1).
    let component_segment_dirs: Vec<(String, PathBuf)> =
        build_component_segment_dirs(&live, &roots);

    let mut out: Vec<Edge> = Vec::new();

    for image_component in &live {
        if image_component.kind != "docker-image" {
            continue;
        }
        let Some(image_dir) = absolute_component_dir(image_component, &roots) else {
            continue;
        };
        let dockerfile_path = image_dir.join("Dockerfile");
        let Some(bytes) = file_content(db, &dockerfile_path) else {
            // The L3 driver only classifies a dir as `docker-image`
            // when it observed a Dockerfile, so this branch is mostly
            // a defensive fallback for direct overrides that pin a
            // dir as docker-image without a Dockerfile on disk.
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let shape = parse_dockerfile(text);
        let image_id = image_component.id.as_str().to_string();

        // Track the set of source ids bundled into *this* image so we
        // can emit `deployed-with` between every pair afterwards.
        // BTreeSet keeps the ids sorted for stable pair enumeration.
        let mut bundled_source_ids: BTreeSet<String> = BTreeSet::new();

        for directive in &shape.copy_directives {
            // `--from=<stage>` references the filesystem of an
            // upstream image stage, not a host-repo path. PR-9 only
            // emits edges for repo-derived sources — Phase 2 may
            // re-introduce a finer rule that traces a stage alias
            // back to its `FROM` and onwards.
            if directive.from_stage.is_some() {
                continue;
            }
            for source in &directive.sources {
                let Some(source_id) =
                    resolve_source_to_component(source, &image_dir, &component_segment_dirs)
                else {
                    continue;
                };
                if source_id == image_id {
                    // Pathological self-reference — the docker-image's
                    // own dir slugged into a source via a leading
                    // `COPY .` directive. Skip rather than emit a
                    // self-edge which would fail Edge::validate.
                    continue;
                }
                let edge = Edge {
                    kind: EdgeKind::BundledInto,
                    lifecycle: LifecycleScope::Deploy,
                    participants: vec![source_id.clone(), image_id.clone()],
                    evidence_grade: EvidenceGrade::Strong,
                    evidence_fields: vec![format!("Dockerfile:COPY:{source}")],
                    rationale: format!(
                        "Dockerfile COPY directive bundles `{source}` into image \
                         component `{image_id}`"
                    ),
                };
                out.push(edge);
                bundled_source_ids.insert(source_id);
            }
        }

        // `deployed-with`: every unordered pair of distinct sources
        // bundled into the same image. The BTreeSet's natural lex
        // ordering means the inner double-loop yields each pair
        // exactly once with `a < b`, satisfying the symmetric-edge
        // canonicalisation rule (sorted participants) up front.
        let sorted_sources: Vec<String> = bundled_source_ids.into_iter().collect();
        for i in 0..sorted_sources.len() {
            for j in (i + 1)..sorted_sources.len() {
                let a = &sorted_sources[i];
                let b = &sorted_sources[j];
                let edge = Edge {
                    kind: EdgeKind::DeployedWith,
                    lifecycle: LifecycleScope::Runtime,
                    participants: vec![a.clone(), b.clone()],
                    evidence_grade: EvidenceGrade::Strong,
                    evidence_fields: vec!["Dockerfile:bundles-both".to_string()],
                    rationale: format!(
                        "Dockerfile of image `{image_id}` bundles both source components \
                         `{a}` and `{b}`"
                    ),
                };
                out.push(edge);
            }
        }
    }

    out
}

/// Given a `COPY` source path and the docker-image's enclosing dir
/// (the build context), return the id of the source component that
/// "owns" the path, or `None` when no resolution succeeds.
///
/// Two-tier resolution:
///
/// 1. **Build-output basename match** — if `source` looks like
///    `target/release/<name>` or `target/debug/<name>` (or the same
///    forms nested under sub-directories — e.g. `target/aarch64-…
///    /release/<name>`) and there is exactly one live component whose
///    leaf id matches `<name>`, that component is the source. This
///    is the case the acceptance criterion's worked example pins:
///    `COPY target/release/billing-core` → `<root>/billing-core`.
/// 2. **Path-prefix match** — fall back to longest-prefix-match
///    against the pre-computed component segment dirs. The build
///    context is the docker-image's enclosing dir; relative source
///    paths are resolved against it. Absolute source paths outside
///    every component segment yield `None`.
///
/// Tier 1 is keyed by *unique* basename; if two components in the
/// workspace share a basename, tier 1 cannot disambiguate and we
/// fall through to tier 2 (which will only succeed for an
/// in-workspace path, again by-design).
fn resolve_source_to_component(
    source: &str,
    image_dir: &Path,
    component_segment_dirs: &[(String, PathBuf)],
) -> Option<String> {
    // Tier 1: build-output basename match.
    if let Some(binary_name) = strip_build_output_prefix(source) {
        let mut matches = component_segment_dirs
            .iter()
            .filter(|(id, _)| component_id_leaf(id) == binary_name);
        let first = matches.next();
        let second = matches.next();
        if let (Some((id, _)), None) = (first, second) {
            return Some(id.clone());
        }
        // Multiple basename matches: do not disambiguate; fall
        // through to tier 2 (which typically returns `None` for a
        // `target/...` path because no component owns the build
        // dir).
    }

    // Tier 2: longest-prefix match. Resolve the source path to an
    // absolute form first.
    let source_path = PathBuf::from(source);
    let absolute = if source_path.is_absolute() {
        source_path
    } else {
        image_dir.join(source_path)
    };
    let absolute = normalise_path(&absolute);

    let mut best: Option<(&str, usize)> = None;
    for (id, dir) in component_segment_dirs {
        if absolute.starts_with(dir) {
            let depth = dir.components().count();
            match best {
                Some((_, best_depth)) if best_depth >= depth => {}
                _ => best = Some((id.as_str(), depth)),
            }
        }
    }
    best.map(|(id, _)| id.to_string())
}

/// If `source` matches `target/(release|debug)[/...]/<name>` (with
/// optional intermediate components such as a target-triple), return
/// the trailing `<name>` (the binary basename, sans extension). Any
/// extension is stripped — `binary.exe` resolves to `binary`. Returns
/// `None` for paths that don't fit the build-output shape.
fn strip_build_output_prefix(source: &str) -> Option<&str> {
    // Tolerate both `/` and `\` (Windows-authored Dockerfiles); the
    // path crate's iterator handles either after normalising.
    let normalised = source.replace('\\', "/");
    let mut segments = normalised
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .peekable();
    if segments.next()? != "target" {
        return None;
    }
    // Walk forward until we hit `release` or `debug`, allowing any
    // number of preceding segments (a target-triple sub-dir, for
    // example: `target/aarch64-unknown-linux-gnu/release/foo`).
    let mut found = false;
    for seg in segments.by_ref() {
        if seg == "release" || seg == "debug" {
            found = true;
            break;
        }
    }
    if !found {
        return None;
    }
    // Take the *last* remaining segment as the binary basename.
    let last = segments.last()?;
    // Strip a trailing extension if present (e.g. `.exe`).
    let basename = last.split('.').next()?;
    if basename.is_empty() {
        return None;
    }
    // Borrow back into `source` — the post-replace string was a
    // temporary, so we re-find the basename in the original to keep
    // the lifetime tied to the input. Easier: scan from the end.
    let bytes = source.as_bytes();
    let mut end = bytes.len();
    // Trim trailing `/` if the source ends in one.
    while end > 0 && (bytes[end - 1] == b'/' || bytes[end - 1] == b'\\') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1] != b'/' && bytes[start - 1] != b'\\' {
        start -= 1;
    }
    let in_place = &source[start..end];
    // Strip a trailing extension if present.
    match in_place.find('.') {
        Some(dot) => Some(&in_place[..dot]),
        None => Some(in_place),
    }
}

/// Extract the leaf segment of a slash-delimited component id (the
/// segment after the final `/`, or the whole id if it has no `/`).
fn component_id_leaf(id: &str) -> &str {
    match id.rfind('/') {
        Some(idx) => &id[idx + 1..],
        None => id,
    }
}

/// Build a `(component_id, absolute_segment_dir)` table for every
/// live component. Each path segment contributes one row; deliverable
/// components with empty `path_segments` contribute none.
fn build_component_segment_dirs(
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

/// Resolve a relative path-segment to an absolute path by joining
/// with the longest-matching root. If the segment is already
/// absolute, return it unchanged. If no root contains it (rare —
/// L4 always relativises against an owning root), fall back to the
/// first root.
fn absolute_under_any_root(rel: &Path, roots: &[PathBuf]) -> PathBuf {
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

/// Pick a representative absolute dir for a component. Falls back to
/// the first segment when the component has multiple segments — the
/// docker-image case is single-segment because L4 produces one
/// segment per component dir, but the helper tolerates the multi-
/// segment shape future PRs may introduce.
fn absolute_component_dir(component: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let seg = component.path_segments.first()?;
    Some(absolute_under_any_root(&seg.path, roots))
}

/// Resolve `..` and `.` components in a path without touching the
/// filesystem. Used to canonicalise relative `COPY` sources before
/// the longest-prefix match — `image_dir/../foo/bar` should compare
/// equal to `(image_dir/..)/foo/bar`.
fn normalise_path(path: &Path) -> PathBuf {
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

/// Read-only accessor exposed for tests in `l6_edges` and the
/// integration suite. Wraps the function in `Arc` for cheap-clone
/// from inside the LLM batch path.
pub fn composition_edges_arc(db: &AtlasDatabase) -> Arc<Vec<Edge>> {
    Arc::new(composition_edges_from_dockerfiles(db))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_build_output_prefix_recognises_release_and_debug() {
        assert_eq!(
            strip_build_output_prefix("target/release/billing-core"),
            Some("billing-core")
        );
        assert_eq!(strip_build_output_prefix("target/debug/foo"), Some("foo"));
    }

    #[test]
    fn strip_build_output_prefix_strips_target_triple_sub_dir() {
        assert_eq!(
            strip_build_output_prefix("target/aarch64-unknown-linux-gnu/release/x"),
            Some("x")
        );
    }

    #[test]
    fn strip_build_output_prefix_strips_extension() {
        assert_eq!(
            strip_build_output_prefix("target/release/foo.exe"),
            Some("foo")
        );
    }

    #[test]
    fn strip_build_output_prefix_returns_none_for_non_build_paths() {
        assert!(strip_build_output_prefix("src/main.rs").is_none());
        assert!(strip_build_output_prefix("/absolute/foo").is_none());
        assert!(strip_build_output_prefix("not-target/release/foo").is_none());
        // `target` without a `release`/`debug` segment.
        assert!(strip_build_output_prefix("target/foo").is_none());
    }

    #[test]
    fn component_id_leaf_returns_last_path_segment() {
        assert_eq!(component_id_leaf("foo"), "foo");
        assert_eq!(component_id_leaf("foo/bar"), "bar");
        assert_eq!(component_id_leaf("a/b/c"), "c");
    }

    #[test]
    fn normalise_path_resolves_dot_and_double_dot() {
        let got = normalise_path(Path::new("/a/b/../c/./d"));
        assert_eq!(got, PathBuf::from("/a/c/d"));
    }

    #[test]
    fn resolve_source_to_component_via_basename_tier() {
        let segs = vec![
            (
                "ws/billing-core".to_string(),
                PathBuf::from("/ws/billing-core"),
            ),
            (
                "ws/billing-image".to_string(),
                PathBuf::from("/ws/deploy/billing"),
            ),
        ];
        let got = resolve_source_to_component(
            "target/release/billing-core",
            Path::new("/ws/deploy/billing"),
            &segs,
        );
        assert_eq!(got.as_deref(), Some("ws/billing-core"));
    }

    #[test]
    fn resolve_source_to_component_via_path_prefix_tier() {
        let segs = vec![("ws/src".to_string(), PathBuf::from("/ws/src"))];
        // Source `../../src/foo.txt` from /ws/deploy/billing resolves
        // to /ws/src/foo.txt, which is under /ws/src.
        let got = resolve_source_to_component(
            "../../src/foo.txt",
            Path::new("/ws/deploy/billing"),
            &segs,
        );
        assert_eq!(got.as_deref(), Some("ws/src"));
    }

    #[test]
    fn resolve_source_to_component_returns_none_for_unmatched_path() {
        let segs = vec![("ws/src".to_string(), PathBuf::from("/ws/src"))];
        let got =
            resolve_source_to_component("/etc/passwd", Path::new("/ws/deploy/billing"), &segs);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_source_to_component_basename_collision_falls_through() {
        // Two components with the same leaf name → tier 1 declines,
        // tier 2 cannot match a `target/release/...` path against
        // any segment dir, so the result is None.
        let segs = vec![
            ("a/foo".to_string(), PathBuf::from("/ws/a/foo")),
            ("b/foo".to_string(), PathBuf::from("/ws/b/foo")),
        ];
        let got = resolve_source_to_component("target/release/foo", Path::new("/ws/deploy"), &segs);
        assert!(got.is_none());
    }
}
