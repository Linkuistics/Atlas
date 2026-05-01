//! End-to-end smoke test for the agent-observer pipeline.
//!
//! Spawns the real `claude -p` binary; gated behind
//! `ATLAS_LLM_RUN_CLAUDE_TESTS=1` so the default `cargo test` run does
//! not burn tokens.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

fn claude_tests_enabled() -> bool {
    std::env::var("ATLAS_LLM_RUN_CLAUDE_TESTS").ok().as_deref() == Some("1")
}

#[test]
fn atlas_index_emits_agent_sub_line_under_progress_always() {
    if !claude_tests_enabled() {
        eprintln!("skipping: opt in with ATLAS_LLM_RUN_CLAUDE_TESTS=1 to spawn `claude`");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "atlas-e2e-fixture"
version = "0.0.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(root.join("src").join("lib.rs"), "pub fn it() {}\n").unwrap();

    Command::cargo_bin("atlas")
        .unwrap()
        .arg("index")
        .arg(root)
        .arg("--no-budget")
        .arg("--progress")
        .arg("--no-gitignore")
        .assert()
        .success()
        .stderr(contains("↳"));

    let atlas_dir = root.join(".atlas");
    for f in [
        "components.yaml",
        "external-components.yaml",
        "related-components.yaml",
        "llm-cache.json",
    ] {
        assert!(atlas_dir.join(f).exists(), "{f} should exist after index");
    }
}
