//! Per-subprocess MCP `serve_client` driver. Spawns `claude-code` or
//! `codex` via `tokio::process::Command`, conveys the initial prompt
//! to the subprocess, waits for completion, drains the per-client MCP
//! transcript, and parses the final output.
//!
//! ## Channel-shape disposition (PR-A vs PR-B)
//!
//! Two channels are involved when Atlas drives a subprocess LLM agent:
//!
//! - **MCP wire** — JSON-RPC over stdio between Atlas (server) and the
//!   subprocess (client). Tool calls + responses + lifecycle.
//! - **Prompt + final-output** — the user prompt the agent operates on,
//!   plus its final answer.
//!
//! Whether these share `child.stdin`/`child.stdout` or sit on disjoint
//! channels (CLI arg for prompt; stdio for MCP) is upstream-version-
//! dependent for both `claude-code` and `codex`. PR-A ships the
//! structural spawn/wait/drain skeleton; PR-B (the
//! `--disallowedTools` live-subprocess probe) is the empirical
//! validation that pins down the exact wire shape per upstream
//! version. Until then this module exercises against POSIX `cat` /
//! `false` stubs that don't speak MCP — they verify the plumbing
//! (subprocess lifecycle, stdin write, exit-status propagation,
//! transcript drain), not the JSON-RPC wire itself.
//!
//! Plan §4 Task 6 Step A.4 is the canonical brief for this module.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::server::McpServer;
use super::ClientId;
use crate::runtime::audit::AgentOutput;
use crate::runtime::AgentError;
use crate::transport::TransportFlavour;

/// Configuration for spawning a subprocess MCP client. Production
/// configs (`claude_code_config`, `codex_config`) embed the agent
/// invocation contract; tests construct minimal configs against POSIX
/// stub binaries.
#[derive(Debug, Clone)]
pub struct SubprocessConfig {
    pub executable_path: PathBuf,
    pub subprocess_args: Vec<String>,
}

/// claude-code subprocess preset.
///
/// The `--mcp-config <path>` flag points the subprocess at an Atlas-
/// authored MCP config file declaring our in-process server. The
/// `--disallowedTools` flag disables claude-code's built-in tools per
/// recast §5.4 (single-trait sourcing — Atlas's `Tool` impls are the
/// only tools available; the unified envelope requires this).
///
/// **PR-B follow-up:** verify the exact prompt-passing flag against
/// the targeted claude-code upstream version. Today's shape uses
/// `--print <prompt>` which several recent versions accept; older
/// versions read from stdin in `claude --print` mode. Pin the version
/// in `restrictions.md` when PR-B validates against the live binary.
pub fn claude_code_config(mcp_config_path: &Path, prompt: &str) -> SubprocessConfig {
    SubprocessConfig {
        executable_path: "claude-code".into(),
        subprocess_args: vec![
            "--mcp-config".into(),
            mcp_config_path.to_string_lossy().into_owned(),
            "--disallowedTools".into(),
            "Read,Grep,Glob,Bash,Write,Edit".into(),
            "--print".into(),
            prompt.to_string(),
        ],
    }
}

/// codex subprocess preset.
///
/// **PR-B follow-up:** verify codex's actual flag set against the live
/// upstream. The flag names below are placeholders; PR-B fills them
/// in. The structural shape (executable + args list) is unchanged.
pub fn codex_config(mcp_config_path: &Path, prompt: &str) -> SubprocessConfig {
    SubprocessConfig {
        executable_path: "codex".into(),
        subprocess_args: vec![
            "--mcp-config".into(),
            mcp_config_path.to_string_lossy().into_owned(),
            // TODO(PR-B): verify codex's actual --disallowedTools-
            // equivalent flag set against the upstream version targeted
            // in `restrictions.md`. Until then, the live-subprocess probe
            // will reveal what's needed.
            "--prompt".into(),
            prompt.to_string(),
        ],
    }
}

