//! Tool wrappers for the LLM-spine runtime (Phase 7).
//!
//! Each sub-module contains `Tool` implementations that wrap existing
//! analysers and parsers as pure pass-throughs. No analysis logic lives
//! here — all decisions come from the underlying analyser crates. The
//! runtime that drives these (per recast spec §5.4) lands in PR-4.
//!
//! # Sub-modules
//!
//! - `classifiers` — L3 classifier wrappers (Cargo, Compose, C#, Dart,
//!   Dockerfile, Elixir, LispKit, Python, Racket, TS/JS).
//! - `manifests` — manifest-parser wrappers (Cargo.toml, package.json,
//!   Dockerfile, docker-compose) exposing the existing parsers as `Tool`s.
//! - `surfaces` — L5 surface-analyser wrappers; in-process for Rust/TS-JS,
//!   subprocess for the rest (C#, Dart, Elixir, LispKit, Python, Racket).

pub mod classifiers;
pub mod manifests;
pub mod surfaces;
