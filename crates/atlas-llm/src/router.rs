use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::config::AtlasConfig;
use crate::{
    BackendCallObserver, LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, Provider,
};

pub struct BackendRouter {
    table: HashMap<PromptId, Arc<dyn LlmBackend>>,
    provider_entries: Vec<ProviderEntry>,
    fingerprint: LlmFingerprint,
}

struct ProviderEntry {
    provider: Provider,
    backend: Arc<dyn LlmBackend>,
}

impl BackendRouter {
    /// Build a `BackendRouter` from a loaded `AtlasConfig`.
    ///
    /// `workspace_path` is the indexed codebase root; it is used as the
    /// cwd of subprocess backends (`claude-code`, `codex`) so their
    /// filesystem tools resolve paths against the user-specified
    /// workspace, not the cwd of the parent `atlas` process.
    pub fn new(
        config: &AtlasConfig,
        prompts_dir: &std::path::Path,
        workspace_path: &std::path::Path,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
        observer: Option<Arc<dyn BackendCallObserver>>,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            config,
            prompts_dir,
            workspace_path,
            template_sha,
            ontology_sha,
            observer,
            false,
        )
    }

    /// Build a router for AgentRuntime. The runtime's HTTP path is an
    /// Atlas-owned tool loop, so HTTP backends are valid for every
    /// stage here even though the deterministic prompt templates still
    /// reject HTTP for filesystem-heavy surface / edge extraction.
    pub fn new_for_agent_runtime(
        config: &AtlasConfig,
        prompts_dir: &std::path::Path,
        workspace_path: &std::path::Path,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
        observer: Option<Arc<dyn BackendCallObserver>>,
    ) -> Result<Self, LlmError> {
        Self::new_inner(
            config,
            prompts_dir,
            workspace_path,
            template_sha,
            ontology_sha,
            observer,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        config: &AtlasConfig,
        prompts_dir: &std::path::Path,
        workspace_path: &std::path::Path,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
        observer: Option<Arc<dyn BackendCallObserver>>,
        allow_http_filesystem_prompts: bool,
    ) -> Result<Self, LlmError> {
        let all_prompt_ids = [
            PromptId::Classify,
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ];

        let mut table: HashMap<PromptId, Arc<dyn LlmBackend>> = HashMap::new();
        let mut provider_entries: Vec<ProviderEntry> = Vec::new();
        let mut model_parts: Vec<String> = Vec::new();
        let mut version_parts: Vec<String> = Vec::new();

        for &prompt_id in &all_prompt_ids {
            let op = config.resolve_operation(prompt_id);
            let model_str = &op.model;
            let (provider_key, model_id) = model_str.split_once('/').ok_or_else(|| {
                LlmError::Setup(format!(
                    "model `{model_str}` must be in `<provider>/<model-id>` format"
                ))
            })?;

            if !allow_http_filesystem_prompts {
                reject_http_for_filesystem_required_prompt(prompt_id, provider_key, model_str)?;
            }

            let backend: Arc<dyn LlmBackend> = match provider_key {
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
                "openrouter" => {
                    let api_key = config
                        .providers
                        .get("openrouter")
                        .map(|p| p.api_key.clone())
                        .unwrap_or_default();
                    let b = crate::OpenAiHttpBackend::new(
                        model_id,
                        api_key,
                        op.params.clone(),
                        prompts_dir,
                        template_sha,
                        ontology_sha,
                    )
                    .with_base_url(
                        "https://openrouter.ai/api/v1/chat/completions",
                        "openrouter",
                    );
                    Arc::new(b)
                }
                "claude-code" => {
                    let mut b =
                        crate::ClaudeCodeBackend::new(model_id, prompts_dir, workspace_path)?
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
            if let Some(provider) = provider_from_config_key(provider_key) {
                provider_entries.push(ProviderEntry {
                    provider,
                    backend: backend.clone(),
                });
            }
            table.insert(prompt_id, backend);
        }

        let fingerprint = LlmFingerprint {
            template_sha,
            ontology_sha,
            model_id: model_parts.join("|"),
            backend_version: version_parts.join("|"),
        };

        Ok(Self {
            table,
            provider_entries,
            fingerprint,
        })
    }

    /// Test-only constructor: build a router directly from a dispatch table.
    #[cfg(test)]
    pub fn from_dispatch_table(
        table: HashMap<PromptId, Arc<dyn LlmBackend>>,
        fingerprint: LlmFingerprint,
    ) -> Self {
        Self {
            table,
            provider_entries: Vec::new(),
            fingerprint,
        }
    }
}

fn provider_from_config_key(provider: &str) -> Option<Provider> {
    match provider {
        "anthropic" | "claude-code" => Some(Provider::Anthropic),
        "openai" | "codex" => Some(Provider::OpenAi),
        _ => None,
    }
}

/// HTTP backends (`anthropic`, `openai`, `openrouter`) cannot service
/// `Stage1Surface` or `Stage2Edges` because their rendered prompts carry no
/// file-content tokens — surface and edge extraction need filesystem access,
/// which only the subprocess backends (`claude-code`, `codex`) provide.
/// Reject the combination at construction time so a misconfigured
/// `.atlas/config.yaml` fails loudly instead of silently producing
/// hallucinated surfaces or edges.
fn reject_http_for_filesystem_required_prompt(
    prompt_id: PromptId,
    provider: &str,
    model_str: &str,
) -> Result<(), LlmError> {
    if !crate::config::HTTP_PROVIDERS.contains(&provider) {
        return Ok(());
    }
    let prompt_label = match prompt_id {
        PromptId::Stage1Surface => "stage1-surface",
        PromptId::Stage2Edges => "stage2-edges",
        PromptId::Classify | PromptId::Subcarve => return Ok(()),
    };
    Err(LlmError::Setup(format!(
        "{prompt_label} requires a filesystem-access provider \
         (claude-code, codex); HTTP providers (anthropic, openai, openrouter) \
         cannot be used here — configured `{model_str}` in .atlas/config.yaml"
    )))
}

#[async_trait::async_trait]
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

    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let backend = self.table.get(&req.prompt_template).ok_or_else(|| {
            LlmError::Setup(format!(
                "BackendRouter has no entry for {:?}",
                req.prompt_template
            ))
        })?;
        backend.call_async(req).await
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }

    /// True iff at least one of the per-prompt routed backends exposes
    /// filesystem tools. The router is the engine-facing wrapper, so a
    /// caller asking the router this question is asking "does this
    /// configuration have *any* filesystem-aware backend wired in" —
    /// per-prompt eligibility is decided at routing time.
    fn supports_filesystem_tools(&self) -> bool {
        self.table.values().any(|b| b.supports_filesystem_tools())
    }
}

