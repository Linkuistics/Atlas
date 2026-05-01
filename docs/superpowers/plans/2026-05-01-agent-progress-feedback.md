# Agent Progress Feedback During Long LLM Calls — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the most recent tool the `claude -p` agent invoked plus a running counter on a dedicated screen line under the activity bar, so users can distinguish a stuck process from a working one during long subcarve calls.

**Architecture:** `atlas-llm` gains an `AgentObserver` side-channel trait emitted by `ClaudeCodeBackend` as it parses `claude -p --output-format stream-json --verbose` output. The non-streaming `--output-format json` path is removed (Approach A — single mental model). `Reporter` in `atlas-cli` adds a third indicatif sub-line that mounts on `CallStart`, refreshes on `ToolUse`, and unmounts on `CallEnd`. Events bypass `BudgetSentinel`/`LlmResponseCache` so cache hits remain byte-identical and the agent sub-line never fires on a no-op re-run.

**Tech Stack:** Rust 2021, `serde_json` (existing), `indicatif 0.17` (existing in atlas-cli), real `claude -p` subprocess via `std::process::Command::spawn`. Workspace deny `warnings = "deny"` so all rustc/clippy warnings must be clean before each commit.

**Spec:** [`docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md`](../specs/2026-05-01-agent-progress-feedback-design.md).

**Companion plan (already shipped):** [`docs/superpowers/plans/2026-05-01-engine-progress-events.md`](2026-05-01-engine-progress-events.md) — established the `Reporter` / `ProgressBackend` / side-channel pattern this plan builds on.

---

## File Structure

| Path | Status | Responsibility |
|---|---|---|
| `crates/atlas-llm/src/agent_observer.rs` | **create** | `AgentObserver` trait, `AgentEvent` enum, `tool_summary_for` lookup table |
| `crates/atlas-llm/src/stream_parse.rs` | **create** | Pure parsing: per-line dispatch (`extract_tool_uses`, `extract_tool_results`), `strip_json_fence`, `parse_stream` driver, `ObserverGuard` |
| `crates/atlas-llm/src/lib.rs` | modify | Re-export `AgentEvent`, `AgentObserver` |
| `crates/atlas-llm/src/claude_code.rs` | modify | Add `observer` field + `with_observer` builder; rewrite `call()` to spawn `claude -p --output-format stream-json --verbose` and feed stdout to `parse_stream` |
| `crates/atlas-llm/tests/stream_fixtures.rs` | **create** | Integration tests: feed each of the three committed fixtures to `parse_stream` via a `RecordingObserver`; assert event count/order/returned value |
| `crates/atlas-cli/src/progress.rs` | modify | `agent_bar` third indicatif bar; `agent_*` fields on `ReporterState`; `impl AgentObserver for Reporter`; `render_agent_line` with truncation; test-only accessors |
| `crates/atlas-cli/src/backend.rs` | modify | `build_production_backend` accepts an optional `Arc<dyn AgentObserver>` and threads it into `ClaudeCodeBackend::with_observer` |
| `crates/atlas-cli/src/main.rs` | modify | Construct reporter first, pass `Arc<Reporter>` (cast to `Arc<dyn AgentObserver>`) into `build_production_backend` only when `reporter.drawing()` |
| `crates/atlas-cli/tests/agent_observer_e2e.rs` | **create** | Single end-to-end test gated behind `ATLAS_LLM_RUN_CLAUDE_TESTS=1`: run `atlas index` against a tiny fixture under `--progress=always`, assert success, four YAMLs, captured stderr contains `↳` |
| `LLM_STATE/core/memory/...` | additions | Four distilled-memory entries per spec §10 (added in Task 13) |

---

## Conventions for every task

- After every step that runs commands, capture the **expected** output literally. If real output differs, stop and fix before moving on.
- Run `cargo fmt --all` before each commit.
- Workspace lints set `warnings = "deny"` — any rustc/clippy warning must be fixed in the same commit that introduced it (per the project's "fix all lints" directive).
- Commit messages follow the existing convention (`feat: …`, `chore: …`, `test: …`). The Co-Authored-By trailer is added by the harness.
- The fixture files at `crates/atlas-llm/tests/fixtures/stream/{simple-classify,subcarve-with-tools,error-max-turns}.jsonl` are **already committed**. Tasks reference them by path; do not regenerate.

---

## Task 1: Add `agent_observer` module skeleton

**Spec ref:** §5.1.

**Files:**
- Create: `crates/atlas-llm/src/agent_observer.rs`
- Modify: `crates/atlas-llm/src/lib.rs`

- [ ] **Step 1: Write the failing test for `AgentEvent` Debug + Clone**

In `crates/atlas-llm/src/agent_observer.rs`, append a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::PromptId;

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
}
```

- [ ] **Step 2: Run the test — should fail (module/types don't exist yet)**

Run: `cargo test -p atlas-llm agent_event_clone_round_trip`
Expected: compile error: `error[E0432]: unresolved import`. The whole module is missing.

- [ ] **Step 3: Create the module skeleton**

Write `crates/atlas-llm/src/agent_observer.rs` (the test block from Step 1 stays at the bottom):

```rust
//! Side-channel telemetry emitted by streaming-capable backends.
//!
//! `LlmBackend::call` returns the canonical response value. While the
//! call is in flight, a streaming backend may also emit transient
//! `AgentEvent`s through an attached `AgentObserver`. The observer is
//! optional; events are discarded when none is attached.
//!
//! Spec: `docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md` §5.1.

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
```

- [ ] **Step 4: Re-export from `lib.rs`**

In `crates/atlas-llm/src/lib.rs`, after the existing `pub mod` lines, add:

```rust
pub mod agent_observer;
```

And after the existing `pub use` block, add:

```rust
pub use agent_observer::{AgentEvent, AgentObserver};
```

- [ ] **Step 5: Run the test — should pass**

Run: `cargo test -p atlas-llm agent_event`
Expected: 2 passed.

- [ ] **Step 6: Lint check**

Run: `cargo clippy -p atlas-llm -- -D warnings`
Expected: clean.

- [ ] **Step 7: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-llm/src/agent_observer.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add AgentObserver trait + AgentEvent enum"
```

---

## Task 2: Implement `tool_summary_for` lookup table

**Spec ref:** §5.1 (table mapping tool name → summary source field).

**Files:**
- Modify: `crates/atlas-llm/src/agent_observer.rs`

- [ ] **Step 1: Write the failing tests for the lookup table**

In `crates/atlas-llm/src/agent_observer.rs`, inside the existing `#[cfg(test)] mod tests`, append:

```rust
    use serde_json::json;

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
```

- [ ] **Step 2: Run the tests — should fail**

Run: `cargo test -p atlas-llm tool_summary`
Expected: compile error: `cannot find function 'tool_summary_for' in this scope`.

- [ ] **Step 3: Implement `tool_summary_for`**

In `crates/atlas-llm/src/agent_observer.rs`, before the `#[cfg(test)]` block, add:

```rust
use serde_json::Value;

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
```

- [ ] **Step 4: Run the tests — should pass**

Run: `cargo test -p atlas-llm tool_summary`
Expected: 9 passed (and the 2 from Task 1 still pass).

- [ ] **Step 5: Add `serde_json` to dependencies**

`serde_json` is already a dep of `atlas-llm` (used by `claude_code.rs`); no Cargo.toml change needed. Verify with: `grep serde_json crates/atlas-llm/Cargo.toml`.
Expected output:
```
serde_json = { workspace = true }
```

