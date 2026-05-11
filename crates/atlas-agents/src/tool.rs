//! The unified `Tool` trait that the LLM-spine runtime drives.
//!
//! Recast spec §5.4 (single-trait sourcing): every filesystem-touching
//! capability the model exercises — Read, Grep, Bash, Tree-sitter
//! parses, structured probes — flows through one `Tool` impl. The MCP
//! server (`crate::mcp`) re-exposes these impls to subprocess backends
//! over stdio JSON-RPC; the HTTP tool-loop (lands in PR-4) drives them
//! in-process.

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// JSON Schema fragment describing a `Tool`'s args object, paired with
/// a model-facing description. Doubles as the MCP `inputSchema` field
/// and the HTTP tool-use API's `parameters` block.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    /// JSON Schema describing the args object. Doubles as the MCP
    /// `inputSchema` field and the HTTP tool-use API's `parameters`.
    pub args_schema: Value,
    /// Human-readable description shown to the LLM.
    pub description: String,
}

/// Caller-supplied arguments to a `Tool::invoke` call. Wraps a JSON
/// `Value` so the trait surface is sourcing-agnostic — MCP and the
/// HTTP tool-loop both speak JSON natively.
#[derive(Debug, Clone)]
pub struct ToolArgs(pub Value);

/// The successful result of a `Tool::invoke` call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// JSON-serialisable output. Returned verbatim to the LLM via MCP
    /// `content` or the HTTP `tool_result` block.
    pub output: Value,
    /// Bytes emitted by this call, for transcript-cache accounting.
    pub bytes: u64,
}

/// Errors a `Tool::invoke` may surface. The runtime maps these into
/// MCP `error` responses or HTTP `tool_result` blocks with
/// `is_error: true`; budget-failures are *not* a tool error — they
/// abort the run via `atlas_llm::LlmError::BudgetExhausted`.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("tool execution failed: {0}")]
    Invocation(String),
}

/// One filesystem input read by a tool invocation, recorded so the
/// transcript cache (recast §6.3) can spot-check replays. PR-2 wires
/// this into the cache; PR-1 only defines the shape.
#[derive(Debug, Clone)]
pub struct FingerprintInput {
    /// Workspace-relative path the tool read.
    pub path: PathBuf,
    /// SHA-256 of the file's bytes at the moment of the read.
    pub sha: [u8; 32],
}

/// Per-invocation context the runtime hands a tool. The workspace root
/// is sufficient for PR-1; PR-2 extends this with cache handles and an
/// event-bus emitter.
#[derive(Clone)]
pub struct ToolContext {
    /// Absolute path to the workspace root the tool may read. Tools
    /// must reject paths that escape this root.
    pub workspace_root: PathBuf,
    // Cache handles, event-bus emitter, etc. land in PR-2.
}

/// The single trait every Atlas-side capability implements. The agent
/// runtime drives `invoke` directly (HTTP tool loop) or via the MCP
/// stdio server (subprocess backends).
///
/// Implementations must be deterministic given `(args, filesystem
/// snapshot)` — the transcript cache replays tool calls against the
/// stored output, and a non-deterministic tool would silently corrupt
/// future runs.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Stable identifier, e.g. `"read"`, `"grep"`, `"tree_sitter.parse"`.
    /// Must be ASCII, lower-snake-case, and globally unique across the
    /// registered tool catalog (recast §5.4: single-trait sourcing).
    fn id(&self) -> &'static str;

    /// Semver-shaped version, bumped when args or output shape changes.
    /// Folded into the transcript-cache fingerprint by PR-2 so a tool
    /// schema change invalidates replays.
    fn version(&self) -> &'static str;

    /// JSON Schema for `args`, plus a model-facing description. The MCP
    /// server (`crate::mcp::descriptors`) lifts this into the
    /// `tools/list` reply; the HTTP tool-loop (PR-4) lifts it into the
    /// `parameters` block of the upstream tool-use payload.
    fn json_schema(&self) -> &ToolSchema;

    /// Run the tool. The implementation is responsible for all I/O —
    /// the runtime supplies `ctx` and forwards `args` verbatim.
    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError>;

    /// What filesystem inputs did this tool invocation read? Returned
    /// before `invoke` runs (so the runtime can pre-compute SHAs) and
    /// stored alongside the cached transcript. Replays re-check each
    /// SHA against the current file and evict on mismatch (recast §6.3).
    fn fingerprint_inputs(&self, args: &ToolArgs) -> Vec<FingerprintInput>;
}

/// Reference-counted, type-erased `Tool` handle. The MCP server and
/// the HTTP tool-loop hold the same `Arc<dyn Tool>` so a registered
/// tool serves both surfaces from one instance.
pub type ToolHandle = Arc<dyn Tool>;
