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
    seed_filesystem_inner(db, root, None, respect_gitignore, DEFAULT_BINARY_SIZE_LIMIT)
}

/// Like [`seed_filesystem`] but additionally prunes any path inside
/// `excluded_dir` from the walk. The CLI uses this to keep its own
/// output directory (default `.atlas/`, override via `--output-dir`)
/// from being ingested as analysis input on a re-run, which would
/// otherwise feed prior `components.yaml` and `llm-cache.json` back
/// into L0 even when the target's `.gitignore` does not list it.
///
/// When `excluded_dir` does not resolve to a path under `root`, the
/// pruning is silently skipped — `output_dir` outside the analysis
/// tree is harmless on its own, since the walker would never reach it.
pub fn seed_filesystem_excluding(
    db: &mut AtlasDatabase,
    root: &Path,
    excluded_dir: &Path,
    respect_gitignore: bool,
) -> Result<()> {
    seed_filesystem_inner(
        db,
        root,
        Some(excluded_dir),
        respect_gitignore,
        DEFAULT_BINARY_SIZE_LIMIT,
    )
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
    seed_filesystem_inner(db, root, None, respect_gitignore, binary_size_limit)
}

fn seed_filesystem_inner(
    db: &mut AtlasDatabase,
    root: &Path,
    excluded_dir: Option<&Path>,
    respect_gitignore: bool,
    binary_size_limit: u64,
) -> Result<()> {
    let mut registered: Vec<File> = Vec::new();
    let mut git_boundary_dirs: Vec<PathBuf> = Vec::new();

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(respect_gitignore)
        .git_exclude(respect_gitignore)
        .git_global(respect_gitignore)
        .parents(false)
        .require_git(false);

    // When the caller designated an excluded directory inside `root`
    // (typically Atlas's own output dir), prune descent into it so
    // prior-run artefacts cannot be re-ingested as L0 inputs.
    if let Some(excluded) = excluded_dir {
        if let Some(excluded_rel) = excluded_relative_to(root, excluded) {
            let root_owned = root.to_path_buf();
            builder.filter_entry(move |entry| match entry.path().strip_prefix(&root_owned) {
                Ok(rel) => !rel.starts_with(&excluded_rel),
                Err(_) => true,
            });
        }
    }

    let walker = builder.build();

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

/// Compute `excluded` as a non-empty relative path under `root`, or
/// return `None` when no exclusion should apply (e.g., `excluded` lives
/// outside `root`, or equals `root` — pruning the latter would empty
/// the walk).
///
/// Uses a lexical `strip_prefix` first (no syscalls, sufficient for the
/// common case where the CLI builds `output_dir = root.join(".atlas")`)
/// and falls back to canonicalising both paths so symlinked or
/// `..`-laden paths still resolve.
fn excluded_relative_to(root: &Path, excluded: &Path) -> Option<PathBuf> {
    if let Ok(rel) = excluded.strip_prefix(root) {
        if !rel.as_os_str().is_empty() {
            return Some(rel.to_path_buf());
        }
    }
    let root_canonical = root.canonicalize().ok()?;
    let excluded_canonical = excluded.canonicalize().ok()?;
    let rel = excluded_canonical.strip_prefix(&root_canonical).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_path_buf())
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
