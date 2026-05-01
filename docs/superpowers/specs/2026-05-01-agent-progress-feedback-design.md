# Agent Progress Feedback During Long LLM Calls

**Status:** draft
**Date:** 2026-05-01
**Companion to:** `2026-05-01-engine-progress-events-design.md`

## 1. Problem

A single subcarve LLM call against `claude -p` can take ten minutes or more. During that time the user sees:

```
  ⠋ iter 0 · subcarve  1/74 (agent)  00:09:39
    tokens 18.0k/200.0k  [█████████░░░░░░░░░░░░]  35%
```

The k/n counter, target relpath, and elapsed timer are all updating, but nothing reveals what the agent is actually doing inside that ten-minute window. Users cannot distinguish a stuck process from a working one, and have no way to tell whether the agent is reading files, running greps, dispatching subagents, or generating its final response.

A heartbeat-only signal ("still alive, X seconds elapsed") was rejected as insufficient: the user wants substantive content about agent activity, not just liveness.

## 2. Goals and non-goals

### Goals

- Surface the **most recent tool the agent invoked** plus a running **count of tools used** for each in-flight LLM call.
- Update in place on a single dedicated screen line — no scrollback flooding.
- Auto-enable when `Reporter` is drawing to a terminal, auto-disable when progress is suppressed.
- Preserve byte-identity of cached re-runs: streaming events fire only on cache misses.
- Preserve all existing error-handling guarantees: `BudgetExhausted`, `Schema`, `Setup`, etc. unchanged.

### Non-goals

- **Streaming partial assistant text.** Tool calls are more diagnostic than the final answer text; surfacing message deltas is out of scope.
- **Recursing into Task subagent activity.** The CLI represents a Task tool call as a single `tool_use`/`tool_result` pair from the parent's perspective. Surfacing what runs *inside* a subagent requires deeper integration and is deferred.
- **Parallel-call rendering.** Atlas drives LLM calls serially today (`fixedpoint.rs` loops over `live` components in order; L5/L6 are also serial). The design assumes one in-flight call at a time. If parallelism is added later, the agent sub-line must become per-call rather than global.
- **Adding visibility to non-`ClaudeCodeBackend` paths.** `TestBackend` does not stream and does not need to.

## 3. User-visible behaviour

When `Reporter` is drawing and an LLM call is in flight, a third indented sub-line appears between the activity bar and the token gauge:

```
  ⠋ iter 0 · subcarve  1/74 (agent)  00:09:39
      ↳ Read crates/atlas-engine/src/l8_recurse.rs · 23 tools
    tokens 18.0k/200.0k  [█████████░░░░░░░░░░░░]  35%
```

The sub-line:

- **Mounts** on the first event of the call and **unmounts** when the call returns (success or error).
- **Refreshes in place** on each `tool_use` event with the most recent tool name + a short summary of its key argument.
- **Increments a counter** (`· N tools`) on each `tool_use` event.
- **Marks errored tool results** with a trailing `(✗)` until the next `tool_use` clears it. Successful tool results are invisible.
- **Truncates** the line to terminal width: the summary (tool argument) is shortened first with a trailing ellipsis (e.g. `crates/atlas-engine/.../l8_recurse.rs`); the tool name and `· N tools` counter are preserved.

When `--progress=never` (or non-TTY): the sub-line never appears. The backend still returns the correct response — streaming continues to run, but with no observer attached, all events are discarded.

## 4. Architecture

Agent events are **out-of-band telemetry**: they flow from `ClaudeCodeBackend` directly to `Reporter` via a side channel, parallel to the existing `Reporter::on_llm_call` pattern documented in the engine-progress-events spec §6.3. They bypass the rest of the backend chain because they are transient — the cache and budget logic still operate on the request/response pair, not on intermediate events.

```
                              ┌─── Reporter ──────────────────┐
                              │   on_llm_call    (existing)   │
                              │   on_agent_event (NEW)        │
                              └─────────▲────────▲────────────┘
                                        │        │ side-channel
                                        │        │
  driver         BudgetSentinel   ProgressBackend   LlmResponseCache   ClaudeCodeBackend
  ──────→  ────→ (token gate)  ─→ (counts calls) ─→ (memo on req)  ─→  (subprocess, streams)
                                        ▲                                      │
                                        └──────────── on_llm_call ◀────────────┤
                                                                               │ for each event
                                                            on_agent_event ◀───┘
```

