//! In-process MCP stdio server backed by `rmcp` (Rust MCP SDK).
//!
//! `McpServer` hosts an Atlas tool catalog; each subprocess client gets
//! its own `serve_client` Tokio task wired to a duplex/process-pipe
//! `(AsyncRead, AsyncWrite)` pair via `rmcp::serve_server`. Multi-client
//! multiplexing is structural: one `Arc<McpServer>` shared across all
//! clients; each `serve_client` invocation builds an `AtlasHandler`
//! snapshot keyed by `ClientId` so per-client transcript recording can
//! demultiplex concurrent traffic.
//!
//! Pre-PR-A this module hand-rolled the JSON-RPC framing + dispatch
//! loop. PR-A migrated to the maintained `rmcp` crate per memory
//! `feedback_prefer_existing_crates`; framing, lifecycle enforcement
//! (initialize-first handshake), error codes, and standard MCP method
//! dispatch are now delegated.

pub mod serve_client;
pub mod server;

/// Per-client identifier, stamped onto every dispatched tool call so
/// the transcript-cache lane (PR-2) and audit lane can demultiplex
/// concurrent clients sharing one `McpServer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

impl ClientId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}
