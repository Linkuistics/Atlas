//! Pure parsing for `codex exec --json` transcripts. The driver lives
//! in `codex.rs`; this module is kept stateless and synchronous so it
//! can be exercised with recorded JSONL fixtures and zero subprocess
//! overhead — parallel to `stream_parse.rs` for the Claude Code path.
//!
//! Spec: `docs/superpowers/specs/2026-05-02-codex-backend-research.md`.

use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;

use serde_json::Value;

use crate::agent_observer::{AgentEvent, AgentObserver};
use crate::stream_parse::{strip_json_fence, ObserverGuard};
use crate::{LlmError, PromptId};

/// Distil a short single-line summary from a `command_execution` item's
/// `command` field. Codex wraps shell commands as `/bin/zsh -lc "<cmd>"`;
/// strip that wrapper so the agent sub-line shows the inner command.
pub(crate) fn codex_command_summary(command: &str) -> String {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix("/bin/zsh -lc ") {
        let rest = rest.trim();
        if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return inner.to_string();
        }
        if let Some(inner) = rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
            return inner.to_string();
        }
        return rest.to_string();
    }
    trimmed.to_string()
}

/// Drive a `BufRead` of `codex exec --json` JSONL output. Emits
/// `AgentEvent`s through `observer` and returns the JSON value carried
/// in the last `item.completed` event whose `item.type ==
/// "agent_message"`.
///
/// `turn.failed` events terminate the stream with `LlmError::Invocation`
/// carrying the inner error message. Unknown event types are skipped so
/// future Codex CLI schema additions do not break the parser.
///
/// `CallStart` fires once at the top; `CallEnd` is guaranteed to fire
/// via `ObserverGuard` on every termination path.
pub(crate) fn parse_codex_stream<R: Read>(
    reader: R,
    observer: Option<&Arc<dyn AgentObserver>>,
    prompt: PromptId,
) -> Result<Value, LlmError> {
    let _guard = ObserverGuard::new(observer);
    if let Some(o) = observer {
        o.on_event(AgentEvent::CallStart { prompt });
    }

    let mut last_agent_message: Option<String> = None;
    let mut turn_failure: Option<String> = None;
    let buf = BufReader::new(reader);
    for line in buf.lines() {
        let line = line.map_err(|e| LlmError::Invocation(format!("stdout read failure: {e}")))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value.get("type").and_then(|v| v.as_str()) {
            Some("item.started") => emit_command_execution_start(&value, observer),
            Some("item.completed") => {
                if let Some(text) = extract_agent_message_text(&value) {
                    last_agent_message = Some(text);
                } else {
                    emit_command_execution_end(&value, observer);
                }
            }
            Some("turn.failed") => {
                turn_failure = Some(extract_turn_failed_message(&value));
            }
            _ => {} // forward-compat: skip thread.started, turn.started, turn.completed, error, ...
        }
    }

    if let Some(msg) = turn_failure {
        return Err(LlmError::Invocation(format!("codex turn.failed: {msg}")));
    }

    let text = last_agent_message.ok_or_else(|| {
        LlmError::Parse(
            "codex stream ended without an `agent_message` item; subprocess may have crashed before emitting a final response".to_string(),
        )
    })?;

    let stripped = strip_json_fence(&text);
    serde_json::from_str(stripped).map_err(|e| {
        LlmError::Parse(format!(
            "codex `agent_message.text` was not valid JSON: {e} (first 200 bytes: {:?})",
            stripped.chars().take(200).collect::<String>()
        ))
    })
}

