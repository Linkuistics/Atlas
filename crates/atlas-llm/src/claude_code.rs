//! Subprocess backend: spawns the `claude` CLI per call.
//!
//! The backend caches `claude --version` at construction so that the
//! [`LlmFingerprint`] it exposes is stable across calls within a run.
//! Each call renders the prompt template for `req.prompt_template`
//! against `req.inputs`, shells out to
//! `claude -p <rendered> --output-format json --model <model-id>`,
//! parses stdout as JSON, and validates against `req.schema`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::{prompt, LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, ResponseSchema};

/// Environment variable consulted as the default for the `--model`
/// argument passed to `claude -p`. If unset, [`DEFAULT_MODEL_ID`] is
/// used.
pub const MODEL_ID_ENV: &str = "ATLAS_LLM_MODEL";

/// Default model id used when no override is supplied. The atlas-cli
/// driver is expected to override this based on the user's flag.
pub const DEFAULT_MODEL_ID: &str = "claude-sonnet-4-6";

pub struct ClaudeCodeBackend {
    model_id: String,
    prompts_dir: PathBuf,
    version: String,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
}

impl ClaudeCodeBackend {
    /// Construct a backend bound to the given model id and prompt
    /// directory. Runs `claude --version` eagerly so a missing or
    /// broken `claude` binary fails construction, not the first
    /// call.
    pub fn new(
        model_id: impl Into<String>,
        prompts_dir: impl Into<PathBuf>,
    ) -> Result<Self, LlmError> {
        let model_id = model_id.into();
        let prompts_dir = prompts_dir.into();
        let version = capture_claude_version()?;
        Ok(Self {
            model_id,
            prompts_dir,
            version,
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
        })
    }

    /// Populate the `template_sha` / `ontology_sha` fields that
    /// downstream fingerprint consumers use as a memoisation key.
    /// The driver owns this computation because it has access to
    /// the rendered prompt corpus and the canonical ontology YAML.
    pub fn with_fingerprint_inputs(
        mut self,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
    ) -> Self {
        self.template_sha = template_sha;
        self.ontology_sha = ontology_sha;
        self
    }

    fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
        let template = std::fs::read_to_string(self.prompt_template_path(req.prompt_template))
            .map_err(|e| {
                LlmError::Invocation(format!(
                    "failed to read prompt template `{:?}` from {}: {e}",
                    req.prompt_template,
                    self.prompts_dir.display()
                ))
            })?;
        let tokens = extract_tokens(&req.inputs)?;
        prompt::render(&template, &tokens)
    }

    fn prompt_template_path(&self, id: PromptId) -> PathBuf {
        self.prompts_dir.join(prompt_template_filename(id))
    }
}

pub(crate) fn prompt_template_filename(id: PromptId) -> &'static str {
    match id {
        PromptId::Classify => "classify.md",
        PromptId::Subcarve => "subcarve.md",
        PromptId::Stage1Surface => "stage1-surface.md",
        PromptId::Stage2Edges => "stage2-edges.md",
    }
}

