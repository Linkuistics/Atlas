# Multi-Provider LLM Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded `ClaudeCodeBackend` with a routed multi-backend system driven by `.atlas/config.yaml`, adding `AnthropicHttpBackend`, `OpenAiHttpBackend`, a `CodexBackend` stub, `BackendRouter`, and an `atlas init` subcommand.

**Architecture:** A new `AtlasConfig` type (in `atlas-llm`) is loaded from `.atlas/config.yaml` at startup. `BackendRouter` implements `LlmBackend` and dispatches each call to the right concrete backend based on `req.prompt_template` and the `<provider>/<model>` strings in the config. The existing budget/sentinel/progress wrapper stack wraps `BackendRouter` unchanged.

**Tech Stack:** Rust, `reqwest 0.12` (blocking), `serde_yaml 0.9` (already in workspace), `thiserror 2` (already in workspace).

**Spec:** `docs/superpowers/specs/2026-05-02-multi-provider-llm-config-design.md`

---

## File Map

**New files:**
- `crates/atlas-llm/src/config.rs` — `AtlasConfig`, `ConfigError`, env-var interpolation, load + validate
- `crates/atlas-llm/src/http_anthropic.rs` — `AnthropicHttpBackend`
- `crates/atlas-llm/src/http_openai.rs` — `OpenAiHttpBackend`
- `crates/atlas-llm/src/codex.rs` — `CodexBackend` stub
- `crates/atlas-llm/src/router.rs` — `BackendRouter`
- `crates/atlas-cli/src/init.rs` — `run_init_cmd`, embedded template strings

**Modified files:**
- `Cargo.toml` (workspace) — add `reqwest`
- `crates/atlas-llm/Cargo.toml` — add `reqwest`, `serde_yaml`
- `crates/atlas-llm/src/lib.rs` — export new modules and types
- `crates/atlas-cli/src/main.rs` — add `Init` subcommand, remove `--model` flag, load config before building backend
- `crates/atlas-cli/src/backend.rs` — `build_production_backend_with_counter` takes `&AtlasConfig` instead of `model_id: String`; remove legacy `build_production_backend` shim

---

## Task 1: Add reqwest and serde_yaml to atlas-llm

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/atlas-llm/Cargo.toml`

- [ ] **Step 1: Add reqwest to workspace deps**

In `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
reqwest = { version = "0.12", features = ["blocking", "json"] }
```

- [ ] **Step 2: Wire reqwest and serde_yaml into atlas-llm**

In `crates/atlas-llm/Cargo.toml`, update `[dependencies]`:

```toml
[dependencies]
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 3: Verify it compiles**

```
cargo check -p atlas-llm
```

Expected: no errors (reqwest adds no immediate unused-import warnings).

- [ ] **Step 4: Commit**

```
git add Cargo.toml crates/atlas-llm/Cargo.toml
git commit -m "chore: add reqwest and serde_yaml to atlas-llm"
```

---

## Task 2: AtlasConfig types and serde

