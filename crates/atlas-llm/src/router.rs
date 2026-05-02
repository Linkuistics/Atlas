use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::config::AtlasConfig;
use crate::{AgentObserver, LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};

pub struct BackendRouter {
    table: HashMap<PromptId, Arc<dyn LlmBackend>>,
    fingerprint: LlmFingerprint,
}

impl BackendRouter {
    /// Build a `BackendRouter` from a loaded `AtlasConfig`.
    pub fn new(
        config: &AtlasConfig,
        prompts_dir: &std::path::Path,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
        observer: Option<Arc<dyn AgentObserver>>,
    ) -> Result<Self, LlmError> {
        let all_prompt_ids = [
            PromptId::Classify,
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ];

        let mut table: HashMap<PromptId, Arc<dyn LlmBackend>> = HashMap::new();
        let mut model_parts: Vec<String> = Vec::new();
        let mut version_parts: Vec<String> = Vec::new();

        for &prompt_id in &all_prompt_ids {
            let op = config.resolve_operation(prompt_id);
            let model_str = &op.model;
            let (provider, model_id) = model_str.split_once('/').ok_or_else(|| {
                LlmError::Setup(format!(
                    "model `{model_str}` must be in `<provider>/<model-id>` format"
                ))
            })?;

            let backend: Arc<dyn LlmBackend> = match provider {
                "anthropic" => {
                    let api_key = config
                        .providers
                        .get("anthropic")
                        .map(|p| p.api_key.clone())
                        .unwrap_or_default();
                    let b = crate::AnthropicHttpBackend::new(
                        model_id,
                        api_key,
                        op.params.clone(),
                        prompts_dir,
                        template_sha,
                        ontology_sha,
                    );
                    Arc::new(b)
                }
                "openai" => {
                    let api_key = config
                        .providers
                        .get("openai")
                        .map(|p| p.api_key.clone())
                        .unwrap_or_default();
                    let b = crate::OpenAiHttpBackend::new(
                        model_id,
                        api_key,
                        op.params.clone(),
                        prompts_dir,
                        template_sha,
                        ontology_sha,
                    );
                    Arc::new(b)
                }
                "claude-code" => {
                    let mut b = crate::ClaudeCodeBackend::new(model_id, prompts_dir)?
                        .with_fingerprint_inputs(template_sha, ontology_sha);
                    if let Some(obs) = observer.clone() {
                        b = b.with_observer(obs);
                    }
                    Arc::new(b)
                }
                "codex" => {
                    let mut b = crate::CodexBackend::new(model_id, prompts_dir)?
                        .with_fingerprint_inputs(template_sha, ontology_sha);
                    if let Some(obs) = observer.clone() {
                        b = b.with_observer(obs);
                    }
                    Arc::new(b)
                }
                other => return Err(LlmError::Setup(format!("unknown provider `{other}`"))),
            };

            let fp = backend.fingerprint();
            model_parts.push(format!("{:?}={}", prompt_id, model_str));
            version_parts.push(format!("{:?}={}", prompt_id, fp.backend_version));
            table.insert(prompt_id, backend);
        }

        let fingerprint = LlmFingerprint {
            template_sha,
            ontology_sha,
            model_id: model_parts.join("|"),
            backend_version: version_parts.join("|"),
        };

        Ok(Self { table, fingerprint })
    }

    /// Test-only constructor: build a router directly from a dispatch table.
    #[cfg(test)]
    pub fn from_dispatch_table(
        table: HashMap<PromptId, Arc<dyn LlmBackend>>,
        fingerprint: LlmFingerprint,
    ) -> Self {
        Self { table, fingerprint }
    }
}

impl LlmBackend for BackendRouter {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let backend = self.table.get(&req.prompt_template).ok_or_else(|| {
            LlmError::Setup(format!(
                "BackendRouter has no entry for {:?}",
                req.prompt_template
            ))
        })?;
        backend.call(req)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResponseSchema, TestBackend};
    use serde_json::json;

    fn make_fingerprint(model_id: &str) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: model_id.to_string(),
            backend_version: "test".to_string(),
        }
    }

    #[test]
    fn routes_classify_to_classify_backend() {
        let classify_backend = {
            let b = TestBackend::new();
            b.respond(
                PromptId::Classify,
                json!({ "dir_relative": "crates/foo" }),
                json!({ "is_component": true }),
            );
            Arc::new(b) as Arc<dyn LlmBackend>
        };
        let other_backend = Arc::new(TestBackend::new()) as Arc<dyn LlmBackend>;

        let mut table = HashMap::new();
        table.insert(PromptId::Classify, classify_backend);
        table.insert(PromptId::Subcarve, other_backend.clone());
        table.insert(PromptId::Stage1Surface, other_backend.clone());
        table.insert(PromptId::Stage2Edges, other_backend);

        let router = BackendRouter::from_dispatch_table(table, make_fingerprint("test-composite"));

        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({ "dir_relative": "crates/foo" }),
            schema: ResponseSchema::accept_any(),
        };
        let result = router.call(&req).unwrap();
        assert_eq!(result["is_component"], true);
    }

    #[test]
    fn missing_table_entry_is_setup_error() {
        let router = BackendRouter::from_dispatch_table(HashMap::new(), make_fingerprint("empty"));
        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({}),
            schema: ResponseSchema::accept_any(),
        };
        let err = router.call(&req).unwrap_err();
        assert!(matches!(err, LlmError::Setup(_)));
    }

    #[test]
    fn fingerprint_returns_composite() {
        let router =
            BackendRouter::from_dispatch_table(HashMap::new(), make_fingerprint("composite-fp"));
        assert_eq!(router.fingerprint().model_id, "composite-fp");
    }
}
