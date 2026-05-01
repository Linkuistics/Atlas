//! Side-channel telemetry emitted by streaming-capable backends.
//!
//! `LlmBackend::call` returns the canonical response value. While the
//! call is in flight, a streaming backend may also emit transient
//! `AgentEvent`s through an attached `AgentObserver`. The observer is
//! optional; events are discarded when none is attached.
//!
//! Spec: `docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md` §5.1.

use serde_json::Value;

use crate::PromptId;

/// Implemented by sinks that want to receive transient agent activity
/// during an in-flight `LlmBackend::call`. The backend's return value
/// (the canonical response JSON) is unaffected — this trait is purely
/// for side-channel UI.
pub trait AgentObserver: Send + Sync {
    fn on_event(&self, event: AgentEvent);
}

/// One transient event emitted by a streaming backend while a call is
/// in flight. Lifecycle is bookended by `CallStart` and `CallEnd`;
/// `ToolUse`/`ToolResult` interleave between them. `CallEnd` is
/// guaranteed to fire on every termination path (see `ObserverGuard`).
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Fired once at the start of an `LlmBackend::call`, after the
    /// subprocess has been spawned and observation begins.
    CallStart { prompt: PromptId },
    /// One per `tool_use` block in any assistant turn.
    ToolUse { name: String, summary: String },
    /// One per `tool_result` block in any user turn.
    ToolResult { ok: bool },
    /// Fired once at the end of `LlmBackend::call`, in every
    /// termination path: success, error, panic recovery.
    CallEnd,
}

/// Distil a short single-line summary from a `tool_use` block's
/// `input` JSON, suitable for rendering on the agent sub-line. The
/// mapping is hard-coded per spec §5.1; unknown tools and malformed
/// inputs return an empty string so telemetry never crashes.
pub fn tool_summary_for(tool_name: &str, input: &Value) -> String {
    let field = match tool_name {
        "Read" | "Edit" | "Write" => "file_path",
        "Grep" | "Glob" => "pattern",
        "Bash" => "command",
        "Task" => "subagent_type",
        _ => return String::new(),
    };
    input
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PromptId;
    use serde_json::json;

    #[test]
    fn agent_event_clone_round_trip() {
        let e = AgentEvent::ToolUse {
            name: "Read".into(),
            summary: "/tmp/x".into(),
        };
        let cloned = e.clone();
        match cloned {
            AgentEvent::ToolUse { name, summary } => {
                assert_eq!(name, "Read");
                assert_eq!(summary, "/tmp/x");
            }
            _ => panic!("clone changed variant"),
        }
    }

    #[test]
    fn agent_event_debug_includes_variant_name() {
        let e = AgentEvent::CallStart {
            prompt: PromptId::Classify,
        };
        let s = format!("{e:?}");
        assert!(s.contains("CallStart"), "got {s:?}");
    }

    #[test]
    fn tool_summary_read_uses_file_path() {
        let s = tool_summary_for("Read", &json!({"file_path": "crates/foo/src/lib.rs"}));
        assert_eq!(s, "crates/foo/src/lib.rs");
    }

    #[test]
    fn tool_summary_grep_uses_pattern() {
        let s = tool_summary_for("Grep", &json!({"pattern": "ProgressEvent::Subcarve"}));
        assert_eq!(s, "ProgressEvent::Subcarve");
    }

    #[test]
    fn tool_summary_bash_uses_command() {
        let s = tool_summary_for("Bash", &json!({"command": "cargo test --workspace"}));
        assert_eq!(s, "cargo test --workspace");
    }

    #[test]
    fn tool_summary_edit_uses_file_path() {
        let s = tool_summary_for("Edit", &json!({"file_path": "crates/x/src/y.rs"}));
        assert_eq!(s, "crates/x/src/y.rs");
    }

    #[test]
    fn tool_summary_write_uses_file_path() {
        let s = tool_summary_for("Write", &json!({"file_path": "crates/x/src/y.rs"}));
        assert_eq!(s, "crates/x/src/y.rs");
    }

    #[test]
    fn tool_summary_glob_uses_pattern() {
        let s = tool_summary_for("Glob", &json!({"pattern": "crates/**/*.rs"}));
        assert_eq!(s, "crates/**/*.rs");
    }

    #[test]
    fn tool_summary_task_uses_subagent_type() {
        let s = tool_summary_for(
            "Task",
            &json!({"subagent_type": "Explore", "description": "find foo"}),
        );
        assert_eq!(s, "Explore");
    }

    #[test]
    fn tool_summary_unknown_tool_returns_empty() {
        let s = tool_summary_for("MysteryTool", &json!({"anything": 1}));
        assert_eq!(s, "");
    }

    #[test]
    fn tool_summary_falls_back_to_empty_when_field_missing() {
        let s = tool_summary_for("Read", &json!({"not_file_path": "/tmp/x"}));
        assert_eq!(s, "");
    }

    #[test]
    fn tool_summary_falls_back_to_empty_when_input_not_object() {
        let s = tool_summary_for("Read", &json!("not-an-object"));
        assert_eq!(s, "");
    }
}
