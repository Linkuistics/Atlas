//! Surface-analyser `Tool` wrappers — weak-tooling tier (PR-3c).
//!
//! Each wrapper delegates to the corresponding subprocess-backed
//! surface analyser proxy. When the binary is not present the wrapper
//! returns `ToolError::Invocation` with a helpful build hint.

pub mod elixir;
pub mod lispkit;
pub mod racket;
