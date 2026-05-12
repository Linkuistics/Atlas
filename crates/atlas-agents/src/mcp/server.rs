//! `McpServer` — multi-client MCP stdio dispatch loop.
//!
//! Multi-client isolation is structural: the server holds an
//! `Arc<HashMap<id, ToolHandle>>` shared read-only across all clients;
//! each client's `serve_client` task owns its own reader/writer pair
//! and a per-task `client_id`. JSON-RPC `id` values from one client
//! never reach another.
//!
//! Spawning model (PR-4 will wire this): the default
//! `claude_code` + `codex` pairing spawns two `serve_client` tasks,
//! one per subprocess, against `tokio::io::duplex` streams piped to
//! the subprocess stdin/stdout. Hosting a remote MCP transport
//! (HTTP/SSE) is out of scope for PR-1 — Atlas's MCP is in-process by
//! design (recast §5.5: "no external surfaces").

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::descriptors::tools_list_response;
use super::{error_codes, ClientId, JsonRpcRequest, JsonRpcResponse};
use crate::tool::{ToolArgs, ToolContext, ToolHandle};

/// MCP protocol version Atlas's server speaks. Bumped when the
/// upstream protocol or our dispatch semantics change.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// `McpServer` holds a shared, read-only tool catalog and a per-server
/// `ToolContext`. Instances are wrapped in `Arc` and shared across
/// every concurrent client task.
///
/// PR-4 added a per-client transcript recorder: every successful
/// `tools/call` dispatch appends a record to a per-client vector,
/// drained by `drain_client_transcript`. The agent runtime's MCP
/// tool-loop (`crate::runtime::tool_loop_mcp`) uses this to merge the
/// subprocess-driven tool-use traffic into the transcript-cache blob.
/// Recording is best-effort — a poisoned mutex degrades to empty
/// drains rather than panicking (the runtime call site still succeeds).
pub struct McpServer {
    tools: HashMap<&'static str, ToolHandle>,
    ctx: ToolContext,
    /// Per-client recorder. Keyed by `ClientId`; values are the
    /// accumulated `tool_use` / `tool_result` records since the last
    /// `drain_client_transcript`. Behind `Mutex` rather than
    /// `RwLock` because every dispatch path needs write access; the
    /// hold-time is bounded by one Value::clone per call.
    transcript: Mutex<HashMap<ClientId, Vec<Value>>>,
}

impl McpServer {
    /// Build a server from a tool list and a shared context. Duplicate
    /// `Tool::id()` values cause a later one to win silently — callers
    /// are expected to register from a single source of truth (the
    /// catalog assembled in PR-3 and wired in PR-7).
    pub fn new(tools: Vec<ToolHandle>, ctx: ToolContext) -> Self {
        let map = tools.into_iter().map(|t| (t.id(), t)).collect();
        Self {
            tools: map,
            ctx,
            transcript: Mutex::new(HashMap::new()),
        }
    }