- [ ] **Step 6: Lint check**

Run: `cargo clippy -p atlas-llm -- -D warnings`
Expected: clean.

- [ ] **Step 7: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-llm/src/agent_observer.rs
git commit -m "feat(atlas-llm): add tool_summary_for lookup table for AgentEvent::ToolUse summaries"
```

---

## Task 3: Add per-line stream-event extraction helpers

**Spec ref:** §6 (data flow), §6.4 (tool extraction). Memory: `stream-json-content-mixes-thinking-tool-use-text-per-event`, `stream-json-tool-result-is-error-absent-on-success`.

This task adds three private helpers used by `parse_stream` (Task 5):

- `extract_tool_uses(value, observer)` — walks an `assistant`-typed event's `content[]` and emits `ToolUse` for each `tool_use` block.
- `extract_tool_results(value, observer)` — walks a `user`-typed event's `content[]` and emits `ToolResult { ok: !is_error }` for each `tool_result` block.
- `strip_json_fence(s) -> &str` — strips ` ```json` / ` ``` ` markers from `result.result`.

**Files:**
- Create: `crates/atlas-llm/src/stream_parse.rs`

- [ ] **Step 1: Write failing tests for `extract_tool_uses`**

Create `crates/atlas-llm/src/stream_parse.rs`:

```rust
//! Pure parsing for `claude -p --output-format stream-json --verbose`
//! transcripts. The driver lives in `claude_code.rs`; this module is
//! kept stateless and synchronous so it can be exercised with recorded
//! JSONL fixtures and zero subprocess overhead.
//!
//! Spec: `docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md` §6.

use std::sync::Arc;

use serde_json::Value;

use crate::agent_observer::{tool_summary_for, AgentEvent, AgentObserver};

