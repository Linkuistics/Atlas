//! Atlas command-line driver, exposed both as a binary (`atlas`) and
//! as a library for integration tests and embedders.
//!
//! The `run_index` entry point is backend-agnostic — callers pass in
//! an already-constructed `Arc<dyn LlmBackend>`. The binary's `main`
//! builds a `ClaudeCodeBackend`, wraps it in `BudgetedBackend`, and
//! forwards. Tests build a `TestBackend` directly, skipping the
//! prompts-on-disk requirement.

pub mod backend;
pub mod cache_io;
pub mod pipeline;
pub mod prompts;
pub mod timestamp;

pub use pipeline::{run_index, IndexConfig, IndexError, IndexSummary, DEFAULT_OUTPUT_SUBDIR};
