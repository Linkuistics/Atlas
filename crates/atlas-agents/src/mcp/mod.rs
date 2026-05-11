//! In-process MCP stdio server (recast §5.5). One `McpServer` hosts an
//! `Arc<ToolCatalog>`; each subprocess client gets its own
//! `serve_client` Tokio task that owns a `DuplexStream` or pipe pair.
//!
//! Wire format is JSON-RPC 2.0 framed as newline-delimited JSON on
//! stdin/stdout (the MCP convention). Multi-client multiplexing is
//! structural: `McpServer` is `Arc`-shared; per-client state is
//! restricted to the dispatch loop's local variables.

pub mod descriptors;
pub mod server;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Per-client identifier, stamped onto every dispatched tool call so
/// the transcript cache (PR-2) and audit lane can demultiplex
/// concurrent clients sharing one `McpServer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

impl ClientId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// JSON-RPC 2.0 request envelope. The MCP protocol uses the standard
/// JSON-RPC framing; `params` is method-specific.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response envelope. Exactly one of `result` / `error`
/// is set per the JSON-RPC spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object. Codes follow the spec: -32601 method not
/// found, -32602 invalid params, -32603 internal error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// JSON-RPC error codes used by the Atlas MCP server. Codes outside
/// the JSON-RPC reserved range (-32768..=-32000) are application-level.
pub mod error_codes {
    /// Method not found (per JSON-RPC 2.0 spec).
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params (per JSON-RPC 2.0 spec).
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error (per JSON-RPC 2.0 spec).
    pub const INTERNAL_ERROR: i32 = -32603;
    /// Application: tool dispatch returned `ToolError`.
    pub const TOOL_INVOCATION_FAILED: i32 = -32000;
}
