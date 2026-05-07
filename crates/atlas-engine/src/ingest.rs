//! Seeding a freshly-constructed [`AtlasDatabase`] from a slice of
//! filesystem roots. One call to [`seed_filesystem`] (or its
//! single-root convenience [`seed_filesystem_one`]) registers every
//! file under each root as an L0 [`File`] input and records every
//! `.git`-containing directory for use by the `git_boundaries` query.
//!
//! Multi-root semantics: each root is walked independently, the file
//! sets are unioned (deduped by path inside [`AtlasDatabase::register_file`]),
//! and the git-boundary set is the union of per-root boundaries. The
//! resulting `Vec<File>` and `Vec<PathBuf>` are sorted for determinism
//! so Salsa sees the same input shape across re-runs that walk the
//! same roots in the same order.
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

/// Walk every root in `roots`, registering every file as an L0
/// [`File`] input on `db` and recording every directory that contains
/// a `.git` marker.
///
/// `respect_gitignore` turns on `.gitignore` filtering via the
/// `ignore` crate; when `false`, every readable file (except those
/// inside a `.git` directory, which we explicitly skip because their
/// contents are not source code) is registered.
///
/// Per-root walks are concatenated; the file set is deduped (by
/// absolute path) and sorted, the git-boundary set is the union of
/// per-root boundaries, also sorted+deduped.
pub fn seed_filesystem(
    db: &mut AtlasDatabase,
    roots: &[PathBuf],
    respect_gitignore: bool,
) -> Result<()> {
    seed_filesystem_inner(db, roots, &[], respect_gitignore, DEFAULT_BINARY_SIZE_LIMIT)
}

/// Single-root convenience wrapper around [`seed_filesystem`]. Most
/// tests in atlas-engine seed exactly one root at a time; this saves
/// the `&[root.to_path_buf()]` boilerplate at every call site.
pub fn seed_filesystem_one(
    db: &mut AtlasDatabase,
    root: &Path,
    respect_gitignore: bool,
) -> Result<()> {
    seed_filesystem(db, &[root.to_path_buf()], respect_gitignore)
}

/// Like [`seed_filesystem`] but additionally prunes any path inside
/// `excluded_dirs` from the walks. The CLI uses this to keep its own
/// output directory (default `.atlas/`, override via `--output-dir`)
/// from being ingested as analysis input on a re-run, which would
/// otherwise feed prior `components.yaml` and the persistent cache's
/// `<stage>/<sha>.blob` files back into L0 even when the target's
/// `.gitignore` does not list it.
///
/// `excluded_dirs.len()` should typically equal `roots.len()` (one
/// excluded dir per root), but the function does not enforce that —
/// each excluded dir is resolved against the matching root by
/// position; surplus exclusions are ignored, surplus roots get no
/// exclusion. When an excluded path does not resolve to a path under
/// its corresponding root, the pruning is silently skipped — output
/// directories outside the analysis tree are harmless on their own,
/// since the walker would never reach them.
pub fn seed_filesystem_excluding(
    db: &mut AtlasDatabase,
    roots: &[PathBuf],
    excluded_dirs: &[PathBuf],
    respect_gitignore: bool,
) -> Result<()> {
    seed_filesystem_inner(
        db,
        roots,
        excluded_dirs,
        respect_gitignore,
        DEFAULT_BINARY_SIZE_LIMIT,
    )
}

/// Single-root convenience wrapper around [`seed_filesystem_excluding`].
pub fn seed_filesystem_excluding_one(
    db: &mut AtlasDatabase,
    root: &Path,
    excluded_dir: &Path,
    respect_gitignore: bool,
) -> Result<()> {
    seed_filesystem_excluding(
        db,
        &[root.to_path_buf()],
        &[excluded_dir.to_path_buf()],
        respect_gitignore,
    )
}

/// Variant of [`seed_filesystem`] that lets the caller override the
/// binary-file size limit. Useful for tests that want every fixture
/// file loaded regardless of size.
pub fn seed_filesystem_with_limit(
    db: &mut AtlasDatabase,
    roots: &[PathBuf],
    respect_gitignore: bool,
    binary_size_limit: u64,
) -> Result<()> {
    seed_filesystem_inner(db, roots, &[], respect_gitignore, binary_size_limit)
}

