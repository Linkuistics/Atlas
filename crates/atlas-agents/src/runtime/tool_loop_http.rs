//! Atlas-owned tool-use loop for HTTP backends (plan §4 Task 4.3).
//!
//! HTTP backends (Anthropic, OpenAI) speak the model-provider
//! `tool_use` JSON shape; this module drives the loop in-process and
//! records every assistant turn + tool result into a `Transcript` for
//! cache materialisation downstream.
//!
//! The loop terminates when the model emits no further `tool_use`
//! blocks (interpreted as "done — final answer in the assistant
//! message") or after `max_steps` iterations (interpreted as a runaway
//! conversation; surfaced as `AgentError::MaxStepsExceeded`).
//!
//! ## Wire-shape compatibility
//!
//! Two provider shapes are supported via `Provider`:
//!
//! - **Anthropic** (`Provider::Anthropic`): tool calls live under
//!   `content[i].type == "tool_use"` with `id` / `name` / `input`
//!   fields. Final-answer text lives under `content[i].type == "text"`.
//! - **OpenAI** (`Provider::OpenAi`): tool calls live under
//!   `tool_calls[i].function.{name,arguments}` with the arguments
//!   carrying a JSON-encoded string. Final-answer text lives under
//!   `content` (a string) or `message.content`.
//!
//! Parsing helpers are exposed for unit tests; the production caller
//! is `crate::runtime::AgentRuntime::call_agent`.

use std::collections::HashSet;
use std::path::PathBuf;

use atlas_llm::{LlmBackend, LlmRequest, Provider, ResponseSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::runtime::audit::{AgentOutput, Stage};
use crate::{ToolArgs, ToolContext, ToolResult};

use super::semaphores::Semaphores;
use super::{AgentError, ToolCatalog};

/// One `tool_use` block lifted from a backend response. The args
/// payload is a `serde_json::Value` so the `Tool` impl receives the
/// model's input verbatim.
#[derive(Debug, Clone)]
pub struct ToolUse {
    /// Provider-supplied id (Anthropic `id`; OpenAI `tool_calls[i].id`).
    /// Echoed back in the corresponding `tool_result` block so the
    /// upstream conversation threads correctly.
    pub id: String,
    /// Tool name (matches `Tool::id()`).
    pub name: String,
    /// Args object the model wants the tool invoked with.
    pub args: Value,
}

/// One record in the transcript byte-stream. Lifted to JSON before
/// being framed into transcript-cache bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptRecord {
    /// One assistant turn (a backend response). Stored verbatim for
    /// cache replay.
    AssistantTurn { value: Value },
    /// One tool result. Stored alongside the tool name + id so the
    /// replay can reconstruct the `tool_result` block on the wire.
    /// PR-2 adds `args` so the Lane A evidence-floor scorer can match
    /// the agent's per-call tool inputs against the dispatched candidate
    /// set. `#[serde(default)]` keeps pre-PR-2 cached transcripts
    /// deserialisable (the field defaults to `Value::Null`).
    ToolResult {
        tool_use_id: String,
        tool_name: String,
        #[serde(default)]
        args: Value,
        output: Value,
        bytes: u64,
    },
    /// One MCP-side transcript line, merged from
    /// `mcp_server.drain_client_transcript`.
    McpEvent { value: Value },
}

/// Accumulating record of an in-progress agent invocation. The runtime
/// hands one `Transcript` per agent call; on success it is framed via
/// `into_bytes(grade)` and written to the transcript cache.
///
/// `Clone` is derived so PR-4's Lane B audit closure can render the
/// producer's tool-call trail after `into_bytes` consumes the original
/// — the clone lives inside the audit closure's environment until the
/// audit call resolves.
#[derive(Debug, Default, Clone)]
pub struct Transcript {
    records: Vec<TranscriptRecord>,
}

impl Transcript {
    /// Construct an empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one assistant turn.
    pub fn record_assistant_turn(&mut self, value: &Value) {
        self.records.push(TranscriptRecord::AssistantTurn {
            value: value.clone(),
        });
    }

    /// Record one tool result, correlated with the originating
    /// `tool_use` block. The originating call's args are captured so
    /// Lane A's evidence-floor scorer (PR-2) can introspect which
    /// files / candidates the agent's tool calls referenced.
    pub fn record_tool_result(&mut self, tu: &ToolUse, result: &ToolResult) {
        self.records.push(TranscriptRecord::ToolResult {
            tool_use_id: tu.id.clone(),
            tool_name: tu.name.clone(),
            args: tu.args.clone(),
            output: result.output.clone(),
            bytes: result.bytes,
        });
    }

    /// Append a batch of MCP-side records merged from the MCP server.
    pub fn merge_mcp_events(&mut self, events: impl IntoIterator<Item = Value>) {
        for ev in events {
            self.records.push(TranscriptRecord::McpEvent { value: ev });
        }
    }