### 4.1 Invariants preserved

- **Cache layer is unchanged.** `LlmResponseCache` keys on the canonicalised request and stores only the final JSON response. A cached re-run produces zero `AgentEvent`s and the existing "no-op re-run is byte-identical" exit criterion still holds.
- **`ProgressBackend` ordering preserved.** `ProgressBackend::call` continues to fire `on_llm_call` *after* the inner backend returns. Agent events fire *during* the inner call, before `ProgressBackend.call` returns. So `on_llm_call` (which sets the per-prompt counter and target) lands after all `AgentEvent`s for that call — the natural ordering for the final `· N tools` tally.

### 4.2 Streaming subprocess invocation

The streaming subprocess invocation lives entirely inside `ClaudeCodeBackend::call`. Caller signature is unchanged:

```rust
fn call(&self, req: &LlmRequest) -> Result<Value, LlmError>;
```

Approach A from brainstorming was selected: the streaming code path is the *only* code path. When no observer is attached, the backend still parses the stream but discards events. The non-streaming `--output-format json` path is removed. Rationale: a single mental model, single test surface, no flag plumbing, and the parsing cost is negligible compared with the LLM call itself.

## 5. Components

### 5.1 New types in `atlas-llm`

A new module `atlas-llm/src/agent_observer.rs` introduces:

```rust
/// Side-channel emitted by streaming-capable backends as the agent
/// works. Implementors receive transient events; the backend's
/// `LlmBackend::call` return value still carries the canonical
/// response.
pub trait AgentObserver: Send + Sync {
    fn on_event(&self, event: AgentEvent);
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Fired once at the start of an `LlmBackend::call`.
    CallStart { prompt: PromptId },
    /// One per `tool_use` block in any assistant turn.
    ToolUse { name: String, summary: String },
    /// One per `tool_result` block in any user turn.
    ToolResult { ok: bool },
    /// Fired once at the end of an `LlmBackend::call`, in every
    /// termination path (success, error, panic recovery).
    CallEnd,
}
```

`ToolUse.summary` is a short single-line string distilled from the tool's `input` JSON by the backend, so the Reporter does not need to know tool-specific input shapes. The mapping is a small lookup table:

| Tool name | Source field | Example summary |
|---|---|---|
| `Read` | `input.file_path` | `crates/atlas-engine/src/l8_recurse.rs` |
| `Grep` | `input.pattern` | `"ProgressEvent::Subcarve"` |
| `Bash` | `input.command` | `cargo test --workspace` |
| `Edit` | `input.file_path` | `crates/atlas-cli/src/progress.rs` |
| `Write` | `input.file_path` | `crates/atlas-llm/src/agent_observer.rs` |
| `Glob` | `input.pattern` | `crates/**/*.rs` |
| `Task` | `input.subagent_type` | `Explore: <description>` |
| Unknown | (none) | empty string; Reporter renders just the tool name |

Tool blocks with malformed `input` shapes fall back to `summary = ""`. Telemetry never crashes the call.

### 5.2 Modifications to `ClaudeCodeBackend`

```rust
pub struct ClaudeCodeBackend {
    model_id: String,
    prompts_dir: PathBuf,
    version: String,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    observer: Option<Arc<dyn AgentObserver>>,   // NEW
}

impl ClaudeCodeBackend {
    pub fn with_observer(mut self, observer: Arc<dyn AgentObserver>) -> Self {
        self.observer = Some(observer);
        self
    }
    // ... existing methods unchanged ...
}
```

The `call` implementation is rewritten end-to-end (see §6).

### 5.3 New state and methods on `Reporter`

`Reporter` adds:

- `agent_bar: ProgressBar` — third `MultiProgress` bar mounted under `activity`. Hidden by default; revealed on `CallStart`, hidden on `CallEnd`.
- New fields on `ReporterState`:
  - `agent_tools: u64`
  - `agent_last_tool: Option<(String, String)>` — `(name, summary)`
  - `agent_last_failed: bool` — whether the most recent `ToolResult` was `ok: false`
