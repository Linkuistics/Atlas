//! Atlas command-line driver, exposed both as a binary (`atlas`) and
//! as a library for integration tests and embedders.
//!
//! The `run_index` entry point is backend-agnostic — callers pass in
//! an already-constructed `Arc<dyn LlmBackend>`. The binary's `main`
//! builds a `ClaudeCodeBackend`, wraps it in `BudgetedBackend`, and
//! forwards. Tests build a `TestBackend` directly, skipping the
//! prompts-on-disk requirement.

pub mod backend;
pub mod cli_args;
pub mod jsonl_subscriber;
pub mod pipeline;
pub mod progress;
pub mod prompts;
pub mod reports;
pub mod timestamp;
pub mod validate;

pub use cli_args::{index_error_exit_code, IndexArgs};
pub use pipeline::{
    build_engine_database, resolve_component_dir, run_index, IndexConfig, IndexError, IndexSummary,
    DEFAULT_OUTPUT_SUBDIR,
};