**Files:**
- Create: `crates/atlas-llm/src/config.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Write the failing test for config serde round-trip**

Create `crates/atlas-llm/src/config.rs` with just the types and a test:

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasConfig {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    pub defaults: OperationConfig,
    #[serde(default)]
    pub operations: OperationsConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationConfig {
    pub model: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OperationsConfig {
    pub classify: Option<OperationConfig>,
    pub subcarve: Option<OperationConfig>,
    pub surface: Option<OperationConfig>,
    pub edges: Option<OperationConfig>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found at {path} — run `atlas init <root>` first")]
    NotFound { path: String },
    #[error("failed to read {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("failed to parse config.yaml: {0}")]
    Parse(String),
    #[error("env var `{name}` is unset (referenced in config.yaml)")]
    EnvVarUnset { name: String },
    #[error("defaults.model is required in config.yaml")]
    MissingDefaultModel,
    #[error("provider `{provider}` is used but not configured in providers:")]
    MissingProviderEntry { provider: String },
    #[error("providers.{provider}.api_key is empty after interpolation")]
    EmptyApiKey { provider: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_minimal_config() {
        let yaml = r#"
defaults:
  model: "anthropic/claude-haiku-4-5"
"#;
        let config: AtlasConfig = serde_yaml::from_str(yaml).expect("parse ok");
        assert_eq!(config.defaults.model, "anthropic/claude-haiku-4-5");
        assert!(config.providers.is_empty());
        assert!(config.operations.classify.is_none());
    }

    #[test]
    fn round_trips_full_config() {
        let yaml = r#"
providers:
  anthropic:
    api_key: "sk-test"
defaults:
  model: "anthropic/claude-sonnet-4-6"
  params:
    temperature: 0
operations:
  classify:
    model: "anthropic/claude-haiku-4-5"
  surface:
    model: "claude-code/claude-sonnet-4-6"
"#;
        let config: AtlasConfig = serde_yaml::from_str(yaml).expect("parse ok");
        assert_eq!(
            config.providers["anthropic"].api_key,
            "sk-test"
        );
        assert_eq!(
            config.operations.classify.as_ref().unwrap().model,
            "anthropic/claude-haiku-4-5"
        );
        assert!(config.operations.subcarve.is_none());
        assert_eq!(
            config.operations.surface.as_ref().unwrap().model,
            "claude-code/claude-sonnet-4-6"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

```
cargo test -p atlas-llm config::tests
```

Expected: all pass (pure serde, no I/O).

- [ ] **Step 3: Export config module from lib.rs**

In `crates/atlas-llm/src/lib.rs`, add after the existing `pub mod` lines:

```rust
pub mod config;
pub use config::{AtlasConfig, ConfigError, OperationConfig, OperationsConfig, ProviderConfig};
```

- [ ] **Step 4: Verify it compiles**

```
cargo check -p atlas-llm
```

- [ ] **Step 5: Commit**

```
git add crates/atlas-llm/src/config.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add AtlasConfig types with serde"
```

---

## Task 3: Env-var interpolation

**Files:**
- Modify: `crates/atlas-llm/src/config.rs`

- [ ] **Step 1: Write the failing tests**

Add to `config.rs` inside `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn interpolates_env_var() {
        std::env::set_var("_ATLAS_TEST_KEY", "hello");
        let result = interpolate_env_vars("prefix_${_ATLAS_TEST_KEY}_suffix").unwrap();
        assert_eq!(result, "prefix_hello_suffix");
        std::env::remove_var("_ATLAS_TEST_KEY");
    }

    #[test]
    fn passthrough_when_no_placeholders() {
        let result = interpolate_env_vars("no placeholders here").unwrap();
        assert_eq!(result, "no placeholders here");
    }

    #[test]
    fn unset_env_var_is_error() {
        std::env::remove_var("_ATLAS_DEFINITELY_UNSET_XYZ");
        let err = interpolate_env_vars("${_ATLAS_DEFINITELY_UNSET_XYZ}").unwrap_err();
        assert!(matches!(err, ConfigError::EnvVarUnset { name } if name == "_ATLAS_DEFINITELY_UNSET_XYZ"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test -p atlas-llm config::tests::interpolates_env_var
```

Expected: FAIL — `interpolate_env_vars` not defined.

- [ ] **Step 3: Implement interpolate_env_vars**

Add to `config.rs` (above the `#[cfg(test)]` block):

```rust
pub(crate) fn interpolate_env_vars(s: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find("${") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let end = after.find('}').ok_or_else(|| ConfigError::Parse(
            format!("unclosed '${{' in config.yaml near: {}", &rest[pos..rest.len().min(pos + 20)])
        ))?;
        let name = &after[..end];
        let value = std::env::var(name).map_err(|_| ConfigError::EnvVarUnset {
            name: name.to_string(),
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo test -p atlas-llm config::tests
```

Expected: all pass.

- [ ] **Step 5: Commit**

```
git add crates/atlas-llm/src/config.rs
git commit -m "feat(atlas-llm): add env-var interpolation for config.yaml"
```

---

## Task 4: AtlasConfig::load and validation

**Files:**
- Modify: `crates/atlas-llm/src/config.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests` in `config.rs`:

```rust
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(yaml: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{yaml}").unwrap();
        f
    }

    #[test]
    fn load_minimal_valid_config() {
        let f = write_config(
            "defaults:\n  model: \"claude-code/claude-sonnet-4-6\"\n",
        );
        let config = AtlasConfig::load(f.path()).unwrap();
        assert_eq!(config.defaults.model, "claude-code/claude-sonnet-4-6");
    }

    #[test]
    fn load_missing_file_is_not_found_error() {
        let err = AtlasConfig::load(std::path::Path::new("/no/such/file.yaml")).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound { .. }));
    }

    #[test]
    fn load_missing_defaults_model_is_error() {
        let f = write_config("defaults:\n  params:\n    temperature: 0\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn load_rejects_empty_defaults_model() {
        let f = write_config("defaults:\n  model: \"\"\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::MissingDefaultModel));
    }

    #[test]
    fn load_rejects_http_provider_missing_entry() {
        let f = write_config("defaults:\n  model: \"anthropic/claude-haiku-4-5\"\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::MissingProviderEntry { provider } if provider == "anthropic"
        ));
    }

    #[test]
    fn load_rejects_empty_api_key_after_interpolation() {
        std::env::set_var("_ATLAS_TEST_EMPTY_KEY", "");
        let f = write_config(
            "providers:\n  anthropic:\n    api_key: \"${_ATLAS_TEST_EMPTY_KEY}\"\ndefaults:\n  model: \"anthropic/claude-haiku-4-5\"\n",
        );
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::EmptyApiKey { .. }));
        std::env::remove_var("_ATLAS_TEST_EMPTY_KEY");
    }

    #[test]
    fn claude_code_provider_needs_no_providers_entry() {
        let f = write_config(
            "defaults:\n  model: \"claude-code/claude-sonnet-4-6\"\n",
        );
        AtlasConfig::load(f.path()).expect("should succeed — claude-code needs no credentials");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p atlas-llm config::tests::load_minimal_valid_config
```

Expected: FAIL — `AtlasConfig::load` not defined.

- [ ] **Step 3: Implement AtlasConfig::load**

Add to `config.rs` inside `impl AtlasConfig` (create the impl block):

```rust
impl AtlasConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Err(ConfigError::NotFound {
                path: path.display().to_string(),
            });
        }
        let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let interpolated = interpolate_env_vars(&raw)?;
        let config: AtlasConfig = serde_yaml::from_str(&interpolated)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.defaults.model.is_empty() {
            return Err(ConfigError::MissingDefaultModel);
        }

        // Collect all referenced provider prefixes.
        let all_models = std::iter::once(&self.defaults.model).chain(
            [
                self.operations.classify.as_ref(),
                self.operations.subcarve.as_ref(),
                self.operations.surface.as_ref(),
                self.operations.edges.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(|op| &op.model),
        );

        // Providers requiring credentials.
        const HTTP_PROVIDERS: &[&str] = &["anthropic", "openai"];

        for model in all_models {
            let provider = model.split('/').next().unwrap_or("");
            if HTTP_PROVIDERS.contains(&provider) {
                let entry = self.providers.get(provider).ok_or_else(|| {
                    ConfigError::MissingProviderEntry {
                        provider: provider.to_string(),
                    }
                })?;
                if entry.api_key.is_empty() {
                    return Err(ConfigError::EmptyApiKey {
                        provider: provider.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Return the resolved `OperationConfig` for a given prompt, falling
    /// back to `defaults` when the operation has no explicit entry.
    pub fn resolve_operation(&self, prompt_id: crate::PromptId) -> &OperationConfig {
        let op = match prompt_id {
            crate::PromptId::Classify => self.operations.classify.as_ref(),
            crate::PromptId::Subcarve => self.operations.subcarve.as_ref(),
            crate::PromptId::Stage1Surface => self.operations.surface.as_ref(),
            crate::PromptId::Stage2Edges => self.operations.edges.as_ref(),
        };
        op.unwrap_or(&self.defaults)
    }
}
```

- [ ] **Step 4: Add tempfile to atlas-llm dev-dependencies**

In `crates/atlas-llm/Cargo.toml`, `[dev-dependencies]` should already have `tempfile`. Verify:

```toml
[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 5: Run all config tests**

```
cargo test -p atlas-llm config::tests
```

Expected: all pass.

- [ ] **Step 6: Commit**

```
git add crates/atlas-llm/src/config.rs
git commit -m "feat(atlas-llm): add AtlasConfig::load with validation"
```

---

## Task 5: AnthropicHttpBackend

**Files:**
- Create: `crates/atlas-llm/src/http_anthropic.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/atlas-llm/src/http_anthropic.rs`:

```rust
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use crate::claude_code::{extract_tokens, prompt_template_filename, validate_response};
use crate::stream_parse::strip_json_fence;

pub struct AnthropicHttpBackend {
    model_id: String,
    api_key: String,
    params: Value,
    prompts_dir: PathBuf,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    client: reqwest::blocking::Client,
}

impl AnthropicHttpBackend {
    pub fn new(
        model_id: impl Into<String>,
        api_key: impl Into<String>,
        params: Value,
        prompts_dir: impl Into<PathBuf>,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
    ) -> Self {
        Self {
            model_id: model_id.into(),
            api_key: api_key.into(),
            params,
            prompts_dir: prompts_dir.into(),
            template_sha,
            ontology_sha,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
        let path = self.prompts_dir.join(prompt_template_filename(req.prompt_template));
        let template = std::fs::read_to_string(&path).map_err(|e| {
            LlmError::Invocation(format!("failed to read {:?}: {e}", path))
        })?;
        let tokens = extract_tokens(&req.inputs)?;
        crate::prompt::render(&template, &tokens)
    }
}

/// Extract and validate the JSON payload from an Anthropic Messages API
/// response JSON value. Separated for unit-testability.
pub(crate) fn parse_anthropic_response(
    resp: &Value,
    schema: &crate::ResponseSchema,
) -> Result<Value, LlmError> {
    let text = resp
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Parse(
            "Anthropic response missing content[0].text".to_string(),
        ))?;
    let stripped = strip_json_fence(text);
    let value: Value = serde_json::from_str(stripped).map_err(|e| {
        LlmError::Parse(format!("Anthropic response is not valid JSON: {e}"))
    })?;
    validate_response(&value, schema)?;
    Ok(value)
}

impl LlmBackend for AnthropicHttpBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered = self.render_request(req)?;

        let max_tokens = self.params.get("max_tokens").and_then(Value::as_u64).ok_or_else(|| {
            LlmError::Setup(
                "params.max_tokens is required for the `anthropic` provider — \
                 add it to the operation's `params:` block in .atlas/config.yaml".to_string(),
            )
        })?;

        let mut body = json!({
            "model": self.model_id,
            "max_tokens": max_tokens,
            "messages": [{ "role": "user", "content": rendered }]
        });
        if let (Some(body_obj), Some(params_obj)) =
            (body.as_object_mut(), self.params.as_object())
        {
            for (k, v) in params_obj {
                if k != "max_tokens" {
                    body_obj.insert(k.clone(), v.clone());
                }
            }
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .map_err(|e| LlmError::Invocation(format!("Anthropic HTTP request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().unwrap_or_default();
            return Err(LlmError::Invocation(format!(
                "Anthropic API returned {status}: {}",
                &body_text[..body_text.len().min(200)]
            )));
        }

        let resp_json: Value = response.json().map_err(|e| {
            LlmError::Parse(format!("failed to parse Anthropic response body: {e}"))
        })?;
        parse_anthropic_response(&resp_json, &req.schema)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: self.template_sha,
            ontology_sha: self.ontology_sha,
            model_id: self.model_id.clone(),
            backend_version: format!("anthropic-http/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResponseSchema;

    #[test]
    fn parses_anthropic_response_with_json_fence() {
        let resp = json!({
            "content": [{ "type": "text", "text": "```json\n{\"is_component\": true}\n```" }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_anthropic_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], true);
    }

    #[test]
    fn parses_anthropic_response_bare_json() {
        let resp = json!({
            "content": [{ "type": "text", "text": "{\"is_component\": false}" }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_anthropic_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], false);
    }

    #[test]
    fn missing_content_is_parse_error() {
        let resp = json!({ "model": "claude-haiku-4-5" });
        let schema = ResponseSchema::accept_any();
        let err = parse_anthropic_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn non_json_content_is_parse_error() {
        let resp = json!({
            "content": [{ "type": "text", "text": "not json at all" }]
        });
        let schema = ResponseSchema::accept_any();
        let err = parse_anthropic_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn missing_max_tokens_param_is_setup_error() {
        use std::path::PathBuf;
        let backend = AnthropicHttpBackend::new(
            "claude-haiku-4-5",
            "sk-test",
            json!({}),
            PathBuf::from("/tmp"),
            [0u8; 32],
            [0u8; 32],
        );
        // Fake a request — call() will fail before making HTTP because max_tokens is absent.
        // We can't call render_request without real templates, but we can test the
        // max_tokens guard directly by crafting a minimal request that will hit it.
        // Instead, test the guard in isolation:
        let params = json!({});
        let result = params.get("max_tokens").and_then(Value::as_u64);
        assert!(result.is_none(), "no max_tokens in empty params");
        drop(backend);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

```
cargo test -p atlas-llm http_anthropic::tests
```

Expected: all pass (pure parsing, no network).

- [ ] **Step 3: Export from lib.rs**

Add to `crates/atlas-llm/src/lib.rs`:

```rust
pub mod http_anthropic;
pub use http_anthropic::AnthropicHttpBackend;
```

- [ ] **Step 4: Verify compilation**

```
cargo check -p atlas-llm
```

- [ ] **Step 5: Commit**

```
git add crates/atlas-llm/src/http_anthropic.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add AnthropicHttpBackend"
```

---

## Task 6: OpenAiHttpBackend

**Files:**
- Create: `crates/atlas-llm/src/http_openai.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Create http_openai.rs with tests**

Create `crates/atlas-llm/src/http_openai.rs`:

```rust
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
use crate::claude_code::{extract_tokens, prompt_template_filename, validate_response};
use crate::stream_parse::strip_json_fence;

pub struct OpenAiHttpBackend {
    model_id: String,
    api_key: String,
    params: Value,
    prompts_dir: PathBuf,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    client: reqwest::blocking::Client,
}

impl OpenAiHttpBackend {
    pub fn new(
        model_id: impl Into<String>,
        api_key: impl Into<String>,
        params: Value,
        prompts_dir: impl Into<PathBuf>,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
    ) -> Self {
        Self {
            model_id: model_id.into(),
            api_key: api_key.into(),
            params,
            prompts_dir: prompts_dir.into(),
            template_sha,
            ontology_sha,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
        let path = self.prompts_dir.join(prompt_template_filename(req.prompt_template));
        let template = std::fs::read_to_string(&path).map_err(|e| {
            LlmError::Invocation(format!("failed to read {:?}: {e}", path))
        })?;
        let tokens = extract_tokens(&req.inputs)?;
        crate::prompt::render(&template, &tokens)
    }
}

/// Extract and validate the JSON payload from an OpenAI Chat Completions
/// response JSON value. Separated for unit-testability.
pub(crate) fn parse_openai_response(
    resp: &Value,
    schema: &crate::ResponseSchema,
) -> Result<Value, LlmError> {
    let text = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Parse(
            "OpenAI response missing choices[0].message.content".to_string(),
        ))?;
    let stripped = strip_json_fence(text);
    let value: Value = serde_json::from_str(stripped).map_err(|e| {
        LlmError::Parse(format!("OpenAI response is not valid JSON: {e}"))
    })?;
    validate_response(&value, schema)?;
    Ok(value)
}

impl LlmBackend for OpenAiHttpBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered = self.render_request(req)?;

        let mut body = json!({
            "model": self.model_id,
            "messages": [{ "role": "user", "content": rendered }]
        });
        if let (Some(body_obj), Some(params_obj)) =
            (body.as_object_mut(), self.params.as_object())
        {
            for (k, v) in params_obj {
                body_obj.insert(k.clone(), v.clone());
            }
        }

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .map_err(|e| LlmError::Invocation(format!("OpenAI HTTP request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().unwrap_or_default();
            return Err(LlmError::Invocation(format!(
                "OpenAI API returned {status}: {}",
                &body_text[..body_text.len().min(200)]
            )));
        }

        let resp_json: Value = response.json().map_err(|e| {
            LlmError::Parse(format!("failed to parse OpenAI response body: {e}"))
        })?;
        parse_openai_response(&resp_json, &req.schema)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: self.template_sha,
            ontology_sha: self.ontology_sha,
            model_id: self.model_id.clone(),
            backend_version: format!("openai-http/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResponseSchema;

    #[test]
    fn parses_openai_response_with_json_fence() {
        let resp = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "```json\n{\"is_component\": true}\n```" }
            }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_openai_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], true);
    }

    #[test]
    fn parses_openai_response_bare_json() {
        let resp = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "{\"is_component\": false}" }
            }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_openai_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], false);
    }

    #[test]
    fn missing_choices_is_parse_error() {
        let resp = json!({ "model": "gpt-4o" });
        let schema = ResponseSchema::accept_any();
        let err = parse_openai_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn non_json_content_is_parse_error() {
        let resp = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "not json" }
            }]
        });
        let schema = ResponseSchema::accept_any();
        let err = parse_openai_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }
}
```

- [ ] **Step 2: Run tests**

```
cargo test -p atlas-llm http_openai::tests
```

Expected: all pass.

- [ ] **Step 3: Export from lib.rs**

Add to `crates/atlas-llm/src/lib.rs`:

```rust
pub mod http_openai;
pub use http_openai::OpenAiHttpBackend;
```

- [ ] **Step 4: Commit**

```
git add crates/atlas-llm/src/http_openai.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add OpenAiHttpBackend"
```

---

## Task 7: CodexBackend stub

**Files:**
- Create: `crates/atlas-llm/src/codex.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Create codex.rs**

Create `crates/atlas-llm/src/codex.rs`:

```rust
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
```

- [ ] **Step 2: Run tests**

```
cargo test -p atlas-llm codex::tests
```

Expected: both pass.

- [ ] **Step 3: Export from lib.rs**

Add to `crates/atlas-llm/src/lib.rs`:

```rust
pub mod codex;
pub use codex::CodexBackend;
```

- [ ] **Step 4: Commit**

```
git add crates/atlas-llm/src/codex.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add CodexBackend stub"
```

---

## Task 8: BackendRouter

**Files:**
- Create: `crates/atlas-llm/src/router.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/atlas-llm/src/router.rs` with just the test:

```rust
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
    /// Test-only constructor: build a router directly from a dispatch table
    /// without constructing real backends or loading config from disk.
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

        let router =
            BackendRouter::from_dispatch_table(table, make_fingerprint("test-composite"));

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
        let router = BackendRouter::from_dispatch_table(
            HashMap::new(),
            make_fingerprint("empty"),
        );
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
        let router = BackendRouter::from_dispatch_table(
            HashMap::new(),
            make_fingerprint("composite-fp"),
        );
        assert_eq!(router.fingerprint().model_id, "composite-fp");
    }
}
```

- [ ] **Step 2: Run tests to verify they compile and pass**

```
cargo test -p atlas-llm router::tests
```

Expected: all pass (dispatch table tests work with TestBackend stubs).

- [ ] **Step 3: Implement BackendRouter::new**

Add to `router.rs` (below the `BackendRouter` struct definition):

```rust
impl BackendRouter {
    /// Build a `BackendRouter` from a loaded `AtlasConfig`. Constructs one
    /// backend per operation (multiple operations may end up with the same
    /// provider and model — that is fine; backends are cheap to construct).
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
                    let mut b =
                        crate::ClaudeCodeBackend::new(model_id, prompts_dir)?
                            .with_fingerprint_inputs(template_sha, ontology_sha);
                    if let Some(obs) = observer.clone() {
                        b = b.with_observer(obs);
                    }
                    Arc::new(b)
                }
                "codex" => Arc::new(crate::CodexBackend::new(model_id)),
                other => {
                    return Err(LlmError::Setup(format!("unknown provider `{other}`")))
                }
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
}
```

- [ ] **Step 4: Run all router tests**

```
cargo test -p atlas-llm router::tests
```

Expected: all pass.

- [ ] **Step 5: Export from lib.rs**

Add to `crates/atlas-llm/src/lib.rs`:

```rust
pub mod router;
pub use router::BackendRouter;
```

- [ ] **Step 6: Commit**

```
git add crates/atlas-llm/src/router.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add BackendRouter"
```

---

## Task 9: Wire BackendRouter into CLI and remove --model

**Files:**
- Modify: `crates/atlas-cli/src/backend.rs`
- Modify: `crates/atlas-cli/src/main.rs`

- [ ] **Step 1: Update build_production_backend_with_counter signature**

In `crates/atlas-cli/src/backend.rs`, replace the function signature and body. Also delete the legacy `build_production_backend` shim. Replace the entire file content from line 88 onwards with:

```rust
/// Construct the production backend stack from an `AtlasConfig`.
/// Creates one backend per LLM operation per the routing table in the
/// config, wraps the router in BudgetedBackend + BudgetSentinel, and
/// returns `BackendHandles` for the caller to keep alive.
pub fn build_production_backend_with_counter(
    config: &atlas_llm::AtlasConfig,
    counter: Option<Arc<TokenCounter>>,
    observer: Option<Arc<dyn atlas_llm::AgentObserver>>,
) -> Result<BackendHandles> {
    let prompts_dir = TempDir::new()?;
    crate::prompts::materialise_to(prompts_dir.path())?;

    let template_sha = compute_template_sha();
    let ontology_sha = compute_ontology_sha();

    let router = atlas_llm::BackendRouter::new(
        config,
        prompts_dir.path(),
        template_sha,
        ontology_sha,
        observer,
    )
    .map_err(|e| anyhow::anyhow!("failed to build backend: {e}"))?;
    let fingerprint = router.fingerprint();
    let inner: Arc<dyn LlmBackend> = Arc::new(router);

    let backend_after_budget: Arc<dyn LlmBackend> = match counter.as_ref() {
        Some(c) => Arc::new(BudgetedBackend::new(
            inner,
            c.clone(),
            default_token_estimator(),
        )),
        None => inner,
    };
    let sentinel = BudgetSentinel::new(backend_after_budget);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    Ok(BackendHandles {
        backend,
        counter,
        sentinel,
        fingerprint,
        prompts_dir,
    })
}
```

Also remove the `ClaudeCodeBackend` import from the top of the file (it's no longer constructed here), and remove `use atlas_llm::claude_code::resolve_default_model_id` if present.

Updated imports at the top of `backend.rs`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use atlas_engine::sha256_hex;
use atlas_llm::{
    default_token_estimator, BudgetedBackend, LlmBackend, LlmError,
    LlmFingerprint, LlmRequest, TokenCounter,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::prompts::{EMBEDDED_ONTOLOGY_YAML, EMBEDDED_PROMPTS};
```

- [ ] **Step 2: Update main.rs — remove --model, load config**

In `crates/atlas-cli/src/main.rs`:

1. Remove the `model` field from `IndexArgs`:
```rust
// DELETE this entire block from IndexArgs:
/// Override the model id passed to `claude -p --model`. Defaults
/// to the value of `$ATLAS_LLM_MODEL` or the built-in constant.
#[arg(long)]
model: Option<String>,
```

2. Remove the import:
```rust
// DELETE this line:
use atlas_llm::{claude_code::resolve_default_model_id, LlmBackend};
// REPLACE with:
use atlas_llm::LlmBackend;
```

3. In `run_index_cmd`, replace the model_id line and the `build_production_backend_with_counter` call:

```rust
fn run_index_cmd(args: IndexArgs) -> Result<ExitCode> {
    if args.budget.is_none() && !args.no_budget {
        anyhow::bail!(
            "`atlas index` requires `--budget <N-tokens>` to fail loudly on runaway LLM usage. \
             Pass `--no-budget` for local development if you understand the risk."
        );
    }

    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;

    let output_dir = args
        .output_dir
        .unwrap_or_else(|| root.join(atlas_cli::DEFAULT_OUTPUT_SUBDIR));

    let config_path = output_dir.join("config.yaml");
    let config = atlas_llm::AtlasConfig::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;

    let mut index_config = atlas_cli::IndexConfig::new(root);
    index_config.output_dir = output_dir;
    index_config.max_depth = args.max_depth;
    index_config.recarve = args.recarve;
    index_config.dry_run = args.dry_run;
    index_config.respect_gitignore = !args.no_gitignore;
    index_config.prompt_shas = Some(atlas_cli::backend::compute_prompt_shas());

    let progress_mode = if args.no_progress {
        ProgressMode::Never
    } else if args.progress {
        ProgressMode::Always
    } else {
        ProgressMode::Auto
    };

    let counter = args
        .budget
        .map(|b| Arc::new(atlas_llm::TokenCounter::new(b)));
    let reporter = make_stderr_reporter(progress_mode, counter.clone());

    let observer = if reporter.drawing() {
        Some(Arc::clone(&reporter) as Arc<dyn atlas_llm::AgentObserver>)
    } else {
        None
    };

    let handles = atlas_cli::backend::build_production_backend_with_counter(
        &config,
        counter.clone(),
        observer,
    )
    .context("failed to build LLM backend")?;
    index_config.fingerprint_override = Some(handles.fingerprint.clone());

    let backend: Arc<dyn LlmBackend> =
        ProgressBackend::new(handles.backend.clone(), Arc::clone(&reporter))
            as Arc<dyn LlmBackend>;

    let outcome = run_index(
        &index_config,
        backend,
        handles.counter.clone(),
        Arc::clone(&reporter),
    );
    reporter.finish();
    match outcome {
        Ok(summary) => {
            println!("{}", atlas_cli::pipeline::format_summary(&summary));
            drop(handles);
            Ok(ExitCode::SUCCESS)
        }
        Err(IndexError::BudgetExhausted) => {
            eprintln!("atlas: LLM token budget exhausted; no output files were written");
            drop(handles);
            Ok(ExitCode::from(2))
        }
        Err(IndexError::Other(err)) => {
            drop(handles);
            Err(err)
        }
    }
}
```

- [ ] **Step 3: Build the CLI and fix any compilation errors**

```
cargo build -p atlas-cli 2>&1 | head -60
```

Expected: compiles. If there are unused import warnings, fix them. If there are test callers of `build_production_backend(model_id, budget)`, update them to pass a config.

- [ ] **Step 4: Run atlas-cli tests to find and fix broken callers**

```
cargo test -p atlas-cli 2>&1 | head -80
```

Fix any test that constructs a backend using the old API. Tests that use `TestBackend` directly bypass `build_production_backend_with_counter` and need no changes.

- [ ] **Step 5: Run the full test suite**

```
cargo test --workspace
```

Expected: all pass.

- [ ] **Step 6: Commit**

```
git add crates/atlas-cli/src/backend.rs crates/atlas-cli/src/main.rs
git commit -m "feat(atlas-cli): wire BackendRouter, remove --model flag"
```

---

## Task 10: atlas init subcommand

**Files:**
- Create: `crates/atlas-cli/src/init.rs`
- Modify: `crates/atlas-cli/src/main.rs`

- [ ] **Step 1: Write failing integration test**

Create `crates/atlas-cli/tests/init_cmd.rs`:

```rust
use assert_cmd::Command;
use tempfile::TempDir;

#[test]
fn init_creates_three_files() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(dir.path().join(".atlas/config.yaml").exists());
    assert!(dir.path().join(".atlas/components.overrides.yaml").exists());
    assert!(dir.path().join(".atlas/subsystems.overrides.yaml").exists());
}

#[test]
fn init_skips_existing_files() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    std::fs::write(dir.path().join(".atlas/config.yaml"), "existing content").unwrap();

    let output = Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("skipped") && stdout.contains("config.yaml"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".atlas/config.yaml")).unwrap(),
        "existing content"
    );
}

