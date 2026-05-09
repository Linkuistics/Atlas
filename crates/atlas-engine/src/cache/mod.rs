//! Persistent content-addressed cache (Atlas vNext, design §5.5, §8.3).
//!
//! Filesystem-native object store keyed by `(stage, fingerprint)`,
//! with the value a `Vec<u8>` — typically the serialised stage
//! output. Layout:
//!
//! ```text
//! <root>/cache/<stage>/<sha>.blob
//! ```
//!
//! The `<stage>` directory name is the lowercase Stage form (`l1`,
//! `l2`, …, `l9`). The `<sha>` file stem is the hex-encoded
//! fingerprint produced by [`FingerprintBuilder::finalise`] (or any
//! caller that builds a sha256 hex by other means — the cache itself
//! treats the fingerprint as opaque bytes).
//!
//! Writes are atomic via tempfile-then-rename (see
//! [`crate::atomic_write::atomic_write`]). Reads return `Ok(None)` on
//! miss and propagate other I/O errors.
//!
//! ## Greenfield
//!
//! There are no `From<v1>` adapters. The v1 `cache_io.rs` JSON
//! serialisation is being deleted in PR-10; this module is the new
//! durability boundary. The in-memory `LlmResponseCache` remains
//! unchanged in PR-2 — it is wired through this cache by PR-10.
//!
//! ## Concurrency
//!
//! `PersistentCache` is `Send + Sync`. The atomic-rename `put` is
//! safe across processes — last writer wins, but the rename is
//! atomic so a reader never observes a partial blob. GC enumerates
//! the cache directory deterministically and tolerates entries that
//! disappear mid-walk (a concurrent GC or a normal `put` racing the
//! mark phase).

pub mod fingerprint;
pub mod layout;

#[cfg(test)]
pub mod test_fixtures;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use atlas_index::Stage;

pub use fingerprint::FingerprintBuilder;

/// A SHA-256 digest rendered as 64-character lowercase hex.
///
/// The cache uses `Sha256Hex` values as filename components, so callers
/// must produce well-formed hex; non-hex characters are silently
/// embedded in the filename and behave unpredictably across
/// filesystems. Use `crate::sha256_hex` (or any conventional sha2 hex
/// output, e.g. [`FingerprintBuilder::finalise`]) to construct.
///
/// A `String` alias keeps the API ergonomic — callers pass `&String`
/// (which derefs to `&str`); a future newtype refactor stays a
/// one-liner.
pub type Sha256Hex = String;

/// Persistent content-addressed cache rooted at a directory.
///
/// The cache root is the parent of the `cache/` subdirectory; e.g.
/// `PersistentCache::open("/repo/.atlas")` lays blobs under
/// `/repo/.atlas/cache/<stage>/<sha>.blob`. Open is idempotent and
/// creates `cache/` if absent.
#[derive(Debug, Clone)]
pub struct PersistentCache {
    root: PathBuf,
}

/// Result of a [`PersistentCache::gc`] sweep.
///
/// `kept` and `removed` count cache entries (one entry per `<sha>.blob`
/// file). `bytes_freed` is the sum of the on-disk sizes of removed
/// blobs immediately before deletion.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcReport {
    pub kept: usize,
    pub removed: usize,
    pub bytes_freed: u64,
}

impl PersistentCache {
    /// Open (or create) a cache rooted at `root`. The `cache/`
    /// subdirectory is created lazily — it is materialised on the
    /// first `put`, not eagerly. Open succeeds for any path the
    /// process can `create_dir_all` against; it does not require
    /// the path to already exist.
    pub fn open(root: &Path) -> Result<Self> {
        // Eagerly create the root directory (but not yet the per-stage
        // subdirectories — those land lazily). Calling open against a
        // missing directory is a common case during the very first
        // run; we don't want to push that responsibility onto every
        // caller.
        fs::create_dir_all(root)
            .with_context(|| format!("creating cache root {}", root.display()))?;
        Ok(PersistentCache {
            root: root.to_path_buf(),
        })
    }

    /// The cache root directory. Useful for tests and for debug
    /// tooling that wants to inspect the on-disk layout.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Look up the blob for `(stage, fingerprint)`. Returns
    /// `Ok(None)` for a miss and `Ok(Some(blob))` for a hit. Other
    /// I/O errors propagate so that a flaky filesystem never silently
    /// degrades to a cache miss (which would cause spurious LLM calls).
    pub fn get(&self, stage: Stage, fingerprint: &Sha256Hex) -> Result<Option<Vec<u8>>> {
        let path = layout::blob_path(&self.root, stage, fingerprint);
        layout::read_if_exists(&path)
    }

    /// Atomically write `blob` to the cache for `(stage, fingerprint)`.
    /// Overwrites any existing entry with the same key — this is a
    /// content-addressed store, so equal fingerprints by construction
    /// produce equal blobs (modulo non-deterministic serialisation,
    /// which is a bug the caller must fix).
    pub fn put(&self, stage: Stage, fingerprint: &Sha256Hex, blob: &[u8]) -> Result<()> {
        let path = layout::blob_path(&self.root, stage, fingerprint);
        crate::atomic_write::atomic_write(&path, blob)
            .with_context(|| format!("atomic_write to {} failed", path.display()))?;
        Ok(())
    }

