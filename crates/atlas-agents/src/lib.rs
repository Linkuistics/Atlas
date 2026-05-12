//! LLM-spine agent runtime — async Tokio runtime that drives unified
//! `Tool` invocations across subprocess and HTTP backends. See
//! `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`
//! and `docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md`.
//!
//! # Crate layout
//!
//! - `tool` — the `Tool` trait, args/result/error types, fingerprint
//!   hooks.
//! - `mcp` — in-process MCP stdio server that re-exposes `Tool` impls
//!   to subprocess backends.
//! - `events` — runtime event bus (`EventBus`, `AgentEvent`, `Subscriber`).
//! - `transport` — `TransportFlavour` enum (cache-key contributor).
//! - `agent_cache_writer` — subscriber that materialises transcript-cache
//!   entries from `AgentComplete`. Lives here (not in `atlas-engine`)
//!   because the engine cannot depend on `atlas-agents` — but the
//!   subscriber drives a cache that lives in the engine. PR-4 wires
//!   it; PR-2 ships the shape.
//! - `runtime` — async agent runtime; populated by PR-4+.

pub mod agent_cache_writer;
pub mod events;
pub mod mcp;
pub mod runtime;
pub mod tool;
pub mod transport;

pub use events::{AgentEvent, CacheHitSource, EventBus, Grade, Subscriber};
pub use mcp::server::McpServer;
pub use tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolHandle, ToolResult, ToolSchema,
};
pub use transport::{Provider, TransportFlavour};
