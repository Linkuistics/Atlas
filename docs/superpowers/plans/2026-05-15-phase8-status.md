# Atlas vNext — Phase 8 — Cargo retirement — Status

Companion to `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`. This file tracks per-WI completion state across sessions.

**Last updated:** 2026-05-15 (WI-2 status flip).

## WI status

- [x] WI-1 — Agent-runtime HTTP-backend bypass
- [x] WI-2 — HardFail event emission in call_agent
- [ ] WI-3 — Cargo classifier retirement
- [ ] WI-4 — Atlas-on-Atlas calibration + closeout

## Per-WI notes

### WI-1

**Shipped:** 2026-05-15. Code commit `8839cb8` (`phase8 WI-1: agent-runtime HTTP-backend bypass`).

**What landed:**

- `LlmRequest` schema migrated to a tagged-union shape: `prompt_template: Option<PromptId>` + new `rendered_prompt: Option<String>`; `#[non_exhaustive]` added; two public constructors `from_template(id, inputs, schema)` + `from_rendered(rendered, schema)` with the "exactly one is Some" invariant enforced by `debug_assert!` (rendered) and the constructor shape (templated). The plan's optional `#[should_panic]` debug-assert test was dropped per § 7 Step 1.3 — `#[non_exhaustive]` makes struct-literal construction a compile error outside the crate anyway.
- Four backends short-circuit on `req.rendered_prompt.is_some()` before `prompts_dir` lookup: `http_anthropic.rs`, `http_openai.rs`, `codex.rs`, `claude_code.rs`. Each `render_request` returns the rendered string verbatim on the bypass path; templated path is unchanged.
- `BackendRouter` gained `default_provider: Provider` (seeded from the `Classify` operation's configured provider during `new_inner`). Rendered requests route via `backend_for_provider(default_provider)`. Both `call` (sync) and `call_async` carry the bypass branch.
- 18 `LlmRequest` call sites migrated (plan estimated ~17 — one-site delta is non-material). Three production agent-runtime sites use `from_rendered`: `tool_loop_http.rs::build_llm_request_with_tools`, `tool_loop_mcp.rs::build_llm_request_subprocess`, the auditor in `runtime/mod.rs`. Fifteen templated sites use `from_template`.
- Cascade beyond the plan's anchors (driven by `Option<PromptId>` propagation): `LlmCacheKey.prompt` widened to `Option<PromptId>`; `TestBackend`'s HashMap key widened to `(Option<PromptId>, String)`; `BackendCallEvent::CallStart` carries `Option<PromptId>` (logging-only; renderered-prompt calls show as `None`); both stream parsers (`parse_stream`, `parse_codex_stream`) accept `Option<PromptId>`; `prompt_label` in `atlas-cli/src/progress.rs` handles `Option<PromptId>` by labelling `None` as `"agent"`. ~13 test backends across the workspace that pattern-match on `req.prompt_template` each gained `.expect("test backend services templated requests")` — those backends only serve the deterministic spine. This was the bulk of R1's "mechanically wide" mitigation.
- New test: `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` (2 tests).
- `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored (closes sprint PR-4 note item 9). The bypass alone wasn't sufficient — the test's `provider_router` was a real `BackendRouter::new_for_agent_runtime` whose `Provider::OpenAi` entry was a real OpenAI HTTP backend, so Lane B audit still made live HTTP calls. Per PR-4 note item 9(b), `BackendRouter::from_dispatch_table` was lifted out of `#[cfg(test)]` and a sibling `from_test_dispatch_table_with_providers(table, providers, default_provider, fingerprint)` was added that supports cross-provider audit lookups. The test now builds a mock router whose Anthropic and OpenAi provider entries both resolve to the same `StagedBackend`, with a canned auditor verdict for the substring `"Verdict shape"`.
- Stale R9 comment cleanup: `tool_loop_http.rs` and `tool_loop_mcp.rs` "PR-5 will introduce a dedicated PromptId variant" comments removed.
- Status doc authored (this file) per kickoff Step 0 pre-flight.

**Plan-time deviations (worth noting for WI-2's executor):**

- Call-site count: plan said "~17", actual = 18. Within tolerance, no escalation triggered. The extra site is `tool_loop_http.rs`'s test (`build_llm_request_carries_tool_descriptors`) which was rewritten anyway — it asserted on the dead `inputs.tools` shape, vacuously satisfied since the HTTP backends never read `inputs.tools` off the request body.
- `BackendRouter::call_async` was at `router.rs:266`, not the plan's "~117" — the plan flagged this anchor as "find at plan-time". Both `call` (sync) and `call_async` needed bypass branches.
- The HTTP smoke test file lives at `crates/atlas-cli/tests/agent_runtime_http_smoke.rs`, NOT `crates/atlas-agents/tests/agent_runtime_http_smoke.rs` as the plan's § 7 Files block stated. Non-material file-location drift.
- The plan's Step 1.9 framed un-ignoring the test as a single-action close. Empirically the bypass closed the `prompts_dir` problem but Lane B audit still required mock injection (PR-4 note item 9(b)). The `from_test_dispatch_table_with_providers` shape lifted out of `#[cfg(test)]` is the WI-1 in-scope expansion.
- `BackendCallEvent::CallStart` field widened from `prompt: PromptId` to `prompt: Option<PromptId>` — affects telemetry consumers in `atlas-cli/src/progress.rs` (handled inline).
- `LlmCacheKey.prompt` widened to `Option<PromptId>` — affects the deterministic spine's response cache key (still works because the spine's call sites always set `Some(_)`).
- `ProgressBackend::call` / `call_async` in `atlas-cli/src/progress.rs` now guards `on_llm_call` behind `if let Some(prompt) = req.prompt_template` — rendered requests don't increment the per-stage breakdown counter, which is semantically correct (the agent runtime owns its own logging).

**Regression gates (all six clean):**

- `cargo build --workspace`: clean.
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`: 110 test-result entries, 0 failures.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean (auto-fmt run mid-session reformatted 18 files with long `.expect("...")` chains).
- `cargo build --release --workspace`: clean.
- Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`): 2 tests pass, wall-time 99.44s — within the 100–110s expected range, cold-count in `0 < cold < 100` (loose bound satisfied by the test's internal assertion), warm + reports = 0.

**Sprint PR-4 / PR-5 note items closed:**

- PR-4 note item 9: HTTP smoke `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored. Closure required two changes — the bypass (template lookup no longer poisoned on empty `prompts_dir`) AND the `from_test_dispatch_table_with_providers` lift (Lane B audit no longer hits live OpenAI).
- PR-5 closeout note item 1 (the agent-runtime HTTP-backend wiring gap): the rendered-prompt path no longer pre-fails at `LlmError::TemplateSyntax("unknown token \`{{COMPONENT_KINDS}}\`")`. End-to-end success on Atlas-on-Atlas is WI-4's acceptance — WI-1 may still surface downstream issues (likely Lane A schema mismatch on the agent's first production-prompt output), but the failure mode has shifted from "pre-HTTP template render" to "real LLM round-trip + downstream issue".

**Cargo SHAs:** code commit `8839cb8`; status-flip commit (this) to follow.

### WI-2

**Shipped:** 2026-05-15. Code commit `f142157` (`phase8 WI-2: HardFail event emission for backend errors`).

**What landed:**

- Producer-fail HardFail emit at `crates/atlas-agents/src/runtime/mod.rs:968` (the former `let output = outcome?;` site inside `run_tool_loop_with_lane_a`). The bare `?` becomes a `match` that emits `AgentEvent::HardFail { agent_id: agent_id(request), error_kind: "backend", error_summary: e.to_string(), retry_count: lane_a_retries }` before returning `Err(e)`. The propagated `Err` preserves the backend's verbatim error text (the new arm returns `e`, not a wrapped string).
- Auditor-fail HardFail emit inside `run_real_audit`'s auditor-backend `Err(e)` match arm (lines ~1186–1198 post-edit). Emits `AgentEvent::HardFail { agent_id: agent_id_payload.to_string(), error_kind: "audit_backend", error_summary: e.to_string(), retry_count: 0 }` before returning `AuditVerdict::HardFail(...)`. The cascading `lane_b` HardFail at line 817 (`ResolvedAuditAction::HardFail` arm) still fires afterwards, but subscribers can distinguish auditor-vs-producer via the new `audit_backend` discriminator.
- `run_real_audit` gained an `event_bus: &EventBus` parameter (11th positional). The threading decision (plan § 8 Step 2.3 surfaced two shapes: thread state in OR return `Result<AuditVerdict, AuditorBackendError>`) resolved to thread-in because the closure passed to `audit::lane_b_audit` is constrained to return `AuditVerdict` — option B would have required collapsing the Result back at the closure return site without access to the bus, which doesn't help. The call-site closure captures a cloned `Arc<EventBus>` (one atomic incr) and passes `event_bus.as_ref()` through.
- New test: `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs` (2 tests). `AlwaysErroringBackend` for producer-fail; `AlwaysSucceedingBackend` (returning Weak-grade classify YAML) + auditor `AlwaysErroringBackend` (routed via `ForProviderFn` for `Provider::OpenAi`) for auditor-fail. Both tests assert (1) the expected `error_kind` HardFail lands on the bus and (2) the propagated `Err` carries the backend's verbatim error text.

**Plan-time deviations (worth noting for WI-3's executor):**

- Producer-fail anchor at `mod.rs:965` (`let output = outcome?;`) was at the plan's exact frozen-2026-05-15 line — no drift from WI-1. After my edit the rewritten match-arm spans lines 968–987.
- Auditor-fail anchor: plan said `mod.rs:1167–1172`; post-WI-1 the actual `Err(e) => { return AuditVerdict::HardFail(...) }` arm was at `mod.rs:1160–1164` (4-line drift downward from WI-1's auditor `LlmRequest::from_rendered` edit at line 1158). After my edit the rewritten arm spans lines 1186–1199.
- `agent_id` helper is a private free function — tests can't call it. The test asserts on `error_kind` and `error_summary` substring instead of asserting on the precise `agent_id` string, which is the stable-across-internal-refactoring approach.
- The auditor-fail test surfaces TWO `HardFail` events on the bus: the new `audit_backend` one (mine) and the existing `lane_b` cascade at line 817 (since `ResolvedAuditAction::HardFail` still fires). Test asserts on the `audit_backend` discriminator only, but the test's docstring calls out the coexistence so a future reader understands.

**Regression gates (all six clean):**

- `cargo build --workspace`: clean.
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`: 111 test-result-ok lines, 0 failures.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean (one rustfmt pass applied to the new test file's chained-method assertion calls).
- `cargo build --release --workspace`: clean.
- Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`): 2 tests pass, wall-time 100.78s — within the 100–110s expected band, +1.34s from WI-1's 99.44s baseline (within natural per-run variance). Polyglot HOLDS per plan § 4 (HardFail emit is on a path the smoke never traverses).

**Sprint PR-5 closeout-note item 4 (producer-fail / auditor-fail diagnostic visibility):** closed. Both backend-error sites now emit `HardFail` records that downstream JSONL subscribers (`jsonl_subscriber.rs`) and TUI (PR-6) consumers can ingest. The operator-side calibration verification per plan § 8 acceptance-gate bullet 5 (re-run PR-5's calibration command against a known-failing backend) is **not** a CI gate — it's an out-of-session operator validation step.

**Cargo SHAs:** code commit `f142157`; status-flip commit (this) to follow.

### WI-3

(pending)

### WI-4

(pending)