/// Walk an assistant-typed event's `message.content[]` array and emit
/// one `AgentEvent::ToolUse` per `tool_use` block. `thinking` and
/// `text` blocks are skipped (memory: stream-json content[] mixes
/// thinking/tool_use/text per event).
pub(crate) fn extract_tool_uses(
    value: &Value,
    observer: Option<&Arc<dyn AgentObserver>>,
) {
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
pub(crate) fn extract_tool_results(
    value: &Value,
    observer: Option<&Arc<dyn AgentObserver>>,
) {
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
    after_open.strip_suffix("```").map(str::trim).unwrap_or(after_open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PromptId;
    use serde_json::json;
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
        // Smoke: no panic, no events to assert on.
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
        assert_eq!(strip_json_fence("```json\n{\"ok\":true}\n```"), "{\"ok\":true}");
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
        assert_eq!(strip_json_fence("   ```json\n  {\"ok\":true}  \n```   "), "{\"ok\":true}");
    }

    // PromptId is only used in further tests we'll add later (Task 5);
    // touch it here so the import doesn't lint as unused.
    #[test]
    fn prompt_id_referenced_for_future_tests() {
        let _ = PromptId::Classify;
    }
}
```

Note: `RecordingObserver` is `pub(crate)` inside `mod tests` so Task 5's `parse_stream` tests in this same file can reuse it.

- [ ] **Step 2: Hook the new module into `lib.rs`**

In `crates/atlas-llm/src/lib.rs`, add:

```rust
pub(crate) mod stream_parse;
```

next to the existing `pub mod` declarations. (No public re-exports; the helpers stay crate-private.)

- [ ] **Step 3: Run the tests — should pass**

Run: `cargo test -p atlas-llm stream_parse`
Expected: 8 passed (plus the unrelated tool_summary / agent_event tests).

- [ ] **Step 4: Lint check**

Run: `cargo clippy -p atlas-llm -- -D warnings`
Expected: clean. If `RecordingObserver`'s `clone()` call below is flagged, derive `Arc::clone` semantics — `observer.clone()` calls `Arc::clone` because `RecordingObserver` itself does not implement `Clone`. We rely on `Arc<RecordingObserver>` deriving Clone via Arc.

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-llm/src/stream_parse.rs crates/atlas-llm/src/lib.rs
git commit -m "feat(atlas-llm): add stream_parse helpers for tool_use/tool_result extraction and JSON fence stripping"
```

---

## Task 4: Add `parse_stream` driver and `ObserverGuard`

**Spec ref:** §6 (data flow lines 247-269), §7.5 (CallEnd ordering guarantee).

**Files:**
- Modify: `crates/atlas-llm/src/stream_parse.rs`

- [ ] **Step 1: Write failing tests against an in-memory stream**

In `crates/atlas-llm/src/stream_parse.rs`'s `tests` module, append:

```rust
    use std::io::Cursor;

    /// Build a JSONL byte buffer from the given `serde_json::Value`s,
    /// one per line, suitable for a `Cursor`-backed `BufReader`.
    fn jsonl_bytes(events: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in events {
            out.extend_from_slice(serde_json::to_string(e).unwrap().as_bytes());
            out.push(b'\n');
        }
        out
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

        let response = parse_stream(
            Cursor::new(events),
            Some(&observer_dyn),
            PromptId::Classify,
        )
        .expect("ok");

        assert_eq!(response, json!({"ok": true}));
        assert_eq!(
            observer.names(),
            vec!["CallStart".into(), "CallEnd".into()]
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

        parse_stream(
            Cursor::new(events),
            Some(&observer_dyn),
            PromptId::Subcarve,
        )
        .expect("ok");

        assert_eq!(
            observer.names(),
            vec![
                "CallStart".into(),
                "ToolUse:Read".into(),
                "ToolResult:true".into(),
                "CallEnd".into(),
            ]
        );
    }

    #[test]
    fn parse_stream_terminal_subtype_not_success_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({
                "type": "result",
                "subtype": "error_max_budget_usd",
                "is_error": true
            }),
        ]);

        let err = parse_stream(
            Cursor::new(events),
            Some(&observer_dyn),
            PromptId::Subcarve,
        )
        .expect_err("non-success subtype must error");

        match err {
            crate::LlmError::Invocation(msg) => {
                assert!(msg.contains("error_max_budget_usd"), "got: {msg}");
            }
            other => panic!("expected Invocation, got {other:?}"),
        }
        // Even on the error path CallEnd fires.
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }

    #[test]
    fn parse_stream_missing_terminal_returns_parse_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        let events = jsonl_bytes(&[
            json!({"type": "system", "subtype": "init"}),
        ]);

        let err = parse_stream(
            Cursor::new(events),
            Some(&observer_dyn),
            PromptId::Classify,
        )
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

        let response = parse_stream(
            Cursor::new(bytes),
            Some(&observer_dyn),
            PromptId::Classify,
        )
        .expect("ok");

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

        parse_stream(
            Cursor::new(events),
            Some(&observer_dyn),
            PromptId::Classify,
        )
        .expect("ok");
    }

    #[test]
    fn observer_guard_fires_call_end_on_drop() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();
        {
            let _g = ObserverGuard::new(Some(&observer_dyn));
            // dropped at scope exit; CallEnd should fire.
        }
        assert_eq!(observer.names(), vec!["CallEnd".to_string()]);
    }

    #[test]
    fn observer_guard_no_op_when_observer_none() {
        let _g = ObserverGuard::new(None);
        // Nothing to assert: just ensure no panic.
    }
```

- [ ] **Step 2: Run the tests — should fail**

Run: `cargo test -p atlas-llm parse_stream`
Expected: compile errors: `cannot find type ObserverGuard`, `cannot find function parse_stream`.

- [ ] **Step 3: Implement `ObserverGuard`**

In `crates/atlas-llm/src/stream_parse.rs`, add (above the existing helpers, after the `use` block):

```rust
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
```

- [ ] **Step 4: Implement `parse_stream`**

In the same file, append:

```rust
use std::io::{BufRead, BufReader, Read};

use crate::{LlmError, PromptId};

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
        let line = line.map_err(|e| {
            LlmError::Invocation(format!("stdout read failure: {e}"))
        })?;
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
            LlmError::Parse(
                "claude `result` event missing `result` string field"
                    .to_string(),
            )
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
```

- [ ] **Step 5: Run the tests — should pass**

Run: `cargo test -p atlas-llm stream_parse`
Expected: 16 passed (8 from Task 3 + 8 new).

- [ ] **Step 6: Lint check**

Run: `cargo clippy -p atlas-llm -- -D warnings`
Expected: clean.

- [ ] **Step 7: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-llm/src/stream_parse.rs
git commit -m "feat(atlas-llm): add parse_stream driver + ObserverGuard for stream-json transcripts"
```

---

## Task 5: Fixture-backed integration tests for `parse_stream`

**Spec ref:** §8.1.

Tests use the three real captures already at `crates/atlas-llm/tests/fixtures/stream/`:
- `simple-classify.jsonl` — no tools, terminal success.
- `subcarve-with-tools.jsonl` — one Read tool, terminal success.
- `error-max-turns.jsonl` — three tool turns, terminal `error_max_budget_usd`.

Reading the fixtures requires `parse_stream` and `RecordingObserver` to be reachable from an integration test. Strategy: keep `parse_stream` `pub(crate)` and add **one** thin public re-export gated behind `#[cfg(any(test, feature = "test-helpers"))]` so the integration test can reach it without exposing it on the production API surface. Atlas already has no `test-helpers` Cargo feature; the simpler path is to put the fixture tests as a `#[cfg(test)]` submodule inside `crates/atlas-llm/src/stream_parse.rs` itself, so they have crate-internal visibility.

We'll go with the in-crate location.

**Files:**
- Modify: `crates/atlas-llm/src/stream_parse.rs` (extend existing `tests` module)

- [ ] **Step 1: Write the failing fixture tests**

In `crates/atlas-llm/src/stream_parse.rs`'s `tests` module, append:

```rust
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
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        parse_stream(Cursor::new(bytes), Some(observer), prompt)
    }

    #[test]
    fn fixture_simple_classify_returns_ok_true_with_no_tool_events() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let response =
            parse_fixture("simple-classify.jsonl", &observer_dyn, PromptId::Classify)
                .expect("simple-classify must succeed");

        assert_eq!(response, json!({"ok": true}));
        let names = observer.names();
        assert_eq!(names.first().map(String::as_str), Some("CallStart"));
        assert_eq!(names.last().map(String::as_str), Some("CallEnd"));
        // No tool_use/tool_result events between Start and End.
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
        let tool_results: Vec<_> =
            names.iter().filter(|n| n.starts_with("ToolResult:")).collect();
        assert_eq!(tool_uses.len(), 1, "want exactly one tool_use; got {names:?}");
        assert_eq!(tool_uses[0], "ToolUse:Read");
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0], "ToolResult:true");
    }

    #[test]
    fn fixture_error_max_turns_returns_invocation_error() {
        let observer = RecordingObserver::new();
        let observer_dyn: Arc<dyn AgentObserver> = observer.clone();

        let err = parse_fixture(
            "error-max-turns.jsonl",
            &observer_dyn,
            PromptId::Subcarve,
        )
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
        // CallEnd still fires on the error path.
        assert!(observer.names().iter().any(|n| n == "CallEnd"));
    }
```

- [ ] **Step 2: Run the new fixture tests — should pass**

Run: `cargo test -p atlas-llm fixture_`
Expected: 3 passed.

If `subcarve-with-tools.jsonl`'s `result.result` differs from the literal `{"package_name": "atlas-llm"}` (re-recorded fixture, different model wording), update only the asserted value, not the structure.

- [ ] **Step 3: Run the full atlas-llm test suite to make sure nothing else regressed**

Run: `cargo test -p atlas-llm`
Expected: all tests pass.

- [ ] **Step 4: Lint check**

Run: `cargo clippy -p atlas-llm --tests -- -D warnings`
Expected: clean.

- [ ] **Step 5: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-llm/src/stream_parse.rs
git commit -m "test(atlas-llm): add fixture-backed parse_stream tests covering success, with-tools, and error-budget paths"
```

---

## Task 6: Add `observer` field + `with_observer` builder to `ClaudeCodeBackend`

**Spec ref:** §5.2.

This is a no-op refactor — the field exists but is unused until Task 7 rewrites `call`. Splitting it out keeps the call rewrite focused.

**Files:**
- Modify: `crates/atlas-llm/src/claude_code.rs`

- [ ] **Step 1: Add the field**

In `crates/atlas-llm/src/claude_code.rs`, add to imports:

```rust
use std::sync::Arc;

use crate::agent_observer::AgentObserver;
```

Then change the struct:

```rust
pub struct ClaudeCodeBackend {
    model_id: String,
    prompts_dir: PathBuf,
    version: String,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    observer: Option<Arc<dyn AgentObserver>>,
}
```

Update `new` to initialise the new field:

```rust
Ok(Self {
    model_id,
    prompts_dir,
    version,
    template_sha: [0u8; 32],
    ontology_sha: [0u8; 32],
    observer: None,
})
```

- [ ] **Step 2: Add the builder**

After the existing `with_fingerprint_inputs` method, add:

```rust
/// Attach a side-channel observer that receives `AgentEvent`s while
/// the streaming subprocess is running. When `None` (the default),
/// stream events are parsed and discarded. Spec §5.2.
pub fn with_observer(mut self, observer: Arc<dyn AgentObserver>) -> Self {
    self.observer = Some(observer);
    self
}
```

- [ ] **Step 3: Verify the crate still compiles cleanly**

Run: `cargo check -p atlas-llm`
Expected: clean.

The `observer` field is read by `call()` after Task 7, which will silence any `dead_code` warning. Until then, the workspace's `warnings = "deny"` may trigger `dead_code` because `observer` is currently unread. Since the field is `pub` only via the builder method, and the struct itself is `pub`, the field counts as part of a public API surface — but it's a *private* field on a public struct. `dead_code` will fire.

Mitigation: either (a) add `#[allow(dead_code)]` on the field temporarily, or (b) skip Task 6's commit and merge Tasks 6+7 into one commit. We'll take option (b): leave Step 4 as a *don't commit yet* directive and roll forward to Task 7.

- [ ] **Step 4: Do NOT commit yet — proceed to Task 7**

The change is intentionally left uncommitted: combining with Task 7 avoids a transient `dead_code` warning that the workspace's `warnings = "deny"` would reject.

---

## Task 7: Rewrite `ClaudeCodeBackend::call` to use streaming subprocess

**Spec ref:** §6 (data flow), §7.7 (stderr capture), Approach A from §4.2.

Rewrite the `LlmBackend for ClaudeCodeBackend` impl to:
1. Render the prompt (existing behaviour).
2. Spawn `claude -p <rendered> --output-format stream-json --verbose --model <id>`.
3. Pipe stdout (drained by `parse_stream`) and stderr (drained by a small thread to avoid deadlock).
4. Call `parse_stream(stdout, self.observer.as_ref(), req.prompt_template)`.
5. Validate the parsed JSON against `req.schema` (existing helper).
6. On non-zero exit with no terminal event, return `LlmError::Invocation` with captured stderr.

**Files:**
- Modify: `crates/atlas-llm/src/claude_code.rs`

- [ ] **Step 1: Replace the `LlmBackend for ClaudeCodeBackend` impl**

Replace the existing `impl LlmBackend for ClaudeCodeBackend { fn call ... }` block with:

```rust
impl LlmBackend for ClaudeCodeBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered_prompt = self.render_request(req)?;
        let mut child = std::process::Command::new("claude")
            .arg("-p")
            .arg(&rendered_prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--model")
            .arg(&self.model_id)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                LlmError::Invocation(format!(
                    "failed to spawn `claude`: {e} (is it still on PATH?)"
                ))
            })?;

        // Drain stderr in a worker thread so the child does not block
        // on a full stderr pipe buffer while we read stdout.
        let stderr_pipe = child.stderr.take().expect("stderr piped");
        let stderr_handle = std::thread::spawn(move || -> Vec<u8> {
            use std::io::Read;
            let mut buf = Vec::new();
            let mut reader = stderr_pipe;
            let _ = reader.read_to_end(&mut buf);
            buf
        });

        let stdout_pipe = child.stdout.take().expect("stdout piped");
        let parsed = crate::stream_parse::parse_stream(
            stdout_pipe,
            self.observer.as_ref(),
            req.prompt_template,
        );

        let status = child.wait().map_err(|e| {
            LlmError::Invocation(format!("failed to wait on `claude`: {e}"))
        })?;
        let stderr_bytes = stderr_handle
            .join()
            .unwrap_or_else(|_| b"<stderr drainer panicked>".to_vec());

        let value = match parsed {
            Ok(v) => v,
            Err(LlmError::Parse(msg)) if !status.success() => {
                let stderr_snippet = String::from_utf8_lossy(&stderr_bytes);
                return Err(LlmError::Invocation(format!(
                    "`claude` exited with status {} before emitting a terminal event: {msg}; stderr={}",
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
            backend_version: self.version.clone(),
        }
    }
}
```

Also remove the now-unused top-level `use std::process::Command;` from the imports if present, since the implementation now uses `std::process::Command::new` inline. (`capture_claude_version` still uses `Command`, so keep that import.) Verify the use list is minimal.

- [ ] **Step 2: Update the gated integration test in `claude_code.rs`**

The existing `call_roundtrips_json_response` test inside `crates/atlas-llm/src/claude_code.rs::tests` writes a stub prompt that asks for `{"ok": true}` literally. Under the streaming path the agent now wraps that in a markdown fence, which `parse_stream` strips. The assertion `assert_eq!(response, json!({"ok": true}))` therefore continues to hold. No code change needed; just re-run it locally if `ATLAS_LLM_RUN_CLAUDE_TESTS=1` is set:

```bash
ATLAS_LLM_RUN_CLAUDE_TESTS=1 cargo test -p atlas-llm \
    -- --ignored call_roundtrips_json_response
```

Expected (when opted in): pass. (The default test run skips it.)

- [ ] **Step 3: Run the full atlas-llm test suite (cached path)**

Run: `cargo test -p atlas-llm`
Expected: clean. The `dead_code` warning on `observer` is gone since `call` now reads `self.observer`.

- [ ] **Step 4: Lint check**

Run: `cargo clippy -p atlas-llm --tests -- -D warnings`
Expected: clean.

- [ ] **Step 5: Format + commit (Tasks 6 + 7 together)**

```bash
cargo fmt --all
git add crates/atlas-llm/src/claude_code.rs
git commit -m "feat(atlas-llm): stream claude -p output via stream-json --verbose; remove non-streaming JSON path"
```

---

## Task 8: Add `agent_bar` and `ReporterState` fields for the agent sub-line

**Spec ref:** §5.3.

Reporter currently has two indicatif bars: `activity` and `tokens`. Insert a third bar **between** them so the rendered order is:

```
  ⠋ activity bar
      ↳ agent sub-line
    tokens N/N  [bar]  N%
```

`MultiProgress::insert(idx, bar)` lets us specify position. Currently `activity` is at index 0 and `tokens` at index 1, so the new bar mounts at index 1 (pushing `tokens` to index 2).

The bar starts hidden (no events to show until a `CallStart` arrives).

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Write the failing test**

In `crates/atlas-cli/src/progress.rs`'s `tests` module, append:

```rust
    #[test]
    fn reporter_starts_with_agent_bar_hidden_and_zero_tools() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        assert!(!r.agent_visible());
        assert_eq!(r.agent_tools(), 0);
    }
```

- [ ] **Step 2: Run the test — should fail**

Run: `cargo test -p atlas-cli reporter_starts_with_agent_bar_hidden`
Expected: compile error: `agent_visible`/`agent_tools` not defined.

- [ ] **Step 3: Add fields to `Reporter` and `ReporterState`**

In `crates/atlas-cli/src/progress.rs`:

1. Extend the `ReporterState` struct (the `#[derive(Default, Debug, Clone)]` block):

   ```rust
   #[derive(Default, Debug, Clone)]
   struct ReporterState {
       breakdown: PromptBreakdown,
       last_msg: String,
       iter_history: Vec<String>,
       summary: Option<String>,
       sticky_kn: Option<(u64, u64, &'static str)>,
       last_llm_target: Option<PathBuf>,
       iter_live: u64,
       last_iteration: u32,
       /// Counter incremented per `AgentEvent::ToolUse`, reset on
       /// `AgentEvent::CallStart`.
       agent_tools: u64,
       /// `(name, summary)` of the most recent `AgentEvent::ToolUse`.
       agent_last_tool: Option<(String, String)>,
       /// Sticky flag set by `AgentEvent::ToolResult { ok: false }`.
       /// Cleared by the next `AgentEvent::ToolUse` so the `(✗)`
       /// marker only persists until the agent moves on.
       agent_last_failed: bool,
   }
   ```

2. Extend `Reporter`:

   ```rust
   pub struct Reporter {
       multi: MultiProgress,
       activity: ProgressBar,
       agent: ProgressBar,
       tokens: ProgressBar,
       state: Mutex<ReporterState>,
       counter: Option<Arc<TokenCounter>>,
       drawing: bool,
   }
   ```

3. Update `Reporter::new`. Replace the body where `tokens` is added:

   Before:
   ```rust
       let tokens = multi.add(ProgressBar::new(0));
       tokens.set_style(
           ProgressStyle::with_template("    tokens {msg}  {bar:50}  {percent:>3}%")
               .expect("static template"),
       );

       Arc::new(Self {
           multi,
           activity,
           tokens,
           state: Mutex::new(ReporterState::default()),
           counter,
           drawing,
       })
   ```

   After:
   ```rust
       let agent = multi.add(ProgressBar::new(0));
       agent.set_style(
           ProgressStyle::with_template("      {msg}").expect("static template"),
       );
       agent.set_draw_target(ProgressDrawTarget::hidden());

       let tokens = multi.add(ProgressBar::new(0));
       tokens.set_style(
           ProgressStyle::with_template("    tokens {msg}  {bar:50}  {percent:>3}%")
               .expect("static template"),
       );

       Arc::new(Self {
           multi,
           activity,
           agent,
           tokens,
           state: Mutex::new(ReporterState::default()),
           counter,
           drawing,
       })
   ```

   Note: `MultiProgress::add` appends to the end. We rely on insertion order: `activity` first, then `agent`, then `tokens`, which gives the correct top-to-bottom render.

4. Add the test-only accessors (next to existing `#[cfg(test)] pub(crate) fn ...`):

   ```rust
   #[cfg(test)]
   pub(crate) fn agent_visible(&self) -> bool {
       !self.agent.is_hidden()
   }
   #[cfg(test)]
   pub(crate) fn agent_tools(&self) -> u64 {
       self.lock().agent_tools
   }
   #[cfg(test)]
   pub(crate) fn agent_msg(&self) -> String {
       self.agent.message().to_string()
   }
   ```

   Note: `ProgressBar::is_hidden()` returns `true` when `set_draw_target(ProgressDrawTarget::hidden())` was the most recent call. (`indicatif` 0.17 exposes this via `is_hidden`.)

- [ ] **Step 4: Run the test — should pass**

Run: `cargo test -p atlas-cli reporter_starts_with_agent_bar_hidden`
Expected: pass.

- [ ] **Step 5: Run the existing reporter tests to make sure nothing regressed**

Run: `cargo test -p atlas-cli progress::`
Expected: all existing tests pass (the new field defaults to None/0/false; existing assertions are unaffected).

- [ ] **Step 6: Lint check**

Run: `cargo clippy -p atlas-cli --tests -- -D warnings`
Expected: clean.

- [ ] **Step 7: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(atlas-cli): add hidden agent_bar + agent state to Reporter"
```

---

## Task 9: Implement `AgentObserver` for `Reporter`

**Spec ref:** §5.3 (the four-arm `match` on `AgentEvent`).

`Reporter` already implements `ProgressSink`; this task adds a parallel `impl AgentObserver for Reporter`. The agent bar shows/hides; the message text comes from a `render_agent_line` helper added in Task 10.

For now the `ToolUse` and `ToolResult` arms call a stub `render_agent_line(&state) -> String` that returns `format!("↳ {} {} · {} tools", name, summary, tools)` (full width, no truncation). Truncation is added in Task 10.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Write failing tests**

In `crates/atlas-cli/src/progress.rs`'s `tests` module, append:

```rust
    use atlas_llm::{AgentEvent, AgentObserver};

    #[test]
    fn reporter_call_start_makes_agent_bar_visible_and_resets_counters() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        // Pre-state simulates a previous call having ticked.
        {
            let mut s = r.lock();
            s.agent_tools = 5;
            s.agent_last_tool = Some(("Read".into(), "/x".into()));
            s.agent_last_failed = true;
        }
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        assert!(r.agent_visible());
        assert_eq!(r.agent_tools(), 0);
        // `agent_last_tool` and `agent_last_failed` are also reset.
        let s = r.lock();
        assert!(s.agent_last_tool.is_none());
        assert!(!s.agent_last_failed);
    }

    #[test]
    fn reporter_tool_use_increments_counter_and_renders_line() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        r.on_event(AgentEvent::ToolUse {
            name: "Read".into(),
            summary: "crates/atlas-engine/src/l8_recurse.rs".into(),
        });
        assert_eq!(r.agent_tools(), 1);
        assert_eq!(
            r.agent_msg(),
            "↳ Read crates/atlas-engine/src/l8_recurse.rs · 1 tools"
        );
    }

    #[test]
    fn reporter_tool_result_failure_marks_line_with_cross() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        r.on_event(AgentEvent::ToolUse {
            name: "Read".into(),
            summary: "/tmp/x".into(),
        });
        r.on_event(AgentEvent::ToolResult { ok: false });
        assert!(
            r.agent_msg().ends_with("(✗)"),
            "expected (✗) marker; got {:?}",
            r.agent_msg()
        );
    }

    #[test]
    fn reporter_tool_result_success_does_not_change_line() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        r.on_event(AgentEvent::ToolUse {
            name: "Read".into(),
            summary: "/tmp/x".into(),
        });
        let before = r.agent_msg();
        r.on_event(AgentEvent::ToolResult { ok: true });
        assert_eq!(r.agent_msg(), before);
    }

    #[test]
    fn reporter_subsequent_tool_use_clears_failure_marker() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        r.on_event(AgentEvent::ToolUse {
            name: "Read".into(),
            summary: "/x".into(),
        });
        r.on_event(AgentEvent::ToolResult { ok: false });
        r.on_event(AgentEvent::ToolUse {
            name: "Grep".into(),
            summary: "foo".into(),
        });
        assert!(!r.agent_msg().contains("(✗)"));
    }

    #[test]
    fn reporter_call_end_hides_agent_bar() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        r.on_event(AgentEvent::CallStart {
            prompt: PromptId::Subcarve,
        });
        r.on_event(AgentEvent::CallEnd);
        assert!(!r.agent_visible());
    }
