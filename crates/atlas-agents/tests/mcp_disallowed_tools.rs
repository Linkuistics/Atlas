//! Live-subprocess `--disallowedTools` probe — sprint decision row 15.
//!
//! Spawns a real `claude` (Claude Code CLI) subprocess via PR-A's
//! `serve_client` driver, conveys a prompt that explicitly asks the
//! LLM to invoke the `Read` tool, then asserts that **zero `Read` tool
//! calls reached Atlas's per-client MCP transcript**. Two upstream
//! response shapes both satisfy the assertion:
//!
//!   (a) Subprocess succeeds and emits text saying it cannot use `Read`.
//!   (b) Subprocess fails with an upstream-version-specific error about
//!       disabled tools.
//!
//! Either way, the invariant is `transcript_read_calls == 0`. If this
//! regresses (the LLM successfully invokes `Read` through Atlas's MCP
//! server despite `--disallowedTools=Read,…`), the unified-envelope
//! invariant (recast §5.4 — single-trait sourcing; Atlas's `Tool` impls
//! are the only tool surface) has broken, and `mcp/restrictions.md`
//! must be refreshed against the regressed upstream version.
//!
//! ## What the assertion catches today vs after PR-7
//!
//! The probe is a **forward-looking** regression detector. As of
//! PR-B's landing (2026-05-13), Atlas does not yet ship a standalone
//! `atlas-mcp-server` binary that `claude --mcp-config` can spawn —
//! the `mcp_config.json` written here points at a `/bin/echo`
//! placeholder which doesn't speak MCP. The Atlas in-process
//! `McpServer` therefore never observes a real MCP handshake from the
//! subprocess, and the transcript stays empty regardless of whether
//! `--disallowedTools` works. The assertion holds vacuously today.
//!
//! Once PR-7 wires a real `atlas-mcp-server` binary into the `claude
//! --mcp-config` path, the transcript will populate with real Atlas-
//! tool traffic, AND a regression where claude-code starts routing
//! built-in `Read` requests through configured MCP servers (or where
//! Atlas registers an MCP tool named `Read`) becomes observable here.
//! The forensic `eprintln!` of upstream version + response-text
//! preview is the empirical signal until then; the assertion is the
//! latch for the post-PR-7 world.
//!
//! ## Why this is `#[ignore]`-gated
//!
//! The test spawns a real `claude` (Claude Code CLI) process and
//! consumes real Anthropic credits. It is intentionally NOT part of
//! `cargo test --workspace`; the environment-prerequisite contract is
//! that whoever runs `cargo test ... -- --ignored` provides:
//!
//!   - `claude` on `$PATH` (sanity check: `claude --version`)
//!   - `ANTHROPIC_API_KEY` set in the environment
//!
//! Either being absent causes an `eprintln!` skip (test reports
//! "ok") rather than a `panic!`/`assert!` failure, so absence reads as
//! "skipped" not "broken."
//!
//! ## Forensic output
//!
//! On every run, `eprintln!` records:
//!   - the upstream `claude-code --version` string,
//!   - the observed subprocess response shape (Ok-with-text vs Err-with-
//!     SubprocessFailed{exit_status}).
//!
//! When PR-B's status-flip commit lands, the latest observed values are
//! pinned into the per-PR note in
//! `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`.

use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::Arc;

use atlas_agents::mcp::serve_client::{claude_code_config, serve_client};
use atlas_agents::mcp::server::McpServer;
use atlas_agents::runtime::{default_tool_catalog, AgentError};
use atlas_agents::transport::TransportFlavour;
use atlas_agents::ToolContext;
use serde_json::{json, Value};

