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
//! - `runtime` — async agent runtime; populated by PR-4+.

pub mod mcp;
pub mod runtime;
pub mod tool;

pub use mcp::server::McpServer;
pub use tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolHandle, ToolResult, ToolSchema,
};
