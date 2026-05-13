//! `serve_client` exercised against POSIX stub subprocesses
//! (`cat` echoes stdin to stdout; `false` always exits 1). Verifies
//! stdio wiring + drain handshake + exit-status propagation without
//! needing real claude-code / codex upstreams. The MCP-wire-shape
//! validation against live subprocesses is PR-B's scope.

use std::sync::Arc;

use atlas_agents::mcp::serve_client::{serve_client, SubprocessConfig};
use atlas_agents::mcp::server::McpServer;
use atlas_agents::runtime::AgentError;
use atlas_agents::transport::TransportFlavour;
use atlas_agents::ToolContext;

fn build_empty_server() -> Arc<McpServer> {
    Arc::new(McpServer::new(
        vec![],
        ToolContext {
            workspace_root: std::env::current_dir().unwrap(),
        },
    ))
}

#[tokio::test]
async fn serve_client_with_cat_subprocess_drains_handshake() {
    let server = build_empty_server();
    let config = SubprocessConfig {
        executable_path: "cat".into(),
        subprocess_args: vec![],
    };
    let result = serve_client(
        server,
        TransportFlavour::ClaudeCode,
        "test prompt\n".to_string(),
        config,
    )
    .await;
    // `cat` echoes the prompt then exits cleanly when stdin closes.
    // Acceptable outcomes:
    //   - Ok(_) when the placeholder parse returns a structural
    //     payload from non-empty stdout (PR-A current path).
    //   - Err(NoFinalOutput) when stdout happens to be empty (e.g.
    //     under unusual scheduling).
    // Both signal the structural plumbing is intact.
    assert!(
        matches!(result, Err(AgentError::NoFinalOutput) | Ok(_)),
        "expected clean handshake outcome, got {result:?}"
    );
}

#[tokio::test]
async fn serve_client_propagates_subprocess_nonzero_exit() {
    let server = build_empty_server();
    let config = SubprocessConfig {
        executable_path: "false".into(),
        subprocess_args: vec![],
    };
    let result = serve_client(
        server,
        TransportFlavour::ClaudeCode,
        "ignored prompt".to_string(),
        config,
    )
    .await;
    assert!(
        matches!(result, Err(AgentError::SubprocessFailed { .. })),
        "expected SubprocessFailed for `false`, got {result:?}"
    );
}

#[tokio::test]
async fn serve_client_surfaces_spawn_failure_on_unknown_binary() {
    let server = build_empty_server();
    let config = SubprocessConfig {
        executable_path: "atlas-this-binary-definitely-does-not-exist".into(),
        subprocess_args: vec![],
    };
    let result = serve_client(
        server,
        TransportFlavour::ClaudeCode,
        "anything".to_string(),
        config,
    )
    .await;
    assert!(
        matches!(result, Err(AgentError::SubprocessSpawn(_))),
        "expected SubprocessSpawn for nonexistent binary, got {result:?}"
    );
}
