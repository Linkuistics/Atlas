# Atlas vNext Phase 7 — Status

Companion to `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-12-vnext-continue.md` (Phase-7-shaped) reads this file (via the `*phase7-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-13 (Phase 7 SHIPPED — PR-7 status-flip commit lands the final checkbox; all 8 Phase 7 PRs are now `[x]`. Two-stage review verdict: spec compliance COMPLIANT-WITH-ACCEPTABLE-DEFERRALS; code quality APPROVED-WITH-MINORS. Orchestrator-side cargo gates re-verified clean: 6/6 gates green; release polyglot smoke 2-tests-passed in 106.81s — same envelope as PR-5's 104.32s and PR-6's 109.34s. Steps 7.4 Atlas-on-Atlas calibration + 7.6 subprocess `--disallowedTools` probe explicitly DEFERRED to a follow-up production-prompt sprint with documented preconditions in the Phase 7 → Phase 8 handoff section below).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — `atlas-agents` crate + `Tool` trait + MCP server + async `LlmBackend` (large)
- [x] PR-2 — Transcript cache + event bus + JSON-Lines subscriber (medium)
- [x] PR-3 — 22 tool wrappers across three parallel subagents (medium) — revised from 26 per PR-3 user-decided deferral of 5 non-existent manifest parsers
- [x] PR-4 — Agent runtime (single-iteration) + Lane A schema validation (large)
- [x] PR-5 — Fixed-point iteration + LLM-decided dispatch + Lane B cross-provider audit (large)
- [x] PR-6 — `ratatui` TUI subscriber + `--replay-from-cache` mode (medium)
- [x] PR-7 — End-to-end wiring + polyglot smoke extension + Atlas-on-Atlas calibration + closeout (large) — Steps 7.4 + 7.6 DEFERRED (see PR-7 notes)

When every box is `[x]`, Phase 7 is complete and the continuation prompt should report success and route to "brainstorm Phase 8 (Cargo retirement per recast spec §11.2)?".

## Dependency graph (canonical in plan §3)

```
PR-0 (plan + status + continuation prompt)
  │
  ▼
PR-1 (atlas-agents + Tool trait + MCP server + async LlmBackend)
  │
  ▼
PR-2 (transcript cache + event bus + JSON-Lines subscriber)
  │
  ├──► PR-3a (mature wrappers — Rust + TS/JS + 9 manifests) ──┐
  │                                                            │
  ├──► PR-3b (mid-tier wrappers — Python + C# + Dart) ─────────┤
  │                                                            │
  ├──► PR-3c (weak-tooling — Elixir + Racket + LispKit + Compose + Dockerfile) ──┤
  │                                                            │
  │                                              PR-3 merge ◄──┘
  │                                                  │
  │                                                  ▼
  │                                              PR-4 (runtime single-iter + Lane A)
  │                                                  │
  │                                                  ▼
  │                                              PR-5 (fixedpoint + LLM dispatch + Lane B)
  │                                                  │
  └──► PR-6 (TUI + replay-from-cache) ──────────────►┤
                                                     │
                                                     ▼
                                                 PR-7 (end-to-end wiring + closeout)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (landed).
- **Wave 1 (after PR-0):** PR-1, then PR-2 (sequential; PR-2 depends on PR-1's `Tool` trait + event-bus skeleton + async `LlmBackend`).
- **Wave 2 (after PR-2):** PR-3a + PR-3b + PR-3c dispatched concurrently in three pre-created worktrees. Use `superpowers:dispatching-parallel-agents`; verify each worktree's base commit matches current main before subagent proceeds (memory `feedback_worktree_base_verification`). For PR-3 specifically, prefer **pre-creating the worktrees yourself** via `git worktree add -b <branch> <path> main` rather than relying on the harness's `isolation: "worktree"` mode (per the same memory). Merge to a `phase7-pr3` integration branch after all three subagents report DONE.
- **Wave 3 (after PR-3):** PR-4, then PR-5 (sequential; PR-5 extends PR-4's runtime with fixedpoint + LLM dispatch + Lane B).
- **Wave 4 (parallel-safe with PR-4 and PR-5; depends only on PR-2):** PR-6 may be dispatched as soon as PR-2 has merged. The TUI subscriber consumes the event bus alone and does not depend on the runtime internals.
- **Wave 5 (after PR-5 and PR-6):** PR-7 — end-to-end wiring + polyglot smoke extension + Atlas-on-Atlas calibration + closeout.

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard. Each PR-1+ re-runs it (after `cargo build --release --workspace` per memory `feedback_release_workspace_build_for_polyglot`; do NOT pipe through tail per memory `feedback_no_tail_pipe_for_long_tests`) before flipping its checkbox. Cold = ~40 (calibrated codebase baseline per Phase 6 PR-5 closeout); warm + reports = 0.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; pivot-table item interpretations confirmed; reference-output comparisons; cross-cutting refactor surfaces; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0

2026-05-12 — Landed: the Phase 7 plan, this status file, and the continuation prompt. Plan: `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-12-vnext-continue.md`. Design anchor: the Phase 7 brainstorm at `docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md` (commit `f4ea770` on main; the 14-item decision table is locked). Parent design spec: the LLM-spine recast at `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` (commit `409dcc5`). Phase 6 SHIPPED 2026-05-11 (final commit `9350735`) as the final deterministic-spine release; the canonical §4.3/§7/§8/§10 retext landed in Phase 6 PR-5 — **Phase 7 itself does not retext canon** (Phase 6 already did). Commit: `ddf553b` (first commit of the two-commit PR-0 pattern); status flip + SHA backfill in this commit.

Key PR-0 design call-outs to surface to PR-1+:

1. **`AgentEvent` naming collision** between today's `atlas-llm::AgentEvent` (backend-level streaming-call observer) and PR-2's new `atlas-agents::AgentEvent` (agent-level runtime event log). **PR-1 must rename** the former to `BackendCallEvent` (and `AgentObserver` to `BackendCallObserver`) in `crates/atlas-llm/src/agent_observer.rs` (file renamed to `backend_call_observer.rs`) before PR-2 can land its `AgentEvent` enum. See plan §4 Task 1 Step 1.3.

2. **`tokio` + `async-trait` are new workspace dependencies.** PR-1 adds them under `[workspace.dependencies]` in the root `Cargo.toml`. PR-1 also pre-adds `ratatui` + `crossterm` (no-cost when unused; PR-6 picks them up). Path-deps carry path only, no `version` field (memory `feedback_no_version_on_workspace_path_deps`).

3. **`crates/atlas-engine/src/override_warnings.rs` already exists** (Phase 6 PR-4). Plan refers to it as a *modify*, never a *create*. Same for `crates/atlas-engine/src/atomic_write.rs` (PR-2 adds `atomic_write_pair` alongside today's `atomic_write`).

4. **Wrapper count is 27 (not 26).** Brainstorm §5 cited "26 wrappers" but enumerated 27 modules across PR-3a/3b/3c (9 manifests + 4 mature + 6 mid-tier + 8 weak-tooling = 27). Plan §4 Task 3 locks 27.

5. **PR-1 and PR-5 are the scope-creep risks** (plan §7.6, §7.7). PR-1 stacks: new crate + `Tool` trait + MCP multi-client server + async `LlmBackend` for 5 backends + rename + 4 new deps. PR-5 stacks: fixed-point + LLM dispatch + Lane B. Either subagent should stop-and-surface if it hits >2x its time/LOC estimate at any checkpoint.

### PR-1

2026-05-12 — Landed: new crate `crates/atlas-agents/` containing the `Tool` trait + args/result/error types + `ToolContext` + `FingerprintInput` + `ToolSchema` + `ToolHandle` alias (in `src/tool.rs`); an in-process MCP stdio server with multi-client multiplexing via `Arc<McpServer>` + per-client tokio task (in `src/mcp/server.rs`); JSON-RPC framing types in `src/mcp/mod.rs`; `Tool::json_schema()` → MCP tool-descriptor conversion in `src/mcp/descriptors.rs`; subprocess built-in tool restrictions documented in `src/mcp/restrictions.md`; and an empty-but-forward-pointer-stubbed `src/runtime/mod.rs` for PR-4+. `crates/atlas-llm` extended: `LlmBackend` trait now carries `async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError>` alongside the sync `call`; subprocess backends (`claude_code`, `codex`) implement `call_async` natively via `tokio::process::Command` reusing existing render/parse helpers; HTTP backends (`http_anthropic`, `http_openai`) implement `call_async` natively via `reqwest::Client`'s async API; `test_backend`/`router`/`budget` delegate. Pre-PR-2 rename: today's `AgentEvent` → `BackendCallEvent` and `AgentObserver` → `BackendCallObserver` (file renamed `agent_observer.rs` → `backend_call_observer.rs`); cascade-updated call sites in `crates/atlas-llm/src/stream_parse.rs`, `crates/atlas-llm/src/codex_stream.rs`, `crates/atlas-cli/src/progress.rs`, and fixture-README docstrings. Trait-impl cascade: `BudgetSentinel` (atlas-cli backend), `AlwaysBoundaryBackend` (atlas-engine fixedpoint test), `PR14Backend` (atlas-cli polyglot test), and every other workspace `LlmBackend` impl gained `#[async_trait::async_trait]` + a `call_async` that delegates to sync `call`. New workspace deps: `tokio` (rt-multi-thread, sync, macros, process, io-util, time), `async-trait`, `ratatui`, `crossterm` (ratatui/crossterm pre-added so PR-6 needn't touch the workspace manifest). Commit: `0ec69f3` (PR-1 main commit).

Acceptance gates met:
- `cargo build --workspace` clean; `cargo test --workspace --no-fail-fast` all green across all crates including the new `atlas-agents` (22 tests, including `mcp_multiplex::two_concurrent_clients_isolated_dispatch` — the load-bearing multi-client MCP integration test).
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo fmt --check` clean (one trivial line-wrap drift in `crates/atlas-cli/src/progress.rs` import block was fixed via `cargo fmt --all`).
- `cargo build --release --workspace` clean.
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` pass in 101.37s — the cumulative regression guard held (test asserts `0 < cold < 100`; the Phase 6 PR-5 closeout's calibrated ~40 baseline carries forward unchanged).

Notes for PR-2:
- `crates/atlas-agents::AgentEvent` is now a free name — PR-2 introduces it in `crates/atlas-agents/src/events.rs` with the variants enumerated in brainstorm §4 (`IterationBoundary`, `AgentStart`, `ToolCall`, `ToolResult`, `AgentComplete`, `AuditFire`, `AuditVerdict`, `AuditDegraded`, `HardFail`, `CacheHit`, `RuntimeComplete`).
- `crates/atlas-agents/src/runtime/mod.rs` has module-doc forward-pointers but no submodules yet; PR-2 + PR-4 + PR-5 progressively populate. Today the crate exports `Tool`, `ToolArgs`, `ToolResult`, `ToolError`, `ToolContext`, `FingerprintInput`, `ToolSchema`, `McpServer` from `lib.rs`.
- `tokio` is in the workspace at `version = "1"` with features `rt-multi-thread, sync, macros, process, io-util, time`. PR-2 may need additional features (`tokio::sync::broadcast` is in `sync`; `tokio::io::duplex` is in `io-util`; `tokio::test` for the test runtime is gated by the `test-util` feature already in `atlas-agents`'s dev-deps). PR-2 should not need to extend workspace tokio features for the event bus.
- `crates/atlas-engine/src/atomic_write.rs` and `crates/atlas-engine/src/llm_cache.rs` are pre-existing — PR-2 modifies, never creates. (Confirmed by PR-0 forward-pointer #3.)
- `tracing` is NOT in the workspace deps today; the implementer correctly dropped it from `crates/atlas-agents/Cargo.toml`. Brainstorm §4 expects PR-2's transcript-cache subscriber to log lagged-receiver `RecvError::Lagged(n)` as an error — PR-2 needs to either (a) add `tracing` to the workspace deps and use `tracing::error!`, or (b) substitute `eprintln!` or `log::error!` if simpler. (a) is preferred for telemetry consistency once the event bus emits events to subscribers.
- The cumulative regression-guard test (`phase3_polyglot_fixture::polyglot_phase3_acceptance`) asserts `0 < cold < 100` rather than `cold == 40` exactly — the "~40 calibrated" framing in Phase 6 PR-5 closeout is a human-tracked expectation, not a strict assertion. PR-2+ doesn't need to preserve the exact count; only the loose bound.

Deviations from plan: none material. `cargo fmt --all` applied a one-line import wrap fix in `crates/atlas-cli/src/progress.rs:15` (legitimate cascade from the long `BackendCallEvent` name pushing the import block past rustfmt's column threshold). Fixture-README docstrings in `crates/atlas-llm/tests/fixtures/{codex-stream,stream}/README.md` updated to reference `BackendCallEvent::ToolUse`/`ToolResult` instead of the pre-rename name; doc-only, no test-behaviour change.

### PR-2

2026-05-12 — Landed across three commits: `faa5fd9` (main PR-2 code), `4123011` (clippy::disallowed_methods narrowing follow-up), `87a193c` (contract polish + diagnostics follow-up). The two follow-ups are PR-2-scope; together they constitute the PR-2 code release, and this status-flip is the orchestrator's final commit per the two-commit pattern.

**What landed (main commit `faa5fd9`):**
- `crates/atlas-agents/src/events.rs` — `AgentEvent` enum (11 variants per recast §9.1: `IterationBoundary`, `AgentStart`, `ToolCall`, `ToolResult`, `AgentComplete`, `AuditFire`, `AuditVerdict`, `AuditDegraded`, `HardFail`, `CacheHit`, `RuntimeComplete`), `EventBus` (`tokio::broadcast` capacity 1024 per brainstorm §2 row 10), `Subscriber` type alias, helper enums `Grade` + `CacheHitSource`.
- `crates/atlas-agents/src/transport.rs` — `TransportFlavour` enum (`ClaudeCode`/`Codex`/`HttpAnthropic`/`HttpOpenai`) with stable `as_str()` wire form (`claude_code`/`codex`/`http_anthropic`/`http_openai`) + `Provider` rollup (`Anthropic`/`OpenAi`). Lock-down test `as_str_is_stable_snake_case` prevents a future enum rename from silently invalidating on-disk transcript caches.
- `crates/atlas-agents/src/agent_cache_writer.rs` — async subscriber stub that consumes `AgentComplete` and signals drain-handshake on `RuntimeComplete`. Lagged-receiver logs via `tracing::error!` (NOT silent drop, per recast §6.4 cache-corruption hazard). PR-4 wires the actual cache-write call site at the `// TODO(PR-4):` marker.
- `crates/atlas-engine/src/atomic_write.rs` — `atomic_write_pair` two-file primitive (rename a then rename b; half-pair window is detectable on next read via fingerprint-spot-check eviction, never panics). Three unit tests: both-files-present-after-success, neither-partial-on-first-write-failure (uses `NotADirectory` injection via a regular-file-masquerading-as-parent-directory trick rather than a flaky `chmod`), concurrent-writers-disjoint-temp-paths (20-write contention stress).
- `crates/atlas-engine/src/llm_cache.rs` — `call_agent_cached` multi-shot extension + `AgentInputFingerprint` carrying every recast §6.1 cache-key contributor (`stage_id`, `agent_id`, `agent_version`, `prompt_template_sha`, `tool_catalog_sha`, `model_id`, `backend_version`, **`transport_flavour: String`**, `target_input_shas`, `iteration_number`, `prior_model_sha`). Two-tier L1 + L2 write-through; on L2 hit, spot-check recorded fingerprint inputs against current file shas; evict on mismatch (recast §6.3). Six unit tests (4 plan-required + 1 cache-hit short-circuit + 1 half-pair-treated-as-miss from `87a193c`'s M8). `AgentRequest` and `AgentResult` are `#[non_exhaustive]` placeholders with `new(...)` constructors — additive contract locked in for PR-4+ extension. `frame_transcript_with_grade` + `parse_transcript_grade` are both `pub`, share `TRANSCRIPT_FRAME_PREFIX: &[u8] = b"# grade: "` and private `grade_label`/`grade_from_label` helpers (one source of truth for the four grade labels).
- `crates/atlas-engine/src/cache/layout.rs` — `agents_transcript_path` + `agents_output_path` helpers under `<root>/cache/agents/<stage>/`, isolated from the single-shot `<root>/cache/<stage>/` layout (no key collision).
- `crates/atlas-cli/src/jsonl_subscriber.rs` — JSON-Lines event-stream subscriber (stdout when `--no-tui`; file when `--log-events`). One event per line, drain-handshake-compliant. Lagged-receiver emits an in-band sentinel line `{"event":"LaggedReceiver","dropped":N}` (NOT silent drop).
- `crates/atlas-cli/src/cli_args.rs` — `--no-tui` (default false; PR-4+ implies it when stdout is not a terminal) and `--log-events PATH` (parallel to other subscribers) wired into `IndexArgs`. Three new parse tests.
- `crates/atlas-agents/tests/drain_handshake.rs` — integration test asserting `try_join!(wait_a, wait_b)` returns only after both slow subscribers flush past `RuntimeComplete`.
- New workspace dep: `tracing = "0.1"` (added per PR-1 handoff's explicit recommendation for telemetry consistency; consumed in `agent_cache_writer.rs` for the lagged-receiver error log).

**What landed (follow-up `4123011`):** Narrowed `clippy::disallowed_methods` in both `crates/atlas-agents/clippy.toml` and `crates/atlas-engine/clippy.toml` to keep only `tokio::runtime::Handle::block_on` forbidden (the actual nested-call deadlock vector). Removed `tokio::runtime::Runtime::block_on` from the disallowed list because the `#[tokio::test]` macro expands to `Runtime::new()...block_on(async {...})` and clippy attributes inner `.await` desugarings to the enclosing `Runtime::block_on`, which forced every async test file to `#![allow(clippy::disallowed_methods)]`. The PR-1 follow-up (`55131de`) added the rule with the claim "no existing block_on call sites in either crate" — but at commit time, `mcp_multiplex.rs` already had `.await` patterns that the over-broad rule flagged. PR-2 surfaced the misfire and the orchestrator chose the root-cause fix (narrow the rule) over the symptom suppression (scatter `#![allow]` across test files). All three test-side allow directives the implementer had added (in `drain_handshake.rs`, `mcp_multiplex.rs`, and `events.rs::tests`) were removed in the same commit.

**What landed (follow-up `87a193c`):** Four code-quality-review minor fixes — M1 (transcript-frame symmetry: shared `TRANSCRIPT_FRAME_PREFIX` constant + private `grade_label`/`grade_from_label` helpers; `parse_transcript_grade` made `pub`; M7 capacity-guess fix folded in), M3 (one paragraph added to `EventBus::emit` doc explaining that "Subscriber health is not monitored here" and the runtime owns liveness via the drain-handshake `try_join!` on `done_rx`), M6 (`#[non_exhaustive]` on `AgentRequest` and `AgentResult` + `new(...)` constructors + all 6 in-crate construction sites converted to `::new`; M9 vestigial-binding shed in the same sweep), M8 (`read_agent_pair` half-pair branch emits `tracing::warn!` with stage + key + which-half-present, then returns `None` to trigger recompute; new unit test plants a half-pair on disk and asserts the recompute path replaces the residue). Six code-quality minor items deferred (M2 Stage lock-down test; M4 `Arc<LlmResponseCache>` placeholder in `agent_cache_writer`; M5 typed `LaggedReceiver` variant; M10 `debug_assert_ne!` on `atomic_write_pair` paths) — none are blocking; PR-4 will absorb M4 naturally and M2/M5/M10 are cleanup-class.

**Acceptance gates met (orchestrator-side independent verification on `main` post-`87a193c`):**
- `cargo build --workspace` clean (2.57s)
- `cargo fmt --check` clean (empty output)
- `cargo clippy --all-targets -- -D warnings` clean (exit 0)
- `cargo test --workspace --no-fail-fast` exit 0 (isolated re-run; the first attempt hit a 20+-minute slowdown on dev-mode `phase3_polyglot_fixture` because I had launched it concurrently with the release-mode polyglot smoke — they don't share a cargo lock but do share the system process table and subprocess fan-out; lesson logged for PR-3+ orchestration to keep heavy-subprocess tests serial)
- `cargo build --release --workspace` clean (3.21s)
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — `polyglot_phase3_acceptance ok` in 104.72s. Cumulative regression guard held; cold count stays in loose-bound `0 < cold < 100`.

**Concerns triaged at orchestration time (all accepted; documented for forensic context):**

1. **`agent_cache_writer.rs` relocated from `crates/atlas-engine/src/` (plan) to `crates/atlas-agents/src/` (landed).** Plan placement would have inverted the workspace dependency direction — atlas-agents already depends on atlas-engine, and the subscriber needs `atlas_agents::events::{AgentEvent, EventBus}`. Cycle fix; cache writer reaches into atlas-engine via `Arc<LlmResponseCache>` parameter. Documented in the file header (`crates/atlas-agents/src/agent_cache_writer.rs:1-15`).

2. **`AgentInputFingerprint.transport_flavour` stored as `String`** (the `as_str()` wire form), not as the `TransportFlavour` enum. Same cycle reason — `crates/atlas-engine/src/llm_cache.rs` cannot import from `atlas-agents`. Cache-key contribution invariant is preserved: the wire form is hashed into the key, and the `agent_cache_key_includes_transport_flavour` test exercises `"claude_code"` vs `"codex"` producing different keys.

3. **`AgentRequest` / `AgentResult` minimal placeholder shape** — `AgentRequest { payload: Vec<u8> }`, `AgentResult { transcript_bytes, output_bytes, confidence_grade }`. Both `#[non_exhaustive]` with `new(...)` constructors. PR-4 will extend additively.

4. **5th cache-hit short-circuit test** added beyond the plan's 4 required (`agent_cache_hit_short_circuits_compute_when_spot_check_clean`). Legitimate completeness addition — the plan's 4 tests cover key+eviction+write+no-write-on-fail; none cover the positive cache-hit path with a clean spot-check.

5. **`tracing` added to workspace deps.** Per PR-1 handoff explicit recommendation; consumed only in `agent_cache_writer.rs:81` (lagged-receiver error log) and `llm_cache.rs::read_agent_pair` (half-pair warn).

6. **`AuditDegraded { reason: String }`** matches the plan literal; brainstorm §4 wrote `&'static str` but that's incompatible with serde round-trip (no owned storage). Plan implicitly overrode the impractical brainstorm form. The `jsonl_subscriber` golden-file test round-trips this variant; `&'static str` would have failed `Deserialize`.

**Forward-pointers to PR-3 (parallel tool wrappers) and PR-4 (runtime + Lane A):**

- **PR-3 (Wave 2, three parallel subagents):** Depends on PR-2's `Tool` trait (PR-1) + nothing PR-2 added. PR-3 subagents own disjoint module sets under `crates/atlas-agents/src/tools/`. Use pre-created worktrees per memory `feedback_worktree_base_verification`; wrapper count is **27 not 26** (PR-0 forward-pointer #4). The shared re-export list in `crates/atlas-agents/src/tools/mod.rs` is the conflict surface during the integration merge.
- **PR-4 (runtime single-iteration + Lane A):** Consumes `AgentRuntime` + `Tool` + `EventBus` + `AgentEvent` + `call_agent_cached` + the `AgentRequest`/`AgentResult` shape. Specific contract obligations to PR-2:
  - Construct `AgentRequest`/`AgentResult` via `AgentRequest::new(...)` and `AgentResult::new(...)` — the `#[non_exhaustive]` is enforced from outside the crate.
  - Produce transcript bytes through `frame_transcript_with_grade(grade, body)` and validate inbound transcripts (e.g., on cache-hit replay) via `parse_transcript_grade`. Both are `pub`; `TRANSCRIPT_FRAME_PREFIX` is the shared constant.
  - Wire `agent_cache_writer::run(bus, Arc<LlmResponseCache>, done_tx)` and `jsonl_subscriber::run(bus, dest, done_tx)` as subscribers; runtime owns subscriber liveness via `try_join!(done_rx_a, done_rx_b, ...)` after emitting `RuntimeComplete`. The `EventBus::emit` doc explicitly warns that send-side errors are silent — subscriber health is the runtime's responsibility.
  - The TUI subscriber (PR-6) is parallel; `--log-events PATH` is parallel to TUI; `--no-tui` is the runtime's "stdout JSON-Lines" toggle. PR-2 ships the flag plumbing only; PR-4+ activates them.
- **Cumulative regression guard:** PR-3+ must re-run `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` before flipping its checkbox. Cold is calibrated to ~40; the test self-asserts the loose bound `0 < cold < 100`. **Do NOT run dev-mode workspace tests concurrently with release-mode polyglot smoke** — they share the system process table and subprocess-spawn capacity, and the dev polyglot can stall for 20+ minutes under contention. Run heavy-subprocess tests serially.

**Deviations from plan:** All accepted concerns above are deviations; each is documented inline at the relevant call site. No iterator stubs for singletons; workspace path-deps carry path only; no hand-rolled TOML/YAML parsing. The agents_layout helpers landed in `cache/layout.rs` (plan said "or wherever the persistent-cache layout helpers live" — `layout.rs` is the canonical location).

**Test count deltas in PR-2:**
- `atlas-agents` lib: +5 (events) + +2 (transport) = +7 lib tests; +1 drain_handshake integration; mcp_multiplex unchanged at 4.
- `atlas-engine` lib: +4 (atomic_write_pair) + +6 (agent_cache: 4 plan + 1 short-circuit + 1 half-pair from M8) + +3 (cache::layout agents helpers) = +13 lib tests.
- `atlas-cli` lib: +3 (cli_args for `--no-tui`/`--log-events`); +1 jsonl_subscriber integration.
- Workspace total: +25 PR-2-attributable tests, all green. Cumulative regression guard unchanged.

### PR-3

2026-05-12 — Landed: 22 pure pass-through Tool wrappers across three subagent-owned tiers, merged into main via a `phase7-pr3` integration branch and fast-forwarded. Final tip on main: `b6f4b3d`.

**Scope deviation from plan §4 Task 3 (user-decided Option A):** Plan and brainstorm both enumerated 9 manifest parsers (4 of which existed, 5 of which did not). The 5 non-existent parsers (`parse_pyproject`, `parse_csproj`, `parse_k8s_manifest`, `parse_helm_chart`, `parse_release_toml`) were deferred to a later phase rather than authored as net-new tools under a "pure pass-through" charter (which they violated by construction). Revised wrapper count: 22 (10 classifiers + 8 surface analysers + 4 manifest parsers) — the 8/6/8 subagent distribution was preserved.

**Per-subagent commits (pre-integration):**
- `6317da6` — PR-3a initial: 8 wrappers (mature tier: 4 manifest parsers, 2 classifiers, 2 in-process surface analysers)
- `420d6bf` — PR-3a follow-up: `TsJsSurfaceTool` fixed-filename probe (replaced recursive `src/` walk with the engine's 8-entry-point allowlist per `crates/atlas-engine/src/l5_surface.rs:439-456`, restoring pass-through invariant)
- `eb6a19a` — PR-3b: 6 wrappers (mid-tier: 3 classifiers + 3 subprocess surface analysers for Python/C#/Dart)
- `87ffacf` — PR-3c: 8 wrappers (weak-tooling tier: 5 classifiers including Compose/Dockerfile-as-classifier — distinct from PR-3a's `parse_compose`/`parse_dockerfile` manifest parsers — + 3 subprocess surface analysers for Elixir/Racket/LispKit)

**Integration commits:**
- `e5a74c8` — merge phase7-pr3a (clean)
- `7a11a8b` — merge phase7-pr3b (resolved 3 expected `mod.rs` conflicts: top-level + classifiers + surfaces)
- `b09919c` — merge phase7-pr3c (resolved same 3 `mod.rs` conflicts after concatenation)
- `fed0da2` — review-feedback follow-up (4 fixes — see below)
- `b6f4b3d` — Cargo.lock update (atlas-agents → atlas-analyzers path-dep edge)

**Two-stage review (per `superpowers:subagent-driven-development`):**
- Spec compliance review (3 parallel subagents, one per branch): PR-3b ✅, PR-3c ✅, PR-3a flagged one MEDIUM issue — `TsJsSurfaceTool::collect_sources` did a recursive `src/` walk producing inputs the engine's analyser never sees. Fixed in `420d6bf`. Re-verified ✅.
- Code quality review (3 parallel `feature-dev:code-reviewer` subagents): four MEDIUM issues flagged + one fix-on-merge test issue. All resolved in `fed0da2`:
  1. **Path-traversal guard (cross-cutting, all 22 wrappers):** new shared helper `crates/atlas-agents/src/tools/path_utils.rs` exposing `require_within_root` + `require_path_arg`. Lexical check (no FS round-trip, no symlink resolution); rejects absolute paths and `..` components. Honours the `ToolContext` doc requirement ("Tools must reject paths that escape this root") that no wrapper enforced. 5 unit tests added.
  2. **CargoClassifyTool parity coverage:** added `rust-library` and `rust-cli` parity tests. The prior single test only exercised `[workspace]`; the other two classifier rules were not pinned.
  3. **PR-3b surface wrapper silent-drop:** python/csharp/dart surface wrappers' optional explicit `manifest_path` arg was being silently dropped on read failure via `if let Ok(bytes) = ...`. Changed to propagate the `ToolError::Filesystem` so callers see real failures when a path they explicitly named is unreadable.
  4. **PR-3c surface test robustness (test bug):** `elixir_surface_tool_returns_error_when_binary_missing` and `racket_surface_tool_returns_error_when_binary_missing` failed when `locate_*_analyzer_binary()` found a sibling-target binary (e.g. after a prior workspace build); the wrapper proceeded past the binary check and hit the un-written manifest. Fixed by writing the manifest to the tempdir before invoking the tool.

**Deferred (with rationale; not blocking PR-3):**
- Code quality reviewer flagged `fingerprint_inputs` returning `FingerprintInput { path, sha: [0u8; 32] }` zero-sentinel as HIGH. The `Tool` trait doc (`tool.rs:108-113`) says "Returned before `invoke` runs (so the runtime can pre-compute SHAs)" — the runtime fills shas, not the tool. The zero sentinel is consistent with the implementer brief I shipped to subagents. The contract ambiguity (struct doc says "SHA-256 of the file's bytes at the moment of the read") will be resolved by PR-4 when it wires the actual runtime. If PR-4 chooses tool-side sha computation, this becomes a tightening sweep; if runtime-side, the sentinel was correct.

**Convergent design choices across all three subagents (accepted):**
- `std::sync::OnceLock` (Rust 1.70+) replaced the brief's `once_cell::sync::Lazy`. Stdlib equivalent; one fewer dep.
- Subprocess surface wrappers (PR-3b + PR-3c) forward `SubprocessOutput.payload` (raw `serde_json::Value`) rather than typing it as a per-language `*SurfaceOutput`. The subprocess wire protocol is JSON-native — re-deserializing into a typed struct would round-trip with no value. Consequently no path-deps to the per-language analyzer crates (atlas-{python,csharp,dart,elixir,racket,lispkit}-analyzer) were needed.
- PR-3a surface wrappers (Rust + TS/JS) call `extract_rust_surface` / `extract_ts_js_surface` directly rather than `Analyzer::analyse()`. The trait `.analyse()` impls intentionally return `Declines` for these in-process analysers (they're driven by the engine via the direct functions); the wrappers mirror that engine driver pattern.

**Acceptance gates met (orchestrator-side independent verification on `phase7-pr3` post-`b6f4b3d`):**
- `cargo build --workspace` clean (3.68s)
- `cargo fmt --check` clean
- `cargo clippy --all-targets -- -D warnings` clean
- `cargo test --workspace --no-fail-fast` — all atlas-agents lib tests pass (38+ tests including 22 wrapper parity/error tests + 5 new path_utils tests + 2 added cargo parity tests); all other workspace tests pass (atlas-engine, atlas-llm, atlas-reports, atlas-index, atlas-cli, component-ontology, analyzer crates). Lesson: dev-mode `phase3_polyglot_fixture` was killed mid-run after ~13min — it's redundant with the release-mode cumulative regression guard run separately and adds no signal not covered by the release run. Recommend future PRs use `cargo test --workspace -- --skip phase3_polyglot` to elide it.
- `cargo build --release --workspace` clean (5.62s on incremental)
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — `polyglot_phase3_acceptance ok` in 104.34s. Cumulative regression guard held; cold count remains in loose-bound `0 < cold < 100`. The 22 wrappers added are dormant code in this PR — PR-4 wires them — so cold count by construction does not change.

**Forward-pointers to PR-4 (Wave 3 — runtime single-iteration + Lane A):**
- The 22 wrappers are all `Tool` impls at `atlas_agents::tools::{classifiers,manifests,surfaces}::*`. Re-exports at `atlas_agents::tools::{classifiers,surfaces}::{ToolStructName}` for ergonomic access; manifest parser wrappers are accessible via `atlas_agents::tools::manifests::{parse_cargo_toml::ParseCargoTomlTool, ...}`.
- The `path_utils` module is public (`atlas_agents::tools::path_utils`) and re-exports `require_within_root` + `require_path_arg` from `atlas_agents::tools` for convenience. PR-4's `AgentRuntime` should use these when constructing `ToolContext` (the workspace root) and when validating runtime-supplied paths that flow into tools.
- Tool catalog: PR-3 ships individual `Tool` impls but no `ToolCatalog` (the plan's "ToolCatalog registration" framing in §4 Task 3 was interpreted as just the module re-exports). PR-4's `AgentRuntime` is the natural home for a `BTreeMap<String, Arc<dyn Tool>>` keyed on `id()` if it needs central dispatch.
- `fingerprint_inputs` returns paths with zero-sha sentinels. PR-4 must decide whether to (a) keep the sentinel and have the runtime fill in shas via `tokio::fs::read` + `sha2::Sha256::digest`, or (b) modify the `Tool` trait to make `fingerprint_inputs` async + reading. Either path is small; (a) keeps `Tool` simpler.
- **No new LLM call sites.** PR-3 wrappers do not call into LLM backends — they're pure pass-throughs. PR-4's runtime + PR-5's LLM-decided dispatch are where LLM calls first land.
- **Compose-classifier + Dockerfile-classifier** (PR-3c) coexist with **parse_compose + parse_dockerfile** (PR-3a). Distinct tools producing distinct outputs (classifier kind+evidence vs parser structural shape). Both are useful to the LLM-spine; PR-4 catalogues both.

**Pre-merge cleanup performed:** worktrees `/tmp/atlas-phase7-pr3{a,b,c}` and branches `phase7-pr3{a,b,c}` + `phase7-pr3` (the integration branch) to be removed by the status-flip's housekeeping step.

### PR-4

2026-05-12 — Landed across three commits, fast-forwarded onto main:
- `80dac2f` — main PR-4 commit: `AgentRuntime` struct + deterministic-only dispatch (mandatory `subsystems.overrides.yaml` + `components.overrides.yaml` inputs; PR-5 relaxes), HTTP tool-use loop with byte-for-byte transcript recording, MCP tool-loop observation via per-client drain, Lane A schema validation with one-retry-then-hard-fail, per-transport + per-stage semaphores (HTTP=8, subprocess=2, per-stage=8), and a single-iteration smoke test against `test_backend` with canned responses. 12 files, +2666/-42 LOC.
- `3a5c986` — follow-up addressing spec-compliance review feedback: created `crates/atlas-agents/src/runtime/agent.rs` carrying the `Agent` value-object (stage, target_id, iteration) + `Agent::id()` formatter + `impl From<&AgentRequest>` that plan §4 file list required. The existing free `agent_id(req)` now delegates to `Agent::from(req).id()`; the dozen call sites in `mod.rs` are unchanged. 2 files, +108/-8 LOC. atlas-agents lib test count 84 → 87.
- `7f61e94` — follow-up addressing two HIGH issues from code-quality review:
  - **HIGH-1 (cache bypass):** `call_agent` ran the async tool loop unconditionally before consulting the transcript cache, defeating the cache's purpose. Root cause: `LlmResponseCache::call_agent_cached` is a sync probe-or-compute API; the implementer pre-computed and injected via `RefCell`. Fix: split the engine cache API into `probe_agent_pair` (sync probe) + `write_agent_pair` (sync write); restructured `call_agent` to **probe-first → async compute on miss → write**. The engine's `call_agent_cached` survives as a backward-compatible delegating wrapper so engine-side tests (5 cache tests in `llm_cache.rs`) keep working without modification. The `PrecomputedAgentBytes` struct + `RefCell` smuggling are gone.
  - **HIGH-2 (drain handshake):** `run_workspace` never emitted `AgentEvent::RuntimeComplete` on any exit path. The integration test masked this with a manual emit after `run_workspace` returned. Fix: wrap `run_workspace`'s body so `RuntimeComplete` fires unconditionally before returning (`let result = run_iteration(...).await; emit(RuntimeComplete); result`); removed the test workaround.
  - 3 files, +147/-108 LOC.

**Acceptance gates met (orchestrator-side independent verification on main post-`7f61e94`):**
- `cargo build --workspace` clean
- `cargo fmt --check` clean (one trivial PR-3-era drift was already fixed; no PR-4 drift)
- `cargo clippy --all-targets -- -D warnings` clean
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` — 1498 tests across 85 result blocks all green. **Lesson re-confirmed:** the `--skip` substring pattern for cargo test is literal — PR-3 closeout's recommendation `--skip phase3_polyglot` does NOT match the actual test function name `polyglot_phase3_acceptance`; the correct substring is `polyglot_phase3`. Memory recommendation: update `feedback_atlas_test_subprocess_concurrency` or add a sibling memory for the cargo `--skip` substring gotcha so future PR briefs use the working pattern.
- `cargo build --release --workspace` clean
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — `polyglot_phase3_acceptance ok` in 104.20s on the post-`80dac2f` worktree (the cumulative regression guard for the main PR-4 commit). The follow-ups (`3a5c986` + `7f61e94`) are additive in atlas-agents/engine only and don't affect the polyglot smoke's deterministic-dispatcher path; re-running was triaged as not required.

**Two-stage review (per `superpowers:subagent-driven-development`):**
- Spec compliance review (background `feature-dev:code-reviewer` subagent): flagged the missing `agent.rs` file from plan §4 file list as the single load-bearing issue. Fixed in `3a5c986`. Re-review implicit (file existence + spec match was the entire delta). ✅
- Code quality review (background subagent on the post-`3a5c986` tip): flagged two HIGH issues (cache bypass + RuntimeComplete missing) and two MEDIUMs deferred to PR-5/PR-7 (see below). HIGHs fixed in `7f61e94`. Re-review on `7f61e94` ✅ APPROVED — both HIGHs correctly fixed, no regressions introduced, deferred MEDIUMs confirmed absent.

**Deferred from PR-4 code-quality review (route into PR-5 or PR-7 cleanup as appropriate):**
- **MEDIUM-1: Semaphore acquisition-order invariant.** `call_agent` acquires the transport permit first; `run_tool_loop_http` acquires the stage permit inside. Both held across the `backend.call_async` await. Under defaults (HTTP=8, per-stage=8) this is fine, but `Semaphores::with_caps` lets callers create asymmetric configs where two concurrent tasks holding transport permits could deadlock waiting on stage permits. Fix: document the fixed acquisition order on `Semaphores` and consider merging both acquire calls into a single `acquire_for_agent(transport, stage)` method that enforces order. Track in PR-5 or PR-7.
- **MEDIUM-2: Lane A retry test doesn't assert `call_count == 2`.** The `lane_a_retry_fires_exactly_once_on_classify_schema_fail` test asserts the final projection but doesn't pin "exactly one retry" at the call-count level. An accidental retry-count regression (zero retries OR two retries) could pass undetected. Fix: add `assert_eq!(backend.call_count_for_stage("classify"), 2)` after `run_workspace`. Track as a PR-5 / PR-7 test-suite hardening item.

**Forward-pointers to PR-5 (LLM-decided dispatch + fixed-point iteration + Lane B):**
- `AgentError::LaneBFail(String)` variant is **already present** as a PR-5 placeholder (added in `80dac2f`) so PR-5's pattern matches don't need to grow. PR-5 fills in the real error type via `#[from]` when Lane B lands.
- PR-4's logical `Stage` enum (`DispatchSubsystem`, `DispatchComponent`, `Classify`, `Surface`, `Reduce`, `Project`) is in `crates/atlas-agents/src/runtime/audit/lane_a.rs`. PR-5's `dispatch_subsystems` and `dispatch_components` need to handle the override-shortcircuit case (override-file present → emit synthetic CacheHit transcript; absent → call the LLM dispatch agent). PR-4 hard-errors with `AgentError::OverrideRequired` when the file is missing; PR-5 relaxes this.
- `AgentRuntime::backend_router` is `Arc<dyn LlmBackend>` not `Arc<BackendRouter>` — PR-7 wiring will plug a real `BackendRouter` instance there (which implements `LlmBackend`).
- The runtime emits 11 of the 11 `AgentEvent` variants (PR-2's full enumeration) — except `AuditDegraded`, `AuditFire`, `AuditVerdict` which are PR-5's responsibility (Lane B).

**Forward-pointers to PR-7 (end-to-end wiring + closeout):**
- The runtime's sync→async boundary moves to `atlas-cli/src/main.rs` in PR-7 via a single `Handle::block_on(runtime.run_workspace(workspace))`. PR-4's clippy::disallowed_methods rule forbids `block_on` in atlas-engine and atlas-agents/runtime/, so the boundary is structurally enforced.
- `fingerprint_inputs` zero-sentinel resolution: PR-4 chose path (a) — keep the sentinel + runtime fills SHAs via `tokio::fs::read` + `sha2::Sha256::digest` at the `call_agent` call site (mod.rs:455–471). This works correctly with the `Tool` trait's sync surface; no Trait modification needed.
- PR-4 defines its own logical `Stage` enum (DispatchSubsystem..Project) distinct from `atlas_index::Stage` (L1..L9 engine cache layout). PR-6 uses `atlas_index::Stage` for replay. PR-7 will reconcile if the divergence proves load-bearing; today the two stages live on different axes (logical agent stage vs. on-disk cache layout) and don't need to unify.
- **`AgentRuntime::run_workspace` is NOT wired into `atlas index`** — PR-7 adds the wiring + the Atlas-on-Atlas baseline calibration + the cross-transport parity test extension to the polyglot smoke. The runtime is exercised only by the single-iteration smoke test in PR-4.

**Deviations from plan §4 Task 4 (all documented inline at the relevant call sites):**
- `AgentRuntime::backend_router: Arc<dyn LlmBackend>` (plan said `Arc<BackendRouter>`). `BackendRouter::from_dispatch_table` is `#[cfg(test)]`-gated on the producer crate; the integration test couldn't construct one without an upstream lift. PR-7 will wire a real `BackendRouter` (which implements `LlmBackend`).
- `AgentInputFingerprint.transport_flavour: String` (the wire form) rather than the `TransportFlavour` enum, because `atlas-engine::llm_cache` cannot import from `atlas-agents` (would invert dep direction). Round-trip invariant preserved.
- `current_sha_fn = |_path: &str| None` in the cache probe — PR-4's placeholder. PR-7 wires the real `AtlasDatabase`-backed sha lookup.
- Cache write happens on `IndexStage::L8` (a fixed stage axis for now). PR-5 may parameterise this if subsequent stages need distinct cache namespaces.

**Test count deltas in PR-4:**
- `atlas-agents` lib: +3 (agent.rs) + +5+ (lane_a.rs + dispatch.rs + tool_loop_http.rs + semaphores.rs unit tests) = atlas-agents lib went from 72 to 87 (cumulative across the three commits).
- `atlas-agents` integration: +3 in `agent_runtime_single_iteration.rs` (end-to-end + retry + staged-backend pop).
- `atlas-engine` lib: probe/write split tested via the existing `call_agent_cached` tests (no new tests needed — the back-compat wrapper exercises both new methods).
- Workspace total: +6 to +10 PR-4-attributable tests across the three commits. Cumulative regression guard unchanged.

### PR-5

2026-05-12 — Landed across three code commits, fast-forwarded onto main:
- `12bbbec` — main PR-5 commit: `run_fixedpoint` wrapping `run_iteration` with content-sha convergence + `IterationBoundary` emission moved here from `run_iteration`; `dispatch_subsystems`/`dispatch_components` override-shortcircuit path (Lane-A-validates + emits `CacheHit { source: DispatchedFromOverride }` + writes synthetic transcript-cache entry framed with `Grade::Strong`); `lane_b_audit` cross-provider auditor module (Anthropic↔OpenAI mapping + same-model fallback emitting `AuditDegraded`) with `AuditVerdict { Accept, RequestRevision, HardFail, Skipped, Degraded(Box<inner>) }` (last variant additive vs spec's 4); `AgentInputFingerprint` extended with `override_content_sha: Option<[u8; 32]>` contributor for the cache-invariant rule; `AgentRuntime::max_iterations` (default 5) + `AgentRuntime::for_provider` injectable closure (approach (c) from PR-5 brief's known-unknown #1 — keeps `Provider` in atlas-agents and avoids advancing the PR-7 `BackendRouter` wiring); `AgentError::FixedpointDiverged { iterations, last_changed_agents }` variant added (last_changed_agents is empty Vec for PR-5 minimum-viable diagnostic per known-unknown #3). 10 files, +1899/-122 LOC. 13 new tests across `tests/{dispatch_shortcircuit,audit_lane_b,fixedpoint_convergence}.rs`.
- `7f51393` — follow-up addressing two MUST-FIX spec-compliance gaps from spec review: **(A)** the no-override path in `dispatch_subsystems`/`dispatch_components` was raising `OverrideRequired` instead of firing the LLM agent (collapsed PR-5's stated purpose). Wired the LLM-dispatch call via `runtime.call_agent(...)`; canned-response shape matches `SubsystemsOverrideFile`/`ComponentsOverrideFile` so both override and LLM paths converge on the same `*_from_parsed` projectors; stub prompt marked `PR-7-WIRES-REAL-PROMPT`; the spec-required test `dispatch_without_override_file_fires_llm_agent` was renamed from its inverted PR-5 form and asserts (a) backend called, (b) parsed partitions match canned response, (c) no `DispatchedFromOverride` cache-hit emitted, (d) normal `AgentComplete` emitted for `dispatch_subsystem::*`. **(B)** Lane B was defined as primitives in `lane_b.rs` but never reached from `call_agent` — wired `audit::lane_b_audit(...)` into `call_agent` after `run_tool_loop_with_lane_a` returns and before cache write; introduced `ToolLoopOutcome { result, lane_a_retries }` to thread cumulative-retry count; new helper `resolve_audit_verdict(verdict, lane_a_retries)` handles all 5 verdict cases including `Degraded(inner)` recursion and the cumulative-budget rule (`lane_a_retries >= 1` → `RequestRevision` escalates to `HardFail` per recast §4.3); on `HardFail` emits `AgentEvent::HardFail` + returns `AgentError::LaneBFail(reason)`. Audit closure stub returns `AuditVerdict::Accept` (marked `PR-7-WIRES-REAL-AUDITOR`). New wiring test `lane_b_wired_into_call_agent_skips_on_strong_grade` exercises the full `run_workspace` → `call_agent` → `lane_b_audit` path end-to-end. 4 files, +489/-62 LOC.
- `7ec3da7` — follow-up addressing the single non-deferred MEDIUM from PR-5 code-quality review: removed dead no-op self-assignment `runtime_result.grade = runtime_result.grade.clone();` in the `ResolvedAuditAction::RequestRevision(_)` arm (the arm's intent is to fall through to the cache-write below, which the surrounding multi-line comment already documents); dropped the now-unnecessary `mut` on `let runtime_result = tool_outcome.result;` to keep `-D unused-mut` clean. 1 file, +3/-3 LOC. Zero behaviour change.

**Acceptance gates met (orchestrator-side independent verification on main post-`7ec3da7`):**
- `cargo build --workspace` clean (2.07s)
- `cargo fmt --check` clean
- `cargo clippy --all-targets -- -D warnings` clean (9.23s)
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` exit 0; zero failures grep'd workspace-wide
- `cargo build --release --workspace` clean (3.99s)
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — `polyglot_phase3_acceptance ok` in 104.32s on main post-`7ec3da7`. Cumulative regression guard held; cold count stays in loose-bound `0 < cold < 100`. Timing envelope matches PR-4's 104.20s and PR-6's 109.34s — the LLM-dispatch site PR-5 introduced is unreachable from the polyglot smoke because the fixture has full override coverage (the load-bearing protection per plan §2.2 non-negotiable #8).

**Two-stage review (per `superpowers:subagent-driven-development`):**
- **Spec compliance review v1** (on `12bbbec`): ⚠️ COMPLIANT WITH GAPS — two MUST-FIX issues. Implementer's Deviation A (no-override path raises `OverrideRequired`) directly contradicted plan §2.2 non-negotiable #8 + brainstorm §2 row 2 (locked) + brainstorm §6 (i) pseudocode; the rationale "firing an LLM call without production prompts defeats the polyglot guard" inverted the actual protection mechanism (the *shortcircuit* + full-override polyglot fixture is what keeps the regression guard green, not the absence of the LLM call site). Implementer's Deviation B (Lane B primitives-only, not wired into `call_agent`) made the entire Lane B subsystem unreachable from production code. Both routed to a follow-up fix.
- **Spec compliance re-review** (on `7f51393`): ✅ COMPLIANT — both gaps fully closed; no regressions in the four pre-existing dispatch tests, four pre-existing lane_b tests, three fixedpoint tests, or the fixedpoint_loop module.
- **Code quality review** (on `12bbbec + 7f51393`): ⚠️ APPROVED WITH MINORS — strengths called out: clean `fixedpoint_loop.rs` extraction with `IterationBoundary` emission relocated cleanly; `lane_b_audit` design with `AuditorChoice` + `Degraded(Box<inner>)` excellent for testability + verdict preservation; cache-invariant rule enforcement via sentinel-byte `Some`/`None` prefix is rigorous; well-layered test suite (unit tests inside lane_b.rs + integration tests in tests/audit_lane_b.rs). Three MEDIUMs flagged: (1) dead no-op self-assignment at mod.rs:689 (fixed in `7ec3da7`); (2) `OverrideRequired` error variant semantically wrong for LLM output parse failures — explicitly deferred to PR-7 when production prompts make this user-facing; (3) `now_iso` duplicated between mod.rs and dispatch.rs — explicitly deferred to PR-7 with millisecond-precision upgrade. Five LOW nits and four AWARENESS items (none blocking).

**Deferred from PR-5 code-quality review (route into PR-7 cleanup):**
- **MEDIUM-2 (semantic): `OverrideRequired` is used as the error variant for LLM dispatch output parse failures** in `dispatch.rs` (the two `parse_*_from_output_value` map_err sites). The variant's display text ("override file missing or malformed") will mislead anyone who hits a real LLM output-malformed failure in production. Fix in PR-7: add `AgentError::LlmOutputMalformed(String)` variant; update both map_err sites; add a `// TODO(PR-7)` comment at each site for forensic traceability.
- **MEDIUM-3 (duplication): `now_iso` duplicated** between `crates/atlas-agents/src/runtime/mod.rs` (~line 995) and `crates/atlas-agents/src/runtime/dispatch.rs` (~line 510). Both have identical bodies and identical "upgrade to milliseconds" TODO comments. Fix in PR-7: hoist to `crate::runtime::util` (or `pub(super)` at top of mod.rs) when the millisecond-precision upgrade happens; dispatch.rs calls the consolidated helper.
- **AWARENESS-A: `lane_b_wired_into_call_agent_skips_on_strong_grade`** asserts `!saw_audit_fire` — a negative assertion that passes both when Lane B is wired-and-skips AND when Lane B is removed entirely. PR-5's documented constraint (no multi-grade test backend available). Complementary PR-7 test: inject a `Weak`-grading backend and assert `AuditFire` is emitted (positive assertion that closes the gap).
- **AWARENESS-B: `run_fixedpoint`'s `unwrap_or(l9)` fallback at lines 65-73** is technically unreachable once `effective_max >= 2` (the `effective_max == 1` single-iteration sentinel branch is handled earlier). Not a bug — defensive programming + clear intent — but a future cleanup-class item if the single-iteration sentinel goes away.

**Convergent design choices across PR-5 (accepted):**
- **Approach (c) for known-unknown #1 (BackendRouter wiring):** `AgentRuntime::for_provider: Option<Arc<ForProviderFn>>` injectable closure rather than (a) extending the `LlmBackend` trait or (b) advancing the PR-7 `BackendRouter` wiring. Mirrors the existing `current_sha_fn` placeholder pattern; keeps `Provider` confined to atlas-agents (no atlas-llm/atlas-agents circular dep); PR-7 plugs in a real `BackendRouter`-backed closure.
- **Synthetic-transcript shape for dispatch-from-override (known-unknown #2):** override-yaml bytes framed via `frame_transcript_with_grade(Grade::Strong, override_yaml_bytes)`; output bytes are JSON-serialised `Vec<SubsystemPartition>`/`Vec<ComponentPartition>`; cache fingerprint carries `target_input_shas: vec![]` + `override_content_sha: Some(sha)` for cache-invariant enforcement.
- **`collect_shifted_agents` minimum-viable form (known-unknown #3):** `last_changed_agents: Vec<String> = vec![]` empty stub; PR-7 enriches with per-agent transcript-sha diffs across iterations once those structures are tracked in `AgentRuntime` state.
- **`max_iterations` on `AgentRuntime` not `IndexConfig` (known-unknown #4):** field with default `5` lives on `AgentRuntime`; PR-7 threads `IndexConfig::max_iterations` through during the `atlas index` wiring.
- **JSON envelope for LLM dispatch output (Fix 1 follow-up design call):** the LLM emits a JSON object matching `SubsystemsOverrideFile` / `ComponentsOverrideFile` deserialise shape. New helpers `parse_subsystems_from_output_value` / `parse_components_from_output_value` feed parsed structs into the existing `subsystems_from_parsed` / `components_from_parsed` projectors. Same projection step regardless of whether the source is YAML-override or JSON-LLM-output.

**Deviations from plan §4 Task 5 (all documented in commit messages + status notes):**
- `AuditVerdict` has 5 variants instead of plan's 4 (`Degraded(Box<inner>)` additive) — distinguishes cross-provider audit from same-model-fallback audit without an extra field.
- 13 spec-mandated tests landed in `12bbbec`; +1 wiring test in `7f51393` (`lane_b_wired_into_call_agent_skips_on_strong_grade`); +1 symmetry test in `12bbbec` (`dispatch_components_shortcircuit_emits_dispatched_from_override`). Total: 15 new tests (12 mandated + 3 additive).
- `AgentError::LaneBFail(String)` kept as stringified variant rather than `#[from]` on a structured Lane B error type — the audit verdict already carries structured failure detail via `AuditVerdict::HardFail(String)`, and the wiring at `call_agent`'s HardFail arm threads the reason string directly.
- The `RequestRevision` retry harness fires the `AuditVerdict` event but accepts the producer result on the non-escalation branch (`lane_a_retries == 0`). The cumulative-budget rule IS enforced (`lane_a_retries >= 1` → `HardFail` escalation). Full prompt-revision-retry mechanism marked `PR-7-ENRICHES-PROMPT-WITH-REVISION-REASON`.
- Lane B's audit closure stub returns `AuditVerdict::Accept` regardless of producer output (marked `PR-7-WIRES-REAL-AUDITOR`). PR-7 wires the real cross-provider audit prompt round-trip; PR-5's wiring is the spec deliverable, not the audit's empirical firing frequency.

**Forward-pointers to PR-7 (Wave 5 — end-to-end wiring + polyglot smoke extension + Atlas-on-Atlas calibration + closeout):**
- The runtime is NOT wired into `atlas index` yet. PR-7's Step 7.x: `Handle::block_on(runtime.run_workspace(workspace))` at the CLI entry point (single sync→async boundary).
- `AgentRuntime::for_provider` needs a real wired closure delegating to a `BackendRouter`-shaped per-provider lookup. Closure shape: `Fn(Provider) -> Option<Arc<dyn LlmBackend>>`. PR-7 either (a) un-gates `BackendRouter::from_dispatch_table` from `#[cfg(test)]` so production code can build one, or (b) writes a small helper that maps the default `claude_code + codex` dispatch table to per-provider backends.
- `dispatch_subsystems` / `dispatch_components`'s no-override path needs the production LLM-dispatch prompt template; PR-5 stubs are marked `PR-7-WIRES-REAL-PROMPT`. Until PR-7, the test backend's canned-response shape keyed on the `"dispatch subsystems"` / `"dispatch components"` substring is the only way to exercise the no-override path.
- Lane B's audit closure needs the production cross-provider audit prompt; PR-5 stub is marked `PR-7-WIRES-REAL-AUDITOR`. The audit transcript on-disk layout (`.atlas/audit/<stage>/<target>.yaml` per brainstorm §6 (iii)) is NOT yet implemented — PR-5 only emits the events; PR-7 materialises the on-disk artefacts.
- `AgentRuntime::max_iterations` should be threaded from `IndexConfig::max_iterations`; PR-5 hardcodes default `5`.
- `AgentError::FixedpointDiverged.last_changed_agents` is currently empty `Vec<String>`; PR-7 enriches once per-agent transcript shas are tracked in `AgentRuntime` state across iterations.
- The two deferred MEDIUMs from code-quality review (`OverrideRequired` semantic misuse on LLM output-parse failures; `now_iso` duplication between mod.rs and dispatch.rs) are PR-7 cleanup-class items.
- The `AWARENESS-A` positive-assertion Lane B test (inject `Weak`-grading backend → assert `AuditFire` emitted) is the complementary test PR-7 should add when production prompts can emit non-Strong grades.

**Polyglot regression behaviour:** unchanged. The fixture has full override coverage for every L2 candidate, so the LLM-decided dispatch site PR-5 introduced is unreachable from the smoke. Cold count stays in loose-bound `0 < cold < 100` (calibrated ~40 per Phase 6 PR-5 closeout); warm + reports = 0. PR-7's Atlas-on-Atlas calibration (no overrides → dispatch fires) produces a *separate* baseline that is NOT comparable to the polyglot baseline.

**Test count deltas in PR-5:**
- `atlas-agents` lib: +5 unit tests in `fixedpoint_loop.rs` (content_sha determinism, order invariance, distinct payloads, default-max constant, equal-projections short-circuit) + ~10 unit tests in `dispatch.rs` (parser symmetry, fingerprint hashing, override-content-sha sensitivity, Lane A validation paths) + ~12 unit tests in `audit/lane_b.rs` (provider mapping, verdict resolution, AuditDegraded emission, recursion on Degraded).
- `atlas-agents` integration: +6 in `tests/dispatch_shortcircuit.rs` + +4+1 in `tests/audit_lane_b.rs` (4 original + 1 wiring) + +3 in `tests/fixedpoint_convergence.rs` = +14 integration tests.
- `atlas-engine` lib: +1 in `llm_cache.rs` (override_content_sha sensitivity in `AgentInputFingerprint::to_cache_key`).
- Workspace total: ~+42 PR-5-attributable tests across the three commits, all green. Cumulative regression guard unchanged.

**LOC totals:**
- `12bbbec`: +1899 / -122 (10 files)
- `7f51393`: +489 / -62 (4 files; modifies dispatch.rs + mod.rs + 2 test files)
- `7ec3da7`: +3 / -3 (1 file; mod.rs cleanup)
- Net PR-5 lineage: ~+2391 LOC / -187 LOC. Roughly 1.5× the brief's implicit envelope (~700-1200 LOC predicted from the 5 task steps + 3 test files) — within the brief's "stop and surface at 2×" threshold; not flagged by implementer at any checkpoint.

### PR-6

2026-05-12 — Landed as two commits, dispatched in parallel with PR-4 (per plan §3 Wave-4 || Wave-3 lane), implemented in worktree `/tmp/atlas-phase7-pr6`, then rebased onto PR-4's tip and fast-forwarded to main. Post-rebase SHAs:
- `9618040` — main PR-6 commit (pre-rebase was `d1d9039`): TUI subscriber rendering `AgentEvent` stream into a four-widget frame (tree view, token panel, iteration bar, stuck detector at 90s); `--replay-from-cache` mode that re-emits cached transcript pairs onto the bus so the TUI renders identically against recorded runs (without invoking any backend); CLI flags `--replay-from-cache` + `--tui-show-providers` added to `IndexArgs`; `TerminalGuard` RAII restores raw mode + leaves alternate screen even on panic. 16 files, +2058/-20 LOC.
- `f2ce6d5` — follow-up (pre-rebase was `267c60d`): restored spec-mandated `Constraint::Length(2)` bottom slot per plan §4 Task 6.2 (main commit shipped `Length(4)` without spec-traceable rationale; flagged by spec-compliance reviewer as the single literal deviation). 1 file, +4/-3 LOC. Purely cosmetic — affects rendered pixel rows only; the replay test's `TuiSnapshot` byte-equality assertion remained green.

**Acceptance gates met (orchestrator-side independent verification on the rebased worktree):**
- `cargo build --workspace` clean
- `cargo fmt --check` clean (one trailing-newline drift in `replay.rs:442` fixed via `cargo fmt --all` before initial commit)
- `cargo clippy --all-targets -- -D warnings` clean
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` — 1484 tests across 85 result blocks all green
- `cargo build --release --workspace` clean
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — `polyglot_phase3_acceptance ok` in 109.34s on the pre-rebase tip. The follow-up rebase touched only the doc-comment + numeric constants in `tui/mod.rs`'s layout — non-load-bearing for the polyglot smoke's deterministic-dispatcher path; re-running was triaged as not required.
- Post-rebase verification: workspace build clean (8.80s); `cargo test -p atlas-cli --lib --test replay` — 93 lib + 3 replay = 96 tests green.

**Two-stage review (per `superpowers:subagent-driven-development`):**
- Spec compliance review (background `feature-dev:code-reviewer` subagent on `d1d9039`): flagged the layout deviation (`Length(4)` vs spec-mandated `Length(2)`) as the single literal issue. Fixed in `267c60d` (post-rebase `f2ce6d5`). All other §6.1–§6.4 checks PASS — TUI subscriber skeleton, state model + four widgets, replay-from-cache with `TransportMismatch` clean-cache invariant, CLI flag wiring, PR-2 surface preservation, PR-7 scope containment.
- Code quality review (background subagent on `267c60d`): ⚠️ APPROVED WITH MINORS — three MEDIUMs flagged as deferrable to PR-7 (see below) plus one awareness note on `Length(2)` border-clipping (a stuck_detect line rendered inside a bordered Block needs Length(3); Length(2) clips the content row — but Length(2) is spec-mandated, so this is the spec's design choice not a code-quality bug).

**Deferred from PR-6 code-quality review (route into PR-7 wiring as appropriate):**
- **MEDIUM-1: Drain handshake `done_rx.await` is unbounded.** If the TUI subscriber hangs (e.g. broken pipe to redirected stdout in `atlas index . --replay-from-cache | head`), `replay_from_cache` blocks forever and the CLI hangs. PR-7 should either (a) treat persistent draw `io::Error` as a `break 'outer` condition in the TUI select loop, or (b) wrap `done_rx.await` with `tokio::time::timeout(Duration::from_secs(30), ...)`. Source: `crates/atlas-cli/src/replay.rs:166`.
- **MEDIUM-2: `canonicalize` called before suffix-filter in replay walker.** A broken-symlink `.meta.json` file produces a misleading `ReplayError::PathTraversal` message when the real cause is an unrelated IO error. Fix: reorder to skip non-transcript entries (`strip_suffix(PUB_TRANSCRIPT_SUFFIX)` check) before calling `canonicalize`, or map the canonicalize error to `ReplayError::Io` for non-traversal failures. Source: `crates/atlas-cli/src/replay.rs:225`.
- **MEDIUM-3: TUI `select!` event-per-tick latency.** The select processes exactly one event per 50ms tick. On a large replay burst (200+ agents) this serialises to ~10s of busy spinning while the TUI lags behind reality. Fix: drain all immediately-ready events in the event arm via `while let Ok(ev) = rx.try_recv()` inside the sleep tick, or switch to `biased` select with the event arm first. Source: `crates/atlas-cli/src/tui/mod.rs:142`. Not load-bearing for PR-6 (TUI unwired from live `atlas index`) but worth fixing before PR-7 lights up the live path.
- **LOW awareness:** `atlas index . --replay-from-cache --no-tui` silently ignores `--no-tui` (replay arm short-circuits before consulting `no_tui`). Not a bug in PR-6 since the flag is dormant until PR-7, but a future footgun. Either error out on the conflict or route `--no-tui` to a TUI-less replay subscriber.

**Convergent design choices accepted across PR-6:**
- Used `atlas_index::Stage` (L1..L9 engine cache layout) for replay rather than PR-4's logical `Stage` enum (DispatchSubsystem..Project per recast §6.1). These are different axes — engine cache stages vs logical agent stages — and PR-7 will reconcile if it proves useful. Documented in the file header of `crates/atlas-cli/src/replay.rs`.
- Single-transport invariant (plan §7.4) enforced via a sibling `.meta.json` file carrying `transport_flavour`; on mismatch with the requested transport, `ReplayError::TransportMismatch` is returned **before** any events are emitted on the bus (so the bus is clean — subscribers don't see a partial sequence).
- `TerminalGuard` RAII newtype (`crates/atlas-cli/src/tui/mod.rs`) — strictly better than the spec pseudocode's inline `disable_raw_mode` because raw-mode cleanup runs on panic as well as normal exit.
- `TuiSnapshot` excludes `Instant`-typed fields from serde round-trip, making the byte-equality assertion deterministic across live vs replay (the spec's load-bearing acceptance criterion).

**Forward-pointers to PR-7 (end-to-end wiring + closeout):**
- The TUI is NOT wired into `atlas index`. PR-6 leaves an explicit `// TODO(PR-7): wire the LLM-spine AgentRuntime here. PR-6 keeps atlas index on the deterministic dispatcher; the TUI subscriber is reachable only through --replay-from-cache above.` marker at `crates/atlas-cli/src/main.rs:171-176`.
- TUI activation logic for non-replay `atlas index` invocations is PR-7's call: use `crossterm::tty::IsTty::is_tty(&std::io::stdout())` AND `!args.no_tui` to decide TUI vs JSON-Lines fallback. `--log-events PATH` is always-parallel regardless of mode.
- The three deferred MEDIUMs (drain timeout, canonicalize ordering, event-per-tick latency) should be addressed in PR-7 before lighting up the live runtime, OR captured as Phase 8+ follow-on items if PR-7 scope is already at the brief's "stop and surface" threshold.

**Test count deltas in PR-6:**
- `atlas-cli` lib: +21 unit tests (TuiState apply/note_lag/snapshot, four widget render unit tests, ReplayError variants, TerminalGuard drop test, CLI flag parse tests).
- `atlas-cli` integration: +3 in `crates/atlas-cli/tests/replay.rs` (`replay_snapshot_matches_synthetic_live_snapshot`, `transport_mismatch_error_path`, `no_cache_error_path`).
- Workspace total: +24 PR-6-attributable tests. Cumulative regression guard unchanged.

**Rebase note:** PR-6 was implemented off the same base as PR-4 (`3259bdd`). After PR-4 fast-forwarded to main at `7f61e94`/`a006955`, PR-6 was rebased onto current main and the two PR-6 commits replayed cleanly — no file conflicts (PR-4 and PR-6 touched disjoint file sets per the wave-design intent) and only a trivial `Cargo.lock` regeneration. Post-rebase: `9618040` + `f2ce6d5`. Pre-rebase SHAs (`d1d9039`, `267c60d`) survive in the worktree's reflog for forensic interest but the canonical lineage on main is the rebased pair.

### PR-7

2026-05-12 — Landed across four code commits, all fast-forwarded onto main. Plan §4 Task 7 Steps 7.1, 7.2, 7.3, 7.5, 7.7, 7.8 fully shipped; Steps 7.4 (Atlas-on-Atlas calibration) and 7.6 (subprocess `--disallowedTools` probe) explicitly DEFERRED with documented rationale and follow-up pointers.

**Code commits:**
- `b83a49e` — deferred MEDIUMs from PR-5/PR-6 + AWARENESS-A positive Lane B test. Adds `AgentError::LlmOutputMalformed(String)` variant; switches the two `parse_*_from_output_value` map_err sites in `dispatch.rs` from `OverrideRequired` (semantically wrong for LLM output parse failures) to `LlmOutputMalformed`. Hoists `now_iso` to `pub(super) fn` in `runtime/mod.rs` and deletes the duplicate in `dispatch.rs`. Replay drain `done_rx.await` wrapped in `tokio::time::timeout(30s)` (PR-6 MEDIUM-1). Canonicalize moved AFTER suffix-filter in replay walker (PR-6 MEDIUM-2). TUI `select!` event arm now drains all immediately-ready events via `rx.try_recv()` before yielding to the sleep tick (PR-6 MEDIUM-3). Two new positive-assertion tests in `tests/audit_lane_b.rs` (`lane_b_audit_fires_audit_fire_event_on_weak_grade` + `lane_b_audit_fires_audit_fire_event_on_declines_grade`) — the AWARENESS-A complement to the negative `lane_b_wired_into_call_agent_skips_on_strong_grade`. 5 files, +176/-25 LOC.
- `88cbad7` — wire AgentRuntime into atlas index via single `block_on`. New `pipeline::run_index_agent_runtime` (~205 LOC) gated behind a new `--agent-runtime` CLI flag (default false). Opens `PersistentCache` at `<output_dir>/cache/`; constructs `EventBus`; spawns subscribers (always agent_cache_writer; conditionally `--log-events PATH` JSON-Lines; then TUI when stdout is a TTY AND !`--no-tui` else JSON-Lines-to-stdout); builds `AgentRuntime` with `for_provider: None` (Lane B falls back to same-model auditor with `AuditDegraded`); runs `tokio_rt.block_on(runtime.run_workspace(&workspace))` as the SINGLE sync→async boundary; joins each subscriber's handle (drain handshake); serialises the returned `L9Projection` to `<output_dir>/cache/agent-runtime-projection.json`. 3 files, +312/-6 LOC.
- `5473abc` — cross-transport parity test in polyglot smoke. New `polyglot_smoke_cross_transport_parity_claude_code_vs_codex`: materialises the fixture twice, runs the deterministic engine pipeline with two `LabeledTransportBackend` instances (`pr13-test-backend-claude-code` vs `pr13-test-backend-codex`), asserts cold-call counts match, component_id sets match, edge_kind multisets match. 1 file, +267/-0 LOC.
- `0820844` — memory updates + status closeout note + Phase 7 final summary (implementer's fourth code commit, written before the orchestrator's status-flip).

**Decisions taken (vs deferred) — Step-by-step:**

- ✅ **Step 7.1 Verify polyglot fixture has full override coverage** — Both `subsystems.overrides.yaml` and `components.overrides.yaml` present at `crates/atlas-cli/tests/fixtures/phase3_polyglot/.atlas/`; cold count remains in loose bound `0 < cold < 100` (~40 calibrated). No mutations required.
- ✅ **Step 7.2 Wire AgentRuntime into atlas index** — Implemented as opt-in `--agent-runtime` flag rather than as default `run_index_cmd` path. Rationale: the production prompt templates (`PR-7-WIRES-REAL-PROMPT` stubs in dispatch.rs; `PR-7-WIRES-REAL-AUDITOR` stub in mod.rs) are NOT replaced in PR-7. Flipping the binary's default would break `atlas index` for real users. The wiring is the load-bearing PR-7 deliverable; production prompts are a follow-up sprint.
- ✅ **Step 7.3 Cross-transport parity test** — New test in `tests/phase3_polyglot_fixture.rs` passes alongside the existing cumulative regression guard. Asserts cold-call count parity, component_id set parity, edge_kind multiset parity.
- ⚠️ **Step 7.4 Atlas-on-Atlas baseline calibration — DEFERRED.** The brief's invocation (`cargo run --release --package atlas-cli -- index --workspace-root .`) requires three preconditions not yet met: (a) `<atlas-root>/.atlas/config.yaml` populated with a real claude_code + codex backend pair, (b) production dispatch prompt templates that emit valid JSON envelopes (`SubsystemsOverrideFile` / `ComponentsOverrideFile` shape), (c) Lane B auditor closure wired through to a cross-provider audit prompt. Each preconditions is a focused engineering item; together they exceed the brief's "stop and surface at 1 hour" threshold. Follow-up sprint should: (1) author the production dispatch + classify + reduce + project prompt templates, (2) replace the `PR-7-WIRES-REAL-AUDITOR` stub with a real audit prompt round-trip, (3) wire per-provider `for_provider` closure to the `BackendRouter` (un-gate `from_dispatch_table` or add `BackendRouter::backend_for_provider`), (4) run the calibration and record baseline numbers in this status file.
- ✅ **Step 7.5 Final verification suite** — All six cargo gates clean. See "Cargo gates" section below.
- ⚠️ **Step 7.6 Subprocess `--disallowedTools` probe — DEFERRED.** PR-7 has no production code path that drives a live subprocess + MCP server end-to-end. The runtime's subprocess-transport branch in `tool_loop_http.rs` returns a clean error ("PR-4 runtime does not drive subprocess transports directly; PR-7 wires the MCP `serve_client` task"). Authoring the live probe requires: (a) implementing the subprocess MCP `serve_client` task driver, (b) running an actual `claude-code` or `codex` subprocess with the Atlas-internal MCP server attached over stdio, (c) provoking a `Read` tool call and asserting the upstream's "tool not available" error shape. All three are non-trivial. Follow-up sprint should ship this probe as a `crates/atlas-agents/tests/mcp_disallowed_tools.rs` integration test once the subprocess driver lands.
- ✅ **Step 7.7 Update memory** — `.claude/memory/project_phase4_plus_roadmap.md` updated: Phase 7 SHIPPED 2026-05-12 with explicit "calibration-pending follow-up" framing; Phase 8 (Cargo retirement) now next-up. `.claude/memory/MEMORY.md` roadmap hook line refreshed.
- ✅ **Step 7.8 Append closeout note** — *(this section)*.

**Deferred MEDIUMs from PR-5 / PR-6 reviews — all addressed in `b83a49e`:**
- ✅ PR-5 MEDIUM-2: `OverrideRequired` misuse for LLM output parse failures → `LlmOutputMalformed(String)` variant added; both map_err sites updated.
- ✅ PR-5 MEDIUM-3: `now_iso` duplication → hoisted to `pub(super) fn` in `runtime/mod.rs`; dispatch.rs's copy deleted.
- ✅ PR-5 AWARENESS-A: positive-assertion Lane B test → two new tests in `tests/audit_lane_b.rs` cover Weak and Declines grades.
- ✅ PR-6 MEDIUM-1: replay drain `done_rx.await` unbounded → wrapped in 30s timeout.
- ✅ PR-6 MEDIUM-2: canonicalize before suffix-filter → reordered to filter first.
- ✅ PR-6 MEDIUM-3: TUI event-per-tick latency → event arm drains all immediately-ready events via `rx.try_recv()`.

**Deferred from PR-4 / PR-5 reviews — not addressed in PR-7:**
- PR-4 MEDIUM-1 (semaphore acquisition order — document fixed order + consider merged acquire helper) and PR-4 MEDIUM-2 (Lane A retry `call_count == 2` assertion) carry forward to the production-prompt follow-up sprint. Neither is load-bearing for the PR-7 wiring deliverable.

**Acceptance gates met (PR-7 implementer-side verification on main post-`5473abc`):**
- `cargo build --workspace` clean
- `cargo fmt --check` clean
- `cargo clippy --all-targets -- -D warnings` clean
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` — all green workspace-wide; `atlas-agents` lib 101 passed; `atlas-cli` lib 95 passed; `audit_lane_b` integration 7 passed (+2 PR-7); `replay` integration 3 passed
- `cargo build --release --workspace` clean
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — **2 tests passed in 108.71s**: `polyglot_smoke_cross_transport_parity_claude_code_vs_codex` (new; PR-7) AND `polyglot_phase3_acceptance` (cumulative regression guard; cold count in loose bound `0 < cold < 100`)

**Convergent design choices accepted across PR-7:**
- **Default-false `--agent-runtime` flag (vs. default-replacement of `run_index_cmd`):** documented above. The PR-7 wiring deliverable is structural; the binary's default behavior remains the deterministic engine pipeline until production prompts are validated.
- **`for_provider: None` MVP for Lane B:** Lane B emits `AuditDegraded` and falls back to the same-model auditor. The `Option<Arc<ForProviderFn>>` closure shape is preserved so a follow-up can plug in a `BackendRouter`-backed lookup additively.
- **Projection write target:** `<output_dir>/cache/agent-runtime-projection.json` (JSON, not the engine's canonical YAMLs). The deterministic pipeline's `components.yaml` / `related-components.yaml` / `subsystems.yaml` are NOT written by the `--agent-runtime` path. A projection-to-ontology shim ships with the production-prompt sprint.

**LOC totals (PR-7 lineage):**
- `b83a49e`: +176 / -25 (5 files; deferred MEDIUMs + AWARENESS-A)
- `88cbad7`: +312 / -6 (3 files; AgentRuntime wiring)
- `5473abc`: +267 / -0 (1 file; cross-transport parity test)
- `0820844`: +86 / -6 (3 files; closeout note + memory)
- Net PR-7 lineage: +841 / -37 LOC.

**Two-stage review — orchestrator-side (subagent-driven-development skill):**

- **Spec compliance review** verdict: **COMPLIANT WITH ACCEPTABLE DEFERRALS**. No blocking issues. All 5 deferred MEDIUMs from PR-5 / PR-6 reviews verified closed. Two deviations flagged for user ratification (not code fixes): (A) `--agent-runtime` gated default-false vs spec text "atlas index runs end-to-end through AgentRuntime"; (B) cross-transport parity test exercises the deterministic engine path with two transport labels rather than `AgentRuntime` through two real transports (B is a consequence of A). The implementer's rationale on A is sound — production prompts aren't ready; flipping the default would break `atlas index` for real users — and the spec acceptance text "atlas index runs end-to-end through AgentRuntime via single Handle::block_on" is ambiguous about whether the default must be on. User adjudicates.
- **Code quality review** verdict: **APPROVED WITH MINORS**. No HIGH issues. Three MEDIUMs all deferrable per the reviewer's recommendation: (1) drain-handshake-no-timeout in `run_index_agent_runtime` — symmetric to PR-6's `replay.rs` 30s timeout fix; should land in the first Phase 8 PR; (2) `now_iso()` returns epoch-integer string, not actual ISO-8601 — cosmetic naming mismatch; rename or fix when MCP subprocess wiring lands; (3) `default_transport: ClaudeCode` + canonical Atlas backend = `atlas index --agent-runtime` immediately hard-errors at first `call_agent` against the canonical user config — see "User-visible note" below.

**Orchestrator-side independent verification (post-`0820844`, 2026-05-13):**

- `cargo fmt --check` clean
- `cargo build --workspace` clean (4.50s)
- `cargo clippy --all-targets -- -D warnings` clean (8.14s)
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` exit 0; zero failures grep'd
- `cargo build --release --workspace` clean (3.81s)
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` — **2 passed in 106.81s** (`polyglot_phase3_acceptance` + `polyglot_smoke_cross_transport_parity_claude_code_vs_codex`). Cumulative regression guard held; cold count stays in loose bound `0 < cold < 100`. Timing envelope matches PR-5 (104.32s) and PR-6 (109.34s) — the LLM-dispatch site PR-5 introduced remains unreachable from the polyglot smoke because the fixture has full override coverage (the load-bearing protection per plan §2.2 non-negotiable #8).

**User-visible note for the post-shipping ratification check (code-quality MEDIUM-3):**

The `--agent-runtime` flag, as wired in PR-7, will **immediately hard-error** when invoked against the canonical Atlas backend configuration (`claude_code` + `codex` per memory `project_atlas_common_backend_config`). The runtime's `run_tool_loop_with_lane_a` returns `Err(AgentError::Backend("PR-4 runtime does not drive subprocess transports directly; PR-7 wires the MCP serve_client task"))` for `TransportFlavour::ClaudeCode | Codex`. This is intentional — subprocess MCP `serve_client` is item #5 of the Phase 7 → Phase 8 handoff prerequisites — but it means an early adopter who follows the canonical Atlas setup and tries `atlas index --agent-runtime` today will see an immediate failure rather than a degraded-but-functional partial run. The HTTP transports (`http_anthropic` / `http_openai`) work fully. Two remediation options for the production-prompt sprint: (a) a CLI pre-flight check that detects subprocess backends + emits a clear "subprocess wiring is a follow-up" error before entering the runtime loop, or (b) ship the subprocess MCP `serve_client` driver (prerequisite item #5) so `--agent-runtime` works against the canonical config out-of-box. Option (b) is the proper fix; option (a) is the one-week mitigation.

---

## Phase 7 — complete

2026-05-12. All eight PRs merged to main:

- **PR-0** (plan + status + continuation prompt; docs only): `ddf553b` + status flip `4d7e75c`
- **PR-1** (atlas-agents + Tool trait + MCP server + async LlmBackend): `0ec69f3` + status flip
- **PR-2** (transcript cache + event bus + JSON-Lines subscriber): `faa5fd9` + `4123011` + `87a193c` + status flip
- **PR-3** (22 tool wrappers via three parallel subagents): `6317da6` + `420d6bf` + `eb6a19a` + `87ffacf` + `e5a74c8` + `7a11a8b` + `b09919c` + `fed0da2` + `b6f4b3d` + status flip
- **PR-4** (agent runtime single-iteration + Lane A): `80dac2f` + `3a5c986` + `7f61e94` + status flip `a006955`
- **PR-5** (fixed-point + LLM dispatch + Lane B): `12bbbec` + `7f51393` + `7ec3da7` + status flip `888ef13`
- **PR-6** (TUI + replay-from-cache): `9618040` + `f2ce6d5` + status flip `6c326e1`
- **PR-7** (end-to-end wiring + cross-transport parity + closeout): `b83a49e` + `88cbad7` + `5473abc` + `0820844` + status flip *(this commit; SHA at HEAD post-merge)*

**Polyglot smoke cumulative regression guard:** cold = ~40 LLM calls (calibrated codebase baseline since Phase 6 PR-5; loose-bound `0 < cold < 100`); warm + reports = 0; cross-transport parity (claude_code vs codex labels) holds. No drift across the seven code PRs.

**Atlas-on-Atlas cold token total baseline:** DEFERRED (Step 7.4). The Atlas-on-Atlas baseline number is the regression detector for future Phase 7+ changes; it is RECORDED IN A FOLLOW-UP SPRINT once production prompt templates ship. Tracked as the headline follow-up item.

### Phase 7 → Phase 8 handoff

Phase 7 ships the LLM-spine runtime *wiring*; no language analyser retires. Phase 8 (Cargo retirement, recast §11.2) is next — the Cargo classifier retires first because it has the cleanest deterministic baseline (Phase 6 PR-3 overlay discipline; well-bounded manifest parsing) for empirical cold-token-budget calibration.

**Phase 8 prerequisites (the production-prompt sprint blocks Phase 8):**

1. **Production dispatch prompt templates** — replace `PR-7-WIRES-REAL-PROMPT` stubs in `crates/atlas-agents/src/runtime/dispatch.rs::build_dispatch_subsystems_prompt` + `build_dispatch_components_prompt` with real prompt templates that consume the workspace's L1 candidates + ontology + per-component file listing. JSON envelope shape (`SubsystemsOverrideFile` / `ComponentsOverrideFile`) is locked.
2. **Production classify / reduce / project prompts** — replace `build_classify_prompt` and `build_reduce_prompt` (~lines 910 / 920 of `runtime/mod.rs`) with the canonical templates. Output must satisfy `lane_a_validate` for each stage; the existing four-stage Lane A check pins the shape.
3. **Cross-provider auditor wiring** — replace `PR-7-WIRES-REAL-AUDITOR` stub closure in `runtime/mod.rs::call_agent` with a real audit-prompt round-trip. The closure shape is `|auditor_backend| async { ... auditor_backend.call_async(audit_request).await ... }`.
4. **`for_provider` closure wired through `BackendRouter`** — either un-gate `BackendRouter::from_dispatch_table` from `#[cfg(test)]` so production code can construct one, or add `BackendRouter::backend_for_provider(provider: Provider) -> Option<&Arc<dyn LlmBackend>>` and pass an `Arc::new(move |p| router.backend_for_provider(p).cloned())` closure into the runtime.
5. **Subprocess MCP `serve_client` driver** — the runtime's `tool_loop_http.rs` subprocess-transport branch currently errors out; the production driver spawns `claude-code` / `codex` subprocesses with the Atlas-internal MCP server attached over stdio per brainstorm §4.
6. **Atlas-on-Atlas calibration (Step 7.4)** — once 1–5 land, run `atlas index --agent-runtime --workspace-root . --log-events /tmp/atlas-on-atlas-events.jsonl` against the Atlas repo itself; record cold token total + per-provider split + wall time + iteration count; update this status file's "Atlas-on-Atlas cold token total baseline" line above.
7. **Subprocess `--disallowedTools` probe (Step 7.6)** — once item 5 lands, author `crates/atlas-agents/tests/mcp_disallowed_tools.rs` that drives a live subprocess + MCP server, provokes a `Read` tool call, and asserts the upstream's "tool not available" error.

Brainstorm Phase 8 via `superpowers:brainstorming` only after items 1–4 are validated (the production prompt + cross-provider audit are PR-8 prerequisites; the subprocess driver and live probe can ship in parallel with PR-8 work).
