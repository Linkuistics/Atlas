# Codex Backend Research

**Date:** 2026-05-02
**Status:** Findings + recommendation
**Backlog item:** `research-codex-cli-interface-for-codexbackend-implementation`
**Source code touched:** `crates/atlas-llm/src/codex.rs` (stub-comment update only)

## Context

`atlas-llm/src/codex.rs` ships as a stub: `CodexBackend::call` returns
`LlmError::Setup("CodexBackend is not yet implemented — pending research task on
the Codex CLI subprocess interface")`. The multi-provider router
(`atlas-llm/src/router.rs`) accepts the `codex/` provider prefix in
`.atlas/config.yaml` model strings, but the dispatch lands on this stub.

This document records the research that the backlog task asked for, decides
between subprocess / HTTP / library approaches, and sketches the implementation
path tightly enough for a follow-up implementation task to pick up without
re-doing the discovery work.

## Findings

### Binary identity

```sh
$ codex --version
codex-cli 0.125.0
```

The Rust rewrite is the canonical implementation as of early 2026 — the
project is now ~96% Rust, distributed as a single static binary
(`/opt/homebrew/bin/codex` on this machine).

Version-format note: the prefix is `codex-cli`, not `codex`. A backend that
parses the version string for `LlmFingerprint::backend_version` should keep
the full first line verbatim and not assume a leading `codex `.

### Non-interactive entry: `codex exec`

```
codex exec [OPTIONS] [PROMPT]
codex exec [OPTIONS] <COMMAND> [ARGS]
```

`PROMPT` may be passed as an arg, piped on stdin, or set to `-` to read from
stdin explicitly. Stream events go to **stdout** under `--json`; progress
indicators (when `--json` is absent) go to **stderr**.

Flags relevant to a programmatic backend:

| Flag                                        | Purpose                                                                       |
|---------------------------------------------|-------------------------------------------------------------------------------|
| `--json`                                    | Print events to stdout as JSONL — required for parseable output.              |
| `-o, --output-last-message <FILE>`          | Write only the final agent message to a file (still also prints to stdout).   |
| `--output-schema <FILE>`                    | Path to a JSON Schema that constrains the final response shape.               |
| `-m, --model <MODEL>`                       | Model override per-call.                                                      |
| `-s, --sandbox <read-only\|workspace-write\|danger-full-access>` | Filesystem/exec policy.                          |
| `-C, --cd <DIR>`                            | Working root the agent operates in.                                           |
| `--skip-git-repo-check`                     | Allow running outside a Git repo (Atlas may run on non-git fixtures).         |
| `--ephemeral`                               | Don't persist session rollout files to `$CODEX_HOME`.                         |
| `--ignore-user-config`                      | Skip `$CODEX_HOME/config.toml`. Auth still uses `$CODEX_HOME`.                |
| `--ignore-rules`                            | Skip user/project execpolicy `.rules`.                                        |
| `--full-auto`                               | Convenience alias for low-friction sandboxed automatic execution.             |
| `--dangerously-bypass-approvals-and-sandbox`| Skip all confirmation prompts and run unsandboxed. **Avoid** outside CI.      |
| `-c, --config <key=value>` (repeatable)     | Inline TOML config override (e.g. `-c model=\"o3\"`).                         |
| `-p, --profile <NAME>`                      | Named profile from `~/.codex/config.toml`.                                    |
| `-i, --image <FILE>...`                     | Attach images.                                                                |
| `--add-dir <DIR>`                           | Additional writable dirs alongside primary workspace.                         |
| `--oss` / `--local-provider`                | Route through Ollama / LMStudio instead of the OpenAI cloud.                  |

### Auth model

Codex stores credentials in `$CODEX_HOME` (default `~/.codex`) after `codex
login`. For headless / CI-style use:

```sh
printenv OPENAI_API_KEY | codex login --with-api-key
```

This writes the key into `$CODEX_HOME/auth.json`. There is no documented
"pass an API key inline per-call" flag — the auth lives on disk between runs.
A backend that wants stateless invocation must either:

1. Require the user to have already run `codex login` (matches today's
   `claude` UX where `claude` must be authenticated before Atlas calls it), or
2. Set `CODEX_HOME` to a temp dir and run `codex login --with-api-key` once
   per Atlas process, before the first `exec` call.

Option 1 is simpler and consistent with `ClaudeCodeBackend`. Option 2 is the
pure-API-key path if config sharing with the user's interactive sessions is
unwanted.

### JSONL event schema (`--json`)

Empirically captured against a trivial prompt:

```jsonl
{"type":"thread.started","thread_id":"019de884-f6bf-7782-8b21-59c888a235b0"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"ok"}}
{"type":"turn.completed","usage":{"input_tokens":13217,"cached_input_tokens":11648,"output_tokens":5,"reasoning_output_tokens":0}}
```

