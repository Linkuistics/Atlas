//! On-disk path layout and atomic I/O for the persistent cache.
//!
//! Layout: `<root>/cache/<stage>/<sha>.blob`. Stage directory names are
//! the lowercase form (`l1`, `l2`, …, `l9`), matching the kebab-case
//! `Stage` serialisation used in `analyzers.yaml` (atlas-contracts
//! design §6.6).
//!
//! Atomic write: a `tempfile::NamedTempFile` is created in the same
//! parent directory as the target (cross-fs renames are not atomic,
//! per the tempfile crate docs), the blob is written and flushed, and
//! the temp file is `persist`-ed onto the target path. A kill -9 mid
//! write either leaves the target absent (preferred) or, very rarely,
//! a stray `.tmp*` sibling — which never matches `<sha>.blob` and is
//! therefore unreachable from `get()`.
//!
//! Read: `fs::read` of the target path, with `NotFound` mapped to
//! `Ok(None)`. Other I/O errors propagate.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use atlas_index::Stage;
use tempfile::NamedTempFile;

use super::Sha256Hex;

/// Subdirectory under the cache root holding stage-keyed blobs.
/// `<root>/cache/<stage>/<sha>.blob`.
pub(crate) const CACHE_DIRNAME: &str = "cache";

/// Lowercase stage directory name. Matches the `Stage` kebab-case
/// serialisation used in `analyzers.yaml`.
pub(crate) fn stage_dir_name(stage: Stage) -> &'static str {
    match stage {
        Stage::L1 => "l1",
        Stage::L2 => "l2",
        Stage::L3 => "l3",
        Stage::L4 => "l4",
        Stage::L5 => "l5",
        Stage::L6 => "l6",
        Stage::L7 => "l7",
        Stage::L8 => "l8",
        Stage::L9 => "l9",
    }
}

/// All nine stages, in declaration order. Used by GC to enumerate the
/// per-stage subdirectories of the cache root.
pub(crate) const ALL_STAGES: [Stage; 9] = [
    Stage::L1,
    Stage::L2,
    Stage::L3,
    Stage::L4,
    Stage::L5,
    Stage::L6,
    Stage::L7,
    Stage::L8,
    Stage::L9,
];

/// Canonical blob suffix. The GC pass uses this to distinguish
/// fully-written cache entries from stray `.tmp*` files left behind
/// by an interrupted atomic write.
pub(crate) const BLOB_SUFFIX: &str = ".blob";

/// Resolve the absolute on-disk path for `(stage, fingerprint)`.
pub(crate) fn blob_path(root: &Path, stage: Stage, fingerprint: &Sha256Hex) -> PathBuf {
    root.join(CACHE_DIRNAME)
        .join(stage_dir_name(stage))
        .join(format!("{fingerprint}{BLOB_SUFFIX}"))
}

/// Resolve the per-stage directory under the cache root.
pub(crate) fn stage_dir_path(root: &Path, stage: Stage) -> PathBuf {
    root.join(CACHE_DIRNAME).join(stage_dir_name(stage))
}

/// Atomically write `blob` to `target`. Creates the parent directory
/// chain if absent; uses `tempfile::NamedTempFile::new_in` so the
/// rename target lives on the same filesystem (cross-fs rename is not
/// atomic).
pub(crate) fn atomic_write(target: &Path, blob: &[u8]) -> Result<()> {
    let dir = target
        .parent()
        .with_context(|| format!("cache target has no parent directory: {}", target.display()))?;
    fs::create_dir_all(dir)
        .with_context(|| format!("creating cache directory {}", dir.display()))?;

    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating temp file in {}", dir.display()))?;
    tmp.write_all(blob)
        .with_context(|| format!("writing cache blob to temp file in {}", dir.display()))?;
    tmp.flush()
        .with_context(|| format!("flushing cache blob to temp file in {}", dir.display()))?;
    // `persist` performs the atomic rename onto `target`. On POSIX the
    // rename is atomic within the same filesystem.
    tmp.persist(target).map_err(|e| {
        anyhow::anyhow!("persisting cache blob to {}: {}", target.display(), e.error)
    })?;
    Ok(())
}

/// Read `target` if present. `NotFound` maps to `Ok(None)`; other
/// I/O errors propagate so the caller can distinguish "miss" from
/// "filesystem broken".
pub(crate) fn read_if_exists(target: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(target) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading cache blob {}", target.display())),
    }
}

/// Parse a directory entry filename of the form `<sha>.blob`. Returns
/// `Some(sha)` for valid blob filenames; `None` for anything else
/// (including stray `.tmp*` files left by an interrupted atomic
/// write — those are unreachable from `get()` and ignored by GC).
pub(crate) fn fingerprint_from_blob_filename(name: &str) -> Option<&str> {
    name.strip_suffix(BLOB_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_dir_name_matches_kebab_lowercase() {
        // The kebab-case lowercase form is the normative wire form for
        // `analyzers.yaml`; the cache layout reuses it so a future
        // `<stage>` lookup based on serde::Serialize<Stage> stays in
        // sync.
        for (s, want) in [
            (Stage::L1, "l1"),
            (Stage::L2, "l2"),
            (Stage::L3, "l3"),
            (Stage::L4, "l4"),
            (Stage::L5, "l5"),
            (Stage::L6, "l6"),
            (Stage::L7, "l7"),
            (Stage::L8, "l8"),
            (Stage::L9, "l9"),
        ] {
            assert_eq!(stage_dir_name(s), want);
        }
    }

    #[test]
    fn blob_path_has_three_components_under_root() {
        let p = blob_path(Path::new("/r"), Stage::L3, &"deadbeef".to_string());
        assert_eq!(p, PathBuf::from("/r/cache/l3/deadbeef.blob"));
    }

    #[test]
    fn fingerprint_from_blob_filename_strips_suffix_only() {
        assert_eq!(fingerprint_from_blob_filename("abc.blob"), Some("abc"));
        assert_eq!(fingerprint_from_blob_filename("abc.tmp1234"), None);
        assert_eq!(fingerprint_from_blob_filename("abc"), None);
    }
}
