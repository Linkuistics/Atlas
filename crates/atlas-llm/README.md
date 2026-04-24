# atlas-llm

The LLM-facing layer of Atlas. Defines the [`LlmBackend`] trait that
the engine calls into, ships two concrete backends ([`TestBackend`]
for unit tests, [`ClaudeCodeBackend`] for production), plus a prompt
template renderer and a token budget tracker.

The crate is intentionally small:

- **`prompt::render`** — `{{TOKEN}}` substitution with
  `{{{{...}}}}` escape. Used by backends that shell out to an LLM
  and need a pre-rendered prompt string.
- **`TestBackend`** — canned-response, deterministic, in-memory.
  Failing closed (unmapped request → error) so accidental LLM calls
  from code that should short-circuit never quietly succeed.
- **`ClaudeCodeBackend`** — spawns the `claude` CLI as a
  subprocess. Caches `claude --version` at construction so the
  fingerprint is stable across calls.
- **`TokenCounter` + `BudgetedBackend`** — thread-safe budget
  accounting; a charge that would exceed the ceiling returns
  `LlmError::BudgetExhausted` per design §7.4 (fail-loud, no
  fallback).

## Invariants a backend must satisfy

Salsa-level memoisation keys every LLM-derived query on
`(LlmFingerprint, LlmRequest)`. For that to be sound, a backend must:

1. **Be deterministic given fingerprint + inputs.** For a fixed
   `LlmFingerprint` and fixed `LlmRequest`, `call` must yield the
   same JSON value. Stochastic sampling belongs behind a cache, not
   behind the trait.
2. **Expose a stable fingerprint.** Two consecutive `fingerprint()`
   calls on the same backend must return equal values. Model
   identity, backend version, and the ontology/template state that
   shapes responses all belong in the fingerprint — Salsa treats a
   change in the fingerprint as invalidation of every cached LLM
   response.
3. **Not read uncaptured external state.** The backend may not read
   environment variables, disk, or network sources whose identity
   isn't reflected in the fingerprint. Subprocess-based backends
   (like `ClaudeCodeBackend`) bake the subprocess version into
   `backend_version`.

## Adding a new backend

1. Create a module, e.g. `src/my_backend.rs`.
2. Define a struct carrying whatever state is needed (HTTP client,
   cached version string, model id).
3. Implement `LlmBackend`:
   - `call(&self, req)` — produce a `serde_json::Value` response or
     an `LlmError`.
   - `fingerprint(&self)` — return an `LlmFingerprint` populated
     from cached state.
4. Validate LLM output against `req.schema` inside `call` before
   returning. Atlas uses a small subset of JSON Schema
   (`type`, `required`); see `claude_code::validate_response` for
   the shape of that check.
5. Re-export the type from `lib.rs` if consumers should see it.
6. Add unit tests that cover the happy path, the schema-failure
   path, and the fingerprint stability property. Gate tests that
   require an external dependency (subprocess, network) behind
   `#[ignore]` with an opt-in env var, following the pattern in
   `claude_code.rs`.

## Why not a full JSON Schema validator?

Atlas uses four hand-authored prompt schemas and each is tight: a
small fixed set of top-level fields with concrete types. A full
validator (e.g. the `jsonschema` crate) would add compile time and
dependency surface disproportionate to the value. The minimal
validator in `claude_code::validate_response` covers the subset Atlas
actually uses; it is straightforward to swap in a fuller
implementation if response schemas grow richer.

## Testing

```bash
cargo test -p atlas-llm              # unit tests (subprocess tests ignored)
ATLAS_LLM_RUN_CLAUDE_TESTS=1 \
  cargo test -p atlas-llm -- --ignored   # also run subprocess tests
```

## Model selection

Backends that care about model identity consult
`claude_code::MODEL_ID_ENV` (`ATLAS_LLM_MODEL`) when no override is
passed; the fallback is `claude_code::DEFAULT_MODEL_ID`.
`resolve_default_model_id()` returns the resolved string.

## Dependencies

Runtime: `anyhow`, `serde`, `serde_json`, `sha2`, `thiserror`. Test-only:
`tempfile`.
