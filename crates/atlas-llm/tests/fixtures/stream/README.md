# stream-json fixtures

Real `claude -p --output-format stream-json --verbose` transcripts captured with `claude` v2.1.126 against `claude-haiku-4-5`, used as input to the stream-parser unit tests for `crates/atlas-llm/src/claude_code.rs` (per design spec `docs/superpowers/specs/2026-05-01-agent-progress-feedback-design.md` §8.1).

Each fixture covers one parser branch:

- `simple-classify.jsonl` — direct-answer prompt; assistant produces a `thinking` block then a `text` block, no tool use; terminal `result.subtype = "success"`. Exercises the no-tool-use path.
- `subcarve-with-tools.jsonl` — prompt that requires reading one file; assistant emits a `tool_use` (`Read` with `file_path` input); the next user turn carries a `tool_result` (with **`is_error` absent**, the success encoding); terminal `result.subtype = "success"`. Exercises `AgentEvent::ToolUse`/`ToolResult` extraction.
- `error-max-turns.jsonl` — prompt with several tool steps under `--max-budget-usd 0.001`; agent runs three tool turns, the budget tips over, terminal event is `result` with **`subtype = "error_max_budget_usd"`** and `is_error = true` (no `result` field). Exercises the `subtype == "error_max_budget_usd"` → `LlmError::BudgetExhausted` branch (so `BudgetSentinel` intercepts CLI-driven budget exhaustion the same way it intercepts token-counter exhaustion from `BudgetedBackend`). The filename retains the spec's `error-max-turns` name for cross-reference even though the captured subtype is `error_max_budget_usd`: `--settings '{"maxTurns": ...}'` does not apply to `--print` mode in this CLI version, and budget-exhaustion is the substituted error path.

## Capture commands

```sh
# simple-classify
claude -p 'Reply with only the JSON literal {"ok": true} and nothing else.' \
  --output-format stream-json --verbose --model claude-haiku-4-5

# subcarve-with-tools
claude -p 'Read the file <abs path>/Cargo.toml using the Read tool, then reply with only a JSON literal of shape {"package_name": "<the value of package.name>"} and nothing else.' \
  --output-format stream-json --verbose --model claude-haiku-4-5

# error-max-turns (forces budget exhaustion)
claude -p --max-budget-usd 0.001 'Read <abs path>/Cargo.toml, then read <abs path>/README.md, then read <abs path>/crates/atlas-llm/Cargo.toml, then summarize.' \
  --output-format stream-json --verbose --model claude-haiku-4-5
```

## Redaction

Each capture was post-processed before commit to remove personal data. Two substitutions:

- `system/hook_response` events: the `output` and `stdout` fields embed the recorder's CLAUDE.md and superpowers content verbatim. Both were replaced with the placeholder `"[REDACTED: hook output omitted from fixture]"`. The parser ignores every `system` event (spec §6 forward-compat skip), so payload-only redaction inside `system` events does not affect parser behaviour.
- The username token `antony` was replaced with `USER` in every string value. This catches both the home prefix `/Users/antony/` and the auto-memory directory slug form `-Users-antony-Development-Atlas/memory/`. Path-shaped strings are re-encoded into slugs elsewhere in the system, so substituting the bare username is more reliable than substituting the home prefix alone.

Re-recording produces a transcript with new UUIDs, timestamps, and (for the success cases) potentially differently-worded `result.result` text. Run the capture commands above and re-redact.

## Re-capturing

The redaction script lives at `/tmp/atlas-stream-probe/redact.py` while this work-phase is active; if you re-record, copy the script into a stable location or recreate it from this README's redaction description before committing fresh fixtures.
