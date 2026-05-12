//! Pass-through `Tool` wrappers exposing existing analysers/parsers to
//! the LLM-spine agent runtime (recast spec §5.4).
//!
//! PR-3 ships pure pass-through impls; no new reasoning or behaviour.
//! The runtime that drives them lands in PR-4.

pub mod classifiers;
pub mod surfaces;
