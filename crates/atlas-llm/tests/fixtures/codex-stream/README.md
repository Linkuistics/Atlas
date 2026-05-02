# codex stream-json fixtures

Real `codex exec --json` transcripts captured against `codex-cli 0.125.0`,
used as input to the codex stream-parser unit tests for
`crates/atlas-llm/src/codex.rs` (per research spec
`docs/superpowers/specs/2026-05-02-codex-backend-research.md`).

Each fixture covers one parser branch:

- `simple-classify.jsonl` — direct-answer prompt; agent emits one
  `item.completed` event with `item.type == "agent_message"`; terminal
  `turn.completed` with token usage. Exercises the no-tool-use path.
- `with-tools.jsonl` — prompt that requires reading one file; agent
  emits `item.started` and `item.completed` events for a
  `command_execution` (codex's tool-use shape — it shells out via
  `/bin/zsh -lc` rather than calling a typed tool), then a final
  `agent_message`. Exercises `AgentEvent::ToolUse` / `ToolResult`
  extraction from `command_execution` items.
- `turn-failed.jsonl` — prompt that fails upstream at the OpenAI API
  layer (an `--output-schema` lacking the required
  `additionalProperties: false`). Stream is `thread.started`,
  `turn.started`, `error`, `turn.failed`. Exercises the `turn.failed`
  → `LlmError::Invocation` branch and confirms that no
  `agent_message` is emitted on failure.

## Capture commands

```sh
# simple-classify (no --output-schema; bare-JSON agent_message.text)
codex exec --json --skip-git-repo-check --ephemeral --sandbox read-only \
  -- 'Reply with only the JSON {"ok": true} and nothing else.' < /dev/null

# with-tools (reads a fixture file; emits one command_execution)
codex exec --json --skip-git-repo-check --ephemeral --sandbox read-only \
  -- 'Read the file <abs path>/fixture.toml and reply with JSON of shape {"package_name": "<the value of package_name>"} and nothing else.' \
  < /dev/null

# turn-failed (forces an OpenAI 400 via a non-strict schema)
echo '{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}' > /tmp/bad-schema.json
codex exec --json --skip-git-repo-check --ephemeral --sandbox read-only \
  --output-schema /tmp/bad-schema.json \
  -- 'Reply with {"ok": true}' < /dev/null
```

## Notes

- Codex emits one `item.completed` per logical step. The **last**
  `agent_message` carries the final response payload; intermediate
  `agent_message` events (rare) are interim narrative.
- `command_execution` items model shell-out tool use. The `command`
  string is `/bin/zsh -lc "..."`; the agent does not call typed
  `Read`/`Grep` tools the way Claude Code does.
- `with-tools.jsonl` was captured against a tmpfile path; the fixture
  has been path-normalised to `fixture.toml` (a non-existent relative
  filename) so the test is path-independent. Functionally the parser
  doesn't read the file — it only reads the JSONL events — so the
  bogus path is harmless.
- No personal data appears in any of these fixtures (no usernames, no
  home paths, no API keys). Re-recording produces fresh thread IDs and
  token counts but should not introduce sensitive content as long as
  the prompts above are used unmodified.

## Re-capturing

The captures above are deterministic in shape but not in content:
thread IDs and token counts vary per run. Re-record with the commands
above; verify no sensitive content crept in via prompt changes; commit.