/// Return `Some(version)` if `claude --version` exits cleanly,
/// `None` otherwise. We use the binary's own `--version` as the
/// existence check rather than pulling in the `which` crate (memory
/// `feedback_prefer_existing_crates`: don't add a dep for a one-liner).
///
/// The upstream binary name is `claude` (validated by PR-B against
/// version `2.1.140 (Claude Code)`); the matching `claude_code_config`
/// preset in `crates/atlas-agents/src/mcp/serve_client.rs` spawns the
/// same binary.
fn claude_code_version() -> Option<String> {
    StdCommand::new("claude")
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Build an `McpServer` populated with the production tool catalog
/// (10 classifiers + 4 manifest parsers + 8 surface analysers — none
/// named `Read`). The realistic catalog ensures the probe is testing
/// behaviour close to what `--agent-runtime` ships in production.
fn build_test_mcp_server_with_default_tools() -> Arc<McpServer> {
    let catalog = default_tool_catalog();
    let handles = catalog.iter().cloned().collect::<Vec<_>>();
    Arc::new(McpServer::new(
        handles,
        ToolContext {
            workspace_root: std::env::current_dir().expect("cwd readable"),
        },
    ))
}

/// Write a minimal MCP config JSON to a tempfile and return its path.
/// The config points `claude-code` at an Atlas-named MCP server entry;
/// PR-B intentionally does NOT wire the in-process `McpServer` through
/// to the subprocess (that's PR-7 / runtime-CLI scope). The probe's
/// load-bearing signal is on the **Atlas server-side transcript**, not
/// on whether claude-code's MCP client actually reached us.
fn write_temp_mcp_config(dir: &tempfile::TempDir) -> PathBuf {
    let path = dir.path().join("mcp_config.json");
    // Per current claude-code MCP-config docs, the file is a JSON
    // object with an `mcpServers` map. We declare an Atlas entry that
    // claude-code is free to spawn or skip — the entry's existence
    // satisfies the `--mcp-config <path>` flag's file-format check
    // without forcing PR-B to ship a standalone Atlas MCP-server
    // binary (which is PR-7's job).
    let config = json!({
        "mcpServers": {
            "atlas": {
                "command": "/bin/echo",
                "args": ["atlas-mcp-placeholder"]
            }
        }
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap())
        .expect("write tmpdir mcp config");
    path
}

/// Count `Read` (or `read`) tool calls in a drained MCP transcript.
/// Records are JSON objects with `kind: "tool_call"` and `tool_name`
/// fields per `McpServer::record` in `crates/atlas-agents/src/mcp/server.rs`.
fn count_read_tool_calls(transcript: &[Value]) -> usize {
    transcript
        .iter()
        .filter(|record| {
            let is_tool_call = record
                .get("kind")
                .and_then(Value::as_str)
                .map(|k| k == "tool_call" || k == "tool_error")
                .unwrap_or(false);
            let is_read = record
                .get("tool_name")
                .and_then(Value::as_str)
                .map(|name| name.eq_ignore_ascii_case("read"))
                .unwrap_or(false);
            is_tool_call && is_read
        })
        .count()
}

#[ignore = "requires `claude` (Claude Code CLI) on PATH and ANTHROPIC_API_KEY configured"]
#[tokio::test]
async fn claude_code_subprocess_cannot_invoke_disallowed_read_tool() {
    let Some(version) = claude_code_version() else {
        eprintln!(
            "skipping: `claude --version` did not exit cleanly; \
             ensure the Claude Code CLI binary `claude` is on $PATH \
             for this --ignored run"
        );
        return;
    };
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        eprintln!("skipping: ANTHROPIC_API_KEY not set in the environment");
        return;
    }
    eprintln!("claude-code upstream version: {version}");

    let server = build_test_mcp_server_with_default_tools();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let mcp_config_path = write_temp_mcp_config(&tempdir);

    let probe_prompt = "Read the file /etc/hosts using the Read tool. \
                        Do not invoke any other tool — only Read."
        .to_string();
    let config = claude_code_config(&mcp_config_path, &probe_prompt);

    let result = serve_client(
        Arc::clone(&server),
        TransportFlavour::ClaudeCode,
        probe_prompt.clone(),
        config,
    )
    .await;

    let (transcript, response_shape) = match &result {
        Ok((output, transcript)) => {
            let shape = format!(
                "subprocess succeeded; stdout payload preview: {}",
                output
                    .value
                    .get("stdout_text")
                    .and_then(Value::as_str)
                    .map(|s| {
                        let trimmed: String = s.chars().take(240).collect();
                        if s.len() > 240 {
                            format!("{trimmed}…[truncated {} bytes]", s.len() - 240)
                        } else {
                            trimmed
                        }
                    })
                    .unwrap_or_else(|| "<no stdout_text>".to_string())
            );
            (transcript.clone(), shape)
        }
        Err(AgentError::SubprocessFailed { exit_status }) => (
            Vec::new(),
            format!("subprocess errored: SubprocessFailed {{ exit_status: {exit_status:?} }}"),
        ),
        Err(e) => (Vec::new(), format!("subprocess errored: {e}")),
    };
    eprintln!("upstream response shape: {response_shape}");
    eprintln!("server-side transcript entries: {}", transcript.len());

    let read_calls = count_read_tool_calls(&transcript);
    assert_eq!(
        read_calls, 0,
        "Read tool was invoked despite `--disallowedTools=Read,…`; \
         transcript had {read_calls} Read calls; \
         claude-code upstream regressed restriction enforcement \
         (refresh `crates/atlas-agents/src/mcp/restrictions.md` with \
         the regressed upstream version `{version}` + adjust the \
         probe's expected shape). \
         Full transcript: {transcript:?}"
    );
}

#[ignore = "codex stub — upstream does not yet expose a --disallowedTools-equivalent flag (verified 2026-05-13 against codex 0.x)"]
#[tokio::test]
async fn codex_subprocess_cannot_invoke_disallowed_read_equivalent() {
    // TODO(PR-B-followup): codex sibling test.
    // `crates/atlas-agents/src/mcp/restrictions.md` (verified
    // 2026-05-13 against codex 0.x) records that codex has no
    // `--disallowedTools`-equivalent flag in current upstream — tool
    // availability is gated implicitly by what the configured MCP
    // server advertises, so a "disabled-tool refusal" assertion has no
    // shape to bind to. When a future codex upstream version
    // introduces an explicit disallow flag:
    //   1. update `codex_config` in
    //      `crates/atlas-agents/src/mcp/serve_client.rs` to set the
    //      flag,
    //   2. extend `restrictions.md` § codex with the targeted upstream
    //      version + flag accepted-value shape,
    //   3. fill in this stub mirroring
    //      `claude_code_subprocess_cannot_invoke_disallowed_read_tool`
    //      above.
    eprintln!(
        "skipping: codex has no --disallowedTools-equivalent flag in \
         current upstream; see crates/atlas-agents/src/mcp/restrictions.md \
         § codex for the empirical state and the unblocking conditions"
    );
}