- `impl AgentObserver for Reporter`:

  ```rust
  fn on_event(&self, event: AgentEvent) {
      match event {
          AgentEvent::CallStart { prompt } => {
              let mut s = self.lock();
              s.agent_tools = 0;
              s.agent_last_tool = None;
              s.agent_last_failed = false;
              drop(s);
              self.agent_bar.set_draw_target(/* visible */);
              self.agent_bar.set_message(format!("starting {}", prompt_label(prompt)));
          }
          AgentEvent::ToolUse { name, summary } => {
              let mut s = self.lock();
              s.agent_tools += 1;
              s.agent_last_tool = Some((name, summary));
              s.agent_last_failed = false;
              let line = render_agent_line(&s);
              drop(s);
              self.agent_bar.set_message(line);
          }
          AgentEvent::ToolResult { ok } => {
              if !ok {
                  let mut s = self.lock();
                  s.agent_last_failed = true;
                  let line = render_agent_line(&s);
                  drop(s);
                  self.agent_bar.set_message(line);
              }
          }
          AgentEvent::CallEnd => {
              self.agent_bar.set_draw_target(/* hidden */);
          }
      }
  }
  ```

- New test-only accessors: `agent_msg() -> String`, `agent_tools() -> u64`, `agent_visible() -> bool`.

### 5.4 Driver wiring in `atlas-cli::pipeline`

After constructing `Reporter` and before constructing the backend chain:

```rust
let backend = ClaudeCodeBackend::new(model_id, prompts_dir)?
    .with_fingerprint_inputs(template_sha, ontology_sha);

let backend = if reporter.drawing() {
    backend.with_observer(reporter.clone() as Arc<dyn AgentObserver>)
} else {
    backend
};

// ... existing BudgetSentinel / ProgressBackend / LlmResponseCache wrapping unchanged ...
```

`Reporter` already exists as `Arc<Reporter>`. Casting to `Arc<dyn AgentObserver>` is a single `as` since `Reporter: AgentObserver`.

### 5.5 Cross-crate considerations

- `AgentObserver`/`AgentEvent` live in `atlas-llm` (where `LlmBackend` lives). `Reporter` in `atlas-cli` imports them — same direction as the existing `LlmBackend` and `PromptId` imports. No new dependency edges.
- `TestBackend` does not implement streaming; it gets no observer plumbing. Tests that need to exercise tool-event flow use a fresh `MockStreamBackend` defined in `atlas-llm` test code that synthesises canned `AgentEvent`s without a subprocess.

## 6. Data flow: lifecycle of one streaming call

1. **Driver** (`pipeline.rs`) issues an `LlmRequest` via the wrapped backend chain. The request reaches `ClaudeCodeBackend::call` after passing `BudgetSentinel`, `ProgressBackend`, and `LlmResponseCache`. If the cache hits, none of the streaming logic runs.

2. **`ClaudeCodeBackend::call`** renders the prompt, then spawns:

   ```
   claude -p <rendered>
          --output-format stream-json
          --verbose
          --model <model-id>
   ```

   `--verbose` is required by the CLI when `--output-format stream-json` is used. stdin is closed; stdout and stderr are both captured via `Stdio::piped()` (stderr drained by a small thread to avoid pipe-buffer deadlock).

3. **Stream loop.** A `BufReader` wraps the child's stdout. The backend reads lines until EOF:

   ```rust
   let _guard = ObserverGuard::new(self.observer.as_ref());
   if let Some(o) = &self.observer {
       o.on_event(AgentEvent::CallStart { prompt: req.prompt_template });
   }
   let mut terminal: Option<Value> = None;
   for line in reader.lines() {
       let line = line.map_err(|e| LlmError::Invocation(format!(
           "stdout read failure: {e}"
       )))?;
       let Ok(value) = serde_json::from_str::<Value>(&line) else {
           continue;  // non-JSON line: skip
       };
       match value.get("type").and_then(|v| v.as_str()) {
           Some("assistant") => emit_tool_uses(&value, self.observer.as_ref()),
           Some("user")      => emit_tool_results(&value, self.observer.as_ref()),
           Some("result")    => terminal = Some(value),
           _                 => {} // unknown event: skipped (forward-compat)
       }
   }
   // ObserverGuard fires CallEnd here, regardless of return path.
   ```

4. **Tool extraction.**
   - `emit_tool_uses` walks the assistant message's `content[]` array. For each block with `"type": "tool_use"`, it calls `tool_summary_for(name, input)` and emits `AgentEvent::ToolUse { name, summary }`.
   - `emit_tool_results` walks `content[]` for `"type": "tool_result"` blocks and emits `AgentEvent::ToolResult { ok: !is_error }`.