```

- [ ] **Step 2: Run the tests — should fail**

Run: `cargo test -p atlas-cli reporter_call_start_makes_agent_bar_visible`
Expected: compile error: `Reporter` does not implement `AgentObserver`.

- [ ] **Step 3: Implement `AgentObserver for Reporter`**

In `crates/atlas-cli/src/progress.rs`, after the `impl ProgressSink for Reporter { ... }` block, append:

```rust
impl AgentObserver for Reporter {
    fn on_event(&self, event: AgentEvent) {
        match event {
            AgentEvent::CallStart { prompt } => {
                {
                    let mut s = self.lock();
                    s.agent_tools = 0;
                    s.agent_last_tool = None;
                    s.agent_last_failed = false;
                }
                self.agent.set_draw_target(if self.drawing {
                    ProgressDrawTarget::stderr()
                } else {
                    ProgressDrawTarget::hidden()
                });
                self.agent.set_message(format!("↳ starting {}", prompt_label(prompt)));
            }
            AgentEvent::ToolUse { name, summary } => {
                let line = {
                    let mut s = self.lock();
                    s.agent_tools = s.agent_tools.saturating_add(1);
                    s.agent_last_tool = Some((name, summary));
                    s.agent_last_failed = false;
                    render_agent_line(&s)
                };
                self.agent.set_message(line);
            }
            AgentEvent::ToolResult { ok } => {
                if !ok {
                    let line = {
                        let mut s = self.lock();
                        s.agent_last_failed = true;
                        render_agent_line(&s)
                    };
                    self.agent.set_message(line);
                }
            }
            AgentEvent::CallEnd => {
                self.agent.set_draw_target(ProgressDrawTarget::hidden());
            }
        }
    }
}
```

Add the imports at the top of the file (next to existing `atlas_llm` import):

```rust
use atlas_llm::{AgentEvent, AgentObserver};
```

(The existing `use atlas_llm::{...}` line should be extended in place to include `AgentEvent, AgentObserver` rather than adding a duplicate `use`.)

Also add the helpers (just below the existing `pub(crate) fn render_activity_msg` block):

```rust
fn prompt_label(prompt: PromptId) -> &'static str {
    match prompt {
        PromptId::Classify => "classify",
        PromptId::Subcarve => "subcarve",
        PromptId::Stage1Surface => "surface",
        PromptId::Stage2Edges => "edges",
    }
}

