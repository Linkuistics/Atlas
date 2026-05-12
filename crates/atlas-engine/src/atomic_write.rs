//! Shared atomic-write helper.
//!
//! [`atomic_write`] writes `bytes` to `path` via a temp-file +
//! `fsync` + `rename` sequence so a crash mid-write cannot leave the
//! destination half-written. The temp file lives in the same directory
//! as `path` (so the rename stays on one filesystem and remains atomic
//! on POSIX) and is named `<file_name>.tmp.<pid>.<rand-u64>` to avoid
//! collisions between concurrent writers.
//!
//! Phase 3 design §6.3 (atomic write requirements). Used by:
//!
//! - PR-1 itself, for `<scope>/.atlas/.gitignore` (eat-our-own-dogfood).
//! - PR-8 (drift snapshot) and PR-10 (modularity history) — stateful
//!   files where a half-written state would corrupt the next run's
//!   baseline.
//! - The persistent cache writer (`cache::PersistentCache::put`),
//!   which converged on this helper in Phase 4 PR-4 (the previously
//!   duplicated `cache::layout::atomic_write` was deleted).

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Atomically write `bytes` to `path`.
///
/// Sequence:
///
/// 1. Create the parent directory chain (if missing).
/// 2. Open `<path>.tmp.<pid>.<rand-u64>` in the same directory with
///    `create(true).truncate(true).write(true)`.
/// 3. `write_all(bytes)` then `sync_all()` to flush data + metadata.
/// 4. `rename` the temp file onto `path` (atomic within one filesystem
///    on POSIX).
///
/// On any error mid-write the temp file is best-effort cleaned up; the
/// destination is never partially overwritten.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "atomic_write target has no parent directory: {}",
                path.display()
            ),
        )
    })?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = temp_path_for(path);

    // Best-effort cleanup wrapper: if any step fails, remove the temp
    // file (ignoring errors from the cleanup itself) before returning
    // the original error.
    let result = write_then_rename(&temp_path, path, bytes);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_then_rename(temp_path: &Path, dest_path: &Path, bytes: &[u8]) -> io::Result<()> {
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    // Test-only hook: between temp-write and rename, optionally
    // panic to simulate a kill mid-write. Gated behind the
    // `atomic_write_panic_after_temp` feature so the hook is
    // compiled out of release builds AND of any in-tree consumer
    // that does not opt in (continuation-prompt PR-12
    // non-negotiable). The internal `cfg(test)` form is preserved
    // for the unit test in this module.
    #[cfg(test)]
    self::test_hooks::maybe_panic_before_rename();
    #[cfg(feature = "atomic_write_panic_after_temp")]
    self::test_hooks_pub::maybe_panic_before_rename();

    fs::rename(temp_path, dest_path)?;

    // Symmetric test-only hook: between rename and return,
    // optionally panic to simulate a kill *after* the rename has
    // landed. This pairs with the before-rename hook so PR-12's
    // fixture suite can prove both branches of the atomic-write
    // semantic: a kill before-rename leaves the destination
    // fully-old, a kill after-rename leaves it fully-new. Same
    // feature gate; same one-shot, thread-local discipline.
    #[cfg(feature = "atomic_write_panic_after_temp")]
    self::test_hooks_pub::maybe_panic_after_rename();

    Ok(())
}

/// Atomically write two related files. Either both land or neither
/// does (modulo the residual half-pair window between the two
/// renames). Used by the transcript cache (Phase 7 PR-2), where
/// `<sha>.transcript` and `<sha>.output` must move together; a crash
/// between the two single-file writes would leave a transcript without
/// its output and corrupt the cache.
///
/// Sequence:
///
/// 1. Create the parent directory chain of each path (if missing).
///    The two paths are allowed to live in different parents; the
///    helper does not enforce a shared parent because the cache layout
///    happens to colocate them but the primitive itself is general.
/// 2. Open `<path_a>.tmp.<pid>.<rand-u64>` and `<path_b>.tmp...` —
///    independent nonces so concurrent writers on the same target pair
///    do not collide.
/// 3. `write_all` + `sync_all` both temp files.
/// 4. Rename `path_a` first, then `path_b`. A crash between the two
///    renames is the residual failure mode — the cache eviction path
///    detects half-pair entries via the recorded-fingerprint spot
///    check (recast §6.3) on read and triggers re-run, not corruption.
///
/// Forensic value of the two-file primitive over an envelope-wrapper:
/// transcripts remain debuggable side-by-side even if the output is
/// corrupt. The half-pair window (post-a-rename / pre-b-rename) is
/// detectable on next read via fingerprint mismatch and triggers
/// re-run, not corruption.
///
/// On any error mid-write both temp files are best-effort cleaned up
/// before returning the original error.
pub fn atomic_write_pair(
    path_a: &Path,
    bytes_a: &[u8],
    path_b: &Path,
    bytes_b: &[u8],
) -> io::Result<()> {
    let parent_a = path_a.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "atomic_write_pair: path_a has no parent directory: {}",
                path_a.display()
            ),
        )
    })?;
    if !parent_a.as_os_str().is_empty() {
        fs::create_dir_all(parent_a)?;
    }
    let parent_b = path_b.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "atomic_write_pair: path_b has no parent directory: {}",
                path_b.display()
            ),
        )
    })?;
    if !parent_b.as_os_str().is_empty() {
        fs::create_dir_all(parent_b)?;
    }

    let temp_a = temp_path_for(path_a);
    let temp_b = temp_path_for(path_b);

    // Both temps must be written + fsynced before either rename. A
    // failure at any of the four steps best-effort cleans up both
    // temps.
    let result = write_pair_then_rename(&temp_a, &temp_b, path_a, path_b, bytes_a, bytes_b);
    if result.is_err() {
        let _ = fs::remove_file(&temp_a);
        let _ = fs::remove_file(&temp_b);
    }
    result
}

