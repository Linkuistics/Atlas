//! Subprocess backend: spawns the `codex` CLI per call.
//!
//! Structurally parallel to [`crate::ClaudeCodeBackend`]. Per-call shape is
//! `codex exec --json --skip-git-repo-check --ephemeral --sandbox read-only
//!  --model <id> -- <rendered-prompt>`. Stream events are walked via
//! [`crate::codex_stream::parse_codex_stream`] which picks the last
//! `item.completed` whose `item.type == "agent_message"` for the final
//! payload. `--output-schema` is intentionally not passed: every Atlas
//! production call site uses [`crate::ResponseSchema::accept_any`] today,
//! so the OpenAI structured-output dialect's strict-mode constraints
//! (e.g. `additionalProperties: false`, exhaustive `required`) would
//! either reject all calls or require a schema-rewriter. Defense in depth
//! happens via [`validate_response`] after parsing.
//!
//! See `docs/superpowers/specs/2026-05-02-codex-backend-research.md` for
//! the full subprocess vs HTTP vs `codex-core` rationale.
//!
//! Auth model mirrors `claude-code/`: the binary must be logged in
//! (`codex login` or `printenv OPENAI_API_KEY | codex login --with-api-key`)
//! before Atlas calls it. Construction runs `codex login status` eagerly
//! so an unauthenticated install fails fast with a remediation hint.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use serde_json::Value;

use crate::agent_observer::AgentObserver;
use crate::claude_code::{extract_tokens, prompt_template_filename, validate_response};
use crate::codex_stream::parse_codex_stream;
use crate::{prompt, LlmBackend, LlmError, LlmFingerprint, LlmRequest};

pub struct CodexBackend {
    model_id: String,
    prompts_dir: PathBuf,
    version: String,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    observer: Option<Arc<dyn AgentObserver>>,
}

impl CodexBackend {
    /// Construct a backend bound to the given model id and prompt
    /// directory. Runs `codex --version` and `codex login status`
    /// eagerly so a missing binary or unauthenticated install fails
    /// construction, not the first call.
    pub fn new(
        model_id: impl Into<String>,
        prompts_dir: impl Into<PathBuf>,
    ) -> Result<Self, LlmError> {
        let model_id = model_id.into();
        let prompts_dir = prompts_dir.into();
        let version = capture_codex_version()?;
        check_codex_auth()?;
        Ok(Self {
            model_id,
            prompts_dir,
            version,
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            observer: None,
        })
    }

    /// Populate the `template_sha` / `ontology_sha` fields that
    /// downstream fingerprint consumers use as a memoisation key. The
    /// driver owns this computation because it has access to the
    /// rendered prompt corpus and the canonical ontology YAML.
    pub fn with_fingerprint_inputs(
        mut self,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
    ) -> Self {
        self.template_sha = template_sha;
        self.ontology_sha = ontology_sha;
        self
    }

    /// Attach a side-channel observer that receives `AgentEvent`s while
    /// the streaming subprocess is running. When `None` (the default),
    /// stream events are parsed and discarded.
    pub fn with_observer(mut self, observer: Arc<dyn AgentObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
        let path = self
            .prompts_dir
            .join(prompt_template_filename(req.prompt_template));
        let template = std::fs::read_to_string(&path).map_err(|e| {
            LlmError::Invocation(format!(
                "failed to read prompt template `{:?}` from {}: {e}",
                req.prompt_template,
                self.prompts_dir.display()
            ))
        })?;
        let tokens = extract_tokens(&req.inputs)?;
        prompt::render(&template, &tokens)
    }
}