/// Render the agent sub-line from the latest event state. The
/// truncation pass added in Task 10 wraps this helper.
pub(crate) fn render_agent_line(state: &ReporterState) -> String {
    let (name, summary) = state.agent_last_tool.as_ref().cloned().unwrap_or_default();
    let mut line = if summary.is_empty() {
        format!("↳ {name} · {} tools", state.agent_tools)
    } else {
        format!("↳ {name} {summary} · {} tools", state.agent_tools)
    };
    if state.agent_last_failed {
        line.push_str(" (✗)");
    }
    line
}
```

- [ ] **Step 4: Run the tests — should pass**

Run: `cargo test -p atlas-cli`
Expected: all tests pass (existing + 6 new).

- [ ] **Step 5: Lint check**

Run: `cargo clippy -p atlas-cli --tests -- -D warnings`
Expected: clean.

- [ ] **Step 6: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(atlas-cli): impl AgentObserver for Reporter; add render_agent_line"
```

---

## Task 10: Truncate the agent sub-line to fit terminal width

**Spec ref:** §3 (truncation rules), §8.2 (truncation test).

Spec rule: "the summary (tool argument) is shortened first with a trailing ellipsis (e.g. `crates/atlas-engine/.../l8_recurse.rs`); the tool name and `· N tools` counter are preserved." We pick a fixed nominal width of **120 chars** rather than querying `console::Term::stderr().size()` at runtime — adequate for any 80-200 column terminal and avoids a new dep. The full unrendered line is still emitted to indicatif, which will visually wrap if the terminal is narrower than the pre-truncation budget.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Write the failing truncation tests**