fn write_pair_then_rename(
    temp_a: &Path,
    temp_b: &Path,
    dest_a: &Path,
    dest_b: &Path,
    bytes_a: &[u8],
    bytes_b: &[u8],
) -> io::Result<()> {
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_a)?;
        f.write_all(bytes_a)?;
        f.sync_all()?;
    }
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_b)?;
        f.write_all(bytes_b)?;
        f.sync_all()?;
    }

    // Rename a, then b. A crash between renames leaves a fully-new
    // path_a + still-old path_b; the cache's recorded-fingerprint
    // spot-check evicts the half-pair on next read (recast §6.3).
    fs::rename(temp_a, dest_a)?;
    fs::rename(temp_b, dest_b)?;
    Ok(())
}

/// Compose the temp-file path: `<dest>.tmp.<pid>.<rand-u64>`. The temp
/// lives in the same directory as `dest` so the final rename stays on
/// one filesystem (cross-fs rename is not atomic on POSIX).
fn temp_path_for(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new(""));
    let file_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "atomic_write".to_string());
    let pid = std::process::id();
    let nonce = nonce_u64();
    parent.join(format!("{file_name}.tmp.{pid}.{nonce:016x}"))
}

/// Cheap u64 nonce. The plan permits any small random source — we
/// hash a monotonic `Instant::now()` together with a thread-local
/// counter to keep concurrent writers from picking the same nonce.
fn nonce_u64() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static COUNTER: Cell<u64> = const { Cell::new(0) };
    }
    let counter = COUNTER.with(|c| {
        let v = c.get().wrapping_add(1);
        c.set(v);
        v
    });
    let mut hasher = DefaultHasher::new();
    Instant::now().hash(&mut hasher);
    counter.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    hasher.finish()
}

/// Cross-crate test hooks for the atomic-write panic-injection
/// fixture. Gated behind the `atomic_write_panic_after_temp` cargo
/// feature so the hooks compile out of every build that does not
/// explicitly opt in (Phase 3 plan §4 PR-8 + PR-12 acceptance
/// criteria).
///
/// Two symmetric one-shot hooks are exposed:
///
/// - [`arm_panic_before_rename`] / [`disarm_panic_before_rename`] —
///   panics between the temp-file write and the rename, so the
///   destination remains fully-old (PR-8).
/// - [`arm_panic_after_rename`] / [`disarm_panic_after_rename`] —
///   panics after the rename has landed, so the destination is
///   fully-new (PR-12).
///
/// Both arm flags are thread-local and one-shot: each hook
/// auto-disarms the moment it fires, so a single armed bit cannot
/// trip a subsequent atomic_write (e.g. a follow-on assertion
/// path or a sibling write in the same test).
///
/// Usage from a consumer crate's integration test:
/// ```ignore
/// atlas_engine::atomic_write::test_hooks_pub::arm_panic_before_rename();
/// let _ = std::panic::catch_unwind(|| my_atomic_write_caller());
/// // assert the destination file is intact
/// ```
#[cfg(feature = "atomic_write_panic_after_temp")]
pub mod test_hooks_pub {
    use std::cell::Cell;

    thread_local! {
        static PANIC_BEFORE_RENAME: Cell<bool> = const { Cell::new(false) };
        static PANIC_AFTER_RENAME: Cell<bool> = const { Cell::new(false) };
    }

