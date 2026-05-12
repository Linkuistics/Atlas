//! MCP-side tool-loop observation (plan §4 Task 4.4).
//!
//! Subprocess backends (`claude_code`, `codex`) drive their own
//! tool-use loop internally — they receive the MCP server's tool
//! catalog via the standard MCP handshake and choose when to call.
//! Atlas's runtime does NOT drive the inner loop; it observes via the
//! in-process MCP server, which records every dispatched tool call to
//! a per-client recorder.
//!
//! PR-4 wires the observation: after `backend.call_async` returns,
//! `run_tool_loop_mcp` drains the per-client recorder via
//! `mcp_server.drain_client_transcript(client_id)` and merges the
//! records into the `Transcript` that ultimately gets framed for the
//! transcript cache.

use atlas_llm::{LlmBackend, LlmRequest, PromptId, ResponseSchema};
use serde_json::json;

use crate::mcp::server::McpServer;
use crate::mcp::ClientId;
use crate::runtime::audit::{AgentOutput, Stage};
use crate::runtime::semaphores::Semaphores;
use crate::runtime::tool_loop_http::{parse_final_output, Transcript};

use super::AgentError;

/// Drive the MCP-side observation path: one `backend.call_async`
/// invocation against the subprocess, then drain the MCP server's
/// per-client transcript recorder into `transcript`.
///
/// `client_id` is the id the runtime used when spawning the
/// subprocess's MCP `serve_client` task. The runtime owns the
/// lifecycle — this function only observes.
pub async fn run_tool_loop_mcp(
    backend: &dyn LlmBackend,
    mcp_server: &McpServer,
    client_id: ClientId,
    semaphores: &Semaphores,
    stage: Stage,
    initial_prompt: String,
    transcript: &mut Transcript,
) -> Result<AgentOutput, AgentError> {
    let _stage_permit = semaphores.acquire_stage(stage).await;
    let req = build_llm_request_subprocess(initial_prompt);
    let response = backend
        .call_async(&req)
        .await
        .map_err(AgentError::from_llm_error)?;
    transcript.record_assistant_turn(&response);

    let recorded = mcp_server.drain_client_transcript(client_id);
    transcript.merge_mcp_events(recorded);

    Ok(parse_final_output(&response))
}

/// Build the per-call `LlmRequest` for a subprocess backend. The
/// subprocess speaks its own tool-use protocol internally, so the
/// payload is just the initial prompt — no tool descriptors. The
/// subprocess discovers the tool catalog via the MCP handshake the
/// runtime set up out-of-band.
fn build_llm_request_subprocess(initial_prompt: String) -> LlmRequest {
    LlmRequest {
        // PR-4 routes the subprocess agent's call through the
        // Classify table entry; PR-5 will introduce a dedicated
        // PromptId variant for the multi-step agent path.
        prompt_template: PromptId::Classify,
        inputs: json!({ "conversation": initial_prompt }),
        schema: ResponseSchema::accept_any(),
    }
}