    /// How many tools are registered. Useful for tests and diagnostics.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Drain (and clear) the per-client transcript recorder. Returns
    /// the records accumulated since the last drain — one entry per
    /// successful `tools/call` dispatch the client issued.
    ///
    /// PR-4 entry point for `crate::runtime::tool_loop_mcp`. The
    /// runtime calls this once after `backend.call_async` returns, so
    /// every tool call the subprocess emitted during its agent loop
    /// lands in the transcript-cache blob.
    ///
    /// Returns an empty vector if `client_id` has no recorded events
    /// (the typical "subprocess returned immediately with the final
    /// answer" case). A poisoned mutex also yields the empty result
    /// rather than panicking.
    pub fn drain_client_transcript(&self, client_id: ClientId) -> Vec<Value> {
        match self.transcript.lock() {
            Ok(mut guard) => guard.remove(&client_id).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Append one transcript record for `client_id`. Best-effort —
    /// a poisoned mutex is logged via `tracing` but does not abort
    /// the dispatch (we'd rather replay an incomplete transcript than
    /// drop a successful tool call on the floor).
    fn record(&self, client_id: ClientId, event: Value) {
        match self.transcript.lock() {
            Ok(mut guard) => guard.entry(client_id).or_default().push(event),
            Err(_) => tracing::warn!(?client_id, "mcp transcript mutex poisoned; dropping event"),
        }
    }

    /// Drive one client's dispatch loop until EOF on `reader`. Each
    /// JSON-RPC request is read as a single newline-terminated line,
    /// dispatched against the tool catalog, and the response written
    /// back to `writer` on its own line.
    ///
    /// The loop terminates cleanly on EOF (the typical case: the
    /// subprocess client closed its stdin/stdout) or with `Err` on a
    /// transport-level I/O failure.
    pub async fn serve_client<R, W>(
        self: Arc<Self>,
        client_id: ClientId,
        reader: R,
        mut writer: W,
    ) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(()); // EOF
            }
            // Tolerate blank keep-alive lines.
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
                Ok(req) => self.handle_request(client_id, req).await,
                Err(e) => JsonRpcResponse::err(
                    Value::Null,
                    error_codes::INVALID_PARAMS,
                    format!("malformed JSON-RPC request: {e}"),
                ),
            };
            let mut bytes = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
            bytes.push(b'\n');
            writer.write_all(&bytes).await?;
            writer.flush().await?;
        }
    }

    async fn handle_request(&self, client_id: ClientId, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone();
        match req.method.as_str() {
            "initialize" => JsonRpcResponse::ok(
                id,
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "atlas-agents",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ),
            "tools/list" => {
                let tools: Vec<ToolHandle> = self.tools.values().cloned().collect();
                JsonRpcResponse::ok(id, tools_list_response(&tools))
            }
            "tools/call" => self.dispatch_tool_call(id, client_id, req.params).await,
            other => JsonRpcResponse::err(
                id,
                error_codes::METHOD_NOT_FOUND,
                format!("method `{other}` not supported by atlas-agents MCP server"),
            ),
        }
    }

    async fn dispatch_tool_call(
        &self,
        id: Value,
        client_id: ClientId,
        params: Option<Value>,
    ) -> JsonRpcResponse {
        let params = match params {
            Some(p) => p,
            None => {
                return JsonRpcResponse::err(
                    id,
                    error_codes::INVALID_PARAMS,
                    "tools/call requires params",
                );
            }
        };
        let name = match params.get("name").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => {
                return JsonRpcResponse::err(
                    id,
                    error_codes::INVALID_PARAMS,
                    "tools/call params.name (string) is required",
                );
            }
        };
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let tool = match self.tools.get(name.as_str()) {
            Some(t) => t.clone(),
            None => {
                return JsonRpcResponse::err(
                    id,
                    error_codes::METHOD_NOT_FOUND,
                    format!("tool `{name}` is not registered"),
                );
            }
        };
        match tool.invoke(ToolArgs(arguments.clone()), &self.ctx).await {
            Ok(result) => {
                // PR-4: record per-client transcript so the agent
                // runtime's MCP tool-loop can drain it post-call.
                self.record(
                    client_id,
                    json!({
                        "kind": "tool_call",
                        "tool_name": name,
                        "args": arguments,
                        "output": result.output,
                        "bytes": result.bytes,
                    }),
                );
                JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{"type": "json", "json": result.output}],
                        "isError": false,
                    }),
                )
            }
            Err(err) => {
                let err_text = err.to_string();
                self.record(
                    client_id,
                    json!({
                        "kind": "tool_error",
                        "tool_name": name,
                        "args": arguments,
                        "error": err_text,
                    }),
                );
                JsonRpcResponse::err(
                    id,
                    error_codes::TOOL_INVOCATION_FAILED,
                    format!("tool `{name}` failed: {err}"),
                )
            }
        }
    }
}
