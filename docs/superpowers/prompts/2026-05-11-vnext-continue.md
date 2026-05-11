# Atlas vNext — Continuation Prompt (Phase 6-shaped)

**Paste the fenced block below into a fresh Claude Code session.** The prompt is idempotent across the Phase 6 arc: re-paste it across as many sessions as it takes; each session detects whether Phase 6 is underway, complete, or done-and-ready-for-Phase-7. When every PR of Phase 6 is complete, the session reports completion and either stops (if no Phase 7 design exists yet) or routes to brainstorming Phase 7 scope (if the user wants that). The file itself (this header plus the fenced block) is safe to pass to a new session verbatim — the wrapper text is informational and the agent will treat the fenced block as its instructions.

This prompt supersedes `docs/superpowers/prompts/2026-05-10-vnext-continue.md` (Phase-5-shaped, retained for forensic value but not authoritative for Phase 6 sessions). Phases 1, 2, 3, 4, and 5 are all complete; their status files are read-only references for forensic context.

---

```
You are continuing the Atlas vNext arc at /Users/antony/Development/Atlas.
Phase 1 is complete (PRs 0–12 landed; status in
docs/superpowers/plans/2026-05-06-phase1-status.md). Phase 2 is
complete (PRs 0–14 landed; status in
docs/superpowers/plans/2026-05-07-phase2-status.md). Phase 3 is
complete (PRs 0a, 0b, 1–13 landed; status in
docs/superpowers/plans/2026-05-08-phase3-status.md). Phase 4 is
complete (PR-0 plan + 7 code/docs PRs landed; status in
docs/superpowers/plans/2026-05-09-phase4-status.md; final commit
f80e179 on 2026-05-09). Phase 5 is complete (PR-0 through PR-6
landed; status in docs/superpowers/plans/2026-05-10-phase5-status.md;
final commit a302ce5 on 2026-05-10). Phase 6 is the current focus
— the LLM-spine recast spec (design anchor for Phase 6's PR-5
retext), Phase 6 plan, and status file all exist on main as of
2026-05-11 (PR-0 of Phase 6). This prompt is idempotent: re-paste
it across as many sessions as the Phase 6 arc takes; each session
detects the current state and either drives the next PR or reports
Phase 6 complete.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty. Note any commits ahead of origin/main —
   this branch's harness blocks direct push to main, so the user
   handles pushes manually.
2. Read docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md
   (the LLM-spine recast spec — committed to main as 409dcc5). This
   is the **design anchor** for Phase 6's PR-5 retext content. Skim
   §0 (reading order), §1 (summary), §2 (non-negotiable invariants),
   §3 (architectural inversion), §11 (migration shape), §12 (Phase 6
   disposition), §13 (canonical §10/§4.3/§7/§8 retext). Deeper
   reading is per-PR.
3. Read docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md
   (the Phase 6 implementation plan with §4 per-PR sub-sections, §5
   acceptance summary, §3 dependency graph). The plan is canonical
   for sequencing; the recast spec is canonical for scope. Where the
   two disagree on retext content, the recast spec §13 wins.
4. Read docs/superpowers/plans/2026-05-11-phase6-status.md (the PR
   checklist + dependency graph + per-PR notes). The next PR to
   dispatch is the lowest-numbered `[ ]` whose dependencies are all
   `[x]`. The parallel-safe wave for Phase 6 is Wave 1 (PR-1 + PR-2
   + PR-3), all of which may be dispatched concurrently in separate
   worktrees.
5. Check whether every PR-N checkbox is `[x]`:
   - If YES, Phase 6 is complete. Route to Step 4.
   - If NO, Phase 6 is in-progress. Route to Step 2.

## Step 2 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 6 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered `[ ]` whose dependencies are all `[x]`.

**Aggressive parallel dispatch:** PR-1, PR-2, and PR-3 are
parallel-safe — disjoint code surfaces:
  - PR-1 edits only crates/atlas-engine/src/manifest_patterns.rs.
  - PR-2 edits crates/atlas-index/src/rename_match.rs and
    crates/atlas-engine/src/l5_surface.rs/l6_edges.rs (the
    participant-rewrite seam, not the eprintln! sites PR-4 touches).
  - PR-3 edits crates/atlas-engine/src/l4_tree.rs and
    crates/atlas-engine/src/l9_subsystems.rs.

Dispatch all three concurrently in separate worktrees the moment
PR-0 has merged. Use superpowers:dispatching-parallel-agents and
verify each subagent's worktree base commit matches current main
before the subagent proceeds (memory
`feedback_worktree_base_verification`).

PR-4 is sequential after Wave 1 (depends on PR-3's
SubsystemOverrideNonExistent warning class and on PR-2 landing
edits to the same l6_edges.rs file in a different region). PR-5
is final.

Brief each subagent with:
- The full plan §4 task (Task N for PR-N) for that PR, copy-pasted
  verbatim — the plan's tasks include exact file paths, line
  numbers, and code blocks for every step.
- Pointers to the Phase 6 plan file, the LLM-spine recast spec, and
  the canonical system-model design spec (in that priority order).
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.

After each subagent reports DONE, run two-stage review per the
skill:
1. Spec compliance review (against the plan §4 task + plan §5
   per-PR acceptance gate).
2. Code quality review (only after spec is ✅).

Then independently verify on the worktree:
- cargo build --workspace
- cargo test --workspace --release --no-fail-fast
- cargo clippy --all-targets -- -D warnings
- cargo fmt --check
- cargo build --release --workspace (memory
  feedback_release_workspace_build_for_polyglot: standalone
  analyser [[bin]] targets aren't built by cargo test --release
  alone; the polyglot smoke test discovers them via runtime path
  lookup)
- cargo test -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast (the cumulative regression guard; LLM-call-budget
  assertions must pass — cold = Phase 2 PR-14 baseline ~26 calls;
  warm + reports = 0). Do NOT pipe through tail (memory
  feedback_no_tail_pipe_for_long_tests).

All checks must be clean before flipping the checkbox.

When every PR in the Phase 6 status file is `[x]`, route to Step 4.

## Step 3 — Special PR-handling notes (Phase 6 specifics)

Phase 6 is the **final deterministic-spine release**. No new
LLM-spine work; the LLM-spine recast begins in Phase 7. Phase 6
strengthens the user-authoring override discipline that the recast
will depend on. Per-PR specifics:

- **PR-1 (manifest extension, small):** Add `Makefile`/`makefile`/
  `GNUmakefile`/`*.mk`/`*.sh` to EXACT_MANIFEST_BASENAMES and
  is_manifest_file() suffix rules in
  crates/atlas-engine/src/manifest_patterns.rs. **LLM-call risk
  (load-bearing):** adding `.sh` to recognition without a paired
  classifier means future workspaces with shell scripts produce
  LlmClassify fallback calls. The polyglot fixture has no .sh/.mk
  files (verified in plan Step 1.1), so the cumulative regression
  guard cold count is unchanged. The paired classifier ships in
  Phase 9c per recast spec §11.3. Document the risk in the PR
  description; the user accepts the trade-off.

- **PR-2 (contract rename-match owner-follows, medium):** When
  rename-match maps `prior_id A → new_id B`, contracts owned by A
  follow to B. **α implementation chosen (id-embeds-owner):**
  contract ids today use owner-prefix format
  (`<component-id>/<contract-name>`); rename rewrites the prefix
  in-place. β (content-sha-stable) is deferred to Phase 10. The
  rewrite seam lives in crates/atlas-engine/src/l5_surface.rs
  (post-rename, pre-surfaces-write); edge participant rewrites
  land in crates/atlas-engine/src/l6_edges.rs. Independent fuzzy
  contract matching (a contract whose owner did not rename but
  whose content moved or split) is OUT OF SCOPE for PR-2;
  deferred to Phase 10 fuzzy matching.

- **PR-3 (subsystem field overlay, medium):** Wire the
  parsed-but-ignored per-component `subsystem:` field from
  crates/atlas-index/src/schema.rs:516-549 through L9 subsystem
  resolution. **Precedence rule: per-component override wins over
  central subsystems.overrides.yaml** (§4.1 closer-to-source
  authoring discipline). Removes the no-op
  `let _ = fo.subsystem.as_ref();` at l4_tree.rs:~324. Introduces
  new warning class `SubsystemOverrideNonExistent` (fires when
  central yaml references a member that doesn't exist) — this
  warning class is enumerated in PR-4's closed list.

- **PR-4 (--strict-overrides + closed enum + dual-mode test,
  medium):** Adds CLI flag escalating a closed enumeration of
  warnings (EdgesSuppressNoMatch, EdgesAddUnknownKind,
  SubsystemOverrideNonExistent) to errors with non-zero exit.
  Creates new module crates/atlas-engine/src/override_warnings.rs
  with the enum + OverrideWarningCollector trait + Permissive and
  Strict implementations. Refactors existing eprintln!() warning
  sites in l6_edges.rs:244-248 + l6_edges.rs:305-308 +
  l9_subsystems.rs to emit via the collector. New dual-mode
  contract test at crates/atlas-cli/tests/strict_overrides_contract.rs
  exercises every variant in both modes (permissive: exit 0;
  strict: exit non-zero). Subsumes the deferred Phase 3 PR-10
  stderr-capture test.

- **PR-5 (acceptance + closeout + canonical retext):** Final
  acceptance gate. Retexts canonical
  docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
  per recast spec §13: §4.3 inverted (LLM is the spine;
  deterministic is scaffolding); §7.1 + §7.3 marked RETIRED Phase 7;
  §8.1 fingerprint table extended with forward-pointer note;
  §10.6 marked SHIPPED; new §10.7–§10.11 entries for Phases 7–11.
  Memory updates: mark Phase 6 SHIPPED in
  project_phase4_plus_roadmap; mark
  project_phase6_paused_for_llm_spine as SUPERSEDED. Two-commit
  PR pattern (closeout note → backfill PR-5 commit sha in
  canonical §10.6).

## Step 4 — Phase 6 complete; consider Phase 7

When every Phase 6 PR is `[x]`:

1. Verify all checks one final time on a clean workspace:
   - cargo build --workspace
   - cargo test --workspace --release --no-fail-fast
   - cargo clippy --all-targets -- -D warnings
   - cargo fmt --check
   - cargo build --release --workspace
   - cargo test -p atlas-cli --test phase3_polyglot_fixture
     --release --no-fail-fast
2. Append a closing note to the Phase 6 status file's per-PR notes:
   `### Phase 6 — complete` with a one-paragraph summary of what
   shipped, the cumulative LOC delta, and the §10 retext landed.
