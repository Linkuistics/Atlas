//! Pure parsing for `claude -p --output-format stream-json --verbose`
//! transcripts. The driver lives in `claude_code.rs`; this module is
//! kept stateless and synchronous so it can be exercised with recorded
//! JSONL fixtures and zero subprocess overhead.
//!
//! Spec: `docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md` §6.

use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;

use serde_json::Value;

use crate::agent_observer::{tool_summary_for, AgentEvent, AgentObserver};
use crate::{LlmError, PromptId};

/// RAII guard that fires `AgentEvent::CallEnd` on drop, ensuring the
/// terminal event reaches the observer in every termination path of
/// `parse_stream` — success, error, or panic. Spec §7.5.
pub(crate) struct ObserverGuard<'a> {
    observer: Option<&'a Arc<dyn AgentObserver>>,
}

impl<'a> ObserverGuard<'a> {
    pub(crate) fn new(observer: Option<&'a Arc<dyn AgentObserver>>) -> Self {
        Self { observer }
    }
}

impl Drop for ObserverGuard<'_> {
    fn drop(&mut self) {
        if let Some(o) = self.observer {
            o.on_event(AgentEvent::CallEnd);
        }
    }
}

/// Walk an assistant-typed event's `message.content[]` array and emit
/// one `AgentEvent::ToolUse` per `tool_use` block. `thinking` and
/// `text` blocks are skipped (memory: stream-json content[] mixes
/// thinking/tool_use/text per event).
pub(crate) fn extract_tool_uses(value: &Value, observer: Option<&Arc<dyn AgentObserver>>) {
    let Some(o) = observer else { return };
    let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            continue;
        }
        let name = block
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let empty_input = Value::Object(serde_json::Map::new());
        let input = block.get("input").unwrap_or(&empty_input);
        let summary = tool_summary_for(&name, input);
        o.on_event(AgentEvent::ToolUse { name, summary });
    }
}

/// Walk a user-typed event's `message.content[]` array and emit one
/// `AgentEvent::ToolResult` per `tool_result` block. `is_error` is
/// absent on success (memory:
/// stream-json-tool-result-is-error-absent-on-success).
pub(crate) fn extract_tool_results(value: &Value, observer: Option<&Arc<dyn AgentObserver>>) {
    let Some(o) = observer else { return };
    let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let is_error = block
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        o.on_event(AgentEvent::ToolResult { ok: !is_error });
    }
}

/// Strip the ` ```json … ``` ` markdown fence that `claude -p`
/// reliably wraps around the final `result.result` string. Returns the
/// inner JSON text. If no fence is present, the input is returned as
/// is. Memory: stream-json-result-result-is-markdown-fenced-json.
pub(crate) fn strip_json_fence(s: &str) -> &str {
    let trimmed = s.trim();
    let after_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(str::trim_start)
        .unwrap_or(trimmed);
    after_open
        .strip_suffix("```")
        .map(str::trim)
        .unwrap_or(after_open)
}