impl BackendRouter {
    /// Returns the first backend whose configured provider matches the
    /// requested provider. Production code path for Lane B
    /// cross-provider audit.
    pub fn backend_for_provider(&self, provider: Provider) -> Option<&Arc<dyn LlmBackend>> {
        self.provider_entries
            .iter()
            .find(|entry| entry.provider == provider)
            .map(|entry| &entry.backend)
    }

    /// Look up the backend routed for a given `PromptId`. Returns
    /// `None` if no entry is registered for that prompt. Useful for
    /// router-level capability checks (e.g. asking the per-prompt
    /// backend whether it supports filesystem tools, rather than the
    /// composite router).
    pub fn backend_for(&self, prompt_id: PromptId) -> Option<&Arc<dyn LlmBackend>> {
        self.table.get(&prompt_id)
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
    fn provider_cross_returns_opposite_vendor() {
        assert_eq!(Provider::Anthropic.cross(), Provider::OpenAi);
        assert_eq!(Provider::OpenAi.cross(), Provider::Anthropic);
    }

    #[test]
    fn backend_for_provider_returns_configured_http_backends_for_agent_runtime() {
        use crate::{AtlasConfig, OperationConfig, OperationsConfig, ProviderConfig};

        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api_key: "sk-anthropic-test".to_string(),
            },
        );
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                api_key: "sk-openai-test".to_string(),
            },
        );

        let config = AtlasConfig {
            providers,
            defaults: OperationConfig {
                model: "anthropic/claude-opus-4-7".to_string(),
                params: json!({ "max_tokens": 4096 }),
            },
            operations: OperationsConfig {
                classify: Some(OperationConfig {
                    model: "anthropic/claude-opus-4-7".to_string(),
                    params: json!({ "max_tokens": 4096 }),
                }),
                subcarve: Some(OperationConfig {
                    model: "openai/gpt-5-codex".to_string(),
                    params: json!({ "max_tokens": 4096 }),
                }),
                surface: Some(OperationConfig {
                    model: "anthropic/claude-opus-4-7".to_string(),
                    params: json!({ "max_tokens": 4096 }),
                }),
                edges: Some(OperationConfig {
                    model: "openai/gpt-5-codex".to_string(),
                    params: json!({ "max_tokens": 4096 }),
                }),
            },
        };

        let prompts_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let router = BackendRouter::new_for_agent_runtime(
            &config,
            prompts_dir.path(),
            workspace.path(),
            [0u8; 32],
            [0u8; 32],
            None,
        )
        .expect("agent runtime router accepts HTTP backends for tool-loop stages");

        assert!(router.backend_for_provider(Provider::Anthropic).is_some());
        assert!(router.backend_for_provider(Provider::OpenAi).is_some());
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

    #[test]
    fn rejects_anthropic_for_stage1_surface() {
        let err = reject_http_for_filesystem_required_prompt(
            PromptId::Stage1Surface,
            "anthropic",
            "anthropic/claude-haiku-4-5",
        )
        .unwrap_err();
        let LlmError::Setup(msg) = err else {
            panic!("expected Setup error, got {err:?}");
        };
        assert!(msg.contains("stage1-surface"));
        assert!(msg.contains("filesystem-access"));
        assert!(msg.contains("anthropic/claude-haiku-4-5"));
    }

    #[test]
    fn rejects_openai_for_stage2_edges() {
        let err = reject_http_for_filesystem_required_prompt(
            PromptId::Stage2Edges,
            "openai",
            "openai/gpt-4o-mini",
        )
        .unwrap_err();
        let LlmError::Setup(msg) = err else {
            panic!("expected Setup error, got {err:?}");
        };
        assert!(msg.contains("stage2-edges"));
        assert!(msg.contains("openai/gpt-4o-mini"));
    }

    #[test]
    fn rejects_openrouter_for_stage1_surface() {
        let err = reject_http_for_filesystem_required_prompt(
            PromptId::Stage1Surface,
            "openrouter",
            "openrouter/anthropic/claude-sonnet-4-6",
        )
        .unwrap_err();
        let LlmError::Setup(msg) = err else {
            panic!("expected Setup error, got {err:?}");
        };
        assert!(msg.contains("stage1-surface"));
        assert!(msg.contains("openrouter/anthropic/claude-sonnet-4-6"));
    }

    #[test]
    fn accepts_openrouter_for_classify_and_subcarve() {
        for prompt_id in [PromptId::Classify, PromptId::Subcarve] {
            reject_http_for_filesystem_required_prompt(
                prompt_id,
                "openrouter",
                "openrouter/anthropic/claude-sonnet-4-6",
            )
            .unwrap_or_else(|e| panic!("{prompt_id:?} + openrouter should pass: {e:?}"));
        }
    }

    #[test]
    fn nested_model_id_splits_only_on_first_slash() {
        // OpenRouter model ids contain a second `/`; the router uses
        // `split_once('/')` which already splits on the first slash, so
        // the rest passes through verbatim as the model id.
        let s = "openrouter/anthropic/claude-sonnet-4-6";
        let (provider, model_id) = s.split_once('/').unwrap();
        assert_eq!(provider, "openrouter");
        assert_eq!(model_id, "anthropic/claude-sonnet-4-6");
    }

    #[test]
    fn http_providers_constant_includes_openrouter() {
        assert!(crate::config::HTTP_PROVIDERS.contains(&"openrouter"));
        assert!(crate::config::HTTP_PROVIDERS.contains(&"anthropic"));
        assert!(crate::config::HTTP_PROVIDERS.contains(&"openai"));
    }

    #[test]
    fn accepts_http_for_classify_and_subcarve() {
        for prompt_id in [PromptId::Classify, PromptId::Subcarve] {
            for provider in ["anthropic", "openai"] {
                reject_http_for_filesystem_required_prompt(
                    prompt_id,
                    provider,
                    &format!("{provider}/some-model"),
                )
                .unwrap_or_else(|e| panic!("{prompt_id:?} + {provider} should pass: {e:?}"));
            }
        }
    }

    #[test]
    fn accepts_filesystem_providers_for_all_prompts() {
        for prompt_id in [
            PromptId::Classify,
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ] {
            for provider in ["claude-code", "codex"] {
                reject_http_for_filesystem_required_prompt(
                    prompt_id,
                    provider,
                    &format!("{provider}/some-model"),
                )
                .unwrap_or_else(|e| panic!("{prompt_id:?} + {provider} should pass: {e:?}"));
            }
        }
    }

    /// A backend that returns the configured boolean from
    /// `supports_filesystem_tools`. Lets router tests model
    /// filesystem-aware backends without spinning up a real subprocess.
    struct CapBackend {
        fs: bool,
    }

    #[async_trait::async_trait]
    impl LlmBackend for CapBackend {
        fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
            Err(LlmError::Invocation("not used in this test".into()))
        }

        async fn call_async(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
            Err(LlmError::Invocation("not used in this test".into()))
        }

        fn fingerprint(&self) -> LlmFingerprint {
            make_fingerprint("cap-backend")
        }

        fn supports_filesystem_tools(&self) -> bool {
            self.fs
        }
    }

    #[test]
    fn supports_filesystem_tools_is_true_when_any_backend_does() {
        let mut table: HashMap<PromptId, Arc<dyn LlmBackend>> = HashMap::new();
        table.insert(PromptId::Classify, Arc::new(CapBackend { fs: false }));
        table.insert(PromptId::Subcarve, Arc::new(CapBackend { fs: false }));
        table.insert(PromptId::Stage1Surface, Arc::new(CapBackend { fs: true }));
        table.insert(PromptId::Stage2Edges, Arc::new(CapBackend { fs: false }));
        let router = BackendRouter::from_dispatch_table(table, make_fingerprint("mixed"));
        assert!(router.supports_filesystem_tools());
    }

    #[test]
    fn supports_filesystem_tools_is_false_when_no_backend_does() {
        let mut table: HashMap<PromptId, Arc<dyn LlmBackend>> = HashMap::new();
        table.insert(PromptId::Classify, Arc::new(CapBackend { fs: false }));
        table.insert(PromptId::Subcarve, Arc::new(CapBackend { fs: false }));
        let router = BackendRouter::from_dispatch_table(table, make_fingerprint("none"));
        assert!(!router.supports_filesystem_tools());
    }

    #[test]
    fn backend_for_returns_per_prompt_capability() {
        let mut table: HashMap<PromptId, Arc<dyn LlmBackend>> = HashMap::new();
        table.insert(PromptId::Classify, Arc::new(CapBackend { fs: false }));
        table.insert(PromptId::Stage1Surface, Arc::new(CapBackend { fs: true }));
        let router = BackendRouter::from_dispatch_table(table, make_fingerprint("split"));

        assert!(!router
            .backend_for(PromptId::Classify)
            .unwrap()
            .supports_filesystem_tools());
        assert!(router
            .backend_for(PromptId::Stage1Surface)
            .unwrap()
            .supports_filesystem_tools());
        assert!(router.backend_for(PromptId::Stage2Edges).is_none());
    }

    #[test]
    fn router_construction_rejects_http_routed_surface() {
        use crate::{AtlasConfig, OperationConfig, OperationsConfig, ProviderConfig};

        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                api_key: "sk-test".to_string(),
            },
        );
        let config = AtlasConfig {
            providers,
            defaults: OperationConfig {
                model: "anthropic/claude-haiku-4-5".to_string(),
                params: json!({ "max_tokens": 4096 }),
            },
            operations: OperationsConfig::default(),
        };

        let prompts_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let result = BackendRouter::new(
            &config,
            prompts_dir.path(),
            workspace.path(),
            [0u8; 32],
            [0u8; 32],
            None,
        );
        match result {
            Err(LlmError::Setup(msg)) => assert!(
                msg.contains("stage1-surface"),
                "expected stage1-surface mention, got: {msg}"
            ),
            Err(other) => panic!("expected Setup error, got {other:?}"),
            Ok(_) => panic!("expected router construction to fail"),
        }
    }
}
