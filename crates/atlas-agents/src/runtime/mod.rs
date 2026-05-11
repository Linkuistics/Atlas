//! Async agent runtime. Populated by PR-4+.
//!
//! Layout pre-announced so dependent crates can `use atlas_agents::runtime`
//! without breaking builds during the Wave 1–3 rollout:
//!
//! - PR-4 lands:
//!   - `agent` — single-iteration agent driver
//!   - `dispatch` — request → backend.call_async fan-out
//!   - `tool_loop_http` — HTTP-side tool-use loop
//!   - `tool_loop_mcp` — MCP-side tool dispatch glue
//!   - `audit` — Lane A (audit-only) probes
//!
//! - PR-5 lands:
//!   - `fixedpoint_loop` — multi-iteration fixedpoint with Lane B
//!     replaying the deterministic engine where the LLM yields nothing
//!     new.
