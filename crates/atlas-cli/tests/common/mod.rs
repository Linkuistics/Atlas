//! Shared test-only helpers for `atlas-cli` integration tests.
//!
//! Cargo treats every top-level `.rs` file under `tests/` as a separate
//! integration-test crate, but a file inside a sub-directory and declared
//! via `mod common;` from a top-level test is compiled into that test's
//! binary instead. That means this module is NOT an independent test
//! crate — it carries no `#[test]` functions and Cargo will not warn
//! "no tests in `common`".
//!
//! Add new sub-modules here as additional shared helpers emerge across
//! the integration-test suite.

#![allow(dead_code)]

pub mod sweep_support;