    /// Arm the hook so the next call to [`super::atomic_write`] on
    /// this thread panics between the temp-file write and the
    /// rename. One-shot: the hook auto-disarms when it fires (or
    /// when [`disarm_panic_before_rename`] is called).
    pub fn arm_panic_before_rename() {
        PANIC_BEFORE_RENAME.with(|c| c.set(true));
    }

    /// Defensive disarm — call from a panic-safe guard if a test
    /// might bail before the auto-disarm in
    /// [`maybe_panic_before_rename`] fires.
    pub fn disarm_panic_before_rename() {
        PANIC_BEFORE_RENAME.with(|c| c.set(false));
    }

    pub(super) fn maybe_panic_before_rename() {
        let armed = PANIC_BEFORE_RENAME.with(|c| {
            let v = c.get();
            // One-shot: disarm so the very next atomic_write in the
            // same test thread (e.g. a cleanup or a subsequent
            // assertion path) does not re-trip.
            c.set(false);
            v
        });
        if armed {
            panic!("atomic_write test hook: simulated kill before rename");
        }
    }

    /// Arm the hook so the next call to [`super::atomic_write`] on
    /// this thread panics *after* the rename has landed (the
    /// destination is already fully-new at this point). One-shot:
    /// the hook auto-disarms when it fires, mirroring the
    /// before-rename hook's discipline. PR-12's fixture suite uses
    /// this to prove the "kill after rename" branch of the
    /// atomic-write semantic — the destination must be fully-new.
    pub fn arm_panic_after_rename() {
        PANIC_AFTER_RENAME.with(|c| c.set(true));
    }

    /// Defensive disarm — call from a panic-safe guard if a test
    /// might bail before the auto-disarm in
    /// [`maybe_panic_after_rename`] fires.
    pub fn disarm_panic_after_rename() {
        PANIC_AFTER_RENAME.with(|c| c.set(false));
    }

    pub(super) fn maybe_panic_after_rename() {
        let armed = PANIC_AFTER_RENAME.with(|c| {
            let v = c.get();
            // One-shot, matching the before-rename hook.
            c.set(false);
            v
        });
        if armed {
            panic!("atomic_write test hook: simulated kill after rename");
        }
    }
}

#[cfg(test)]
mod test_hooks {
    use std::cell::Cell;

