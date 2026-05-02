use serde_json::Value;

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};

/// Stub backend for OpenAI's Codex CLI tool. Invocation interface
/// and output format are pending the Codex CLI research backlog task.
/// `call()` returns `LlmError::Setup` until the research is complete.
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
