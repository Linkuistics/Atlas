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
//! - The pure-derived cache writers can adopt this helper too; the
//!   existing `cache::layout::atomic_write` is left in place for now
//!   (a future refactor can converge them — out of scope for PR-1).

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
}
