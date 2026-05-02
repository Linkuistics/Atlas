# Multi-Provider LLM Configuration

**Date:** 2026-05-02
**Status:** approved

## Overview

Replace Atlas's single hardcoded `ClaudeCodeBackend` with a routed multi-backend system driven by a per-project config file. Each of the four LLM operations (`classify`, `subcarve`, `surface`, `edges`) can be independently assigned a provider, model, and params. Two new HTTP backends (Anthropic, OpenAI) join the existing `ClaudeCodeBackend` and a new `CodexBackend` stub.

A new `atlas init <root>` subcommand bootstraps the required config and override files before first run.

---

## Architecture

Four components compose into the existing backend stack:

1. **`.atlas/config.yaml`** — required per-project file. Sole source of truth for LLM provider, model, and params. No in-code defaults.
2. **`AnthropicHttpBackend` / `OpenAiHttpBackend`** — new `LlmBackend` implementations in `atlas-llm`. Use `reqwest::blocking`; no subprocess, no tool use.
3. **`CodexBackend`** — new `LlmBackend` in `atlas-llm`, structurally parallel to `ClaudeCodeBackend`. Exact subprocess invocation and output format are TBD pending a research task.
4. **`BackendRouter`** — new `LlmBackend` wrapper that owns the resolved config and dispatches each call by `PromptId` to the correct backend instance. Sits inside the existing budget/sentinel/progress wrapper stack, which remains unaware of routing.
5. **`atlas init <root>`** — new CLI subcommand. Scaffolds `.atlas/config.yaml`, `components.overrides.yaml`, and `subsystems.overrides.yaml` with heavy comments. Skips files that already exist.

The existing `--model` CLI flag and `ATLAS_LLM_MODEL` env var are removed. Config file is the only configuration surface for LLM settings.

---

## Config File

**Location:** `<root>/.atlas/config.yaml`

**Required:** `atlas index` fails immediately if the file is absent or if `defaults.model` is missing.

### Schema

```yaml
# Provider credentials. Only providers used in defaults/operations need entries.
# claude-code and codex require no credentials.
providers:
  anthropic:
    api_key: ${ANTHROPIC_API_KEY}    # env-var interpolation, resolved at load time
  openai:
    api_key: ${OPENAI_API_KEY}

# Global defaults — both fields required.
defaults:
  model: anthropic/claude-sonnet-4-6   # <provider>/<model-id>
  # params: {}                         # optional passthrough map

# Per-operation overrides — all optional; absent entries inherit defaults entirely.
operations:
  classify:                            # L3 is_component
    model: anthropic/claude-haiku-4-5
    # params:
    #   temperature: 0
    #   max_tokens: 1024

  subcarve:                            # L8 subcarve decision
    model: anthropic/claude-haiku-4-5

  surface:                             # L5 Stage1Surface — reads source files
    model: claude-code/claude-sonnet-4-6

  edges:                               # L6 Stage2Edges — cross-component synthesis
    model: claude-code/claude-opus-4-7
```

### `<provider>/<model>` routing table

| Prefix | Backend |
|---|---|
| `anthropic/*` | `AnthropicHttpBackend` |
| `openai/*` | `OpenAiHttpBackend` |
| `claude-code/*` | `ClaudeCodeBackend` |
| `codex/*` | `CodexBackend` |

### Validation at load time

1. Parse YAML structure — hard error on malformed file.
2. Resolve `${VAR}` interpolations — hard error if any referenced env var is unset.
3. Assert `defaults.model` present — hard error if missing.
4. For each provider prefix referenced in `defaults` or `operations`: assert a matching `providers:` entry exists if that provider needs credentials (`anthropic`, `openai`). Hard error otherwise.
5. Assert `providers.*.api_key` values are non-empty after interpolation.

`params` is a passthrough freeform map — no schema enforcement; each backend silently ignores keys it does not understand.

---

## HTTP Backends

Both backends share the same structural approach.

### Prompt rendering

The existing `prompt::render(req)` produces a single rendered string (template + token substitution). For HTTP backends this string is sent as a single `user` message with no system message. Splitting into system/user for prompt caching is reserved for a follow-on epic.

### `AnthropicHttpBackend`

```
POST https://api.anthropic.com/v1/messages
x-api-key: <key>
anthropic-version: 2023-06-01
Content-Type: application/json

{
  "model": "<model-id>",
  "max_tokens": <params.max_tokens>,
  "messages": [{ "role": "user", "content": "<rendered prompt>" }],
  <...any other params keys passed through verbatim...>
}
```

The Anthropic Messages API requires `max_tokens`. If `params.max_tokens` is absent the backend returns `LlmError::Setup` with a message directing the user to add `max_tokens` to the operation's `params` block in `config.yaml`.

Response extraction: `content[0].text` → strip markdown fence if present (reuse existing fence-stripping logic from `stream_parse`) → `serde_json::from_str` → validate against `req.schema`.

### `OpenAiHttpBackend`

```
POST https://api.openai.com/v1/chat/completions
Authorization: Bearer <key>
Content-Type: application/json

{
  "model": "<model-id>",
  "messages": [{ "role": "user", "content": "<rendered prompt>" }],
  <...any other recognised params...>
}
```