Documented event types (per OpenAI Codex docs):

- `thread.started` — `thread_id`
- `turn.started`
- `turn.completed` — `usage.{input_tokens, cached_input_tokens, output_tokens, reasoning_output_tokens}`
- `turn.failed`
- `item.started` / `item.completed` — `item.id`, `item.type`, plus type-specific fields
  (`text` for `agent_message`; `command`, `status` for command executions; etc.)
- `error`

Documented `item.type` values: `agent_message`, `reasoning`, command
executions, file changes, MCP tool calls, web searches, plan updates.

For Atlas's purposes, the **last `item.completed` whose `item.type ==
"agent_message"`** carries the final response payload. With
`--output-schema`, the `text` field is JSON conforming to that schema.

### Output schema enforcement

`--output-schema <FILE>` accepts a standard JSON Schema. The agent's final
`agent_message.text` is constrained to match it. This is **strictly better**
than the freeform JSON-fenced output Atlas parses out of `claude -p` today
(`stream_parse::strip_json_fence` + `validate_response`) — Codex enforces
the schema upstream and reports schema failures as `turn.failed` rather than
shipping malformed JSON to the caller.

### Codex-core library option

`codex-core` exists as a published Rust crate (part of the `codex-codes`
0.101.1 workspace, Apache-2.0). OpenAI's own description positions it as the
"reusable embedding surface" for agentic Rust tooling. The full Codex CLI is
a Cargo workspace of ~70 crates layered on top of this core.

This is technically a *third* integration option: link `codex-core` directly,
no subprocess at all. See trade-off discussion below.

### MCP server option (noted, not used)

`codex mcp-server` runs Codex itself as an MCP server over stdio. Atlas could
theoretically front this via an MCP client. This is interesting but adds a
protocol layer that the existing `claude_code` / `http_openai` / `http_anthropic`
backends don't use, and the Atlas LlmBackend trait is not MCP-shaped. Out of
scope for v1 of `CodexBackend`.

## Approaches considered

| Approach           | Pros                                                                                                                          | Cons                                                                                                                                                    |
|--------------------|-------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Subprocess**     | Mirrors `ClaudeCodeBackend`. User upgrades `codex` independently of Atlas. `--output-schema` cleaner than Claude Code freeform JSON. Auth lives in user's `$CODEX_HOME`. | Spawn overhead per call (~hundreds of ms). Requires the binary on PATH. Same latency profile as Claude Code today. |
| **HTTP via existing `OpenAiHttpBackend`** | Already implemented; sub-second per-call; no extra binary; works against the OpenAI Responses API.                            | Not agentic — no tool use, no file reads, no exploratory work. Already covered by the `openai/` provider; nothing new for `codex/` to add here.         |
| **`codex-core` library link** | No spawn. Tightest integration. Direct access to internal events.                                                              | Pulls in ~70 transitive crates. Couples Atlas's release cycle to `codex-core`'s API stability. Bigger binary. Hard to upgrade independently from a user's CLI install. Overkill for v1. |

## Recommendation: subprocess

`CodexBackend` should be a subprocess driver, structurally parallel to
`ClaudeCodeBackend`.

### Reasons

1. The `codex/` slot in `router.rs` exists *because* the architecture wants
   an agentic Codex peer to `claude-code/`. Anything narrower duplicates the
   `openai/` HTTP path.
2. Subprocess decouples Atlas from Codex's internal API stability. Users can
   upgrade either independently.
3. `--output-schema` plus `--json` gives strictly cleaner data extraction
   than the existing Claude Code path — schema enforcement happens at the
   Codex layer.
4. The existing `AgentObserver` trait already handles a streaming JSONL
   event flow for Claude Code; mapping Codex events into the same shape is a
   small adapter, not new infrastructure.

### Why not the codex-core library

The cost-to-benefit ratio is poor for v1. ~70 transitive crates and a
hard coupling to OpenAI's internal API surface, in exchange for shaving
process spawn latency that Atlas already tolerates from `claude-code`.
Re-evaluate if/when Atlas needs sub-100ms per-call agentic Codex calls and
the spawn cost becomes a measurable bottleneck.

## Implementation sketch

This is the shape the follow-up implementation task should land. Not
prescriptive — directional.

### Construction

```rust
pub struct CodexBackend {
    model_id: String,
    prompts_dir: PathBuf,
    version: String,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    observer: Option<Arc<dyn AgentObserver>>,
}

