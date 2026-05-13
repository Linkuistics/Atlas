//! PR-1 HTTP-backend wiring smoke.
//!
//! No live API calls happen here. The runtime loop uses an in-memory
//! staged backend with canned responses; provider routing uses a real
//! `BackendRouter::new_for_agent_runtime` built from env-substituted
//! HTTP config so Lane B can look up the cross-provider sibling.

use std::path::Path;
use std::sync::Arc;

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::runtime::audit::lane_b::select_auditor_backend;
use atlas_cli::backend::{BackendHandles, BudgetSentinel};
use atlas_cli::pipeline::{run_index_agent_runtime, IndexConfig};
use atlas_cli::IndexArgs;
use atlas_llm::{
    AtlasConfig, BackendRouter, ConfigError, LlmBackend, LlmError, LlmFingerprint, LlmRequest,
    Provider,
};
use clap::Parser;
use serde_json::{json, Value};
use tempfile::{NamedTempFile, TempDir};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    vars: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn capture(names: &[&'static str]) -> Self {
        let vars = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        Self { vars }
    }

    fn set(&self, name: &str, value: &str) {
        std::env::set_var(name, value);
    }

    fn remove(&self, name: &str) {
        std::env::remove_var(name);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.vars {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[derive(Debug, clap::Parser)]
#[command(name = "atlas")]
struct Harness {
    #[command(subcommand)]
    command: HarnessCmd,
}

#[derive(Debug, clap::Subcommand)]
enum HarnessCmd {
    Index(IndexArgs),
}

struct StagedBackend {
    by_substring: Vec<(String, Value)>,
    fingerprint: LlmFingerprint,
}

impl StagedBackend {
    fn new(canned: Vec<(String, Value)>) -> Self {
        Self {
            by_substring: canned,
            fingerprint: LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: "agent-runtime-http-smoke".to_string(),
                backend_version: "0".to_string(),
            },
        }
    }
}

#[async_trait::async_trait]
impl LlmBackend for StagedBackend {
    fn call(&self, _req: &LlmRequest) -> Result<Value, LlmError> {
        Err(LlmError::Invocation("StagedBackend is async-only".into()))
    }

    async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let conversation = req
            .inputs
            .get("conversation")
            .and_then(Value::as_str)
            .unwrap_or("");
        for (substring, value) in &self.by_substring {
            if conversation.contains(substring) {
                return Ok(value.clone());
            }
        }
        Err(LlmError::TestBackendMiss(format!(
            "no canned response matched conversation: {}",
            &conversation[..conversation.len().min(120)]
        )))
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

fn text_block(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn sprint_config_yaml(include_openai: bool) -> String {
    let openai_provider = if include_openai {
        "  openai:\n    api_key: \"${OPENAI_API_KEY}\"\n"
    } else {
        ""
    };
    let subcarve_model = if include_openai {
        "openai/gpt-5-codex"
    } else {
        "anthropic/claude-opus-4-7"
    };
    format!(
        "providers:\n  anthropic:\n    api_key: \"${{ANTHROPIC_API_KEY}}\"\n{openai_provider}\
         defaults:\n  model: \"anthropic/claude-opus-4-7\"\n  params:\n    max_tokens: 4096\n\
         operations:\n  classify:\n    model: \"anthropic/claude-opus-4-7\"\n    params:\n      max_tokens: 4096\n  subcarve:\n    model: \"{subcarve_model}\"\n    params:\n      max_tokens: 4096\n  surface:\n    model: \"anthropic/claude-opus-4-7\"\n    params:\n      max_tokens: 8192\n  edges:\n    model: \"{subcarve_model}\"\n    params:\n      max_tokens: 8192\n"
    )
}

fn write_config(text: &str) -> NamedTempFile {
    let file = NamedTempFile::new().unwrap();
    std::fs::write(file.path(), text).unwrap();
    file
}

fn build_provider_router(config: &AtlasConfig, workspace: &Path) -> (Arc<BackendRouter>, TempDir) {
    let prompts_dir = TempDir::new().unwrap();
    let router = BackendRouter::new_for_agent_runtime(
        config,
        prompts_dir.path(),
        workspace,
        [0u8; 32],
        [0u8; 32],
        None,
    )
    .unwrap();
    (Arc::new(router), prompts_dir)
}

fn parse_index_args(root: &Path, log_events: &Path) -> IndexArgs {
    let root = root.display().to_string();
    let log_events = log_events.display().to_string();
    let parsed = Harness::try_parse_from([
        "atlas",
        "index",
        "--no-budget",
        "--no-tui",
        "--log-events",
        log_events.as_str(),
        root.as_str(),
    ])
    .unwrap();
    let HarnessCmd::Index(args) = parsed.command;
    args
}

fn write_overrides(root: &Path) {
    std::fs::write(
        root.join("subsystems.overrides.yaml"),
        "schema_version: 1\nsubsystems:\n  - id: agents\n    members: [foo]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("components.overrides.yaml"),
        "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n",
    )
    .unwrap();
}

fn backend_handles(
    backend: Arc<dyn LlmBackend>,
    provider_router: Arc<BackendRouter>,
    prompts_dir: TempDir,
) -> BackendHandles {
    let sentinel = BudgetSentinel::new(backend);
    let fingerprint = sentinel.fingerprint();
    let backend = sentinel.clone() as Arc<dyn LlmBackend>;
    BackendHandles {
        backend,
        provider_router,
        counter: None,
        sentinel,
        fingerprint,
        prompts_dir,
    }
}

#[test]
fn config_loader_substitutes_env_vars() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = EnvGuard::capture(&["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    env.set("OPENAI_API_KEY", "test-openai-key");
    let file = write_config(&sprint_config_yaml(true));

    let loaded = AtlasConfig::load(file.path()).unwrap();

    assert_eq!(loaded.providers["anthropic"].api_key, "test-anthropic-key");
    assert_eq!(loaded.providers["openai"].api_key, "test-openai-key");
}

#[test]
fn config_loader_errors_on_missing_env_var() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = EnvGuard::capture(&["ANTHROPIC_API_KEY"]);
    env.remove("ANTHROPIC_API_KEY");
    let file = write_config(
        "providers:\n  anthropic:\n    api_key: \"${ANTHROPIC_API_KEY}\"\n\
         defaults:\n  model: \"anthropic/claude-opus-4-7\"\n  params:\n    max_tokens: 4096\n",
    );

    let err = AtlasConfig::load(file.path()).unwrap_err();

    assert!(matches!(
        err,
        ConfigError::MissingEnvVar { var_name } if var_name == "ANTHROPIC_API_KEY"
    ));
}

#[test]
fn agent_runtime_http_smoke_routes_cross_provider_auditor() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = EnvGuard::capture(&["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    env.set("OPENAI_API_KEY", "test-openai-key");
    let workspace = TempDir::new().unwrap();
    let file = write_config(&sprint_config_yaml(true));
    let config = AtlasConfig::load(file.path()).unwrap();
    let (router, _prompts_dir) = build_provider_router(&config, workspace.path());
    let producer = router
        .backend_for_provider(Provider::Anthropic)
        .unwrap()
        .clone();
    let for_provider = |provider: Provider| router.backend_for_provider(provider).cloned();
    let bus = EventBus::new(32);
    let mut rx = bus.subscribe();

    let choice = select_auditor_backend(Provider::Anthropic, &producer, Some(&for_provider), &bus);

    assert_eq!(choice.provider(), Provider::OpenAi);
    assert!(!choice.is_degraded());
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, AgentEvent::AuditDegraded { .. }),
            "cross-provider config must not emit AuditDegraded"
        );
    }
}

#[test]
fn agent_runtime_http_smoke_single_provider_emits_audit_degraded() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = EnvGuard::capture(&["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    env.remove("OPENAI_API_KEY");
    let workspace = TempDir::new().unwrap();
    let file = write_config(&sprint_config_yaml(false));
    let config = AtlasConfig::load(file.path()).unwrap();
    let (router, _prompts_dir) = build_provider_router(&config, workspace.path());
    let producer = router
        .backend_for_provider(Provider::Anthropic)
        .unwrap()
        .clone();
    let for_provider = |provider: Provider| router.backend_for_provider(provider).cloned();
    let bus = EventBus::new(32);
    let mut rx = bus.subscribe();

    let choice = select_auditor_backend(Provider::Anthropic, &producer, Some(&for_provider), &bus);

    assert_eq!(choice.provider(), Provider::Anthropic);
    assert!(choice.is_degraded());
    assert!(matches!(
        rx.try_recv().unwrap(),
        AgentEvent::AuditDegraded { .. }
    ));
}

#[test]
fn agent_runtime_http_smoke_completes_with_config_loaded_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = EnvGuard::capture(&["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    env.set("OPENAI_API_KEY", "test-openai-key");
    let workspace = TempDir::new().unwrap();
    write_overrides(workspace.path());
    let output_dir = workspace.path().join(".atlas");
    let events_path = workspace.path().join("events.jsonl");
    let config_file = write_config(&sprint_config_yaml(true));
    let atlas_config = AtlasConfig::load(config_file.path()).unwrap();
    let (provider_router, prompts_dir) = build_provider_router(&atlas_config, workspace.path());
    let staged_backend = Arc::new(StagedBackend::new(vec![
        (
            "classify component".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
        (
            "reduce subsystem".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
        (
            "project the workspace".to_string(),
            text_block("{\"components\":[{\"id\":\"foo\"}]}"),
        ),
    ])) as Arc<dyn LlmBackend>;
    let handles = backend_handles(staged_backend, provider_router, prompts_dir);
    let mut index_config = IndexConfig::new(workspace.path().to_path_buf());
    index_config.output_dir = output_dir.clone();
    let args = parse_index_args(workspace.path(), &events_path);

    run_index_agent_runtime(&index_config, &atlas_config, handles, &args).unwrap();

    let projection_path = output_dir
        .join("cache")
        .join("agent-runtime-projection.json");
    assert!(projection_path.exists());
    let events = std::fs::read_to_string(events_path).unwrap();
    assert!(events.contains("AgentStart"));
    assert!(events.contains("AgentComplete"));
    assert!(!events.contains("AuditDegraded"));
}