fn capture_codex_version() -> Result<String, LlmError> {
    let output = Command::new("codex")
        .arg("--version")
        .output()
        .map_err(|e| {
            LlmError::Setup(format!(
                "`codex` binary not available on PATH (required for CodexBackend): {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(LlmError::Setup(format!(
            "`codex --version` exited with status {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn check_codex_auth() -> Result<(), LlmError> {
    let output = Command::new("codex")
        .args(["login", "status"])
        .output()
        .map_err(|e| {
            LlmError::Setup(format!(
                "`codex` binary not available on PATH (required for CodexBackend): {e}"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LlmError::Setup(format!(
            "codex CLI is not authenticated. Run:\n  \
             printenv OPENAI_API_KEY | codex login --with-api-key\nor:\n  \
             codex login\n(`codex login status` reported: {})",
            stderr.trim()
        )));
    }
    Ok(())
}

impl LlmBackend for CodexBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered_prompt = self.render_request(req)?;
        let mut child = Command::new("codex")
            .arg("exec")
            .arg("--json")
            .arg("--skip-git-repo-check")
            .arg("--ephemeral")
            .arg("--sandbox")
            .arg("read-only")
            .arg("--model")
            .arg(&self.model_id)
            .arg("--")
            .arg(&rendered_prompt)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                LlmError::Invocation(format!(
                    "failed to spawn `codex`: {e} (is it still on PATH?)"
                ))
            })?;

        // Drain stderr in a worker thread so the child does not block
        // on a full stderr pipe buffer while we read stdout. Same
        // pattern as ClaudeCodeBackend.
        let stderr_pipe = child.stderr.take().expect("stderr piped");
        let stderr_handle = std::thread::spawn(move || -> Vec<u8> {
            use std::io::Read;
            let mut buf = Vec::new();
            let mut reader = stderr_pipe;
            let _ = reader.read_to_end(&mut buf);
            buf
        });

        let stdout_pipe = child.stdout.take().expect("stdout piped");
        let parsed = parse_codex_stream(stdout_pipe, self.observer.as_ref(), req.prompt_template);

        let status = child
            .wait()
            .map_err(|e| LlmError::Invocation(format!("failed to wait on `codex`: {e}")))?;
        let stderr_bytes = stderr_handle
            .join()
            .unwrap_or_else(|_| b"<stderr drainer panicked>".to_vec());

        let value = match parsed {
            Ok(v) => v,
            Err(LlmError::Parse(msg)) if !status.success() => {
                let stderr_snippet = String::from_utf8_lossy(&stderr_bytes);
                return Err(LlmError::Invocation(format!(
                    "`codex` exited with status {} before emitting an agent_message: {msg}; stderr={}",
                    status,
                    stderr_snippet.trim()
                )));
            }
            Err(e) => return Err(e),
        };

        validate_response(&value, &req.schema)?;
        Ok(value)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: self.template_sha,
            ontology_sha: self.ontology_sha,
            model_id: self.model_id.clone(),
            backend_version: format!("codex/{}", self.version),
        }
    }
}

/// Verify the given directory contains a prompt file for every
/// [`PromptId`]. Re-exported separately so callers (atlas-cli) can
/// fail loudly with a missing-file list rather than deferring until
/// the first call. Mirrors [`crate::claude_code::check_prompts_dir`].
pub fn check_prompts_dir(prompts_dir: &std::path::Path) -> Result<(), LlmError> {
    crate::claude_code::check_prompts_dir(prompts_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PromptId, ResponseSchema};
    use serde_json::json;

    fn codex_tests_enabled() -> bool {
        std::env::var("ATLAS_LLM_RUN_CODEX_TESTS").ok().as_deref() == Some("1")
    }

    #[test]
    fn fingerprint_includes_model_id_and_codex_prefix() {
        // Direct struct construction sidesteps the `codex --version` /
        // `codex login status` invocations so the test runs without
        // the binary present.
        let backend = CodexBackend {
            model_id: "gpt-5".to_string(),
            prompts_dir: PathBuf::from("/nonexistent"),
            version: "codex-cli 0.125.0".to_string(),
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            observer: None,
        };

        let fp = backend.fingerprint();

        assert_eq!(fp.model_id, "gpt-5");
        assert_eq!(fp.backend_version, "codex/codex-cli 0.125.0");
        assert_eq!(fp.template_sha, [1u8; 32]);
        assert_eq!(fp.ontology_sha, [2u8; 32]);
    }

    #[test]
    fn render_request_substitutes_tokens_into_template() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("classify.md"),
            "Classify {{COMPONENT_ID}} of kind {{KIND}}.",
        )
        .unwrap();

        let backend = CodexBackend {
            model_id: "gpt-5".to_string(),
            prompts_dir: dir.path().to_path_buf(),
            version: "codex-cli 0.125.0".to_string(),
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            observer: None,
        };

        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({"COMPONENT_ID": "crates/atlas-llm", "KIND": "rust-library"}),
            schema: ResponseSchema::accept_any(),
        };

        let rendered = backend.render_request(&req).unwrap();

        assert_eq!(rendered, "Classify crates/atlas-llm of kind rust-library.");
    }

    #[test]
    fn render_request_errors_when_template_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = CodexBackend {
            model_id: "gpt-5".to_string(),
            prompts_dir: dir.path().to_path_buf(),
            version: "codex-cli 0.125.0".to_string(),
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            observer: None,
        };

        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({}),
            schema: ResponseSchema::accept_any(),
        };

        let err = backend.render_request(&req).unwrap_err();
        assert!(
            matches!(err, LlmError::Invocation(_)),
            "expected Invocation, got {err:?}"
        );
    }

    // Integration-style tests that spawn the real `codex` binary.
    // Gated by `ATLAS_LLM_RUN_CODEX_TESTS=1` so contributors without
    // `codex` installed aren't blocked. Use `cargo test -p atlas-llm
    // -- --ignored` + the env var to opt in.

    /// Resolve the model id used by gated `codex` integration tests.
    /// Allows env override because Codex model availability differs by
    /// auth mode (ChatGPT-account auth permits a different set than
    /// API-key auth). Default `gpt-5-codex` is the codex CLI default
    /// for coding-agent workloads.
    fn integration_test_model() -> String {
        std::env::var("ATLAS_LLM_CODEX_TEST_MODEL").unwrap_or_else(|_| "gpt-5-codex".to_string())
    }

    #[test]
    #[ignore = "spawns `codex`; opt in with ATLAS_LLM_RUN_CODEX_TESTS=1"]
    fn construction_succeeds_when_codex_is_on_path_and_authenticated() {
        if !codex_tests_enabled() {
            return;
        }
        let prompts = tempfile::tempdir().unwrap();
        CodexBackend::new(integration_test_model(), prompts.path())
            .expect("codex binary should be discoverable and authenticated");
    }

    #[test]
    #[ignore = "spawns `codex`; opt in with ATLAS_LLM_RUN_CODEX_TESTS=1"]
    fn call_roundtrips_json_response() {
        if !codex_tests_enabled() {
            return;
        }
        let prompts = tempfile::tempdir().unwrap();
        std::fs::write(
            prompts.path().join("classify.md"),
            "Reply with the JSON literal {\"ok\": true} and nothing else.",
        )
        .unwrap();
        for id in [
            PromptId::Subcarve,
            PromptId::Stage1Surface,
            PromptId::Stage2Edges,
        ] {
            std::fs::write(prompts.path().join(prompt_template_filename(id)), "stub").unwrap();
        }

        let backend = CodexBackend::new(integration_test_model(), prompts.path())
            .expect("codex must be authenticated for this test");
        let req = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs: json!({}),
            schema: ResponseSchema(json!({
                "type": "object",
                "required": ["ok"]
            })),
        };

        let response = backend.call(&req).expect("codex call");

        assert_eq!(response, json!({"ok": true}));
    }
}
