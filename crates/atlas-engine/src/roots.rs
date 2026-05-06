//! Shared multi-root path utilities.
//!
//! Several layers (L3 classification, L8 sub-carve, L9 projections)
//! need to relativise an absolute path against the workspace root that
//! "owns" it — i.e. the longest path-prefix among `workspace.roots()`.
//! The logic was historically duplicated across those modules with
//! byte-identical bodies; this module is the single source of truth so
//! a fix to one site propagates to all callers.

use std::path::{Path, PathBuf};

/// Returns the root in `roots` that is the longest path-prefix of
/// `path`, or `None` if no root is a prefix of `path`.
///
/// Used to relativise candidate dirs and manifest paths against the
/// root that contains them, so multi-root reports key off
/// `crate-name` rather than `../peer/crate-name`. Pre-vNext callers
/// (single-root) get the same behaviour because a one-element `roots`
/// slice trivially returns its sole entry.
pub fn best_root_for<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a Path> {
    roots
        .iter()
        .filter(|r| path.starts_with(r))
        .max_by_key(|r| r.components().count())
        .map(|p| p.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        let roots = vec![
            PathBuf::from("/a"),
            PathBuf::from("/a/b"),
            PathBuf::from("/c"),
        ];
        let got = best_root_for(&roots, Path::new("/a/b/c/x.rs")).unwrap();
        assert_eq!(got, Path::new("/a/b"));
    }

    #[test]
    fn no_match_returns_none() {
        let roots = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        assert!(best_root_for(&roots, Path::new("/c/x.rs")).is_none());
    }

    #[test]
    fn exact_root_match_returns_root() {
        let roots = vec![PathBuf::from("/a")];
        let got = best_root_for(&roots, Path::new("/a")).unwrap();
        assert_eq!(got, Path::new("/a"));
    }

    #[test]
    fn empty_roots_returns_none() {
        let roots: Vec<PathBuf> = Vec::new();
        assert!(best_root_for(&roots, Path::new("/a/b")).is_none());
    }
}
