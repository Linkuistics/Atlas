use serde_json::Value;

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};

/// Stub backend for OpenAI's Codex CLI tool.
///
/// **Status:** research complete; implementation pending.
///
/// **Approach:** subprocess driver, structurally parallel to
/// [`crate::ClaudeCodeBackend`]. Per-call shape is
/// `codex exec --json --skip-git-repo-check --ephemeral --sandbox read-only
///  --output-schema <tmp> --model <id> -- <prompt>`; parse JSONL on stdout
/// and pick the last `item.completed` event whose `item.type ==
/// "agent_message"` for the final payload.
///
/// See `docs/superpowers/specs/2026-05-02-codex-backend-research.md` for the
/// full findings, the rationale for subprocess-vs-HTTP-vs-`codex-core`, the
/// JSONL event schema, and the implementation sketch.
///
/// `call()` returns [`LlmError::Setup`] until the implementation lands.
pub struct CodexBackend {
    model_id: String,
}

impl CodexBackend {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
        }
    }
}

impl LlmBackend for CodexBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Setup(
            "CodexBackend is not yet implemented — \
             pending research task on the Codex CLI subprocess interface"
                .to_string(),
        ))
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: self.model_id.clone(),
            backend_version: format!("codex-stub/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmRequest, ResponseSchema};

    #[test]
    fn call_returns_setup_error() {
        let backend = CodexBackend::new("gpt-4o");
        let req = LlmRequest {
            prompt_template: crate::PromptId::Classify,
            inputs: serde_json::json!({}),
            schema: ResponseSchema::accept_any(),
        };
        let err = backend.call(&req).unwrap_err();
        assert!(matches!(err, LlmError::Setup(_)));
    }

    #[test]
    fn fingerprint_includes_model_id() {
        let backend = CodexBackend::new("gpt-4o");
        assert_eq!(backend.fingerprint().model_id, "gpt-4o");
    }
}
