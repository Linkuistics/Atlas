//! Test helpers for `cache::*` integration. Lives behind `#[cfg(test)]`
//! and is never compiled into release artifacts.
//!
//! [`TempCache`] wraps a `tempfile::TempDir` (kept alive for the
//! duration of the test) and a [`PersistentCache`] opened against it.
//! `Deref`'ing exposes the cache directly so tests can write
//! `cache.put(...)` instead of `cache.cache.put(...)`.

use std::ops::Deref;
use std::path::Path;

use tempfile::TempDir;

use super::PersistentCache;

/// In-test scaffolding for a fresh persistent cache. Drops the
/// underlying tempdir at end of scope, taking the on-disk cache with
/// it.
pub struct TempCache {
    /// Held to keep the directory alive; not read directly.
    _dir: TempDir,
    cache: PersistentCache,
}

impl TempCache {
    /// Create a new `TempCache` rooted in a fresh tempdir. The
    /// tempdir is removed when the `TempCache` is dropped.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir for TempCache");
        let cache = PersistentCache::open(dir.path()).expect("open TempCache");
        TempCache { _dir: dir, cache }
    }

    /// Path of the cache root. Useful for tests that want to inspect
    /// the on-disk layout directly.
    pub fn root(&self) -> &Path {
        self.cache.root()
    }
}

impl Default for TempCache {
    fn default() -> Self {
        TempCache::new()
    }
}

impl Deref for TempCache {
    type Target = PersistentCache;
    fn deref(&self) -> &PersistentCache {
        &self.cache
    }
}