/// Drive a `BufRead` of `claude -p --output-format stream-json --verbose`
/// JSONL output. Emits `AgentEvent`s through `observer` and returns the
/// JSON value carried in the terminal `result.result` field.
///
/// Unknown event types and non-JSON lines are skipped — only the
/// terminal `result` event is load-bearing (spec §7.4 forward-compat).
///
/// `CallStart` fires once at the top; `CallEnd` is guaranteed to fire
/// via `ObserverGuard` on every termination path.
pub(crate) fn parse_stream<R: Read>(
    reader: R,
    observer: Option<&Arc<dyn AgentObserver>>,
    prompt: PromptId,
) -> Result<Value, LlmError> {
    let _guard = ObserverGuard::new(observer);
    if let Some(o) = observer {
        o.on_event(AgentEvent::CallStart { prompt });
    }

    let mut terminal: Option<Value> = None;
    let buf = BufReader::new(reader);
    for line in buf.lines() {
        let line = line.map_err(|e| LlmError::Invocation(format!("stdout read failure: {e}")))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue; // non-JSON noise
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => extract_tool_uses(&value, observer),
            Some("user") => extract_tool_results(&value, observer),
            Some("result") => terminal = Some(value),
            _ => {} // forward-compat: skip unknown event types
        }
    }

    let terminal = terminal.ok_or_else(|| {
        LlmError::Parse(
            "stream ended without `result` event; subprocess may have crashed before emitting a terminal frame"
                .to_string(),
        )
    })?;

    let subtype = terminal
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if subtype != "success" {
        let detail = terminal
            .get("errors")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let msg = if detail.is_empty() {
            format!("claude reported {subtype}")
        } else {
            format!("claude reported {subtype}: {detail}")
        };
        return Err(LlmError::Invocation(msg));
    }

    let result_text = terminal
        .get("result")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            LlmError::Parse("claude `result` event missing `result` string field".to_string())
        })?;
    let stripped = strip_json_fence(result_text);
    let value: Value = serde_json::from_str(stripped).map_err(|e| {
        LlmError::Parse(format!(
            "claude `result.result` was not valid JSON: {e} (first 200 bytes: {:?})",
            stripped.chars().take(200).collect::<String>()
        ))
    })?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PromptId;
    use serde_json::json;
    use std::io::Cursor;
    use std::sync::Mutex;

    /// Minimal observer that records events to a shared Vec for
    /// assertions.
    pub(crate) struct RecordingObserver {
        pub events: Mutex<Vec<AgentEvent>>,
    }

    impl RecordingObserver {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
            })
        }

        pub fn names(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|e| match e {
                    AgentEvent::CallStart { .. } => "CallStart".into(),
                    AgentEvent::ToolUse { name, .. } => format!("ToolUse:{name}"),
                    AgentEvent::ToolResult { ok } => format!("ToolResult:{ok}"),
                    AgentEvent::CallEnd => "CallEnd".into(),
                })
                .collect()
        }
    }

    impl AgentObserver for RecordingObserver {
        fn on_event(&self, event: AgentEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn jsonl_bytes(events: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in events {
            out.extend_from_slice(serde_json::to_string(e).unwrap().as_bytes());
            out.push(b'\n');
        }
        out
    }

    #[test]
    fn extract_tool_uses_emits_one_event_per_tool_use_block() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let value = json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "thinking", "thinking": "..."},
                    {"type": "tool_use", "name": "Read",
                     "input": {"file_path": "/tmp/x.rs"}},
                    {"type": "text", "text": "..."},
                    {"type": "tool_use", "name": "Grep",
                     "input": {"pattern": "foo"}}
                ]
            }
        });

        extract_tool_uses(&value, Some(&observer_dyn));

        assert_eq!(
            observer.names(),
            vec!["ToolUse:Read".to_string(), "ToolUse:Grep".to_string()]
        );
    }

    #[test]
    fn extract_tool_uses_skips_when_observer_none() {
        let value = json!({"type": "assistant"});
        extract_tool_uses(&value, None);
    }

    #[test]
    fn extract_tool_results_treats_absent_is_error_as_ok_true() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let value = json!({
            "type": "user",
            "message": {
                "content": [
                    {"type": "tool_result", "content": "..."}
                ]
            }
        });

        extract_tool_results(&value, Some(&observer_dyn));

        assert_eq!(observer.names(), vec!["ToolResult:true".to_string()]);
    }

    #[test]
    fn extract_tool_results_treats_is_error_true_as_ok_false() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let value = json!({
            "type": "user",
            "message": {
                "content": [
                    {"type": "tool_result", "is_error": true}
                ]
            }
        });

        extract_tool_results(&value, Some(&observer_dyn));

        assert_eq!(observer.names(), vec!["ToolResult:false".to_string()]);
    }

    #[test]
    fn strip_json_fence_handles_markdown_wrapped_json() {
        assert_eq!(
            strip_json_fence("```json\n{\"ok\":true}\n```"),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn strip_json_fence_handles_bare_json() {
        assert_eq!(strip_json_fence("{\"ok\":true}"), "{\"ok\":true}");
    }

    #[test]
    fn strip_json_fence_handles_fence_without_lang() {
        assert_eq!(strip_json_fence("```\n{\"ok\":true}\n```"), "{\"ok\":true}");
    }

    #[test]
    fn strip_json_fence_handles_surrounding_whitespace() {
        assert_eq!(
            strip_json_fence("   ```json\n  {\"ok\":true}  \n```   "),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn parse_stream_no_tools_returns_terminal_result_value() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "system", "subtype": "init"}),
            json!({
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "..."}]}
            }),
            json!({
                "type": "result", "subtype": "success",
                "result": "```json\n{\"ok\": true}\n```"
            }),
        ]);

        let response =
            parse_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify).expect("ok");

        assert_eq!(response, json!({"ok": true}));
        assert_eq!(
            observer.names(),
            vec!["CallStart".to_string(), "CallEnd".to_string()]
        );
    }

    #[test]
    fn parse_stream_with_tools_emits_tool_use_and_tool_result_events() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({
                "type": "assistant",
                "message": {"content": [
                    {"type": "tool_use", "name": "Read",
                     "input": {"file_path": "/tmp/x"}}
                ]}
            }),
            json!({
                "type": "user",
                "message": {"content": [
                    {"type": "tool_result", "content": "..."}
                ]}
            }),
            json!({
                "type": "result", "subtype": "success",
                "result": "```json\n{\"package_name\": \"x\"}\n```"
            }),
        ]);

        parse_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Subcarve).expect("ok");

        assert_eq!(
            observer.names(),
            vec![
                "CallStart".to_string(),
                "ToolUse:Read".to_string(),
                "ToolResult:true".to_string(),
                "CallEnd".to_string(),
            ]
        );
    }

    #[test]
    fn parse_stream_terminal_subtype_not_success_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[json!({
            "type": "result",
            "subtype": "error_max_budget_usd",
            "is_error": true
        })]);

        let err = parse_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Subcarve)
            .expect_err("non-success subtype must error");

        match err {
            crate::LlmError::Invocation(msg) => {
                assert!(msg.contains("error_max_budget_usd"), "got: {msg}");
            }
            other => panic!("expected Invocation, got {other:?}"),
        }
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }

    #[test]
    fn parse_stream_missing_terminal_returns_parse_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[json!({"type": "system", "subtype": "init"})]);

        let err = parse_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
            .expect_err("no terminal => Parse error");

        assert!(matches!(err, crate::LlmError::Parse(_)));
    }

    #[test]
    fn parse_stream_skips_non_json_lines() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"this is not json\n");
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "result", "subtype": "success",
                "result": "```json\n{\"ok\": true}\n```"
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');

        let response =
            parse_stream(Cursor::new(bytes), Some(&observer_dyn), PromptId::Classify).expect("ok");

        assert_eq!(response, json!({"ok": true}));
    }

    #[test]
    fn parse_stream_skips_unknown_event_types() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "rate_limit_event", "rate_limit_info": {}}),
            json!({
                "type": "result", "subtype": "success",
                "result": "```json\n{\"ok\": true}\n```"
            }),
        ]);

        parse_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify).expect("ok");
    }

    #[test]
    fn observer_guard_fires_call_end_on_drop() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        {
            let _g = ObserverGuard::new(Some(&observer_dyn));
        }
        assert_eq!(observer.names(), vec!["CallEnd".to_string()]);
    }

    #[test]
    fn observer_guard_no_op_when_observer_none() {
        let _g = ObserverGuard::new(None);
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("stream")
            .join(name)
    }

    fn parse_fixture(
        name: &str,
        observer: &Arc<dyn AgentObserver>,
        prompt: PromptId,
    ) -> Result<Value, crate::LlmError> {
        let path = fixture_path(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        parse_stream(Cursor::new(bytes), Some(observer), prompt)
    }

    #[test]
    fn fixture_simple_classify_returns_ok_true_with_no_tool_events() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let response = parse_fixture("simple-classify.jsonl", &observer_dyn, PromptId::Classify)
            .expect("simple-classify must succeed");

        assert_eq!(response, json!({"ok": true}));
        let names = observer.names();
        assert_eq!(names.first().map(String::as_str), Some("CallStart"));
        assert_eq!(names.last().map(String::as_str), Some("CallEnd"));
        for n in &names[1..names.len() - 1] {
            assert!(!n.starts_with("ToolUse"), "unexpected tool_use: {n}");
            assert!(!n.starts_with("ToolResult"), "unexpected tool_result: {n}");
        }
    }

    #[test]
    fn fixture_subcarve_with_tools_emits_one_tool_use_and_one_tool_result() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let response = parse_fixture(
            "subcarve-with-tools.jsonl",
            &observer_dyn,
            PromptId::Subcarve,
        )
        .expect("subcarve-with-tools must succeed");

        assert_eq!(response, json!({"package_name": "atlas-llm"}));
        let names = observer.names();
        let tool_uses: Vec<_> = names.iter().filter(|n| n.starts_with("ToolUse:")).collect();
        let tool_results: Vec<_> = names
            .iter()
            .filter(|n| n.starts_with("ToolResult:"))
            .collect();
        assert_eq!(
            tool_uses.len(),
            1,
            "want exactly one tool_use; got {names:?}"
        );
        assert_eq!(tool_uses[0], "ToolUse:Read");
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0], "ToolResult:true");
    }

    #[test]
    fn fixture_error_max_turns_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let err = parse_fixture("error-max-turns.jsonl", &observer_dyn, PromptId::Subcarve)
            .expect_err("non-success terminal must error");

        match err {
            crate::LlmError::Invocation(msg) => {
                assert!(
                    msg.contains("error_max_budget_usd"),
                    "expected subtype in message; got {msg}"
                );
            }
            other => panic!("expected Invocation, got {other:?}"),
        }
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }
}
