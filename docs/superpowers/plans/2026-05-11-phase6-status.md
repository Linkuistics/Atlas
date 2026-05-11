# Atlas vNext Phase 6 — Status

Companion to `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-11-vnext-continue.md` (Phase-6-shaped) reads this file (via the `*phase6-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-11 (PR-1 deferred to Phase 9c on polyglot-fixture pre-flight finding).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [-] PR-1 — `is_manifest_file` Makefile/shell extension — **DEFERRED to Phase 9c 2026-05-11** (polyglot fixture pre-flight found `build_glue/Makefile` + `scripts/deploy.sh`; recognition-only would break the cumulative regression guard's cold count)
- [x] PR-2 — Contract rename-match owner-follows (medium)
- [x] PR-3 — `subsystem` field overlay (medium)
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

### PR-2
2026-05-11 — Commits `752b469` (PR-2 contract rename-match owner-follows) + `641fa33` (PR-2 follow-up — fold `_for_tests` shim into production helper) on main via fast-forward merge of `phase6-pr2`. α implementation (id-embeds-owner): contracts whose id starts with `prior_id/` get their prefix rewritten to `new_id/` when rename-match maps `prior_id → new_id`. Both surfaces.yaml (in `l9_projections::surfaces_yaml_snapshot` via `rewrite_contract_owner_prefix`) and related-components.yaml (in `l6_edges` via `apply_contract_owner_follows_to_edge_participants` at all three `all_proposed_edges` branches) updated. New Salsa query `rename_map_after_match(db) -> RenameMap` co-located with rename-map construction in `l4_tree::resolve_ids_and_tombstones`. Integration test `crates/atlas-cli/tests/contract_rename_owner_follows.rs` exercises rename → expected surfaces + edges. Plan deviation: seam landed in L4/L6/L9 (not L5; L5 was not the actual write site). All cargo gates clean; polyglot cold = 40 (calibrated codebase baseline; plan's "~26" was wrong — no drift caused by PR-2). β content-sha-stable + independent fuzzy matching remain deferred to Phase 10 per recast spec §11.4. Closes §11.2.4 canonical-design open question.

### PR-3
2026-05-11 — Commits `43c2650` (PR-3 subsystem field overlay; rebased onto post-PR-2 main, was `d917c7e` pre-rebase) + `c0ca23f` (PR-3 follow-up — extract `NOTE_ALL_UNRESOLVED` + walk-consolidation TODO; rebased, was `6a7aa7f` pre-rebase) on main via fast-forward merge of `phase6-pr3`. Per-component `subsystem:` override now wins over central `subsystems.overrides.yaml` (closer-to-source authoring; recast spec §4.1). The no-op `let _ = fo.subsystem.as_ref();` in `l4_tree.rs` retired; new query `per_component_subsystem_overrides(db) -> BTreeMap<ComponentId, String>` extracts the override map; `resolve_subsystems` signature extended with `per_component_overrides: &BTreeMap<ComponentId, String>` + `warnings: &mut dyn Write`; central yaml referencing nonexistent component emits new warning `"...does not exist (no extant component)..."` via `writeln!` (PR-4 will refactor to typed `OverrideWarning::SubsystemOverrideNonExistent`). New `IndexConfig.warnings_buffer: Option<Arc<Mutex<Vec<u8>>>>` + `WarningSink` adapter routes warnings to either a test buffer or stderr; `check_subsystem_id_members` hard-error call retired from `pipeline.rs` but the function retained (PR-4 will reuse). Integration test `crates/atlas-cli/tests/subsystem_overlay.rs` exercises all three cases (silent-central, per-component-wins, nonexistent-emits-warning). Rebase resolved one trivial conflict in `atlas-engine/src/lib.rs` (both PRs added a new export to the `pub use l4_tree::{...}` block). All cargo gates clean; polyglot cold = 40 (no drift). Plan deviation: empty-subsystem cleanup pseudo-code from plan §3.6 dropped — pre-existing tests audit the `"all members unresolved"` note shape and the audit-trail observability is preserved by keeping empty entries with the note re-applied. Closes the Phase 3 PR-9 deferral noted in canonical §10.6.