    thread_local! {
        static PANIC_BEFORE_RENAME: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn arm_panic_before_rename() {
        PANIC_BEFORE_RENAME.with(|c| c.set(true));
    }

    pub(super) fn disarm_panic_before_rename() {
        PANIC_BEFORE_RENAME.with(|c| c.set(false));
    }

    pub(super) fn maybe_panic_before_rename() {
        let armed = PANIC_BEFORE_RENAME.with(|c| {
            let v = c.get();
            // One-shot: disarm so a subsequent atomic_write in the same
            // test thread (e.g. cleanup) does not trip the hook again.
            c.set(false);
            v
        });
        if armed {
            panic!("atomic_write test hook: simulated kill before rename");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_destination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        assert!(!dest.exists());

        atomic_write(&dest, b"hello world").unwrap();

        assert!(dest.exists());
        let got = std::fs::read(&dest).unwrap();
        assert_eq!(got, b"hello world");
    }

    #[test]
    fn atomic_write_overwrites_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        std::fs::write(&dest, b"OLD").unwrap();

        atomic_write(&dest, b"NEW").unwrap();

        let got = std::fs::read(&dest).unwrap();
        assert_eq!(got, b"NEW");
    }

    #[test]
    fn atomic_write_kill_during_write_leaves_destination_intact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        std::fs::write(&dest, b"OLD").unwrap();

        // Arm the panic hook, run atomic_write inside catch_unwind, and
        // assert the destination is unchanged (the rename never fired).
        // The hook auto-disarms, so we don't need a panic-safe guard.
        let dest_for_closure = dest.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            test_hooks::arm_panic_before_rename();
            // SAFETY: we explicitly disarm in the assertion path below
            // in case catch_unwind unwinds before the hook's one-shot
            // disarm fires (it shouldn't — but defensive).
            let _ = atomic_write(&dest_for_closure, b"NEW");
        }));
        test_hooks::disarm_panic_before_rename();
        assert!(result.is_err(), "expected the test hook to panic");

        // Destination must be either fully-old (rename never fired) or
        // absent (would only be the case if we hadn't pre-populated).
        // Here we pre-populated with "OLD", so it must still read "OLD".
        let got = std::fs::read(&dest).unwrap();
        assert_eq!(
            got, b"OLD",
            "destination should be unchanged: half-written destination would corrupt the file"
        );

        // Best-effort cleanup of the temp file: the implementation
        // attempts removal on the error path, but a panic skips that
        // path. We assert that no `.tmp.<pid>.*` sibling outlives the
        // call, since the directory walk is what GC will rely on.
        // (Phase 1 cache GC ignores stray .tmp* files; this is purely
        // hygienic.)
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        // We tolerate a leftover here — the panic path skips the
        // cleanup. Just document the observation; do not fail the test.
        let _ = leftovers;
    }

    #[test]
    fn atomic_write_creates_parent_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("nested/dir/out.txt");
        assert!(!dest.parent().unwrap().exists());

        atomic_write(&dest, b"data").unwrap();

        let got = std::fs::read(&dest).unwrap();
        assert_eq!(got, b"data");
    }

    // ---------- atomic_write_pair --------------------------------------

    #[test]
    fn atomic_pair_both_files_present_after_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let a = tmp.path().join("agents/l3/abc.transcript");
        let b = tmp.path().join("agents/l3/abc.output");

        atomic_write_pair(&a, b"TRANSCRIPT", &b, b"OUTPUT").unwrap();

        assert!(a.exists(), "transcript file must exist after success");
        assert!(b.exists(), "output file must exist after success");
        assert_eq!(std::fs::read(&a).unwrap(), b"TRANSCRIPT");
        assert_eq!(std::fs::read(&b).unwrap(), b"OUTPUT");

        // No .tmp* leftovers in the stage directory.
        let stage_dir = a.parent().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(stage_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no .tmp.* siblings should outlive a successful pair write"
        );
    }

    #[test]
    fn atomic_pair_neither_partial_on_first_write_failure() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Force the first temp-file write to fail by pointing path_a
        // at a path whose parent cannot be created — on POSIX, creating
        // a subdirectory under an existing *file* fails with NotADirectory.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"this is a file, not a directory").unwrap();
        let a = blocker.join("nested/abc.transcript");
        let b = tmp.path().join("ok/abc.output");

        let result = atomic_write_pair(&a, b"T", &b, b"O");
        assert!(
            result.is_err(),
            "pair write must fail when path_a's parent cannot be created"
        );

        // Neither destination exists.
        assert!(!a.exists(), "path_a must not be created on failure");
        assert!(!b.exists(), "path_b must not be created on failure");

        // No .tmp* leftover anywhere — best-effort cleanup ran. Walk
        // both target parents (the failure was before path_b's parent
        // was even tried, but the path_b parent dir created earlier
        // could still have a stray tmp if create_dir_all happened
        // before the failure; ensure neither dir holds a tmp).
        let ok_dir = tmp.path().join("ok");
        if ok_dir.exists() {
            let leftovers: Vec<_> = std::fs::read_dir(&ok_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
                .collect();
            assert!(
                leftovers.is_empty(),
                "no .tmp.* leftovers under path_b parent on failure"
            );
        }
    }

    #[test]
    fn atomic_pair_concurrent_writers_disjoint_temp_paths() {
        // Two threads call atomic_write_pair on the same target paths
        // concurrently. Both must complete without an ENOENT-on-rename
        // collision; the temp paths the helper picks must be disjoint
        // via the pid/nonce composition. Last writer wins on the final
        // contents (content-addressed store semantics — see PersistentCache).
        let tmp = tempfile::TempDir::new().unwrap();
        let a = tmp.path().join("agents/l3/concurrent.transcript");
        let b = tmp.path().join("agents/l3/concurrent.output");

        let a1 = a.clone();
        let b1 = b.clone();
        let a2 = a.clone();
        let b2 = b.clone();

        let h1 = std::thread::spawn(move || {
            for _ in 0..10 {
                atomic_write_pair(&a1, b"T1", &b1, b"O1").unwrap();
            }
        });
        let h2 = std::thread::spawn(move || {
            for _ in 0..10 {
                atomic_write_pair(&a2, b"T2", &b2, b"O2").unwrap();
            }
        });
        h1.join().expect("writer 1");
        h2.join().expect("writer 2");

        // Both destinations exist; final bytes are one of the two
        // winners (we cannot predict which).
        assert!(a.exists());
        assert!(b.exists());
        let got_a = std::fs::read(&a).unwrap();
        assert!(got_a == b"T1" || got_a == b"T2", "got_a = {got_a:?}");
        let got_b = std::fs::read(&b).unwrap();
        assert!(got_b == b"O1" || got_b == b"O2", "got_b = {got_b:?}");

        // No .tmp* leftovers — every pair write either succeeded
        // (renaming both temps) or cleaned up on failure.
        let stage_dir = a.parent().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(stage_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no .tmp.* siblings should outlive concurrent pair writes; got {leftovers:?}"
        );
    }
}