fn seed_filesystem_inner(
    db: &mut AtlasDatabase,
    roots: &[PathBuf],
    excluded_dirs: &[PathBuf],
    respect_gitignore: bool,
    binary_size_limit: u64,
) -> Result<()> {
    if roots.is_empty() {
        anyhow::bail!("seed_filesystem: at least one root is required");
    }

    let mut registered: Vec<File> = Vec::new();
    let mut git_boundary_dirs: Vec<PathBuf> = Vec::new();
    let mut seen_paths: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for (idx, root) in roots.iter().enumerate() {
        let excluded_dir = excluded_dirs.get(idx).map(|p| p.as_path());
        seed_one_root(
            db,
            root,
            excluded_dir,
            respect_gitignore,
            binary_size_limit,
            &mut registered,
            &mut git_boundary_dirs,
            &mut seen_paths,
        )?;
    }

    // Deterministic order for tests and for Salsa: same inputs → same
    // ordering. The dedup-by-path already happened via `seen_paths`.
    registered.sort_by_key(|f| f.path(db).clone());
    git_boundary_dirs.sort();
    git_boundary_dirs.dedup();

    db.set_workspace_files(registered);
    db.set_git_boundary_dirs(git_boundary_dirs);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn seed_one_root(
    db: &mut AtlasDatabase,
    root: &Path,
    excluded_dir: Option<&Path>,
    respect_gitignore: bool,
    binary_size_limit: u64,
    registered: &mut Vec<File>,
    git_boundary_dirs: &mut Vec<PathBuf>,
    seen_paths: &mut std::collections::HashSet<PathBuf>,
) -> Result<()> {
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
    //
    // Additionally — universally — prune every `.atlas/` directory
    // anywhere under the walk. PR-6's per-component writers create
    // `<component>/.atlas/component.yaml` (and PR-7+ adds more); on
    // re-run the file walk would otherwise pick those up and L8's
    // immediate-subdir enumeration would treat `<component>/.atlas`
    // as a candidate component directory. Per the design (§4.6,
    // §5.4), `.atlas/` is tool-owned: its contents are projections
    // of the engine's own outputs, never analysis input. The
    // override-merge walk in `l4_tree.rs` reads per-component
    // override files directly from disk, side-stepping this
    // exclusion.
    let excluded_owned: Option<PathBuf> =
        excluded_dir.and_then(|excluded| excluded_relative_to(root, excluded));
    let root_owned = root.to_path_buf();
    builder.filter_entry(move |entry| {
        let path = entry.path();
        // Universal `.atlas/` prune. The check is against the file
        // name component, so it fires regardless of nesting depth.
        if path.file_name().and_then(|n| n.to_str()) == Some(".atlas") {
            return false;
        }
        match path.strip_prefix(&root_owned) {
            Ok(rel) => match &excluded_owned {
                Some(excluded_rel) => !rel.starts_with(excluded_rel),
                None => true,
            },
            Err(_) => true,
        }
    });

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

        // Multi-root dedup: a file path that already appeared under an
        // earlier root (e.g. nested roots, symlink-walked overlap) is
        // skipped here so the registered vector matches the file set
        // by-path, not by-discovery-order.
        let path_buf = path.to_path_buf();
        if !seen_paths.insert(path_buf.clone()) {
            continue;
        }

        let bytes = read_file_bounded(path, binary_size_limit)
            .with_context(|| format!("reading {}", path.display()))?;
        let file = db.register_file(path_buf, bytes);
        registered.push(file);
    }

    Ok(())
}

/// Returns `Some(rel)` where `rel` is `excluded`'s path relative to
/// `root`, or `None` if `excluded` is not under `root` (or equals
/// `root` — pruning that would empty the walk). An empty `excluded`
/// always returns `None` because `canonicalize("")` fails — callers
/// (e.g. atlas-cli's pipeline) rely on this as a no-op sentinel for
/// roots that don't have an exclusion.
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
