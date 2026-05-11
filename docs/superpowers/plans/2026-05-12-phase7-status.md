# Atlas vNext Phase 7 — Status

Companion to `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-12-vnext-continue.md` (Phase-7-shaped) reads this file (via the `*phase7-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-12 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [ ] PR-1 — `atlas-agents` crate + `Tool` trait + MCP server + async `LlmBackend` (large)
- [ ] PR-2 — Transcript cache + event bus + JSON-Lines subscriber (medium)
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

2026-05-12 — Landed: the Phase 7 plan, this status file, and the continuation prompt. Plan: `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-12-vnext-continue.md`. Design anchor: the Phase 7 brainstorm at `docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md` (commit `f4ea770` on main; the 14-item decision table is locked). Parent design spec: the LLM-spine recast at `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` (commit `409dcc5`). Phase 6 SHIPPED 2026-05-11 (final commit `9350735`) as the final deterministic-spine release; the canonical §4.3/§7/§8/§10 retext landed in Phase 6 PR-5 — **Phase 7 itself does not retext canon** (Phase 6 already did). Commit: `<PR-0-COMMIT-SHA>`.

Key PR-0 design call-outs to surface to PR-1+:

1. **`AgentEvent` naming collision** between today's `atlas-llm::AgentEvent` (backend-level streaming-call observer) and PR-2's new `atlas-agents::AgentEvent` (agent-level runtime event log). **PR-1 must rename** the former to `BackendCallEvent` (and `AgentObserver` to `BackendCallObserver`) in `crates/atlas-llm/src/agent_observer.rs` (file renamed to `backend_call_observer.rs`) before PR-2 can land its `AgentEvent` enum. See plan §4 Task 1 Step 1.3.

2. **`tokio` + `async-trait` are new workspace dependencies.** PR-1 adds them under `[workspace.dependencies]` in the root `Cargo.toml`. PR-1 also pre-adds `ratatui` + `crossterm` (no-cost when unused; PR-6 picks them up). Path-deps carry path only, no `version` field (memory `feedback_no_version_on_workspace_path_deps`).

3. **`crates/atlas-engine/src/override_warnings.rs` already exists** (Phase 6 PR-4). Plan refers to it as a *modify*, never a *create*. Same for `crates/atlas-engine/src/atomic_write.rs` (PR-2 adds `atomic_write_pair` alongside today's `atomic_write`).

4. **Wrapper count is 27 (not 26).** Brainstorm §5 cited "26 wrappers" but enumerated 27 modules across PR-3a/3b/3c (9 manifests + 4 mature + 6 mid-tier + 8 weak-tooling = 27). Plan §4 Task 3 locks 27.

5. **PR-1 and PR-5 are the scope-creep risks** (plan §7.6, §7.7). PR-1 stacks: new crate + `Tool` trait + MCP multi-client server + async `LlmBackend` for 5 backends + rename + 4 new deps. PR-5 stacks: fixed-point + LLM dispatch + Lane B. Either subagent should stop-and-surface if it hits >2x its time/LOC estimate at any checkpoint.

### PR-1
*(populated when PR-1 lands)*

### PR-2
*(populated when PR-2 lands)*

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
