# Atlas vNext Phase 7 — Status

Companion to `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-12-vnext-continue.md` (Phase-7-shaped) reads this file (via the `*phase7-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-12 (PR-2 landed: transcript cache + event bus + JSON-Lines subscriber).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — `atlas-agents` crate + `Tool` trait + MCP server + async `LlmBackend` (large)
- [x] PR-2 — Transcript cache + event bus + JSON-Lines subscriber (medium)
- [ ] PR-3 — 26 tool wrappers across three parallel subagents (medium)
- [ ] PR-4 — Agent runtime (single-iteration) + Lane A schema validation (large)
- [ ] PR-5 — Fixed-point iteration + LLM-decided dispatch + Lane B cross-provider audit (large)
- [ ] PR-6 — `ratatui` TUI subscriber + `--replay-from-cache` mode (medium)
- [ ] PR-7 — End-to-end wiring + polyglot smoke extension + Atlas-on-Atlas calibration + closeout (large)

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
*(populated when PR-3 lands)*

### PR-4
*(populated when PR-4 lands)*

### PR-5
*(populated when PR-5 lands)*

### PR-6
*(populated when PR-6 lands)*

### PR-7
*(populated when PR-7 lands)*

---

## Phase 7 — complete

*(populated when all eight PRs are `[x]`; the PR-7 implementer appends a Phase 7 closeout note here with cumulative LOC, Atlas-on-Atlas baseline numbers, list of plan-time decisions taken vs deferred, and the Phase 7 → Phase 8 handoff per plan §4 Task 7 Step 7.8.)*
