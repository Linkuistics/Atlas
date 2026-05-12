//! Tool wrappers for the LLM-spine runtime (Phase 7).
//!
//! Each sub-module contains `Tool` implementations that wrap existing
//! analysers and parsers as pure pass-throughs. No analysis logic lives
//! here — all decisions come from the underlying analyser crates.
//!
//! # Sub-modules
//!
//! - `classifiers` — L3 classifier wrappers (Python, C#, Dart; Rust, TS/JS,
//!   and others live in sibling PR worktrees).
//! - `surfaces` — L5 subprocess surface-analyser wrappers (Python, C#, Dart;
//!   Rust and TS/JS live in the PR-3a worktree).

pub mod classifiers;
pub mod surfaces;
