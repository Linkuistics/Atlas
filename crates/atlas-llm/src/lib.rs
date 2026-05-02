//! LLM backend trait and concrete implementations (claude-code
//! subprocess; in-memory test backend).
//!
//! # Conceptual model
//!
//! The engine treats every LLM invocation as a pure function of
//! `(prompt_template, inputs, schema)` plus a run-wide
//! [`LlmFingerprint`]. For Salsa-based memoisation to be sound, a
//! backend must obey the following invariants:
//!
//! 1. **Determinism given fingerprint + inputs.** For a fixed
//!    [`LlmFingerprint`] and fixed [`LlmRequest`], `call` must yield
//!    the same JSON value (modulo permitted non-semantic
//!    normalisation). Stochastic sampling belongs behind a cache, not
//!    behind the trait.
//! 2. **Fingerprint stability across calls.** Two consecutive
//!    `fingerprint()` calls on the same backend must return equal
//!    values. Model identity, backend version, and the ontology /
//!    template inputs that shape responses all belong in the
//!    fingerprint — Salsa treats a change in the fingerprint as an
//!    invalidation of every cached LLM response.
//! 3. **Pure inputs.** The backend may not read from environment,
//!    disk, or network sources outside those reflected in the
//!    fingerprint. If it must (e.g. a subprocess), those sources must
//!    not vary within a run, and their identity must be captured in
//!    the fingerprint.
//!
//! See `README.md` for instructions on adding a new backend.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod agent_observer;
pub mod budget;
pub mod claude_code;
pub mod codex;
pub mod config;
pub mod http_anthropic;
pub mod http_openai;
pub mod prompt;
pub mod router;
pub(crate) mod stream_parse;
pub mod test_backend;

pub use agent_observer::{AgentEvent, AgentObserver};
pub use budget::{default_token_estimator, BudgetedBackend, TokenCounter, TokenEstimator};
pub use claude_code::{ClaudeCodeBackend, DEFAULT_MODEL_ID, MODEL_ID_ENV};
pub use codex::CodexBackend;
pub use config::{AtlasConfig, ConfigError, OperationConfig, OperationsConfig, ProviderConfig};
pub use http_anthropic::AnthropicHttpBackend;
pub use http_openai::OpenAiHttpBackend;
pub use router::BackendRouter;
pub use test_backend::TestBackend;

/// Identifier for one of Atlas's built-in prompt templates. The
/// engine refers to prompts by id rather than path so that a backend
/// is free to bundle templates statically or load them from disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptId {
    Classify,
    Subcarve,
    Stage1Surface,
    Stage2Edges,
}

/// JSON Schema document (as JSON) describing the expected shape of a
/// backend's response. Backends that do response validation (e.g.
/// [`claude_code::ClaudeCodeBackend`]) use this to reject malformed
/// LLM output; the [`TestBackend`] treats it as opaque.
#[derive(Debug, Clone)]
pub struct ResponseSchema(pub Value);

impl ResponseSchema {
    /// A schema that accepts any JSON value. Useful for test-backend
    /// fixtures where the test author owns the canned response shape.
    pub fn accept_any() -> Self {
        ResponseSchema(Value::Object(serde_json::Map::new()))
    }
}

/// One engine-issued request to a backend.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub prompt_template: PromptId,
    pub inputs: Value,
    pub schema: ResponseSchema,
}

/// Backend-identity fingerprint fed into the memoisation key of every
/// LLM-derived Salsa query. A change in any field invalidates every
/// cached response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmFingerprint {
    /// SHA-256 over the bundle of rendered prompt templates the
    /// backend will issue. Computed on the *rendered* output (after
    /// ontology substitution), not the raw template file, so that a
    /// change in `ontology.yaml` propagates here even when the raw
    /// template bytes are unchanged.
    pub template_sha: [u8; 32],
    /// SHA-256 over the component ontology the backend was
    /// constructed with.
    pub ontology_sha: [u8; 32],
    /// Model identifier (e.g. `"claude-sonnet-4-6"`). Opaque to the
    /// engine; compared as a whole string.
    pub model_id: String,
    /// Backend-reported version string (e.g. `claude --version`
    /// output). Bumps when the on-disk tool changes.
    pub backend_version: String,
}

/// The trait implemented by every LLM backend wired into the engine.
/// Implementations must satisfy the invariants listed at the module
/// level — otherwise Salsa-backed memoisation becomes unsound.
pub trait LlmBackend: Send + Sync {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError>;
    fn fingerprint(&self) -> LlmFingerprint;
}

/// Errors surfaced by the `atlas-llm` crate. Backends return
/// variants corresponding to the failure mode; the engine matches on
/// [`LlmError::BudgetExhausted`] to honour the fail-loud budget
/// semantics (§7.4).
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("template syntax error: {0}")]
    TemplateSyntax(String),

    #[error("backend invocation failed: {0}")]
    Invocation(String),

    #[error("backend returned non-JSON output: {0}")]
    Parse(String),

    #[error("backend response failed schema validation: {0}")]
    Schema(String),

    #[error("test backend has no canned response: {0}")]
    TestBackendMiss(String),

    #[error("budget exhausted: requested {requested} tokens, remaining {remaining}")]
    BudgetExhausted { requested: u64, remaining: u64 },

    #[error("backend setup failed: {0}")]
    Setup(String),
}