In `crates/atlas-cli/src/progress.rs`'s `tests` module, append:

```rust
    #[test]
    fn render_agent_line_truncates_long_summary_with_ellipsis() {
        let mut s = ReporterState::default();
        s.agent_tools = 23;
        s.agent_last_tool = Some((
            "Read".into(),
            "crates/atlas-engine/src/very/deep/path/that/keeps/going/and/going/l8_recurse.rs".into(),
        ));
        let out = render_agent_line_with_width(&s, 60);
        assert!(out.starts_with("↳ Read "), "tool name preserved: {out:?}");
        assert!(out.ends_with("· 23 tools"), "counter preserved: {out:?}");
        assert!(out.contains('…'), "summary ellipsised: {out:?}");
        assert!(out.chars().count() <= 60, "line within budget: {} > 60", out.chars().count());
    }

    #[test]
    fn render_agent_line_does_not_truncate_short_summary() {
        let mut s = ReporterState::default();
        s.agent_tools = 1;
        s.agent_last_tool = Some(("Read".into(), "/x.rs".into()));
        let out = render_agent_line_with_width(&s, 120);
        assert_eq!(out, "↳ Read /x.rs · 1 tools");
    }

    #[test]
    fn render_agent_line_truncation_keeps_failure_marker() {
        let mut s = ReporterState::default();
        s.agent_tools = 5;
        s.agent_last_tool = Some((
            "Read".into(),
            "very/long/path/that/will/get/truncated/aaaaaaaaaaaaaaaaaaaaaa.rs".into(),
        ));
        s.agent_last_failed = true;
        let out = render_agent_line_with_width(&s, 50);
        assert!(out.ends_with("(✗)"), "failure marker preserved: {out:?}");
        assert!(out.contains('…'), "summary ellipsised: {out:?}");
    }
```

- [ ] **Step 2: Run — should fail**

Run: `cargo test -p atlas-cli render_agent_line_truncates`
Expected: compile error: `cannot find function render_agent_line_with_width`.

- [ ] **Step 3: Refactor `render_agent_line` and add the width-aware helper**

Replace the previously-added `render_agent_line` with:

```rust
const AGENT_LINE_DEFAULT_WIDTH: usize = 120;

pub(crate) fn render_agent_line(state: &ReporterState) -> String {
    render_agent_line_with_width(state, AGENT_LINE_DEFAULT_WIDTH)
}

pub(crate) fn render_agent_line_with_width(state: &ReporterState, max_width: usize) -> String {
    let (name, summary) = state.agent_last_tool.as_ref().cloned().unwrap_or_default();
    let suffix = if state.agent_last_failed { " (✗)" } else { "" };
    let counter_part = format!(" · {} tools{}", state.agent_tools, suffix);
    let prefix_no_summary = format!("↳ {name}");
    // Compute the budget left for the summary after reserving a space and the counter.
    let fixed_len = prefix_no_summary.chars().count()
        + 1 // space after name
        + counter_part.chars().count();
    if summary.is_empty() {
        return format!("{prefix_no_summary}{counter_part}");
    }
    if fixed_len + summary.chars().count() <= max_width {
        return format!("{prefix_no_summary} {summary}{counter_part}");
    }
    // Need to truncate the summary. Reserve at least 1 char + ellipsis.
    let budget = max_width.saturating_sub(fixed_len + 1); // 1 for the ellipsis
    let kept: String = summary.chars().take(budget).collect();
    format!("{prefix_no_summary} {kept}…{counter_part}")
}
```

- [ ] **Step 4: Run — should pass**

Run: `cargo test -p atlas-cli render_agent_line`
Expected: 3 new tests + the existing reporter tests pass. The earlier `reporter_tool_use_increments_counter_and_renders_line` test (Task 9) asserts the full untruncated form on a width-120 default; with summary `"crates/atlas-engine/src/l8_recurse.rs"` (35 chars), the line is `↳ Read crates/atlas-engine/src/l8_recurse.rs · 1 tools` (54 chars), still under 120, so no truncation, no regression.

- [ ] **Step 5: Lint check**

Run: `cargo clippy -p atlas-cli --tests -- -D warnings`
Expected: clean.

- [ ] **Step 6: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(atlas-cli): truncate long agent sub-line summaries with trailing ellipsis"
```

---

## Task 11: Wire `Reporter` as the agent observer in the production backend

**Spec ref:** §5.4.

Today `build_production_backend` constructs `ClaudeCodeBackend`, wraps it in `BudgetedBackend`, then `BudgetSentinel`, and returns the stack. The reporter is not yet visible at construction time. The cleanest change: add an optional `observer` parameter to `build_production_backend`; `main.rs` passes the reporter (cast to `Arc<dyn AgentObserver>`) when `reporter.drawing()`.

**Files:**
- Modify: `crates/atlas-cli/src/backend.rs`
- Modify: `crates/atlas-cli/src/main.rs`

- [ ] **Step 1: Make `Reporter::drawing` a public accessor**

In `crates/atlas-cli/src/progress.rs`, the existing `#[cfg(test)] pub(crate) fn drawing(&self) -> bool` test-only accessor needs to be visible to `main.rs`. Promote it to a regular public accessor (drop the `#[cfg(test)]` and `pub(crate)`):

```rust
pub fn drawing(&self) -> bool {
    self.drawing
}
```

(There is also the `field` `drawing: bool` itself; do not change visibility on the field — only on the accessor.)

- [ ] **Step 2: Extend `build_production_backend`**

In `crates/atlas-cli/src/backend.rs`, modify the function signature and body:

```rust
pub fn build_production_backend(
    model_id: String,
    budget: Option<u64>,
    observer: Option<Arc<dyn atlas_llm::AgentObserver>>,
) -> Result<BackendHandles> {
    let prompts_dir = TempDir::new()?;
    crate::prompts::materialise_to(prompts_dir.path())?;

    let template_sha = compute_template_sha();
    let ontology_sha = compute_ontology_sha();

    let mut inner = ClaudeCodeBackend::new(model_id.clone(), prompts_dir.path())?
        .with_fingerprint_inputs(template_sha, ontology_sha);
    if let Some(o) = observer {
        inner = inner.with_observer(o);
    }
    let version_fingerprint = inner.fingerprint();
    let inner_arc: Arc<dyn LlmBackend> = Arc::new(inner);
    // ... rest unchanged ...
```