/// Monotonic per-process ClientId generator. The first client gets
/// `ClientId(1)`; values are unique within one Atlas process lifetime
/// (sufficient for transcript-cache demultiplexing).
fn next_client_id() -> ClientId {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    ClientId(COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Drive one subprocess LLM agent invocation to completion. Spawns
/// the subprocess, conveys the prompt to it, waits for exit, drains
/// the per-client MCP transcript, returns the final output.
///
/// `_transport` selects between claude-code and codex semantics for
/// future wire-shape decisions; today the caller passes the matching
/// preset's `config` so this parameter is informational. PR-B may
/// branch on it.
pub async fn serve_client(
    server: Arc<McpServer>,
    _transport: TransportFlavour,
    initial_prompt: String,
    config: SubprocessConfig,
) -> Result<AgentOutput, AgentError> {
    let mut child = Command::new(&config.executable_path)
        .args(&config.subprocess_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(AgentError::SubprocessSpawn)?;

    let mut child_stdin = child.stdin.take().expect("stdin piped");
    let child_stdout = child.stdout.take().expect("stdout piped");
    let child_stderr = child.stderr.take().expect("stderr piped");

    let client_id = next_client_id();

    // PR-A structural cut: write the initial prompt onto child stdin
    // then close stdin. This causes stub subprocesses (POSIX `cat`) to
    // EOF cleanly. Production claude-code / codex receive the prompt
    // primarily via CLI args (see `claude_code_config`); the stdin
    // write here is a no-op for them in stdin-not-listened modes, or
    // a redundant nudge in stdin-listened modes. PR-B refines this
    // when it validates against the live subprocess.
    if !initial_prompt.is_empty() {
        child_stdin
            .write_all(initial_prompt.as_bytes())
            .await
            .map_err(AgentError::SubprocessWait)?;
        if !initial_prompt.ends_with('\n') {
            child_stdin
                .write_all(b"\n")
                .await
                .map_err(AgentError::SubprocessWait)?;
        }
    }
    drop(child_stdin);

    // Drain stdout + stderr concurrently. Once PR-B engages a real
    // MCP wire on stdio, the stdout drain will move into the MCP
    // server's serve_client task. For PR-A's stub-subprocess scope
    // the drain is for diagnostic capture only.
    let stdout_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut reader = child_stdout;
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut reader = child_stderr;
        let _ = reader.read_to_end(&mut buf).await;
        buf
    });

    let exit_status = child.wait().await.map_err(AgentError::SubprocessWait)?;

    let stdout_bytes = stdout_task.await.unwrap_or_default();
    let stderr_bytes = stderr_task.await.unwrap_or_default();

    if !exit_status.success() {
        tracing::warn!(
            ?exit_status,
            stderr = %String::from_utf8_lossy(&stderr_bytes),
            "subprocess MCP client exited non-zero"
        );
        return Err(AgentError::SubprocessFailed { exit_status });
    }

    // Drain the per-client MCP transcript. Empty under stub
    // subprocesses (cat, false); populated once PR-B wires real MCP
    // traffic through `server.serve_client`.
    let transcript = server.drain_client_transcript(client_id);

    parse_subprocess_final_output(&stdout_bytes, &transcript)
}

/// Parse the subprocess's final output. The production contract per
/// the production-prompt sprint is a fenced ```yaml block in the last
/// assistant message; for PR-A's structural cut we accept any
/// non-empty stdout. PR-B + the wider sprint replace this with the
/// canonical envelope-deserialisation logic.
fn parse_subprocess_final_output(
    stdout_bytes: &[u8],
    _transcript: &[serde_json::Value],
) -> Result<AgentOutput, AgentError> {
    if stdout_bytes.is_empty() {
        return Err(AgentError::NoFinalOutput);
    }
    let text = String::from_utf8_lossy(stdout_bytes).into_owned();
    Ok(AgentOutput::from_value(serde_json::json!({
        "_pr_a_placeholder": "subprocess stdout drained; envelope parsing is PR-B scope",
        "stdout_text": text,
    })))
}