    /// Number of records accumulated. Useful in tests.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True iff no records have been accumulated.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Borrow the underlying record list (read-only).
    pub fn records(&self) -> &[TranscriptRecord] {
        &self.records
    }

    /// Append a synthetic `ToolResult` record. Test-only helper used by
    /// the Lane A evidence-floor regression suite (PR-2 Step 2.10) to
    /// build transcripts that mimic an agent having read N candidate
    /// manifests without running a real backend.
    #[doc(hidden)]
    pub fn push_synthetic_tool_call(
        &mut self,
        tool_name: impl Into<String>,
        args: Value,
        output: Value,
    ) {
        self.records.push(TranscriptRecord::ToolResult {
            tool_use_id: String::new(),
            tool_name: tool_name.into(),
            args,
            output,
            bytes: 0,
        });
    }

    /// True iff at least one tool call invoked `tool_id`.
    pub fn tool_called(&self, tool_id: &str) -> bool {
        self.records.iter().any(|r| match r {
            TranscriptRecord::ToolResult { tool_name, .. } => tool_name == tool_id,
            _ => false,
        })
    }

    /// Iterate over every `ToolResult` record whose `tool_name` matches
    /// `tool_id`. Borrowed access to the underlying record — no
    /// allocations.
    pub fn tool_calls_for<'a>(
        &'a self,
        tool_id: &'a str,
    ) -> impl Iterator<Item = &'a TranscriptRecord> + 'a {
        self.records.iter().filter(move |r| match r {
            TranscriptRecord::ToolResult { tool_name, .. } => tool_name == tool_id,
            _ => false,
        })
    }

    /// Set of file paths the agent's tool calls referenced via a
    /// top-level `path` arg. Best-effort: any tool whose args carry a
    /// `path` string field contributes — the Atlas tool catalog's
    /// manifest parsers + surface analysers all follow this shape, so
    /// the heuristic matches them all without naming each tool. Returns
    /// an empty set if no tool call carried a `path` field. Lane A's
    /// dispatch-stage evidence scorer (PR-2) uses this to compute the
    /// reads-vs-candidates ratio against the dispatched candidate set.
    pub fn read_file_paths(&self) -> HashSet<PathBuf> {
        self.records
            .iter()
            .filter_map(|r| match r {
                TranscriptRecord::ToolResult { args, .. } => {
                    args.get("path").and_then(Value::as_str).map(PathBuf::from)
                }
                _ => None,
            })
            .collect()
    }

    /// Frame this transcript as bytes for the transcript cache.
    /// Delegates to `atlas_engine::llm_cache::frame_transcript_with_grade`
    /// so the on-disk frame is symmetric with `parse_transcript_grade`.
    pub fn into_bytes(self, grade: atlas_engine::llm_cache::AgentGrade) -> Vec<u8> {
        let body = serde_json::to_vec(&self.records).unwrap_or_else(|_| b"[]".to_vec());
        atlas_engine::llm_cache::frame_transcript_with_grade(&grade, &body)
    }
}

/// Build the per-step `LlmRequest` for the HTTP tool-use loop. The
/// agent runtime owns prompt rendering, so the `conversation` argument
/// is a complete string and goes verbatim to the backend via
/// `LlmRequest::from_rendered` (WI-1 bypass). `_tools` is kept on the
/// signature for the caller's typed accounting; the HTTP backends
/// don't currently read tool descriptors off the request (separate
/// wiring concern outside WI-1's scope).
pub fn build_llm_request_with_tools(conversation: &str, _tools: &ToolCatalog) -> LlmRequest {
    LlmRequest::from_rendered(conversation.to_string(), ResponseSchema::accept_any())
}

/// Extract `tool_use` blocks from a backend response. Supports both
/// Anthropic and OpenAI wire shapes; falls back to "no tool uses" for
/// any shape we don't recognise (interpreted by the caller as "final
/// answer — exit the loop").
pub fn extract_tool_uses(response: &Value, provider: Provider) -> Vec<ToolUse> {
    match provider {
        Provider::Anthropic => extract_anthropic_tool_uses(response),
        Provider::OpenAi => extract_openai_tool_uses(response),
    }
}

fn extract_anthropic_tool_uses(response: &Value) -> Vec<ToolUse> {
    let Some(content) = response.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            continue;
        };
        let id = block
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = block.get("input").cloned().unwrap_or(Value::Null);
        out.push(ToolUse {
            id,
            name: name.to_string(),
            args,
        });
    }
    out
}

