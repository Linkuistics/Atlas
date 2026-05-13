//! `McpServer` — multi-client MCP stdio dispatch on top of `rmcp`.
//!
//! Multi-client isolation is structural: the server holds an
//! `Arc<HashMap<id, ToolHandle>>` shared read-only across all clients;
//! each client's `serve_client` task owns its own transport (built from
//! a duplex/process-pipe `(AsyncRead, AsyncWrite)` pair) and an
//! `AtlasHandler` snapshot stamped with its `ClientId`. JSON-RPC `id`
//! values from one client never reach another — rmcp's per-transport
//! `serve_server` invocation services them on disjoint Tokio tasks.
//!
//! Per-client transcript recording survives the migration: every
//! successful `tools/call` dispatch appends a record to a per-
//! `ClientId` vector behind a `Mutex`, drained by
//! `drain_client_transcript`. The agent runtime's MCP tool-loop
//! (`crate::runtime::tool_loop_mcp`) uses this to merge the
//! subprocess-driven tool-use traffic into the transcript-cache blob.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorCode, ErrorData as McpError, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool as McpTool,
};
use rmcp::serve_server;
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use super::ClientId;
use crate::tool::{ToolArgs, ToolContext, ToolHandle};

/// `McpServer` holds a shared, read-only tool catalog and a per-server
/// `ToolContext`. Instances are wrapped in `Arc` and shared across
/// every concurrent client task.
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
    /// are expected to register from a single source of truth.
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

    /// Drive one client's MCP service to completion. The
    /// `(reader, writer)` pair is wrapped in `rmcp`'s
    /// `AsyncRwTransport` (newline-delimited JSON-RPC framing) and
    /// handed to `rmcp::serve_server`, which performs the initialize
    /// handshake then dispatches `tools/list` + `tools/call` requests
    /// against the per-client `AtlasHandler` snapshot until the
    /// transport closes.
    pub async fn serve_client<R, W>(
        self: Arc<Self>,
        client_id: ClientId,
        reader: R,
        writer: W,
    ) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let handler = AtlasHandler {
            server: Arc::clone(&self),
            client_id,
        };
        let service = serve_server(handler, (reader, writer))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        service
            .waiting()
            .await
            .map(|_quit_reason| ())
            .map_err(|join_err| std::io::Error::other(join_err.to_string()))
    }
}

/// Per-client `rmcp::ServerHandler` snapshot — clones cheaply
/// (`Arc<McpServer>` clone + a 64-bit `ClientId`).
#[derive(Clone)]
struct AtlasHandler {
    server: Arc<McpServer>,
    client_id: ClientId,
}

impl ServerHandler for AtlasHandler {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools: Vec<McpTool> = self
            .server
            .tools
            .values()
            .map(|t| {
                let schema = t.json_schema();
                let input_schema_obj = schema.args_schema.as_object().cloned().unwrap_or_default();
                let mut tool = McpTool::default();
                tool.name = Cow::Borrowed(t.id());
                tool.description = Some(Cow::Owned(schema.description.clone()));
                tool.input_schema = Arc::new(input_schema_obj);
                tool
            })
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let name = request.name.clone();
        let arguments_value = match request.arguments {
            Some(map) => Value::Object(map),
            None => Value::Null,
        };
        let tool_opt = self.server.tools.get(name.as_ref()).cloned();
        async move {
            let Some(tool) = tool_opt else {
                return Err(McpError::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    format!("tool `{name}` is not registered"),
                    None,
                ));
            };
            match tool
                .invoke(ToolArgs(arguments_value.clone()), &self.server.ctx)
                .await
            {
                Ok(result) => {
                    self.server.record(
                        self.client_id,
                        json!({
                            "kind": "tool_call",
                            "tool_name": name.as_ref(),
                            "args": arguments_value,
                            "output": result.output,
                            "bytes": result.bytes,
                        }),
                    );
                    Ok(CallToolResult::structured(result.output))
                }
                Err(err) => {
                    let err_text = err.to_string();
                    self.server.record(
                        self.client_id,
                        json!({
                            "kind": "tool_error",
                            "tool_name": name.as_ref(),
                            "args": arguments_value,
                            "error": err_text,
                        }),
                    );
                    Err(McpError::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("tool `{name}` failed: {err}"),
                        None,
                    ))
                }
            }
        }
    }

    fn get_info(&self) -> InitializeResult {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities).with_server_info(Implementation::new(
            "atlas-agents",
            env!("CARGO_PKG_VERSION"),
        ))
    }
}