fn extract_agent_message_text(value: &Value) -> Option<String> {
    let item = value.get("item")?;
    if item.get("type").and_then(|v| v.as_str())? != "agent_message" {
        return None;
    }
    item.get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn emit_command_execution_start(value: &Value, observer: Option<&Arc<dyn AgentObserver>>) {
    let Some(o) = observer else { return };
    let Some(item) = value.get("item") else {
        return;
    };
    if item.get("type").and_then(|v| v.as_str()) != Some("command_execution") {
        return;
    }
    let command = item.get("command").and_then(|v| v.as_str()).unwrap_or("");
    o.on_event(AgentEvent::ToolUse {
        name: "command_execution".to_string(),
        summary: codex_command_summary(command),
    });
}

fn emit_command_execution_end(value: &Value, observer: Option<&Arc<dyn AgentObserver>>) {
    let Some(o) = observer else { return };
    let Some(item) = value.get("item") else {
        return;
    };
    if item.get("type").and_then(|v| v.as_str()) != Some("command_execution") {
        return;
    }
    let exit_code = item.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
    o.on_event(AgentEvent::ToolResult { ok: exit_code == 0 });
}

fn extract_turn_failed_message(value: &Value) -> String {
    value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("(no error message in turn.failed event)")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;
    use std::sync::Mutex;

    /// Minimal observer that records events to a shared Vec for
    /// assertions. Inlined here rather than shared with `stream_parse`'s
    /// equivalent so that test modules stay self-contained.
    struct RecordingObserver {
        events: Mutex<Vec<AgentEvent>>,
    }

    impl RecordingObserver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
            })
        }

        fn names(&self) -> Vec<String> {
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
    fn codex_command_summary_strips_zsh_lc_double_quotes() {
        assert_eq!(
            codex_command_summary(r#"/bin/zsh -lc "sed -n '1,120p' fixture.toml""#),
            "sed -n '1,120p' fixture.toml"
        );
    }

    #[test]
    fn codex_command_summary_strips_zsh_lc_single_quotes() {
        assert_eq!(codex_command_summary("/bin/zsh -lc 'ls -la'"), "ls -la");
    }

    #[test]
    fn codex_command_summary_passes_through_non_zsh_form() {
        assert_eq!(codex_command_summary("rg foo"), "rg foo");
    }

    #[test]
    fn parse_codex_stream_no_tools_returns_agent_message_value() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "thread.started", "thread_id": "abc"}),
            json!({"type": "turn.started"}),
            json!({
                "type": "item.completed",
                "item": {"id": "item_0", "type": "agent_message", "text": "{\"ok\": true}"}
            }),
            json!({"type": "turn.completed", "usage": {}}),
        ]);

        let response =
            parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
                .expect("ok");

        assert_eq!(response, json!({"ok": true}));
        assert_eq!(
            observer.names(),
            vec!["CallStart".to_string(), "CallEnd".to_string()]
        );
    }

    #[test]
    fn parse_codex_stream_with_command_execution_emits_tool_events() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({
                "type": "item.started",
                "item": {
                    "id": "item_0",
                    "type": "command_execution",
                    "command": "/bin/zsh -lc \"cat fixture.toml\"",
                    "status": "in_progress"
                }
            }),
            json!({
                "type": "item.completed",
                "item": {
                    "id": "item_0",
                    "type": "command_execution",
                    "command": "/bin/zsh -lc \"cat fixture.toml\"",
                    "exit_code": 0,
                    "status": "completed"
                }
            }),
            json!({
                "type": "item.completed",
                "item": {"id": "item_1", "type": "agent_message", "text": "{\"package_name\": \"x\"}"}
            }),
            json!({"type": "turn.completed", "usage": {}}),
        ]);

        parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Subcarve)
            .expect("ok");

        assert_eq!(
            observer.names(),
            vec![
                "CallStart".to_string(),
                "ToolUse:command_execution".to_string(),
                "ToolResult:true".to_string(),
                "CallEnd".to_string(),
            ]
        );
    }

    #[test]
    fn parse_codex_stream_command_execution_non_zero_exit_marks_tool_result_false() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": "/bin/zsh -lc \"false\"",
                    "exit_code": 1
                }
            }),
            json!({
                "type": "item.completed",
                "item": {"type": "agent_message", "text": "{\"ok\": false}"}
            }),
        ]);

        parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
            .expect("ok");

        assert!(observer.names().iter().any(|n| n == "ToolResult:false"));
    }

    #[test]
    fn parse_codex_stream_uses_last_agent_message_when_multiple_present() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({
                "type": "item.completed",
                "item": {"type": "agent_message", "text": "{\"interim\": true}"}
            }),
            json!({
                "type": "item.completed",
                "item": {"type": "agent_message", "text": "{\"final\": true}"}
            }),
        ]);

        let response =
            parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
                .expect("ok");

        assert_eq!(response, json!({"final": true}));
    }

    #[test]
    fn parse_codex_stream_turn_failed_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "thread.started"}),
            json!({"type": "turn.started"}),
            json!({
                "type": "turn.failed",
                "error": {"message": "model rejected request"}
            }),
        ]);

        let err = parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
            .expect_err("turn.failed must error");

        match err {
            LlmError::Invocation(msg) => {
                assert!(msg.contains("model rejected request"), "got: {msg}");
                assert!(msg.contains("turn.failed"), "got: {msg}");
            }
            other => panic!("expected Invocation, got {other:?}"),
        }
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }

    #[test]
    fn parse_codex_stream_missing_agent_message_returns_parse_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "thread.started"}),
            json!({"type": "turn.completed", "usage": {}}),
        ]);

        let err = parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
            .expect_err("no agent_message must error");

        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn parse_codex_stream_skips_unknown_event_types_and_non_json_lines() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"not json garbage line\n");
        bytes.extend_from_slice(
            serde_json::to_string(&json!({"type": "future_event_type", "stuff": 1}))
                .unwrap()
                .as_bytes(),
        );
        bytes.push(b'\n');
        bytes.extend_from_slice(
            serde_json::to_string(&json!({
                "type": "item.completed",
                "item": {"type": "agent_message", "text": "{\"ok\": true}"}
            }))
            .unwrap()
            .as_bytes(),
        );
        bytes.push(b'\n');

        let response =
            parse_codex_stream(Cursor::new(bytes), Some(&observer_dyn), PromptId::Classify)
                .expect("ok");

        assert_eq!(response, json!({"ok": true}));
    }

    #[test]
    fn parse_codex_stream_strips_markdown_fence_defensively() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[json!({
            "type": "item.completed",
            "item": {"type": "agent_message", "text": "```json\n{\"ok\": true}\n```"}
        })]);

        let response =
            parse_codex_stream(Cursor::new(events), Some(&observer_dyn), PromptId::Classify)
                .expect("ok");

        assert_eq!(response, json!({"ok": true}));
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("codex-stream")
            .join(name)
    }

    fn parse_fixture(
        name: &str,
        observer: &Arc<dyn AgentObserver>,
        prompt: PromptId,
    ) -> Result<Value, LlmError> {
        let path = fixture_path(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        parse_codex_stream(Cursor::new(bytes), Some(observer), prompt)
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
    fn fixture_with_tools_emits_one_tool_use_and_one_tool_result() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let response = parse_fixture("with-tools.jsonl", &observer_dyn, PromptId::Subcarve)
            .expect("with-tools must succeed");

        assert_eq!(response, json!({"package_name": "atlas-llm-probe"}));
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
        assert_eq!(tool_uses[0], "ToolUse:command_execution");
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0], "ToolResult:true");
    }

    #[test]
    fn fixture_turn_failed_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let err = parse_fixture("turn-failed.jsonl", &observer_dyn, PromptId::Classify)
            .expect_err("turn.failed terminal must error");

        match err {
            LlmError::Invocation(msg) => {
                assert!(msg.contains("turn.failed"), "got: {msg}");
                assert!(msg.contains("invalid_json_schema"), "got: {msg}");
            }
            other => panic!("expected Invocation, got {other:?}"),
        }
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }
}