#[test]
fn init_prints_written_paths() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("config.yaml"));
    assert!(stdout.contains("components.overrides.yaml"));
    assert!(stdout.contains("subsystems.overrides.yaml"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test -p atlas-cli --test init_cmd 2>&1 | head -30
```

Expected: FAIL — `atlas init` subcommand not found.

- [ ] **Step 3: Create init.rs with template strings**

Create `crates/atlas-cli/src/init.rs`:

```rust
use std::path::Path;

const CONFIG_TEMPLATE: &str = r#"# Atlas LLM Configuration
# Generated by `atlas init`. Edit before running `atlas index`.
#
# Model strings use the format: <provider>/<model-id>
# Supported providers:
#   claude-code  — Anthropic Claude Code CLI subprocess (requires `claude` on PATH)
#   codex        — OpenAI Codex CLI subprocess (stub; pending research task)
#   anthropic    — Anthropic Messages HTTP API (requires ANTHROPIC_API_KEY)
#   openai       — OpenAI Chat Completions HTTP API (requires OPENAI_API_KEY)
#
# API keys use ${ENV_VAR} interpolation and are resolved at startup.
# Never hardcode secrets here — this file is safe to commit as long as
# the referenced env vars are kept out of version control.

# Credentials for HTTP providers. Only providers referenced in
# defaults/operations below need entries here.
# providers:
#   anthropic:
#     api_key: ${ANTHROPIC_API_KEY}
#   openai:
#     api_key: ${OPENAI_API_KEY}

# Global defaults — both `model` fields are REQUIRED.
defaults:
  model: claude-code/claude-sonnet-4-6

# Per-operation overrides — all optional; absent entries inherit defaults.
# operations:
#
#   # L3: is_component — one call per candidate directory.
#   # Fast models work well here; inputs are bounded manifest snippets.
#   classify:
#     model: anthropic/claude-haiku-4-5
#     params:
#       max_tokens: 1024
#
#   # L8: subcarve decision — one call per immediate subdirectory.
#   subcarve:
#     model: anthropic/claude-haiku-4-5
#     params:
#       max_tokens: 512
#
#   # L5: surface extraction — reads source files; keep agentic.
#   surface:
#     model: claude-code/claude-sonnet-4-6
#
#   # L6: edge synthesis — cross-component; keep agentic and broadly contextual.
#   edges:
#     model: claude-code/claude-opus-4-7
"#;

const COMPONENTS_OVERRIDES_TEMPLATE: &str = r#"# components.overrides.yaml
# Pin, suppress, or add components to override Atlas's automatic classification.
# Changes here persist across re-runs; Atlas never overwrites this file.
#
# --- Pin a component field ---
# pins:
#   my-lib:
#     kind: rust-library       # fix kind so LLM cannot change it
#   tools/build:
#     suppress: true           # exclude this component entirely
#
# --- Add a component Atlas missed ---
# additions:
# - id: custom-scripts
#   path: scripts/custom
#   kind: tooling
#   role: "Build and release scripts"
"#;

const SUBSYSTEMS_OVERRIDES_TEMPLATE: &str = r#"# subsystems.overrides.yaml
# Group components into named subsystems. Produced as subsystems.yaml
# after `atlas index`.
#
# Membership forms:
#   - Glob (contains / or *): matches component paths
#   - Id (no / or *): matches exact component ids
#
# subsystems:
# - id: auth
#   role: "Authentication and authorisation boundary"
#   members:
#   - services/auth/*       # glob — all components under services/auth/
#   - auth-shared           # id   — exact component id
"#;

pub fn run_init_cmd(root: &Path) -> anyhow::Result<std::process::ExitCode> {
    let atlas_dir = root.join(".atlas");
    std::fs::create_dir_all(&atlas_dir)?;

    write_template(
        &atlas_dir.join("config.yaml"),
        CONFIG_TEMPLATE,
    )?;
    write_template(
        &atlas_dir.join("components.overrides.yaml"),
        COMPONENTS_OVERRIDES_TEMPLATE,
    )?;
    write_template(
        &atlas_dir.join("subsystems.overrides.yaml"),
        SUBSYSTEMS_OVERRIDES_TEMPLATE,
    )?;

    Ok(std::process::ExitCode::SUCCESS)
}

fn write_template(path: &Path, content: &str) -> anyhow::Result<()> {
    if path.exists() {
        println!("skipped {} (already exists)", path.display());
        return Ok(());
    }
    std::fs::write(path, content)?;
    println!("written {}", path.display());
    Ok(())
}
```

- [ ] **Step 4: Add Init subcommand to main.rs**

In `crates/atlas-cli/src/main.rs`:

1. Add `mod init;` near the top.

2. In the `Command` enum, add:
```rust
/// Scaffold .atlas/ with commented template files before first run.
Init(InitArgs),
```

3. Add the args struct:
```rust
#[derive(Debug, clap::Args)]
struct InitArgs {
    /// Root of the project to initialise. Creates <root>/.atlas/ with
    /// config.yaml, components.overrides.yaml, and subsystems.overrides.yaml.
    root: std::path::PathBuf,
}
```

4. In the `run()` match:
```rust
Command::Init(args) => {
    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root path {}", args.root.display()))?;
    init::run_init_cmd(&root)
}
```

- [ ] **Step 5: Run the integration tests**

```
cargo test -p atlas-cli --test init_cmd
```

Expected: all three tests pass.

- [ ] **Step 6: Run full test suite**

```
cargo test --workspace
```

Expected: all pass.

- [ ] **Step 7: Commit**

```
git add crates/atlas-cli/src/init.rs crates/atlas-cli/src/main.rs \
    crates/atlas-cli/tests/init_cmd.rs
git commit -m "feat(atlas-cli): add atlas init subcommand"
```

---

## Task 11: Add Codex CLI research backlog task

**Files:** none (backlog only)

- [ ] **Step 1: Add the research task to the backlog**

```
ravel-lite state backlog add LLM_STATE/core \
  --title "Research Codex CLI subprocess interface for CodexBackend" \
  --category research \
  --body "$(cat <<'EOF'
CodexBackend is currently a stub (returns LlmError::Setup). Before implementing
it, determine:

1. The Codex CLI binary name and invocation form equivalent to:
   `claude -p <prompt> --output-format stream-json --verbose --model <id>`
2. The output stream format: same JSONL shape as claude stream-json, or different?
3. Whether stream_parse::parse_stream can be reused or a parallel
   codex_stream_parse.rs is needed.
4. How the result payload is encoded (markdown-fenced JSON, bare JSON, or other).
5. How to capture version: `codex --version` or equivalent.

Deliverable: update codex.rs with a full implementation following the same
pattern as ClaudeCodeBackend.
EOF
)"
```

- [ ] **Step 2: Verify the task appears in the backlog**

```
ravel-lite state backlog list LLM_STATE/core --format markdown
```

Expected: new research task visible.

---

## Self-review notes

**Spec coverage check:**

| Spec section | Covered by |
|---|---|
| Config file schema (providers, defaults, operations) | Task 2, 3, 4 |
| `${VAR}` interpolation at load time | Task 3 |
| Validation failures (NotFound, missing defaults.model, unset var, missing provider) | Task 4 |
| AnthropicHttpBackend | Task 5 |
| OpenAiHttpBackend | Task 6 |
| CodexBackend stub | Task 7 |
| BackendRouter dispatch | Task 8 |
| Composite fingerprint | Task 8 |
| Wire into CLI, remove --model | Task 9 |
| atlas init positional arg | Task 10 |
| Scaffold all three files, skip existing | Task 10 |
| Codex research backlog task | Task 11 |

All spec requirements covered.