3. Check whether
   docs/superpowers/specs/2026-05-*-atlas-vnext-phase7-design.md
   exists:
   - If YES, the user already drafted Phase 7 design. Stop the
     session and report success; the user pastes a Phase-7-shaped
     continuation prompt next.
   - If NO, the user has not yet brainstormed Phase 7. Stop the
     session, report Phase 6 success, and surface the question
     "Phase 6 is complete. Phase 7 (LLM-spine runtime per canonical
     §10.7 / recast spec §11.1) is the next phase. Want me to
     brainstorm Phase 7 scope?"
4. Do NOT auto-write a Phase 7 plan; that requires user-driven
   brainstorm via superpowers:brainstorming. The recast spec
   captures architectural intent but per-PR scope still needs
   design.

## Non-negotiables (every session, every subagent)

- **Greenfield + hard upgrade discipline.** No on-disk format
  compatibility with prior phases. No migration command. A user
  upgrading deletes .atlas/ and re-runs.
- **No new LLM call sites in this phase** (modulo PR-1's accepted
  trade-off — shell-script LlmClassify fallback for future
  workspaces; the polyglot fixture is unaffected). Cold polyglot
  smoke test must remain at the Phase 2 PR-14 baseline (~26
  calls); warm + reports = 0. Regression here is a hard failure.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo build/test/clippy/fmt
  clean before reporting DONE; orchestrator must independently
  re-verify before accepting.