impl CodexBackend {
    pub fn new(
        model_id: impl Into<String>,
        prompts_dir: impl Into<PathBuf>,
    ) -> Result<Self, LlmError> {
        let version = capture_codex_version()?; // runs `codex --version`
        // ... mirror ClaudeCodeBackend::new
    }
}
```

`capture_codex_version` parses the first line of `codex --version` — keep
the full string for `LlmFingerprint::backend_version` (e.g.
`codex-cli/0.125.0`).

### Per-call invocation

```sh
codex exec \
  --json \
  --skip-git-repo-check \
  --ephemeral \
  --sandbox read-only \
  --output-schema <tmpfile> \
  --model <id> \
  -- <rendered-prompt>
```

- `--sandbox read-only` is the floor: Atlas's prompts only read code; no
  workspace writes are needed. Easier to justify than `workspace-write`,
  no security surprise.
- `--ephemeral` keeps the user's `$CODEX_HOME/sessions/` clean.
- `--skip-git-repo-check` because Atlas runs on directories that may not be
  git repos (fixtures, etc.).
- `--output-schema` is rendered from `LlmRequest::schema` to a tempfile per
  call. Drop the tempfile when the call returns.

### Streaming + parsing

Walk `--json` stdout line by line. Filter for `item.completed` events with
`item.type == "agent_message"` — the **last** such event holds the final
response. Parse `item.text` as JSON; pass through `validate_response` against
`req.schema` (defense in depth, even though `--output-schema` should have
ensured shape upstream).

For agent observability: feed every `item.started` / `item.completed`
through a Codex→`AgentEvent` mapper, parallel to the existing Claude-Code
mapper in `stream_parse.rs`. `command_execution`, `file_change`, `web_search`,
`plan_update` map naturally; `reasoning` and `agent_message` are lower-noise
than their Claude-Code analogues and can be reported as plain message-level
events.

### Auth handling

At construction time, run `codex login status` and surface a clear
`LlmError::Setup` if not authenticated, with the remediation hint:

```text
codex CLI is not authenticated. Run:
  printenv OPENAI_API_KEY | codex login --with-api-key
or
  codex login
```

This matches the user's mental model for `claude-code/` (the binary must
already be authenticated; Atlas does not manage credentials).

### Fingerprint

```rust
LlmFingerprint {
    template_sha: self.template_sha,
    ontology_sha: self.ontology_sha,
    model_id: self.model_id.clone(),
    backend_version: format!("codex/{}", self.version),
}
```

`self.version` is the trimmed output of `codex --version` (e.g.
`codex-cli 0.125.0`). The `codex/` prefix in `backend_version` parallels
`claude-code/` and `openai-http/` from the existing backends.

### Tests

- Unit test against fixture JSONL (mirroring `parse_openai_response` pattern).
- `#[ignore]` integration test that runs against the real `codex` binary
  under an env-var gate (parallel to `ATLAS_LLM_RUN_CLAUDE_TESTS=1`),
  e.g. `ATLAS_LLM_RUN_CODEX_TESTS=1`.

## Open questions / follow-ups

- **Streaming buffering.** Codex writes JSONL lines as events occur, but
  Rust's `Command` stdout is line-buffered only when a real TTY is attached.
  When piped to a non-TTY parent, `codex exec --json` may flush at chunk
  boundaries rather than per line. The Claude Code backend handles this via
  a streaming line splitter (`stream_parse.rs`); the Codex implementation
  should reuse the same splitter.
- **Sandbox vs Atlas's read-only intent.** Atlas's prompts shouldn't write
  to disk, but L5/L8 prompts may want the agent to *read* across the whole
  workspace. `--sandbox read-only` is correct, but verify against an actual
  L5 surface-extraction prompt before locking it in.
- **`turn.failed` mapping.** Need to map `turn.failed` payloads to specific
  `LlmError` variants (Schema vs Invocation vs Parse). The exact failure
  payload shape isn't documented; capture a real failure during
  implementation.
- **Concurrency.** If `CodexBackend` ends up co-existing with the planned
  L8 map/reduce work (`map-reduce-llm-architecture-with-backend-routing-for-context-light-analysis`),
  the bounded-concurrency executor in that task should treat Codex and
  Claude Code uniformly — one semaphore per agentic provider, sized by
  user's Codex/Anthropic rate-limit headroom.

## Decision

**Subprocess approach.** Implementation deferred to a follow-up backlog
task (added in this triage cycle).

## References

- [OpenAI Codex CLI reference](https://developers.openai.com/codex/cli/reference)
- [Codex non-interactive mode](https://developers.openai.com/codex/noninteractive)
- [Codex CLI features](https://developers.openai.com/codex/cli/features)
- [openai/codex on GitHub](https://github.com/openai/codex)
- [codex-rs Rust rewrite (architecture overview)](https://codex.danielvaughan.com/2026/03/28/codex-rs-rust-rewrite-architecture/)
- Local install at time of writing: `codex-cli 0.125.0`.