The rest of the function (BudgetedBackend wrap, sentinel, return) stays as is.

- [ ] **Step 3: Wire it from `main.rs`**

In `crates/atlas-cli/src/main.rs`, replace:

```rust
let handles = atlas_cli::backend::build_production_backend(model_id, args.budget)
    .context("failed to build LLM backend")?;
config.fingerprint_override = Some(handles.fingerprint.clone());

let progress_mode = if args.no_progress {
    ProgressMode::Never
} else if args.progress {
    ProgressMode::Always
} else {
    ProgressMode::Auto
};
let reporter = make_stderr_reporter(progress_mode, handles.counter.clone());
```

with:

```rust
let progress_mode = if args.no_progress {
    ProgressMode::Never
} else if args.progress {
    ProgressMode::Always
} else {
    ProgressMode::Auto
};

// Construct reporter first (without a counter; we'll attach one
// after the backend is built and we know `args.budget`).
//
// Actually the existing code passes `handles.counter` to the reporter
// constructor — keep that ordering by building the backend in two
// phases is messier. Simpler: build the reporter against an
// Option<Arc<TokenCounter>> that we initialise to None, then patch
// it in. That requires a Reporter setter we don't have. Fall back to
// ordering: build reporter with a fresh counter only if budget is
// Some, else None — derive the counter ourselves up here.
let counter = args.budget.map(|b| {
    std::sync::Arc::new(atlas_llm::TokenCounter::new(b))
});
let reporter = make_stderr_reporter(progress_mode, counter.clone());

let observer = if reporter.drawing() {
    Some(reporter.clone() as std::sync::Arc<dyn atlas_llm::AgentObserver>)
} else {
    None
};

let handles = atlas_cli::backend::build_production_backend_with_counter(
    model_id,
    counter.clone(),
    observer,
)
.context("failed to build LLM backend")?;
config.fingerprint_override = Some(handles.fingerprint.clone());
```

This refactor introduces a slight backend.rs change: `build_production_backend` historically owned the counter creation. We now pull it up to `main.rs` so the reporter gets the **same** counter the backend uses. Add a sibling helper `build_production_backend_with_counter` that takes a pre-made counter:

In `crates/atlas-cli/src/backend.rs`, refactor:

```rust
pub fn build_production_backend_with_counter(
    model_id: String,
    counter: Option<Arc<TokenCounter>>,
    observer: Option<Arc<dyn atlas_llm::AgentObserver>>,
) -> Result<BackendHandles> {
    let prompts_dir = TempDir::new()?;
    crate::prompts::materialise_to(prompts_dir.path())?;

    let template_sha = compute_template_sha();
    let ontology_sha = compute_ontology_sha();

    let mut inner = ClaudeCodeBackend::new(model_id.clone(), prompts_dir.path())?
        .with_fingerprint_inputs(template_sha, ontology_sha);
    if let Some(o) = observer {
        inner = inner.with_observer(o);
    }
    let version_fingerprint = inner.fingerprint();
    let inner_arc: Arc<dyn LlmBackend> = Arc::new(inner);

    let backend_after_budget: Arc<dyn LlmBackend> = match counter.as_ref() {
        Some(c) => Arc::new(BudgetedBackend::new(
            inner_arc,
            c.clone(),
            default_token_estimator(),
        )),
        None => inner_arc,
    };
    let sentinel = BudgetSentinel::new(backend_after_budget);
    let backend: Arc<dyn LlmBackend> = sentinel.clone();

    Ok(BackendHandles {
        backend,
        counter,
        sentinel,
        fingerprint: version_fingerprint,
        prompts_dir,
    })
}

/// Convenience overload preserving the prior API for any caller that
/// hasn't been migrated. Internal CLI now goes through `_with_counter`.
pub fn build_production_backend(model_id: String, budget: Option<u64>) -> Result<BackendHandles> {
    let counter = budget.map(|b| Arc::new(TokenCounter::new(b)));
    build_production_backend_with_counter(model_id, counter, None)
}
```

(The legacy `build_production_backend` is left as a one-line shim so any tests that call it keep working.)

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo build --workspace`
Expected: clean.

- [ ] **Step 5: Run the full atlas-cli test suite**

Run: `cargo test -p atlas-cli`
Expected: all tests pass (no test asserts on the prior counter-creation site, so the refactor is transparent).

- [ ] **Step 6: Smoke-run `atlas index --help` to ensure the binary still launches**

Run:
```bash
cargo run -p atlas-cli -- index --help
```
Expected: prints the help text, exits 0.

- [ ] **Step 7: Lint check**

Run: `cargo clippy --workspace --tests -- -D warnings`
Expected: clean.

- [ ] **Step 8: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-cli/src/progress.rs crates/atlas-cli/src/backend.rs crates/atlas-cli/src/main.rs
git commit -m "feat(atlas-cli): wire Reporter as ClaudeCodeBackend AgentObserver when drawing"
```

---

## Task 12: End-to-end gated integration test

**Spec ref:** §8.3.

Add a single end-to-end test under `crates/atlas-cli/tests/agent_observer_e2e.rs` that:
1. Skips silently unless `ATLAS_LLM_RUN_CLAUDE_TESTS=1`.
2. Builds a tiny fixture project under a `tempfile::TempDir` (one `Cargo.toml`, one `src/lib.rs`).
3. Runs `cargo run -p atlas-cli -- index <fixture> --no-budget --progress=always --no-gitignore` via `assert_cmd::Command`.
4. Asserts: exit 0, four YAMLs exist under `<fixture>/.atlas/`, captured stderr contains the literal `↳` glyph.

Memory: `assert-cmd-real-binary-tests-must-be-gated-behind-atlas-llm-run-claude-tests-1`.

**Files:**
- Create: `crates/atlas-cli/tests/agent_observer_e2e.rs`
- Modify: `crates/atlas-cli/Cargo.toml` (add `assert_cmd` and `predicates` dev-deps if not already present)

- [ ] **Step 1: Confirm `assert_cmd` and `predicates` are already dev-deps**

Run: `grep -E '(assert_cmd|predicates|tempfile)' /Users/antony/Development/Atlas/crates/atlas-cli/Cargo.toml`
Expected (if present):
```
assert_cmd = { workspace = true }
predicates = { workspace = true }
tempfile = { workspace = true }
```

If missing, add them under `[dev-dependencies]` and verify with `cargo check -p atlas-cli --tests`.

- [ ] **Step 2: Write the test**

Create `crates/atlas-cli/tests/agent_observer_e2e.rs`:

```rust
//! End-to-end smoke test for the agent-observer pipeline.
//!
//! Spawns the real `claude -p` binary; gated behind
//! `ATLAS_LLM_RUN_CLAUDE_TESTS=1` so the default `cargo test` run does
//! not burn tokens.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

fn claude_tests_enabled() -> bool {
    std::env::var("ATLAS_LLM_RUN_CLAUDE_TESTS").ok().as_deref() == Some("1")
}

#[test]
fn atlas_index_emits_agent_sub_line_under_progress_always() {
    if !claude_tests_enabled() {
        eprintln!(
            "skipping: opt in with ATLAS_LLM_RUN_CLAUDE_TESTS=1 to spawn `claude`"
        );
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "atlas-e2e-fixture"
version = "0.0.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(root.join("src").join("lib.rs"), "pub fn it() {}\n").unwrap();

    Command::cargo_bin("atlas")
        .unwrap()
        .arg("index")
        .arg(root)
        .arg("--no-budget")
        .arg("--progress=always")
        .arg("--no-gitignore")
        .assert()
        .success()
        .stderr(contains("↳"));

    let atlas_dir = root.join(".atlas");
    for f in [
        "components.yaml",
        "external-components.yaml",
        "related-components.yaml",
        "llm-cache.json",
    ] {
        assert!(
            atlas_dir.join(f).exists(),
            "{f} should exist after index"
        );
    }
}
```

