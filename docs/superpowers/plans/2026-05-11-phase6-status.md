# Atlas vNext Phase 6 — Status

Companion to `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-11-vnext-continue.md` (Phase-6-shaped) reads this file (via the `*phase6-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-11 (PR-1 deferred to Phase 9c on polyglot-fixture pre-flight finding).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [-] PR-1 — `is_manifest_file` Makefile/shell extension — **DEFERRED to Phase 9c 2026-05-11** (polyglot fixture pre-flight found `build_glue/Makefile` + `scripts/deploy.sh`; recognition-only would break the cumulative regression guard's cold count)
- [ ] PR-2 — Contract rename-match owner-follows (medium)
- [ ] PR-3 — `subsystem` field overlay (medium)
- [ ] PR-4 — `--strict-overrides` + closed enum + dual-mode contract test (medium)
- [ ] PR-5 — Acceptance + closeout + canonical §10/§4.3/§7/§8 retext (docs + verification)

When PR-2, PR-3, PR-4, and PR-5 are all `[x]` (PR-1 deferred), Phase 6 is complete and the continuation prompt should report success and route to brainstorm/plan for Phase 7 (LLM-spine runtime per canonical §10.7, recast spec §11.1).

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

**Parallel-safe waves (post-PR-1 deferral):**

- **Wave 0:** PR-0 (landed) + this PR-1-deferral commit.
- **Wave 1 (after PR-0):** PR-2 and PR-3 dispatched in parallel — disjoint code surfaces. (PR-1 was originally in this wave; deferred to Phase 9c 2026-05-11.) Use `superpowers:dispatching-parallel-agents`; verify each worktree's base commit matches current main before subagent proceeds (memory `feedback_worktree_base_verification`).
- **Wave 2 (after Wave 1):** PR-4 alone. Depends on PR-3's `SubsystemOverrideNonExistent` warning class for the closed-enumeration list; also touches `l6_edges.rs:244-248,305-308` (a different region than PR-2's edits but same file, so trivial conflicts possible if PR-2 hasn't landed).
- **Wave 3 (after PR-4):** PR-5 — acceptance + closeout + canonical retext. PR-5 §10.6 narrative records PR-1's deferral to Phase 9c.

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard. Each PR re-runs it; cold = ~26 calls; warm + reports = 0.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; α/β implementation decisions confirmed; reference-output comparisons; cross-cutting refactor surfaces; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0
2026-05-11 — Landed: the Phase 6 plan, this status file, and the continuation prompt. Plan: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-11-vnext-continue.md`. LLM-spine recast spec (design anchor for Phase 6's PR-5 retext): `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` on main as `409dcc5`. Phase 6 is the **final deterministic-spine release** before the LLM-spine recast begins in Phase 7. Pre-pivot brainstorm memory artifacts (`feedback_atlas_llm_spine_intent.md`, `project_phase6_paused_for_llm_spine.md`) committed forensically alongside this PR-0.

### PR-1 — DEFERRED to Phase 9c
2026-05-11 — At PR-1 pre-flight, the polyglot fixture was found to contain two files the plan §1 assumed absent: `crates/atlas-cli/tests/fixtures/phase3_polyglot/build_glue/Makefile` and `crates/atlas-cli/tests/fixtures/phase3_polyglot/scripts/deploy.sh`. Both are surfaced today via `.atlas/components.overrides.yaml` `additions:` entries (`shell-scripts` at `scripts/`, `makefile` at `build_glue/`) — explicitly because `manifest_patterns::is_manifest_file` does *not* recognise them. Landing PR-1's "recognition-only, no paired classifier" change would (a) cause L1 to auto-discover candidates at `scripts/` and `build_glue/`, falling through L3 to `LlmClassify` and raising the cumulative regression guard's strict cold-count assertion from ~26 to ~28; (b) collide auto-discovered components with the existing `additions:` entries at the same paths. The plan's Step 1.1 explicitly mandated STOP on this discovery. Recognition + paired classifier ship together in Phase 9c per recast spec §11.3. No code change for Phase 6; PR-5's §10.6 retext narrative records the deferral.