fn capture_claude_version() -> Result<String, LlmError> {
    let output = Command::new("claude")
        .arg("--version")
        .output()
        .map_err(|e| {
            LlmError::Setup(format!(
                "`claude` binary not available on PATH (required for ClaudeCodeBackend): {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(LlmError::Setup(format!(
            "`claude --version` exited with status {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Flatten a JSON object into the `{{TOKEN}}` substitution map used
/// by `prompt::render`. String-valued fields are passed through
/// verbatim; other shapes are JSON-encoded so they can still appear
/// in a prompt (e.g. a list of strings rendered as a JSON array).
pub(crate) fn extract_tokens(inputs: &Value) -> Result<BTreeMap<String, String>, LlmError> {
    let object = inputs.as_object().ok_or_else(|| {
        LlmError::Invocation(
            "LLM inputs must be a JSON object at the top level so fields can map to template tokens"
                .to_string(),
        )
    })?;
    let mut tokens = BTreeMap::new();
    for (key, value) in object {
        let rendered = match value {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        tokens.insert(key.clone(), rendered);
    }
    Ok(tokens)
}

/// Minimal structural validation against a JSON Schema fragment. The
/// engine does not depend on a fully-featured JSON Schema validator
/// — it depends on LLM output matching a small, curated set of
/// response shapes — so this check covers the subset used by Atlas:
///
/// - `"type": "object"` — the value must be an object.
/// - `"required": ["field", ...]` — each listed field must be
///   present on the value's top-level object.
///
/// Anything else in the schema is accepted. A full JSON Schema
/// dependency is deferred; see `README.md` for the rationale.
pub(crate) fn validate_response(value: &Value, schema: &ResponseSchema) -> Result<(), LlmError> {
    let schema_obj = match schema.0.as_object() {
        Some(obj) => obj,
        None => return Ok(()),
    };

    if let Some(expected_type) = schema_obj.get("type").and_then(|v| v.as_str()) {
        let actual_ok = match expected_type {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !actual_ok {
            return Err(LlmError::Schema(format!(
                "expected value of type {expected_type}, got {}",
                type_name(value)
            )));
        }
    }

    if let Some(required) = schema_obj.get("required").and_then(|v| v.as_array()) {
        let obj = value.as_object().ok_or_else(|| {
            LlmError::Schema(
                "schema declares required fields but value is not an object".to_string(),
            )
        })?;
        for field in required {
            if let Some(name) = field.as_str() {
                if !obj.contains_key(name) {
                    return Err(LlmError::Schema(format!(
                        "required field `{name}` missing from response"
                    )));
                }
            }
        }
    }

    Ok(())
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl LlmBackend for ClaudeCodeBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered_prompt = self.render_request(req)?;
        let output = Command::new("claude")
            .arg("-p")
            .arg(&rendered_prompt)
            .arg("--output-format")
            .arg("json")
            .arg("--model")
            .arg(&self.model_id)
            .output()
            .map_err(|e| {
                LlmError::Invocation(format!(
                    "failed to spawn `claude`: {e} (is it still on PATH?)"
                ))
            })?;
        if !output.status.success() {
            return Err(LlmError::Invocation(format!(
                "`claude` exited with status {}: stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: Value = serde_json::from_str(&stdout).map_err(|e| {
            LlmError::Parse(format!(
                "`claude` stdout was not valid JSON: {e} (first 200 bytes: {:?})",
                stdout.chars().take(200).collect::<String>()
            ))
        })?;
        validate_response(&value, &req.schema)?;
        Ok(value)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: self.template_sha,
            ontology_sha: self.ontology_sha,
            model_id: self.model_id.clone(),
            backend_version: self.version.clone(),
        }
    }
}

/// Resolve the model-id default used when no explicit model is
/// specified: first check `ATLAS_LLM_MODEL`, fall back to
/// [`DEFAULT_MODEL_ID`].
pub fn resolve_default_model_id() -> String {
    std::env::var(MODEL_ID_ENV).unwrap_or_else(|_| DEFAULT_MODEL_ID.to_string())
}

/// Look up the filename for a prompt id under the bundled
/// `defaults/prompts/` directory. Exposed so callers can pre-render
/// the whole template corpus to compute `template_sha`.
pub fn prompt_filename(id: PromptId) -> &'static str {
    prompt_template_filename(id)
}

/// Verify the given directory contains a prompt file for every
/// [`PromptId`]. Returned for driver use so atlas-cli can fail
/// loudly with a missing-file list rather than deferring until the
/// first call.
pub fn check_prompts_dir(prompts_dir: &Path) -> Result<(), LlmError> {
    let missing: Vec<_> = [
        PromptId::Classify,
        PromptId::Subcarve,
        PromptId::Stage1Surface,
        PromptId::Stage2Edges,
    ]
    .iter()
    .filter_map(|id| {
        let path = prompts_dir.join(prompt_template_filename(*id));
        if path.exists() {
            None
        } else {
            Some(path.display().to_string())
        }
    })
    .collect();
    if !missing.is_empty() {
        return Err(LlmError::Setup(format!(
            "missing prompt files under {}: {}",
            prompts_dir.display(),
            missing.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_tokens_flattens_string_fields() {
        let inputs = json!({
            "COMPONENT_ID": "crates/atlas-llm",
            "KIND": "rust-library"
        });

        let tokens = extract_tokens(&inputs).unwrap();

        assert_eq!(tokens.get("COMPONENT_ID").unwrap(), "crates/atlas-llm");
        assert_eq!(tokens.get("KIND").unwrap(), "rust-library");
    }

    #[test]
    fn extract_tokens_json_encodes_non_string_values() {
        let inputs = json!({
            "LIST": ["a", "b"],
            "N": 7
        });

        let tokens = extract_tokens(&inputs).unwrap();

        assert_eq!(tokens.get("LIST").unwrap(), r#"["a","b"]"#);
        assert_eq!(tokens.get("N").unwrap(), "7");
    }

    #[test]
    fn extract_tokens_requires_object_root() {
        let inputs = json!(["not", "an", "object"]);

        let err = extract_tokens(&inputs).unwrap_err();

        assert!(matches!(err, LlmError::Invocation(_)));
    }

    #[test]
    fn validate_response_accepts_matching_object() {
        let value = json!({ "kind": "rust-library" });
        let schema = ResponseSchema(json!({
            "type": "object",
            "required": ["kind"]
        }));

        validate_response(&value, &schema).expect("valid");
    }

    #[test]
    fn validate_response_rejects_wrong_type() {
        let value = json!([1, 2, 3]);
        let schema = ResponseSchema(json!({ "type": "object" }));

        let err = validate_response(&value, &schema).unwrap_err();

        assert!(matches!(err, LlmError::Schema(_)));
    }

    #[test]
    fn validate_response_rejects_missing_required_field() {
        let value = json!({ "kind": "rust-library" });
        let schema = ResponseSchema(json!({
            "type": "object",
            "required": ["kind", "language"]
        }));

        let err = validate_response(&value, &schema).unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("language"),
            "expected error to name `language`, got {msg:?}"
        );
    }

    #[test]
    fn validate_response_with_accept_any_schema_permits_anything() {
        let schema = ResponseSchema::accept_any();

        validate_response(&json!(null), &schema).expect("null ok");
        validate_response(&json!({ "x": 1 }), &schema).expect("object ok");
        validate_response(&json!([1, 2]), &schema).expect("array ok");
    }

    #[test]
    fn prompt_template_filenames_match_spec() {
        assert_eq!(prompt_template_filename(PromptId::Classify), "classify.md");
        assert_eq!(prompt_template_filename(PromptId::Subcarve), "subcarve.md");
        assert_eq!(
            prompt_template_filename(PromptId::Stage1Surface),
            "stage1-surface.md"
        );
        assert_eq!(
            prompt_template_filename(PromptId::Stage2Edges),
            "stage2-edges.md"
        );
    }

    #[test]
    fn check_prompts_dir_lists_missing_files() {
        let empty = tempfile::tempdir().unwrap();

        let err = check_prompts_dir(empty.path()).unwrap_err();

        let msg = err.to_string();
        for expected in [
            "classify.md",
            "subcarve.md",
            "stage1-surface.md",
            "stage2-edges.md",
        ] {
            assert!(
                msg.contains(expected),
                "expected error to mention missing `{expected}`, got {msg:?}"
            );
        }
    }

    #[test]
    fn check_prompts_dir_accepts_complete_directory() {
        let dir = tempfile::tempdir().unwrap();
        for id in [
            PromptId::Classify,
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ] {
            std::fs::write(dir.path().join(prompt_template_filename(id)), "stub").unwrap();
        }

        check_prompts_dir(dir.path()).expect("directory is complete");
    }

    #[test]
    fn resolve_default_model_id_falls_back_to_constant_when_env_unset() {
        // Save and clear the env var for the scope of this test.
        let saved = std::env::var(MODEL_ID_ENV).ok();
        // SAFETY: test binary runs single-threaded within this test
        // module in default cargo configuration; we restore below.
        unsafe {
            std::env::remove_var(MODEL_ID_ENV);
        }

        let got = resolve_default_model_id();

        assert_eq!(got, DEFAULT_MODEL_ID);

        if let Some(v) = saved {
            unsafe {
                std::env::set_var(MODEL_ID_ENV, v);
            }
        }
    }

    // Integration-style tests that spawn the real `claude` binary.
    // Gated by `ATLAS_LLM_RUN_CLAUDE_TESTS=1` so contributors without
    // `claude` installed aren't blocked. Use `cargo test -p atlas-llm
    // -- --ignored` + the env var to opt in.

    fn claude_tests_enabled() -> bool {
        std::env::var("ATLAS_LLM_RUN_CLAUDE_TESTS").ok().as_deref() == Some("1")
    }

    #[test]
    #[ignore = "spawns `claude`; opt in with ATLAS_LLM_RUN_CLAUDE_TESTS=1"]
    fn construction_succeeds_when_claude_is_on_path() {
        if !claude_tests_enabled() {
            return;
        }
        let prompts = tempfile::tempdir().unwrap();
        ClaudeCodeBackend::new(DEFAULT_MODEL_ID, prompts.path())
            .expect("claude binary should be discoverable");
    }

    #[test]
    #[ignore = "spawns `claude`; opt in with ATLAS_LLM_RUN_CLAUDE_TESTS=1"]
    fn call_roundtrips_json_response() {
        if !claude_tests_enabled() {
            return;
        }
        let prompts = tempfile::tempdir().unwrap();
        std::fs::write(
            prompts.path().join("classify.md"),
            "Return JSON {\"ok\": true} and nothing else.",
        )
        .unwrap();
        // Other templates must exist for construction if we extend
        // the API later; for this test we only call Classify.
        for id in [
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ] {
            std::fs::write(prompts.path().join(prompt_template_filename(id)), "stub").unwrap();
        }

        let backend = ClaudeCodeBackend::new(DEFAULT_MODEL_ID, prompts.path()).unwrap();
        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({}),
            schema: ResponseSchema(json!({
                "type": "object",
                "required": ["ok"]
            })),
        };

        let response = backend.call(&req).expect("claude call");

        assert_eq!(response, json!({ "ok": true }));
    }
}