5. **Response extraction.** After the loop:
   - If `terminal` is `None` → `LlmError::Parse("stream ended without `result` event; ...")`.
   - If `terminal["subtype"] != "success"` → `LlmError::Invocation(...)` with the error subtype as message.
   - Otherwise extract `terminal["result"]` (the assistant's final string), `serde_json::from_str` it, validate against `req.schema`, return.

6. **Process cleanup.** Wait on the child after the loop; non-zero exit with no terminal event → `LlmError::Invocation` with stderr captured.

7. **Reporter side-channel handling** (concurrent with steps 3–5):
   - `CallStart` → reset counters, mount agent bar, set initial message.
   - `ToolUse { name, summary }` → increment `agent_tools`, set `agent_last_tool`, refresh sub-line.
   - `ToolResult { ok: false }` → set `agent_last_failed`, refresh sub-line (appends `(✗)`).
   - `CallEnd` → hide agent bar.

8. **`ProgressBackend.call` returns** to `LlmResponseCache.call`, which stores the response under the request's canonical key. Then `Reporter::on_llm_call` fires (existing behaviour, unchanged).

### 6.1 Concurrency

`Reporter` is already `Sync` via `Mutex<ReporterState>`. Stream events fire from the same thread that called `LlmBackend::call`, so there is no new threading concern: Salsa drives queries serially per database, and `fixedpoint.rs` and L5/L6 loops are also serial.

The single-bar design assumes one in-flight LLM call at a time. Indicatif's `MultiProgress` serialises *draws* (no torn writes from concurrent `set_message` calls), but the *shared state* on `ReporterState` (`agent_tools`, `agent_last_tool`) is single-call: two parallel calls would interleave their counters and tool names on the same bar. If parallel LLM calls are introduced later, the bar must become per-call (keyed by a call-id field added to `AgentEvent`) and the shared counter fields removed.

## 7. Error handling

The streaming path adds three new failure modes; everything else degrades through the existing `LlmError` variants.

### 7.1 Stream ended without a `result` event

The CLI exited (cleanly or otherwise) before emitting `{"type": "result", ...}`. Causes: subprocess crashed, killed by SIGINT, or future schema change to the terminal event. Returns:

```rust
LlmError::Parse(format!(
    "stream ended without `result` event; \
     subprocess exit={status}, stderr={truncated}"
))
```

`BudgetSentinel`/cache do not record this call.

### 7.2 `result` event with non-success subtype

CLI completed normally but the agent itself errored — `subtype` is `"error_max_turns"`, `"error_during_execution"`, etc. Returns:

```rust
LlmError::Invocation(format!(
    "claude reported {subtype}: {error_message}"
))
```

This catches "agent ran out of turns on subcarve" cleanly rather than leaving it as a JSON-parse failure further upstream.

### 7.3 `result` event has unparseable / non-JSON `result` text

The agent did its job but the final text isn't valid JSON. Returns:

```rust
LlmError::Parse(format!(
    "claude `result.result` was not valid JSON: {e} \
     (first 200 bytes: {snippet})"
))
```

Same shape as today's `LlmError::Parse` from the non-streaming path.

### 7.4 Non-fatal cases (skipped, not propagated)

- Lines that don't parse as JSON. The CLI may emit non-JSON status lines or blank lines.
- JSON values without a `type` field.
- `type` values we don't recognise. **Forward-compatibility is the explicit design goal.** Only the terminal `result` event extraction is load-bearing.
- Tool blocks with malformed `input` shapes. The extractor falls back to `summary = ""`.

### 7.5 `CallEnd` ordering guarantee

`CallEnd` always fires at the end of `call`, in **all** termination paths — success, error, panic recovery. Enforced by an `ObserverGuard`:

```rust
struct ObserverGuard<'a> {
    observer: Option<&'a Arc<dyn AgentObserver>>,
}
impl<'a> ObserverGuard<'a> {
    fn new(observer: Option<&'a Arc<dyn AgentObserver>>) -> Self {
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

Without this, an early-return on parse error would leave the agent bar mounted indefinitely.

### 7.6 Existing variants — unchanged

- `BudgetExhausted` still propagates from `BudgetSentinel`, never reaches `ClaudeCodeBackend`.
- `Schema` still fires from `validate_response` after the JSON is extracted.
- `Setup` (e.g. missing `claude` binary) still fires at `ClaudeCodeBackend::new`, before any streaming.

### 7.7 Stderr capture

Today's `Command::output()` captures stderr automatically. The streaming path uses `Command::spawn()` with `Stdio::piped()` for stdout. Stderr must also be piped to allow inclusion in error messages, and drained by a small thread that accumulates into a `Vec<u8>`, to avoid pipe-buffer deadlock when the child writes large stderr output.

## 8. Testing

### 8.1 Stream parser unit tests

*Question: given a recorded stream-json transcript, do we extract the right events and the right final response?*

- Test fixtures: capture three real `claude -p --output-format stream-json --verbose` transcripts, check them into `crates/atlas-llm/tests/fixtures/stream/`:
  - `simple-classify.jsonl` — one assistant turn, no tool use, `result` with valid JSON.
  - `subcarve-with-tools.jsonl` — multiple assistant turns interleaved with tool blocks.
  - `error-max-turns.jsonl` — `result` with `subtype: "error_max_turns"`.
- Helper `parse_stream(reader, observer) -> Result<Value, LlmError>` exposed `pub(crate)`. Tests feed each fixture with a `RecordingObserver` and assert event count, order, and returned value.
- Negative cases: truncated stream, malformed JSON line in the middle (skipped, no crash), unknown event type (skipped).

### 8.2 Reporter rendering tests

*Question: given a sequence of `AgentEvent`s, does the Reporter produce the right sub-line text and counter values?*

- Direct `Reporter::on_event(AgentEvent::ToolUse { ... })` calls — no subprocess. Pattern parallels existing `on_llm_call` tests.
- Assertions via `agent_msg()`, `agent_tools()`, `agent_visible()`:
  - `CallStart` → bar visible, message is initial state.
  - `ToolUse {Read, ...}` → counter 1, expected sub-line.
  - Multiple `ToolUse` events → counter ticks correctly.
  - `ToolResult {ok: false}` → next refresh shows `(✗)` marker; cleared by next `ToolUse`.
  - `CallEnd` → bar hidden.
- Truncation test: `summary` of 200 chars on a `terminal_width = 80` Reporter → trailing ellipsis at correct position.

### 8.3 End-to-end integration test

*Question: does the wired-up driver actually drive a real `claude` subprocess in streaming mode and end up with the same final YAML?*

- One new integration test, gated behind `ATLAS_LLM_RUN_CLAUDE_TESTS=1` (existing convention; see memory `assert-cmd-real-binary-tests-must-be-gated-behind-atlas-llm-run-claude-tests-1`):
  - Run `atlas index` against a tiny fixture project under `--progress=always`.
  - Assert success, the four output YAMLs are produced, and captured stderr contains at least one `↳` line.
- No second test for non-streaming — the same `call` runs in both modes (with or without observer); existing test corpus already covers `--progress=never`.

### 8.4 TestBackend invariance

`TestBackend` does not stream. None of its tests change. This is a deliberate property of the design — the streaming concern is fully isolated to `ClaudeCodeBackend`.

### 8.5 Fixture maintenance

When new `claude` versions ship, fixture transcripts may go stale. Mitigation: the parser is forgiving (unknown events skipped) so most CLI changes will not break tests; if a load-bearing field is renamed, the failing fixture localises the breakage. Re-record by running the CLI once and committing the new fixture.

## 9. Open questions and deferred work

- **Parallel LLM calls.** Today: serial. The single-bar design is not parallel-safe. If parallelism is introduced (e.g. via Salsa parallel queries with a multi-thread runtime), the agent bar must become per-call, keyed by some call-id field added to every `AgentEvent`.
- **Subagent recursion.** A `Task` tool call shows up as a single `tool_use`/`tool_result` pair from the parent's stream. Surfacing what runs *inside* a subagent requires either the CLI to relay sub-stream events (not currently supported) or a different integration path (e.g. running subagents via the Claude Agent SDK directly). Deferred.
- **Configurability of summaries.** The `tool_summary_for` lookup table is currently hard-coded. If users want different summary formats for specific tools, a future change could parameterise it.

## 10. Memory entries to write on completion

After implementation lands, capture these distilled facts in plan memory:

- "ClaudeCodeBackend always uses `--output-format stream-json --verbose`; non-streaming path was removed in this change."
- "AgentObserver is a side-channel parallel to `Reporter::on_llm_call`; events bypass the cache and `BudgetSentinel`."
- "stream-json `result` event is the only load-bearing parser path; unknown event types are intentionally skipped for forward compatibility."
- "stderr must be piped *and* drained when using `Command::spawn()` for `claude` — otherwise large stderr output deadlocks on a full pipe buffer."
