//! Seeding a freshly-constructed [`AtlasDatabase`] from a filesystem
//! root. One call to [`seed_filesystem`] registers every file under
//! `root` as an L0 [`File`] input and records every `.git`-containing
//! directory for use by the `git_boundaries` query.
//!
//! Binary files larger than [`DEFAULT_BINARY_SIZE_LIMIT`] bytes are
//! elided (registered with empty bytes) so that a single enormous
//! asset does not swell the database.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;

use crate::db::{AtlasDatabase, File};

/// Default threshold above which a file's contents are elided rather
/// than loaded into the database. One megabyte is the same ceiling
/// used by many source-analysis tools and keeps the file set
/// representative of source code rather than bundled assets.
pub const DEFAULT_BINARY_SIZE_LIMIT: u64 = 1 << 20;

/// Maximum number of bytes inspected when deciding whether a file is
/// "binary" — any NUL byte in this prefix trips the heuristic.
const BINARY_SNIFF_PREFIX: usize = 8192;

/// Walk `root`, registering every file as an L0 [`File`] input on
/// `db` and recording every directory that contains a `.git` marker.
///
/// `respect_gitignore` turns on `.gitignore` filtering via the
/// `ignore` crate; when `false`, every readable file (except those
/// inside a `.git` directory, which we explicitly skip because their
/// contents are not source code) is registered.
pub fn seed_filesystem(db: &mut AtlasDatabase, root: &Path, respect_gitignore: bool) -> Result<()> {
    seed_filesystem_with_limit(db, root, respect_gitignore, DEFAULT_BINARY_SIZE_LIMIT)
}

/// Variant of [`seed_filesystem`] that lets the caller override the
/// binary-file size limit. Useful for tests that want every fixture
/// file loaded regardless of size.
pub fn seed_filesystem_with_limit(
    db: &mut AtlasDatabase,
    root: &Path,
    respect_gitignore: bool,
    binary_size_limit: u64,
) -> Result<()> {
    let mut registered: Vec<File> = Vec::new();
    let mut git_boundary_dirs: Vec<PathBuf> = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(respect_gitignore)
        .git_exclude(respect_gitignore)
        .git_global(respect_gitignore)
        .parents(false)
        .require_git(false)
        .build();

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                return Err(anyhow::Error::new(err))
                    .with_context(|| format!("walk failed under {}", root.display()));
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };

        // `.git` directories or files: record the parent as a git
        // boundary, then skip descending into or reading them.
        if path.file_name().is_some_and(|n| n == ".git") {
            if let Some(parent) = path.parent() {
                git_boundary_dirs.push(parent.to_path_buf());
            }
            continue;
        }

        // The top-level ignore::DirEntry for a directory is yielded
        // alongside its children; we only register files.
        if !file_type.is_file() {
            continue;
        }

        let bytes = read_file_bounded(path, binary_size_limit)
            .with_context(|| format!("reading {}", path.display()))?;
        let file = db.register_file(path.to_path_buf(), bytes);
        registered.push(file);
    }

    // Deterministic order for tests and for Salsa: same inputs → same
    // ordering.
    registered.sort_by_key(|f| f.path(db).clone());
    git_boundary_dirs.sort();
    git_boundary_dirs.dedup();

    db.set_workspace_files(registered);
    db.set_git_boundary_dirs(git_boundary_dirs);
    Ok(())
}

fn read_file_bounded(path: &Path, binary_size_limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > binary_size_limit {
        // Elide: register with empty bytes. The path still matters
        // for enumeration (e.g. `manifests_in`), but the content is
        // not brought into the database.
        return Ok(Vec::new());
    }
    let bytes = fs::read(path)?;
    // NUL-byte sniff: treat as binary and elide. This is intentionally
    // cheap — false positives on UTF-16 files are acceptable because
    // we only elide, never reject.
    let prefix_len = bytes.len().min(BINARY_SNIFF_PREFIX);
    if bytes[..prefix_len].contains(&0) {
        return Ok(Vec::new());
    }
    Ok(bytes)
}