Response extraction: `choices[0].message.content` → same fence-strip + validate pipeline.

### `fingerprint()`

Both backends return `LlmFingerprint { backend: "anthropic-http" | "openai-http", model, version: crate_version }` populated via the same `with_fingerprint_inputs(template_sha, ontology_sha)` pattern as `ClaudeCodeBackend`.

---

## CodexBackend

Structurally parallel to `ClaudeCodeBackend`:
- Spawns a `codex` subprocess with the rendered prompt
- Drains stderr on a worker thread (same deadlock-avoidance pattern)
- Parses stdout as a stream
- Emits `AgentEvent`s via the optional `AgentObserver`

**Exact subprocess flags and output format are TBD.** A new backlog research task covers:
- Codex CLI invocation (`codex -p …` or equivalent)
- Output stream format (same JSONL shape as `claude --output-format stream-json --verbose`, or different)
- Whether a new `codex_stream_parse.rs` is needed or `stream_parse.rs` can be shared

`CodexBackend` ships as a stub in this implementation: the struct exists, `fingerprint()` returns its identity, but `call()` returns `LlmError::Setup("CodexBackend not yet implemented")` until the research task is resolved.

---

## BackendRouter

`BackendRouter` implements `LlmBackend` and is the single backend passed to the budget/sentinel/progress wrapper stack.

### Construction

```
AtlasConfig::load(<root>/.atlas/config.yaml)
  → validate (see Config validation above)
  → construct one instance per referenced provider
  → build PromptId → Arc<dyn LlmBackend> dispatch table
  → BackendRouter { table, fingerprints }
```

`build_production_backend_with_counter` in `atlas-cli/src/backend.rs` is updated to call `BackendRouter::new(config)` instead of constructing `ClaudeCodeBackend` directly.

### Dispatch

`BackendRouter::call(req)` looks up `req.prompt_template` in the table and delegates. All four `PromptId` variants must have a resolved entry (defaults fill in any absent operation).

### `fingerprint()`

`LlmBackend::fingerprint()` returns a single `LlmFingerprint`. `BackendRouter` returns a composite that encodes the resolved model string for all four operations (hashed together). This means changing any operation's backend invalidates the entire `llm-cache.json` — safe, and simpler than a per-`PromptId` fingerprint which would require a trait change. A follow-on can refine this if cache churn from single-operation changes becomes a problem.

---

## `atlas init`

### Invocation

```
atlas init <root>
```

Parallel positional argument convention to `atlas index <root>`. Output directory: `<root>/.atlas/`.

### Behaviour

- Creates `<root>/.atlas/` if absent.
- Writes the three user-authored template files listed below.
- **Skips** any file that already exists (prints a notice: `skipped <path> (already exists)`).
- Prints the path of each file written.
- Exits 0 on success; non-zero if the directory cannot be created.

### Scaffolded files

**`config.yaml`** — includes:
- Full `providers:` section with commented examples for `anthropic` and `openai`
- `defaults:` section with both required fields explained
- All four `operations:` keys commented out, each with an inline explanation of what layer it controls
- Comment explaining `<provider>/<model>` format and the four supported providers
- Comment noting `params` is optional and provider-specific

**`components.overrides.yaml`** — existing format, heavily commented with examples of pin, suppress, and additions forms.

**`subsystems.overrides.yaml`** — existing format, heavily commented with examples of glob and id membership forms.

### Typical first-run workflow

```sh
atlas init /path/to/project
# edit /path/to/project/.atlas/config.yaml
atlas index /path/to/project
```

---

## Error Handling

| Situation | Behaviour |
|---|---|
| `.atlas/config.yaml` absent | Hard error: "config.yaml not found — run `atlas init <root>` first" |
| `defaults.model` missing | Hard error with field path |
| `${VAR}` references unset env var | Hard error: "env var VAR is unset (referenced in config.yaml)" |
| Provider needs credentials but no `providers:` entry | Hard error: "provider `anthropic` used but not configured in providers:" |
| HTTP request fails (network, 4xx, 5xx) | `LlmError::Invocation` with status code and body excerpt |
| Response missing expected field | `LlmError::Parse` |
| Response fails schema validation | `LlmError::Schema` |
| `CodexBackend::call` invoked | `LlmError::Setup("CodexBackend not yet implemented")` |

---

## Testing

- **Config loading:** unit tests for all validation failure modes (missing file, missing defaults.model, unset env var, missing provider entry, empty api_key).
- **`BackendRouter`:** unit tests with `TestBackend` stubs per `PromptId` — assert correct dispatch.
- **HTTP backends:** integration tests gated behind `ATLAS_LLM_RUN_LIVE_TESTS=1` to avoid live API calls in CI.
- **`atlas init`:** integration tests using `assert_cmd` — verify all three files created, existing files skipped, output messages correct.
- **Removal of `--model` flag:** existing tests that pass `--model` must be updated.

---

## Out of Scope

- Prompt caching (system/user split — follow-on epic)
- `CodexBackend` full implementation (blocked on research task)
- Concurrency / parallelism for HTTP backends (map/reduce epic)
- Env-var interpolation beyond `${VAR}` syntax (no `$VAR`, no nested interpolation)
- User-pluggable providers (Ollama, etc.)
