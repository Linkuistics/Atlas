//! Integration tests that shell out to the compiled `atlas` binary.
//!
//! Anything that needs the live pipeline is covered by
//! `pipeline_integration.rs`; this file only exercises error paths
//! and version/help output, which do not require the `claude` CLI to
//! be on PATH.

use assert_cmd::Command;
use predicates::str;

fn atlas() -> Command {
    Command::cargo_bin("atlas").expect("atlas binary must be built")
}

#[test]
fn version_flag_prints_crate_version() {
    let expected = format!("atlas {}", env!("CARGO_PKG_VERSION"));
    atlas()
        .arg("--version")
        .assert()
        .success()
        .stdout(str::contains(expected));
}

#[test]
fn index_requires_budget_flag() {
    atlas()
        .args(["index", "."])
        .assert()
        .failure()
        .stderr(str::contains("--budget"));
}

#[test]
fn index_with_nonexistent_root_fails_with_clear_error() {
    atlas()
        .args([
            "index",
            "/definitely/does/not/exist/for/atlas",
            "--no-budget",
        ])
        .assert()
        .failure()
        .stderr(str::contains("failed to resolve root path"));
}

#[test]
fn help_lists_core_flags() {
    atlas()
        .args(["index", "--help"])
        .assert()
        .success()
        .stdout(str::contains("--output-dir"))
        .stdout(str::contains("--budget"))
        .stdout(str::contains("--max-depth"))
        .stdout(str::contains("--dry-run"))
        .stdout(str::contains("--recarve"));
}