- **Cumulative regression guard.** Every PR re-runs `cargo test
  -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast` before flipping its checkbox. Run `cargo build
  --release --workspace` first. Do NOT pipe through tail.
- **Lints and fmt clean everywhere:** fix any clippy/rustc warnings
  and cargo fmt drift encountered, even outside the code being
  touched.
- **Use the toml crate and serde_yaml** — never hand-rolled line
  scanning.
- **No iterator stubs for singletons** (memory
  `feedback_no_iterator_stubs_for_singletons`).
- **Worktree base verification.** When dispatching parallel
  subagents in Wave 1, confirm each worktree's base commit matches
  current main before the subagent proceeds (memory
  `feedback_worktree_base_verification`).
- **Do not touch mechanisms beyond what the PR's plan §4 task
  authorises.** If implementation pressure suggests a refactor,
  surface the question before doing it.
- **Commit message convention:** `phase6: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies.

## Step 5 — When the plan and reality disagree

The Phase 6 plan was written before any of its code changes exist.
If a plan instruction doesn't match the codebase (path shifted,
function signature changed, missing dep), prefer the plan's intent
and adapt the code. If the plan is genuinely under-specified or
contradicts itself, stop and surface the question rather than
improvising silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. PR-2 (contract
rename-match owner-follows) and PR-4 (--strict-overrides +
collector machinery) are the most likely candidates for scope creep.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- LLM-spine recast spec: docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md
  (commit 409dcc5 on main; the design anchor for Phase 6 PR-5
  retext content).
- Phase 6 plan: docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md
  (committed as part of PR-0; see status file for PR-0 commit sha).
- Workspace members (post-Phase-5): crates/atlas-analyzers,
  crates/atlas-engine, crates/atlas-llm, crates/atlas-reports,
  crates/atlas-cli, crates/component-ontology, crates/atlas-index,
  crates/analyzers/{python,csharp,dart,elixir,racket,lispkit},
  evaluation/harness. Phase 6 adds no new workspace members.
- Phase 3 polyglot fixture lives at
  crates/atlas-cli/tests/fixtures/phase3_polyglot/ and the smoke
  test at crates/atlas-cli/tests/phase3_polyglot_fixture.rs.
  Phase 6 does NOT mutate either; both are read-only regression
  guards (cumulative cross-phase guard).

## Memory state (project-scoped)

Memories live under
~/.claude/projects/-Users-antony-Development-Atlas/memory/ (which
is symlinked to .claude/memory/ in-repo per
`feedback_atlas_memory_in_repo`). The MEMORY.md index lists every
entry. Memories load-bearing for Phase 6:
- `feedback_atlas_llm_spine_intent` — strategic preference for LLM
  as spine; Phase 6 ships before this inversion begins.
- `project_phase6_paused_for_llm_spine` — the four pre-pivot
  candidate items operationalised in this plan.
- `feedback_worktree_base_verification` — Wave 1 parallel dispatch
  must verify each worktree's base sha matches current main.
- `feedback_no_tail_pipe_for_long_tests`,
  `feedback_release_workspace_build_for_polyglot`,
  `feedback_atlas_memory_in_repo`,
  `feedback_no_iterator_stubs_for_singletons` — execution-discipline
  constraints carried forward from prior phases.

Memory updates for Phase 6 land in PR-5 of Phase 6: mark Phase 6
SHIPPED in project_phase4_plus_roadmap; advance Phase 7 to
next-up; mark project_phase6_paused_for_llm_spine as SUPERSEDED.

Begin at Step 1.
```