Note: `--progress=always` is a flag we expose only via the `--progress` boolean today. Verify in `main.rs` whether the flag accepts `=value`. Looking at the current Clap definition, `--progress` is a bare bool flag, so the test should use `--progress` (no `=always`):

Adjust the test to:
```rust
        .arg("--progress")
```
not `.arg("--progress=always")`.

- [ ] **Step 3: Run the gated test in opt-out mode (default)**

Run: `cargo test -p atlas-cli atlas_index_emits_agent_sub_line`
Expected: passes immediately because the function returns early without `ATLAS_LLM_RUN_CLAUDE_TESTS=1`.

- [ ] **Step 4: Run the gated test in opt-in mode (manual; this burns ~$0.05)**

Run (only when the user explicitly approves the cost):

```bash
ATLAS_LLM_RUN_CLAUDE_TESTS=1 cargo test -p atlas-cli atlas_index_emits_agent_sub_line -- --nocapture
```

Expected: pass (one full atlas run, ~30-90s).

- [ ] **Step 5: Lint check**

Run: `cargo clippy -p atlas-cli --tests -- -D warnings`
Expected: clean.

- [ ] **Step 6: Format + commit**

```bash
cargo fmt --all
git add crates/atlas-cli/tests/agent_observer_e2e.rs
# crates/atlas-cli/Cargo.toml only if dev-deps were added
git commit -m "test(atlas-cli): add gated e2e test asserting agent sub-line ↳ glyph appears under --progress"
```

---

## Task 13: Capture distilled-memory entries

**Spec ref:** §10 (memory entries to write on completion).

**Files:**
- New entries via `ravel-lite state memory add`. (No Atlas source-tree changes.)

- [ ] **Step 1: Add the four memory entries**

Run each command from `/Users/antony/Development/Atlas`:

```bash
ravel-lite state memory add LLM_STATE/core \
  --id claudecode-uses-stream-json-verbose-non-streaming-removed \
  --title "ClaudeCodeBackend always uses --output-format stream-json --verbose" \
  --body "ClaudeCodeBackend now spawns `claude -p --output-format stream-json --verbose` for every call; the legacy non-streaming `--output-format json` path was removed in the agent-progress-feedback work. Single mental model, single test surface, no flag plumbing. Spec §4.2."
```

```bash
ravel-lite state memory add LLM_STATE/core \
  --id agentobserver-side-channel-bypasses-cache-and-budget \
  --title "AgentObserver is a side-channel parallel to Reporter::on_llm_call" \
  --body "AgentObserver events flow ClaudeCodeBackend -> Reporter directly, bypassing BudgetSentinel and LlmResponseCache. A cached re-run produces zero AgentEvent instances; budget exhaustion is still signalled via BudgetSentinel. Spec §4.1."
```

```bash
ravel-lite state memory add LLM_STATE/core \
  --id stream-json-result-event-is-only-load-bearing-parser-path \
  --title "stream-json result event is the only load-bearing parser path" \
  --body "parse_stream skips unknown event types and non-JSON lines without erroring; only the terminal `result` event extraction is required. This is the explicit forward-compat goal so future CLI schema additions do not break the parser. Spec §7.4."
```

```bash
ravel-lite state memory add LLM_STATE/core \
  --id claude-stderr-must-be-piped-and-drained-to-avoid-deadlock \
  --title "claude stderr must be piped and drained when using Command::spawn()" \
  --body "ClaudeCodeBackend pipes stderr via Stdio::piped() and drains it on a worker thread. Without the drainer, large stderr output (e.g. setup warnings, deprecation banners) fills the pipe buffer and the child blocks waiting for the parent to consume it, deadlocking the run. Spec §7.7."
```

- [ ] **Step 2: Verify the entries appear**

Run: `ravel-lite state memory list LLM_STATE/core --format markdown | grep -i "stream-json\|AgentObserver\|stderr"`
Expected: four matching lines.

- [ ] **Step 3: No commit needed**

Memory entries are written into `LLM_STATE/core/` and are committed by the orchestrator's analyse-work phase, not by this task.

---

## Self-review

### Spec coverage

| Spec section | Tasks covering it |
|---|---|
| §1, §2 (problem/goals) | covered indirectly by the whole plan |
| §3 (user-visible behaviour) | Tasks 8, 9, 10 |
| §4 (architecture, side-channel) | Tasks 1, 7, 11 |
| §4.1 (invariants) | Task 11 (observer is None on cache hit because reporter sees no `on_llm_call` either when ProgressBackend short-circuits — already true; cache layer is untouched in this plan) |
| §4.2 (streaming subprocess) | Task 7 |
| §5.1 (AgentObserver/AgentEvent/tool_summary_for) | Tasks 1, 2 |
| §5.2 (ClaudeCodeBackend changes) | Tasks 6, 7 |
| §5.3 (Reporter state + impl) | Tasks 8, 9, 10 |
| §5.4 (driver wiring) | Task 11 |
| §5.5 (cross-crate considerations) | architectural; addressed by file layout in Tasks 1, 9 |
| §6 (data flow) | Tasks 4, 7 |
| §6.1 (concurrency) | accepted as is — single in-flight call assumption preserved by single-bar Reporter |
| §7 (error handling) | Tasks 4 (parser), 7 (subprocess wait/stderr) |
| §7.5 (CallEnd ordering) | Task 4 (ObserverGuard) |
| §7.7 (stderr piped + drained) | Task 7 |
| §8.1 (stream parser unit tests) | Tasks 3, 4, 5 |
| §8.2 (Reporter rendering tests) | Tasks 8, 9, 10 |
| §8.3 (e2e integration test) | Task 12 |
| §8.4 (TestBackend invariance) | accepted — no test changes (verified by Task 7 step 3) |
| §10 (memory entries) | Task 13 |

### Placeholder scan

Searched the plan for `TBD`, `TODO`, `implement later`, `add appropriate error handling`, `similar to Task N` — none present. All steps include exact code or commands.

### Type consistency

- `AgentObserver` and `AgentEvent` are introduced in Task 1 with the exact signatures used in Tasks 4, 6, 7, 9, 11.
- `tool_summary_for(tool_name: &str, input: &Value) -> String` (Task 2) matches the call site in Task 3 (`extract_tool_uses`).
- `parse_stream<R: Read>(reader: R, observer: Option<&Arc<dyn AgentObserver>>, prompt: PromptId) -> Result<Value, LlmError>` is consistent across Tasks 4, 5 (fixture tests), and 7 (call site).
- `render_agent_line(state: &ReporterState) -> String` is introduced in Task 9 and refactored in Task 10 to wrap `render_agent_line_with_width`. The earlier Task 9 tests assert the no-truncation form on a 120-default; this is verified to hold (Task 10 step 4 note).
- `Reporter::drawing()` is `pub(crate)` test-only in Task 8 and promoted to `pub` in Task 11 step 1. No collision.
- `build_production_backend_with_counter` is added in Task 11 with the signature called from `main.rs`.

No inconsistencies found.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-01-agent-progress-feedback.md`. Two execution options:

**1. Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

**Which approach?**