fn extract_openai_tool_uses(response: &Value) -> Vec<ToolUse> {
    // OpenAI wire form has `tool_calls` either at the top level
    // (chat-completions) or under `message` (responses API). Try
    // both — neither side carries a meaningful prefix the other
    // accepts, so the union is unambiguous.
    let tool_calls = response
        .get("tool_calls")
        .or_else(|| response.get("message").and_then(|m| m.get("tool_calls")))
        .and_then(Value::as_array);
    let Some(tool_calls) = tool_calls else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for call in tool_calls {
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let function = match call.get("function") {
            Some(f) => f,
            None => continue,
        };
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            continue;
        };
        // OpenAI emits `arguments` as a JSON-encoded string. Anthropic
        // emits it as an object. Tolerate both for robustness.
        let args = match function.get("arguments") {
            Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
            Some(v) => v.clone(),
            None => Value::Null,
        };
        out.push(ToolUse {
            id,
            name: name.to_string(),
            args,
        });
    }
    out
}

/// Extract the final-answer payload from a backend response. PR-4's
/// minimal contract: return whatever JSON the model emitted as the
/// final assistant turn, wrapped in `AgentOutput`. The Anthropic shape
/// places the textual final answer under `content[i].type == "text"`;
/// OpenAI places it under `content` or `message.content`. For a
/// structured-output stage (Classify / Surface / Reduce / Project),
/// the response typically carries a JSON block the caller pulls from.
///
/// PR-4 implements a generous "anything that looks like JSON wins"
/// rule:
///
/// 1. If `response.output` is present, return that.
/// 2. Otherwise, if `response.content` is an array, concatenate every
///    `text` block and try to parse as JSON; on success, return; on
///    failure return the response verbatim.
/// 3. Fallback: return the raw response.
///
/// Stage-specific shape contracts are PR-5's surface — the structured
/// output prompts ship there.
pub fn parse_final_output(response: &Value) -> AgentOutput {
    if let Some(out) = response.get("output") {
        return AgentOutput::from_value(out.clone());
    }
    if let Some(content) = response.get("content").and_then(Value::as_array) {
        let mut text = String::new();
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
        }
        if !text.is_empty() {
            // PR-2: try YAML fence-extract first so dispatch-stage
            // (and PR-3's classify/surface/reduce/project) outputs
            // populate `value` with the structured envelope. Falls
            // through to the pre-PR-2 JSON-parse branch for backends
            // that still emit raw JSON. `text` carries the raw text
            // so dispatch parsers can re-derive the fenced body.
            if let Ok(body) = crate::runtime::prompt_examples::extract_yaml_fence(&text) {
                if let Ok(parsed) = serde_yaml::from_str::<Value>(body) {
                    return AgentOutput::from_value_and_text(parsed, text);
                }
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                return AgentOutput::from_value_and_text(parsed, text);
            }
            return AgentOutput::from_value_and_text(json!({ "text": text.clone() }), text);
        }
    }
    AgentOutput::from_value(response.clone())
}

/// Drive an HTTP tool-use loop against `backend` until the model
/// stops emitting `tool_use` blocks or `max_steps` is reached.
///
/// Per plan §4 Task 4.3 / brainstorm §6: the runtime owns dispatch
/// (calls `Tool::invoke` directly) and threads the running
/// conversation as a textual transcript through each step. The
/// transcript here is a `String` rather than a structured message
/// list — PR-5 may upgrade to structured turns once the backend
/// surface stabilises around `tool_use`.
#[allow(clippy::too_many_arguments)]
pub async fn run_tool_loop_http(
    backend: &dyn LlmBackend,
    tools: &ToolCatalog,
    ctx: &ToolContext,
    semaphores: &Semaphores,
    stage: Stage,
    provider: Provider,
    initial_prompt: String,
    max_steps: u32,
    transcript: &mut Transcript,
) -> Result<AgentOutput, AgentError> {
    let _stage_permit = semaphores.acquire_stage(stage).await;
    let mut conversation = initial_prompt;
    for _step in 0..max_steps {
        let req = build_llm_request_with_tools(&conversation, tools);
        let response = backend
            .call_async(&req)
            .await
            .map_err(AgentError::from_llm_error)?;
        transcript.record_assistant_turn(&response);

        let tool_uses = extract_tool_uses(&response, provider);
        if tool_uses.is_empty() {
            return Ok(parse_final_output(&response));
        }

        for tu in tool_uses {
            let tool = tools
                .get(&tu.name)
                .ok_or_else(|| AgentError::UnknownTool(tu.name.clone()))?;
            let result = tool
                .invoke(ToolArgs(tu.args.clone()), ctx)
                .await
                .map_err(|e| AgentError::ToolFailure(e.to_string()))?;
            transcript.record_tool_result(&tu, &result);
            conversation.push_str(&format_tool_result_for_conversation(&tu, &result));
        }
    }
    Err(AgentError::MaxStepsExceeded(max_steps))
}