    /// Mark-and-sweep GC. Walks every `<stage>/<sha>.blob` file
    /// under the cache root; entries whose `(stage, sha)` pair is
    /// **not** in `mark_set` are deleted. Returns counts and total
    /// bytes freed.
    ///
    /// The mark-set is a `BTreeSet` per the plan's literal API.
    /// PR-2 originally shipped `HashSet` because `Stage` did not
    /// derive `Ord`; PR-5 added `PartialOrd, Ord` to `Stage` and
    /// restored the `BTreeSet` signature.
    ///
    /// The walk is deterministic (per-stage in declaration order;
    /// per-stage entries sorted by filename) so test failures are
    /// reproducible. Entries that disappear between the directory
    /// listing and the unlink are treated as already-collected, not
    /// as errors — this matches the design's "GC is opportunistic,
    /// not synchronous" rule (§8.4).
    ///
    /// Stray non-blob files (e.g. `.tmp*` files left by an
    /// interrupted atomic write) are ignored: GC walks only files
    /// whose names end in `.blob`. Callers that want to clean up
    /// orphaned `.tmp*` files must do so out of band — they have no
    /// effect on cache correctness because `get()` never matches
    /// them.
    pub fn gc(&self, mark_set: &BTreeSet<(Stage, Sha256Hex)>) -> Result<GcReport> {
        let mut report = GcReport::default();
        let cache_root = self.root.join(layout::CACHE_DIRNAME);
        if !cache_root.exists() {
            // Nothing to GC. A fresh cache root with no `put` calls
            // never materialised the `cache/` subdirectory.
            return Ok(report);
        }

        for stage in layout::ALL_STAGES {
            let dir = layout::stage_dir_path(&self.root, stage);
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("reading cache stage directory {}", dir.display())
                    })
                }
            };

            // Collect filenames first so we can sort them. The
            // resulting walk order is deterministic and so is the
            // `removed` / `kept` count attribution under contention.
            let mut filenames: Vec<String> = Vec::new();
            for entry in entries {
                let entry = entry.with_context(|| {
                    format!("iterating cache stage directory {}", dir.display())
                })?;
                if let Ok(name) = entry.file_name().into_string() {
                    filenames.push(name);
                }
            }
            filenames.sort();

            for name in filenames {
                let Some(sha) = layout::fingerprint_from_blob_filename(&name) else {
                    // Not a `<sha>.blob` file — could be a stray
                    // tempfile or a foreign artifact. Leave it
                    // alone; it is unreachable from `get()`.
                    continue;
                };

                let key = (stage, sha.to_string());
                if mark_set.contains(&key) {
                    report.kept += 1;
                    continue;
                }

                let path = dir.join(&name);
                let size = match fs::metadata(&path) {
                    Ok(m) => m.len(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Concurrently removed between listdir and
                        // metadata; treat as already-GC'd.
                        continue;
                    }
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("statting cache blob {}", path.display()))
                    }
                };
                match fs::remove_file(&path) {
                    Ok(()) => {
                        report.removed += 1;
                        report.bytes_freed += size;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Race with a concurrent unlink — the entry is
                        // already gone. Count it as a successful
                        // eviction: from the caller's perspective the
                        // post-condition (this `(stage, sha)` no longer
                        // exists in the cache) is what was wanted, and
                        // `metadata().len()` already paid the cost of
                        // observing the blob's size.
                        report.removed += 1;
                        report.bytes_freed += size;
                    }
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("removing cache blob {}", path.display()))
                    }
                }
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::test_fixtures::TempCache;
    use super::*;

    #[test]
    fn open_creates_root_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested/.atlas");
        assert!(!nested.exists());
        let cache = PersistentCache::open(&nested).unwrap();
        assert!(cache.root().exists());
    }

    #[test]
    fn get_misses_unknown() {
        let cache = TempCache::new();
        let miss = cache
            .get(Stage::L3, &"deadbeef".to_string())
            .expect("get must not error on miss");
        assert!(miss.is_none());
    }

    #[test]
    fn get_hits_existing_put() {
        let cache = TempCache::new();
        let sha = "feedface".to_string();
        cache.put(Stage::L5, &sha, b"hello").unwrap();
        let blob = cache.get(Stage::L5, &sha).unwrap();
        assert_eq!(blob.as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn put_overwrites_existing_entry() {
        // The store is content-addressed: in production, the same
        // fingerprint comes with the same blob. We still want a
        // deterministic last-writer-wins semantics so a stale partial
        // blob from a prior run cannot persist forever.
        let cache = TempCache::new();
        let sha = "abc".to_string();
        cache.put(Stage::L1, &sha, b"v1").unwrap();
        cache.put(Stage::L1, &sha, b"v2").unwrap();
        let blob = cache.get(Stage::L1, &sha).unwrap();
        assert_eq!(blob.as_deref(), Some(&b"v2"[..]));
    }

    #[test]
    fn put_does_not_leak_temp_files() {
        // Verifies the atomic-write contract from layout.rs: after a
        // successful `put`, the only file under the stage directory is
        // the canonical `<sha>.blob`. No `.tmp*` siblings remain.
        let cache = TempCache::new();
        let sha = "abc".to_string();
        cache.put(Stage::L3, &sha, b"hello").unwrap();
        let stage_dir = cache.root().join("cache/l3");
        let names: Vec<String> = fs::read_dir(&stage_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["abc.blob".to_string()]);
    }

    #[test]
    fn stray_tmp_file_is_unreachable_from_get() {
        // Simulate a kill -9 mid-write: a `.tmp*` file remains in the
        // stage directory but its name does not match `<sha>.blob`,
        // so `get(stage, sha)` returns `Ok(None)`.
        let cache = TempCache::new();
        let sha = "abc".to_string();
        let stage_dir = cache.root().join("cache/l3");
        fs::create_dir_all(&stage_dir).unwrap();
        let stray = stage_dir.join(".tmp1234");
        fs::write(&stray, b"junk").unwrap();
        // Also put a real entry so the stage dir is non-empty for a
        // real reason.
        cache.put(Stage::L3, &"other".to_string(), b"data").unwrap();

        let miss = cache.get(Stage::L3, &sha).unwrap();
        assert!(miss.is_none(), "stray .tmp file must not satisfy get()");
        assert!(stray.exists(), "test invariant: stray file still present");
    }

    #[test]
    fn gc_on_empty_cache_is_noop() {
        let cache = TempCache::new();
        let mark_set = BTreeSet::new();
        let report = cache.gc(&mark_set).unwrap();
        assert_eq!(
            report,
            GcReport {
                kept: 0,
                removed: 0,
                bytes_freed: 0
            }
        );
    }

    #[test]
    fn gc_removes_unmarked_entries() {
        let cache = TempCache::new();
        let kept_sha = "1111".to_string();
        let dropped_sha = "2222".to_string();

        cache.put(Stage::L3, &kept_sha, b"keep-me").unwrap();
        cache
            .put(Stage::L3, &dropped_sha, b"drop-me-please")
            .unwrap();
        cache.put(Stage::L5, &kept_sha, b"L5-keep").unwrap();

        let mut mark_set = BTreeSet::new();
        mark_set.insert((Stage::L3, kept_sha.clone()));
        mark_set.insert((Stage::L5, kept_sha.clone()));

        let report = cache.gc(&mark_set).unwrap();
        assert_eq!(report.kept, 2);
        assert_eq!(report.removed, 1);
        assert_eq!(report.bytes_freed, b"drop-me-please".len() as u64);

        assert!(cache.get(Stage::L3, &kept_sha).unwrap().is_some());
        assert!(cache.get(Stage::L5, &kept_sha).unwrap().is_some());
        assert!(cache.get(Stage::L3, &dropped_sha).unwrap().is_none());
    }

    #[test]
    fn gc_ignores_stray_tmp_files() {
        let cache = TempCache::new();
        cache.put(Stage::L3, &"keep".to_string(), b"x").unwrap();
        let stage_dir = cache.root().join("cache/l3");
        let stray = stage_dir.join(".tmpDEAD");
        fs::write(&stray, b"junk").unwrap();

        let mut mark_set = BTreeSet::new();
        mark_set.insert((Stage::L3, "keep".to_string()));

        let report = cache.gc(&mark_set).unwrap();
        assert_eq!(report.kept, 1);
        assert_eq!(report.removed, 0);
        assert!(stray.exists(), "GC must not touch non-blob files");
    }

    #[test]
    fn gc_with_no_mark_clears_everything() {
        let cache = TempCache::new();
        cache.put(Stage::L3, &"a".to_string(), b"a").unwrap();
        cache.put(Stage::L5, &"b".to_string(), b"bb").unwrap();
        cache.put(Stage::L6, &"c".to_string(), b"ccc").unwrap();

        let report = cache.gc(&BTreeSet::new()).unwrap();
        assert_eq!(report.kept, 0);
        assert_eq!(report.removed, 3);
        assert_eq!(report.bytes_freed, 1 + 2 + 3);

        assert!(cache.get(Stage::L3, &"a".to_string()).unwrap().is_none());
        assert!(cache.get(Stage::L5, &"b".to_string()).unwrap().is_none());
        assert!(cache.get(Stage::L6, &"c".to_string()).unwrap().is_none());
    }

    #[test]
    fn gc_handles_missing_cache_root() {
        // `open` creates the cache root itself, so this exercises the
        // path where `cache/` (the subdir) was never materialised
        // because nothing was ever `put`.
        let dir = tempfile::tempdir().unwrap();
        let cache = PersistentCache::open(dir.path()).unwrap();
        assert!(!Path::new(&dir.path().join("cache")).exists());
        let report = cache.gc(&BTreeSet::new()).unwrap();
        assert_eq!(report, GcReport::default());
    }
}
