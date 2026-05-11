# Atlas vNext Phase 6 — Status

Companion to `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-11-vnext-continue.md` (Phase-6-shaped) reads this file (via the `*phase6-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-11 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [ ] PR-1 — `is_manifest_file` Makefile/shell extension (small)
- [ ] PR-2 — Contract rename-match owner-follows (medium)
- [ ] PR-3 — `subsystem` field overlay (medium)
- [ ] PR-4 — `--strict-overrides` + closed enum + dual-mode contract test (medium)
- [ ] PR-5 — Acceptance + closeout + canonical §10/§4.3/§7/§8 retext (docs + verification)

When every box is `[x]`, Phase 6 is complete and the continuation prompt should report success and route to brainstorm/plan for Phase 7 (LLM-spine runtime per canonical §10.7, recast spec §11.1).

## Dependency graph (informational; canonical in plan §3)

```
PR-0 ──► PR-1 ─┐
       │       │
       ├──► PR-2 ─┤
       │       │
       └──► PR-3 ──► PR-4 ──► PR-5
                            ▲
                            │
        (PR-1, PR-2 join here too)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (this commit).
- **Wave 1 (after PR-0):** PR-1, PR-2, PR-3 dispatched in parallel — disjoint code surfaces. Use `superpowers:dispatching-parallel-agents`; verify each worktree's base commit matches current main before subagent proceeds (memory `feedback_worktree_base_verification`).
- **Wave 2 (after Wave 1):** PR-4 alone. Depends on PR-3's `SubsystemOverrideNonExistent` warning class for the closed-enumeration list; also touches `l6_edges.rs:244-248,305-308` (a different region than PR-2's edits but same file, so trivial conflicts possible if PR-2 hasn't landed).
- **Wave 3 (after PR-4):** PR-5 — acceptance + closeout + canonical retext.

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard. Each PR re-runs it; cold = ~26 calls; warm + reports = 0. **PR-1 LLM-call risk caveat:** adding `.sh` to manifest recognition without a paired classifier means future workspaces with shell scripts produce LlmClassify fallback calls; the polyglot fixture has no .sh/.mk files so cold count is unchanged for the cumulative regression guard.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; α/β implementation decisions confirmed; reference-output comparisons; cross-cutting refactor surfaces; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0
2026-05-11 — Landed: the Phase 6 plan, this status file, and the continuation prompt. Plan: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-11-vnext-continue.md`. LLM-spine recast spec (design anchor for Phase 6's PR-5 retext): `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` on main as `409dcc5`. Phase 6 is the **final deterministic-spine release** before the LLM-spine recast begins in Phase 7. Pre-pivot brainstorm memory artifacts (`feedback_atlas_llm_spine_intent.md`, `project_phase6_paused_for_llm_spine.md`) committed forensically alongside this PR-0.
