//! Per-scope `.atlas/.gitignore` writer.
//!
//! Phase 3 design §5.6: each `.atlas/` scope (workspace top-level and
//! every per-component) gets a one-line `.gitignore` listing `cache/`
//! so retrofit cache files (PR-2..PR-5) are not accidentally committed.
//! The `.gitignore` files themselves are committed, so a fresh checkout
//! gets the right ignore rules without the user thinking about it.
//!
//! The writer is idempotent and respectful:
//!
//! - File absent: write `cache/\n` via [`atomic_write`].
//! - File present and lists `cache/` on its own line: no-op.
//! - File present but does NOT list `cache/`: leave it alone, log one
//!   warning to stderr. The user customised it; respect that.

use std::fs;
use std::io;
use std::path::Path;

use crate::atomic_write::atomic_write;

/// Outcome of [`ensure_atlas_gitignore`]. Callers may use this to
/// dedup warnings (the `eprintln!` is fired by this module, but the
/// caller may want to remember which scopes have already been visited
/// so it does not re-walk the file on every cache write).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureGitignoreOutcome {
    /// The file did not exist; we wrote `cache/\n`.
    Wrote,
    /// The file existed and already listed `cache/`. No-op.
    AlreadyPresent,
    /// The file existed but did not list `cache/`. We left it alone
    /// and emitted a warning.
    CustomisedWithoutCacheLine,
}

/// Ensure that `<scope>/.atlas/.gitignore` exists and lists `cache/`.
///
/// `scope` is the directory that contains (or will contain) an
/// `.atlas/` subtree — typically the workspace root or a per-component
/// directory. The function creates `<scope>/.atlas/` if it does not
/// exist (the gitignore file must live inside `.atlas/`).
///
/// See module docs for the three-way outcome.
pub fn ensure_atlas_gitignore(scope: &Path) -> io::Result<EnsureGitignoreOutcome> {
    let gitignore_path = scope.join(".atlas").join(".gitignore");

    if gitignore_path.exists() {
        let contents = fs::read_to_string(&gitignore_path)?;
        if has_cache_line(&contents) {
            return Ok(EnsureGitignoreOutcome::AlreadyPresent);
        }
        eprintln!(
            "warning: .atlas/.gitignore at {} exists but does not list cache/; \
             cache files may be tracked unintentionally",
            gitignore_path.display()
        );
        return Ok(EnsureGitignoreOutcome::CustomisedWithoutCacheLine);
    }

    atomic_write(&gitignore_path, b"cache/\n")?;
    Ok(EnsureGitignoreOutcome::Wrote)
}

/// Returns `true` iff `contents` has at least one line whose trimmed
/// form is exactly `cache/`. Splits on `\n` (CRLF lines are tolerated
/// because `trim` removes the trailing `\r`).
fn has_cache_line(contents: &str) -> bool {
    contents.split('\n').any(|line| line.trim() == "cache/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_writes_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let scope = tmp.path();

        let outcome = ensure_atlas_gitignore(scope).unwrap();
        assert_eq!(outcome, EnsureGitignoreOutcome::Wrote);

        let path = scope.join(".atlas/.gitignore");
        assert!(path.exists(), "expected .atlas/.gitignore to exist");
        let got = fs::read(&path).unwrap();
        assert_eq!(got, b"cache/\n");
    }

    #[test]
    fn ensure_no_op_when_already_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let scope = tmp.path();
        let gitignore = scope.join(".atlas/.gitignore");
        fs::create_dir_all(gitignore.parent().unwrap()).unwrap();
        fs::write(&gitignore, b"cache/\n").unwrap();
        let before = fs::read(&gitignore).unwrap();

        let outcome = ensure_atlas_gitignore(scope).unwrap();
        assert_eq!(outcome, EnsureGitignoreOutcome::AlreadyPresent);

        let after = fs::read(&gitignore).unwrap();
        assert_eq!(before, after, "file must be byte-for-byte unchanged");
    }

    #[test]
    fn ensure_warns_when_customised_without_cache_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        let scope = tmp.path();
        let gitignore = scope.join(".atlas/.gitignore");
        fs::create_dir_all(gitignore.parent().unwrap()).unwrap();
        fs::write(&gitignore, b"*.log\n").unwrap();
        let before = fs::read(&gitignore).unwrap();

        let outcome = ensure_atlas_gitignore(scope).unwrap();
        assert_eq!(outcome, EnsureGitignoreOutcome::CustomisedWithoutCacheLine);

        let after = fs::read(&gitignore).unwrap();
        assert_eq!(
            before, after,
            "customised .gitignore must be left untouched"
        );
    }

    #[test]
    fn ensure_recognises_cache_line_among_other_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let scope = tmp.path();
        let gitignore = scope.join(".atlas/.gitignore");
        fs::create_dir_all(gitignore.parent().unwrap()).unwrap();
        // Multi-line file with `cache/` somewhere in the middle.
        fs::write(&gitignore, b"*.log\ncache/\n!keep.txt\n").unwrap();

        let outcome = ensure_atlas_gitignore(scope).unwrap();
        assert_eq!(outcome, EnsureGitignoreOutcome::AlreadyPresent);
    }

    #[test]
    fn ensure_does_not_match_substrings_or_other_forms() {
        // `cache` (no slash), `mycache/`, `# cache/` should not count.
        let tmp = tempfile::TempDir::new().unwrap();
        let scope = tmp.path();
        let gitignore = scope.join(".atlas/.gitignore");
        fs::create_dir_all(gitignore.parent().unwrap()).unwrap();
        fs::write(&gitignore, b"cache\nmycache/\n# cache/\n").unwrap();

        let outcome = ensure_atlas_gitignore(scope).unwrap();
        assert_eq!(outcome, EnsureGitignoreOutcome::CustomisedWithoutCacheLine);
    }
}
