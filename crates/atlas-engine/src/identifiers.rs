//! Identifier allocation for components. Ids are path-derived and
//! stable across runs when the directory tree is stable; rename-match
//! preserves an id across directory relocations by re-using the prior
//! id even when the path-derived form would differ.
//!
//! Shape: `parent/leaf-slug` for nested components, or `leaf-slug`
//! for the root. Collisions — typically two sibling components with
//! identical leaf names under the same parent — are resolved first by
//! appending `-2`, `-3`, …, and as a last resort by a short
//! content-hash suffix so deterministic output survives pathological
//! collision counts without an unbounded integer scan.

use std::collections::HashSet;
use std::path::Path;

use sha2::{Digest, Sha256};

/// How many numeric suffixes to try before falling back to the
/// content-hash form. A value of 1000 is well beyond realistic
/// collision counts (a single parent with thousands of sibling
/// components is a pathology, not a use case) while keeping the linear
/// scan trivial.
const MAX_NUMERIC_SUFFIX: u32 = 1000;

/// Width of the content-hash fallback suffix in hex characters. 8 hex
/// characters = 32 bits, which makes a second collision after all
/// numeric suffixes are exhausted astronomically unlikely.
const HASH_SUFFIX_WIDTH: usize = 8;

/// Allocate a component id for `candidate_dir` under `parent_id`.
/// The returned id is guaranteed not to appear in `existing_ids`.
///
/// `candidate_dir` should be the absolute or workspace-relative path of
/// the directory; only the final component (the leaf basename) is
/// slugified and consumed. `parent_id` is `None` for the root; any
/// other value is used verbatim as the id prefix, separated from the
/// leaf slug by `/`.
pub fn allocate_id(
    candidate_dir: &Path,
    parent_id: Option<&str>,
    existing_ids: &HashSet<String>,
) -> String {
    let leaf = leaf_slug(candidate_dir);
    let primary = match parent_id {
        Some(parent) if !parent.is_empty() => format!("{parent}/{leaf}"),
        _ => leaf.clone(),
    };
    if !existing_ids.contains(&primary) {
        return primary;
    }

    for suffix in 2..=MAX_NUMERIC_SUFFIX {
        let candidate = format!("{primary}-{suffix}");
        if !existing_ids.contains(&candidate) {
            return candidate;
        }
    }

    // Pathological collision count: derive a short content hash from
    // the path itself as a last-resort disambiguator.
    let hash_suffix = content_hash_suffix(candidate_dir);
    let hashed = format!("{primary}-{hash_suffix}");
    if !existing_ids.contains(&hashed) {
        return hashed;
    }

    // Extraordinarily improbable — content-hash collision plus full
    // numeric-suffix exhaustion. Append a second numeric suffix.
    for suffix in 2..=MAX_NUMERIC_SUFFIX {
        let candidate = format!("{hashed}-{suffix}");
        if !existing_ids.contains(&candidate) {
            return candidate;
        }
    }

    panic!(
        "allocate_id: could not find a free id for {} under {:?} after exhausting numeric \
         and content-hash suffixes; existing_ids size = {}",
        candidate_dir.display(),
        parent_id,
        existing_ids.len()
    );
}

/// Kebab-case slug from a single path-segment string. Non-alphanumeric
/// characters collapse to `-`; runs of `-` are compressed; leading and
/// trailing `-` are stripped. Result is lowercase ASCII.
///
/// Returns `None` for empty input or for inputs that consist entirely
/// of non-ASCII-alphanumeric characters (which would otherwise produce
/// an empty post-trim slug).
pub(crate) fn slugify_segment(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(raw.len());
    let mut last_was_dash = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if last_was_dash {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Kebab-case slug from the leaf component of a path. Empty or
/// un-slugifiable leaves degrade to `"root"`.
fn leaf_slug(candidate_dir: &Path) -> String {
    let raw = candidate_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    slugify_segment(raw).unwrap_or_else(|| "root".to_string())
}

/// Apply [`slugify_segment`] to each `Normal` component of `path` and
/// join the surviving slugs with `/`. Skips empty/un-slugifiable
/// segments. Mirrors the path-derived shape of an id that L4 would
/// allocate for a chain of nested components, so callers can match a
/// candidate dir against the id form a user sees in `components.yaml`.
pub(crate) fn slugify_path(path: &Path) -> String {
    use std::path::Component;
    path.components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str().and_then(slugify_segment),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn content_hash_suffix(candidate_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(candidate_dir.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    hex[..HASH_SUFFIX_WIDTH].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn existing(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn root_component_id_is_just_the_leaf_slug() {
        let id = allocate_id(&PathBuf::from("/ws/atlas-engine"), None, &existing(&[]));
        assert_eq!(id, "atlas-engine");
    }

    #[test]
    fn nested_component_id_prefixes_parent_id() {
        let id = allocate_id(
            &PathBuf::from("/ws/crates/inner"),
            Some("atlas"),
            &existing(&[]),
        );
        assert_eq!(id, "atlas/inner");
    }

    #[test]
    fn collision_at_root_falls_back_to_numeric_suffix() {
        let id = allocate_id(&PathBuf::from("/ws/foo"), None, &existing(&["foo"]));
        assert_eq!(id, "foo-2");
    }

    #[test]
    fn collision_uses_next_free_numeric_suffix() {
        let id = allocate_id(
            &PathBuf::from("/ws/foo"),
            None,
            &existing(&["foo", "foo-2", "foo-3"]),
        );
        assert_eq!(id, "foo-4");
    }

    #[test]
    fn collision_under_parent_uses_parent_slash_suffix() {
        let id = allocate_id(
            &PathBuf::from("/ws/a/bar"),
            Some("a"),
            &existing(&["a/bar"]),
        );
        assert_eq!(id, "a/bar-2");
    }

    #[test]
    fn leaf_slug_kebab_cases_non_alphanumeric() {
        let id = allocate_id(&PathBuf::from("/ws/My Weird Name!"), None, &existing(&[]));
        assert_eq!(id, "my-weird-name");
    }

    #[test]
    fn leaf_slug_compresses_runs_of_dashes() {
        let id = allocate_id(&PathBuf::from("/ws/a---b"), None, &existing(&[]));
        assert_eq!(id, "a-b");
    }

    #[test]
    fn leaf_slug_strips_leading_trailing_dashes() {
        let id = allocate_id(&PathBuf::from("/ws/--foo--"), None, &existing(&[]));
        assert_eq!(id, "foo");
    }

    #[test]
    fn empty_leaf_degrades_to_root() {
        // A path ending in "/" has no file_name; treat as root.
        let id = allocate_id(&PathBuf::from("/"), None, &existing(&[]));
        assert_eq!(id, "root");
    }

    #[test]
    fn hash_fallback_activates_after_numeric_exhaustion() {
        let mut taken: HashSet<String> = HashSet::new();
        taken.insert("foo".to_string());
        for i in 2..=MAX_NUMERIC_SUFFIX {
            taken.insert(format!("foo-{i}"));
        }
        let id = allocate_id(&PathBuf::from("/ws/foo"), None, &taken);
        assert!(
            id.starts_with("foo-") && id.len() == "foo-".len() + HASH_SUFFIX_WIDTH,
            "expected content-hash suffix fallback, got {id}"
        );
    }
}