/// Render one `tool_result` block as a textual addition to the
/// running conversation. PR-4's conversation is a single growing
/// string; PR-5 may upgrade to structured turns.
fn format_tool_result_for_conversation(tu: &ToolUse, result: &ToolResult) -> String {
    format!(
        "\n[tool_result name={} id={} output={}]\n",
        tu.name, tu.id, result.output
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transcript_tool_called_finds_recorded_tool() {
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/foo/Cargo.toml" }),
            json!({ "ok": true }),
        );
        assert!(t.tool_called("parse_cargo_toml"));
        assert!(!t.tool_called("nonexistent"));
    }

    #[test]
    fn transcript_tool_calls_for_filters_by_name() {
        let mut t = Transcript::new();
        t.push_synthetic_tool_call("a", json!({}), json!({}));
        t.push_synthetic_tool_call("b", json!({}), json!({}));
        t.push_synthetic_tool_call("a", json!({}), json!({}));
        assert_eq!(t.tool_calls_for("a").count(), 2);
        assert_eq!(t.tool_calls_for("b").count(), 1);
        assert_eq!(t.tool_calls_for("c").count(), 0);
    }

    #[test]
    fn transcript_read_file_paths_collects_path_args() {
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({ "path": "crates/atlas-cli/Cargo.toml" }),
            json!({}),
        );
        t.push_synthetic_tool_call(
            "rust_surface",
            json!({ "path": "crates/atlas-engine/src/lib.rs" }),
            json!({}),
        );
        t.push_synthetic_tool_call(
            "ts_js_classify",
            json!({ "candidate_dir": "frontend" }), // no `path` field
            json!({}),
        );
        let paths = t.read_file_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("crates/atlas-cli/Cargo.toml")));
        assert!(paths.contains(&PathBuf::from("crates/atlas-engine/src/lib.rs")));
    }

    #[test]
    fn transcript_read_file_paths_is_empty_when_no_tool_calls() {
        let t = Transcript::new();
        assert!(t.read_file_paths().is_empty());
    }

    #[test]
    fn extract_tool_uses_handles_anthropic_shape() {
        let response = json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "tool_use", "id": "tu_1", "name": "read", "input": { "path": "a" } },
                { "type": "tool_use", "id": "tu_2", "name": "grep", "input": { "pattern": "x" } }
            ]
        });
        let uses = extract_tool_uses(&response, Provider::Anthropic);
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0].name, "read");
        assert_eq!(uses[0].args, json!({ "path": "a" }));
        assert_eq!(uses[1].name, "grep");
    }

    #[test]
    fn extract_tool_uses_handles_openai_shape() {
        let response = json!({
            "tool_calls": [
                {
                    "id": "call_a",
                    "function": {
                        "name": "read",
                        "arguments": "{\"path\":\"a\"}"
                    }
                }
            ]
        });
        let uses = extract_tool_uses(&response, Provider::OpenAi);
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].name, "read");
        assert_eq!(uses[0].args, json!({ "path": "a" }));
    }

    #[test]
    fn extract_tool_uses_returns_empty_for_unrecognised_shape() {
        let response = json!({ "completion": "no tools here" });
        let uses = extract_tool_uses(&response, Provider::Anthropic);
        assert!(uses.is_empty());
    }

    #[test]
    fn parse_final_output_prefers_output_field() {
        let response = json!({ "output": { "components": [] } });
        let out = parse_final_output(&response);
        assert_eq!(out.value, json!({ "components": [] }));
    }

    #[test]
    fn parse_final_output_parses_text_block_as_json() {
        let response = json!({
            "content": [
                { "type": "text", "text": "{\"foo\":1}" }
            ]
        });
        let out = parse_final_output(&response);
        assert_eq!(out.value, json!({ "foo": 1 }));
    }

    #[test]
    fn transcript_into_bytes_round_trips() {
        let mut tx = Transcript::new();
        tx.record_assistant_turn(&json!({ "ok": 1 }));
        let bytes = tx.into_bytes(atlas_engine::llm_cache::AgentGrade::Strong);
        let (grade, body) =
            atlas_engine::llm_cache::parse_transcript_grade(&bytes).expect("round trip");
        assert_eq!(grade, atlas_engine::llm_cache::AgentGrade::Strong);
        let records: Vec<TranscriptRecord> =
            serde_json::from_slice(&body).expect("transcript body parses");
        assert_eq!(records.len(), 1);
        match &records[0] {
            TranscriptRecord::AssistantTurn { value } => {
                assert_eq!(value, &json!({ "ok": 1 }));
            }
            other => panic!("unexpected record: {other:?}"),
        }
    }

    #[test]
    fn build_llm_request_uses_rendered_bypass() {
        let cat = ToolCatalog::new(std::iter::empty());
        let req = build_llm_request_with_tools("hello", &cat);
        assert_eq!(req.rendered_prompt.as_deref(), Some("hello"));
        assert!(req.prompt_template.is_none());
    }
}
