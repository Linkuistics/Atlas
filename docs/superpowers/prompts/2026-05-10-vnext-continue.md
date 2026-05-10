# Atlas vNext — Continuation Prompt (Phase 5-shaped)

**Paste the fenced block below into a fresh Claude Code session.** The prompt is idempotent across the Phase 5 arc: re-paste it across as many sessions as it takes; each session detects whether Phase 5 is underway, complete, or done-and-ready-for-Phase-6. When every PR of Phase 5 is complete, the session reports completion and either stops (if no Phase 6 design exists yet) or routes to brainstorming Phase 6 scope (if the user wants that). The file itself (this header plus the fenced block) is safe to pass to a new session verbatim — the wrapper text is informational and the agent will treat the fenced block as its instructions.

This prompt supersedes `docs/superpowers/prompts/2026-05-09-vnext-continue.md` (Phase-4-shaped, retained for forensic value but not authoritative for Phase 5 sessions). Phases 1, 2, 3, and 4 are all complete; their status files are read-only references for forensic context.

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
f80e179 on 2026-05-09). Phase 5 is the current focus — its design
spec, plan, and status file all exist on main as of 2026-05-10
(Phase 5 PR-0 commit). This prompt is idempotent: re-paste it across
as many sessions as the Phase 5 arc takes; each session detects the
current state and either drives the next PR or reports Phase 5
complete.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty. Note any commits ahead of origin/main —
   this branch's harness blocks direct push to main, so the user
   handles pushes manually.
2. Read docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md
   (the canonical Phase 5 design spec — committed to main as e1d3450).
   Skim §0 (reading order), §1 (summary), §2 (scope), and §3 (PR
   enumeration). Deeper reading is per-PR.
3. Read docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md
   (the Phase 5 implementation plan with §4 per-PR sub-sections, §5
   acceptance summary, §3 dependency graph). The plan is canonical
   for sequencing; the design spec is canonical for scope. Where the
   two disagree, the design spec wins.
4. Read docs/superpowers/plans/2026-05-10-phase5-status.md (the PR
   checklist + dependency graph + per-PR notes). The next PR to
   dispatch is the lowest-numbered `[ ]` whose dependencies are all
   `[x]`. The parallel-safe wave for Phase 5 is narrow — most of the
   PRs are sequential — but PR-5 (canonical-design retext, docs only)
   is parallel-safe with PR-2/PR-3/PR-4 and may be dispatched in a
   separate worktree concurrent with the deletion sequence.
5. Check whether every PR-N checkbox is `[x]`:
   - If YES, Phase 5 is complete. Route to Step 4.
   - If NO, Phase 5 is in-progress. Route to Step 2.

## Step 2 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 5 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered `[ ]` whose dependencies are all `[x]`.

**Aggressive parallel dispatch:** PR-5 is the only parallel-safe PR
in Phase 5. It edits only design docs (the canonical system-model
spec at docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
and the override-scoping spec at
docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md).
The moment PR-1 has merged, dispatch PR-5 concurrently in its own
worktree alongside the PR-2 → PR-3 → PR-4 serial chain. Use
superpowers:dispatching-parallel-agents and verify each subagent's
worktree base commit matches current main before the subagent
proceeds (memory `feedback_worktree_base_verification`).

Brief each subagent with:
- The full plan §4 task (Task N for PR-N) for that PR, copy-pasted
  verbatim — the plan's tasks include exact file paths, line
  numbers, and code blocks for every step.
- Pointers to the Phase 5 plan file, the Phase 5 design spec, and
  the canonical system-model design spec (in that priority order).
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.

After each subagent reports DONE, run two-stage review per the
skill:
1. Spec compliance review (against the plan §4 task + plan §5.4
   per-PR gate row).
2. Code quality review (only after spec is ✅).

Then independently verify on the worktree:
- `cargo build --workspace`
- `cargo test --workspace --release --no-fail-fast`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo build --release --workspace` (memory
  `feedback_release_workspace_build_for_polyglot`: standalone
  analyser [[bin]] targets aren't built by `cargo test --release`
  alone; the polyglot smoke test discovers them via runtime path
  lookup)
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast` (the cumulative regression guard; LLM-call-budget
  assertions must pass — cold = Phase 2 PR-14 baseline ~26 calls;
  warm + reports = 0). Do NOT pipe through tail (memory
  `feedback_no_tail_pipe_for_long_tests`).

All checks must be clean before flipping the checkbox.

When every PR in the Phase 5 status file is `[x]`, route to Step 4.

## Step 3 — Special PR-handling notes (Phase 5 specifics)

Phase 5 is a *deletion-shaped consolidation release* — no new
user-facing capability beyond removal of the never-used
--additional-root flag, no schema change, no new LLM call sites.
Per-PR specifics:

- **PR-1 (atlas-contracts fold):** The most complex PR in Phase 5.
  Snapshot copy ~/Development/atlas-contracts/crates/{component-
  ontology,atlas-index}/ into Atlas/crates/ (no git subtree-merge,
  no filter-repo — plain directory copy with a single import commit;
  memory `feedback_user_low_git_history_value`). Add per-crate
  release.toml overrides; rewrite Cargo.toml [workspace.dependencies]
  paths from "../atlas-contracts/crates/..." to "crates/...".
  Cross-repo coordination is critical: Atlas PR-1 lands FIRST, then
  IMMEDIATELY edit ~/Development/Ravel-Lite/Cargo.toml lines 51 + 56
  to repoint at "../Atlas/crates/...", then commit in Ravel-Lite.
  Inverting the order leaves Ravel-Lite's local build broken (risk
  R1 in design §5). Both `cargo publish --dry-run -p
  component-ontology` and `cargo publish --dry-run -p atlas-index`
  must be clean — capture output for the PR description (risk R2,
  metadata gaps surface here). Website content relocates to
  website/docs/schema/ (or a PR-1-proposed alternative); document
  the choice in the PR description (risk R3).

- **PR-2 (drop discovery):** Delete crates/atlas-engine/src/
  root_expansion.rs (469 LOC), --additional-root CLI flag,
  IndexConfig.additional_roots, IndexConfig::all_roots(), and the
  two manual_iter chains in pipeline.rs. Workspace.roots:
  Vec<PathBuf> stays as a length-1 vec at this PR boundary — PR-3
  collapses the type. **Scope-bleed disposition:** if `cargo build
  --workspace` fails after deleting expand_roots because a soon-
  deleted multi-root test (`crates/atlas-engine/tests/
  multi_root_path_deps.rs`) imports it, delete that test inside
  PR-2's scope and document the bleed in PR-2's status note. Do
  not split into a sub-PR. PR-4 then deletes the remaining two
  multi-root tests.

- **PR-3 (singularise Workspace):** Workspace.roots: Vec<PathBuf> →
  root: PathBuf in db.rs; delete primary_root() method; delete
  crates/atlas-engine/src/roots.rs (61 LOC); rewrite ~30 call sites
  across L2/L3/L4/L5/L6/L8/L9 source files (l5_surface.rs alone has
  ~17 sites). The plan's §2 file table enumerates exact line numbers.
  No iterator stubs, no slice helpers, no shim methods — `Workspace`
  ends with a singular `root: PathBuf` field accessed directly via
  `workspace.root(db)` (memory `feedback_no_iterator_stubs_for_
  singletons`). Audit grep `git grep -E
  'multi.root|multi-root|workspace\.roots' crates/` returns zero
  hits in src/; surviving hits in tests/ that the PR explicitly
  retains must be justified in the PR description.

- **PR-4 (salvage tests):** Create crates/atlas-cli/tests/
  contract_edge_in_workspace.rs (~400 LOC) — single-root rewrite of
  the deleted atlas_contracts_in_ravel_lite.rs. Preserves AC#1–5
  (lines 21–36 of the deleted test). Delete the original (593 LOC)
  + crates/atlas-engine/tests/multi_root.rs (154 LOC) +
  crates/atlas-engine/tests/multi_root_path_deps.rs (742 LOC) if
  any survived PR-2/PR-3 scope-bleed. PR description maps each
  AC#1–5 to the corresponding assertion in the new test; redundant
  assertions must be explicitly justified, not silently omitted
  (risk R4).

- **PR-5 (canonical design retext):** Docs-only, parallel-safe with
  PR-2/3/4. Delete §5.3 "Multi-root workspace" in
  docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
  entirely; renumber §5.4 → §5.3, §5.5 → §5.4, §5.6 → §5.5; mark
  Phase 5 SHIPPED in §10.5 with the §7 retext from the design spec
  (verbatim); delete the multi-root architectural-seam bullet from
  §10.1; delete the glossary "Multi-root workspace" entry. Also
  retext lines 19, 177, 253 of
  docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md.
  PR-5 lands the canonical-design retext with a literal
  `<PR-6-COMMIT-SHA>` placeholder in §10.5; PR-6 backfills the
  actual SHA in a follow-up commit. Inbound references in older
  Phase 1/2/3/4 spec/plan files are NOT retroactively updated
  (specs are time-snapshots).

- **PR-6 (acceptance + closeout):** Verification only. Two-commit
  PR: commit 1 lands the closeout note + Upgrade notes subsection
  in the status file; commit 2 backfills `<PR-6-COMMIT-SHA>` in
  canonical §10.5 with the actual SHA from commit 1. Audit greps
  for multi.root|multi-root|additional_root|expand_roots|
  best_root_for in crates/ must return zero non-test, non-deleted-
  file hits. Status file's Upgrade notes subsection states the hard
  upgrade discipline: users delete .atlas/ before upgrading. No
  migration command, no version-aware decoder. Memory updates
  (project_phase4_plus_roadmap, project_monorepo_consolidation,
  project_phase5_split_and_ravel_bazel) happen in this PR.
  Manual post-merge steps (atlas-contracts repo archive, local
  rm -rf) are surfaced to the user as a closeout question, not
  auto-executed.

## Step 4 — Phase 5 complete; consider Phase 6

When every Phase 5 PR is `[x]`:

1. Verify all checks one final time on a clean workspace:
   - `cargo build --workspace`
   - `cargo test --workspace --release --no-fail-fast`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
   - `cargo build --release --workspace`
   - `cargo test -p atlas-cli --test phase3_polyglot_fixture
     --release --no-fail-fast`
2. Append a closing note to the Phase 5 status file's per-PR notes:
   `### Phase 5 — complete` with a one-paragraph summary of what
   shipped, the cumulative LOC delta (expected: net negative inside
   crates/, net positive overall because the two folded schema crates
   are net new code in-tree), and any deferred items the user may
   want to revisit (notably: Ravel + Ravel-Lite fold, possibly tied
   to a Bazel build-system migration; per memory
   `project_phase5_split_and_ravel_bazel`).
3. Check whether
   `docs/superpowers/specs/2026-05-*-atlas-vnext-phase6-design.md`
   exists:
   - If YES, the user already drafted Phase 6 design. Stop the
     session and report success; the user pastes a Phase-6-shaped
     continuation prompt next.
   - If NO, the user has not yet brainstormed Phase 6. Stop the
     session, report Phase 5 success, and surface the question
     "Phase 5 is complete. Phase 6 (user-facing schema cleanups)
     is the next phase per the validated roadmap (memory
     `project_phase4_plus_roadmap`; canonical §10.6). Want me to
     brainstorm Phase 6 scope?"
4. Do NOT auto-write a Phase 6 plan; that requires user-driven
   brainstorm via superpowers:brainstorming.

## Non-negotiables (every session, every subagent)

- **Greenfield + hard upgrade discipline.** No on-disk format
  compatibility with prior phases. No migration command. A user
  upgrading deletes .atlas/ and re-runs. Phase 5 PR-6's Upgrade
  notes subsection in the status file documents this rule
  explicitly because the multi-root → single-root collapse changes
  persisted output state.
- **No new LLM call sites.** Phase 5 is a deletion-shaped
  consolidation release; every report stays a deterministic
  projection of L4–L8 outputs. A subagent introducing a new LLM
  call must surface it for design-spec review, not absorb it
  silently. Cold polyglot smoke test must remain at the Phase 2
  PR-14 baseline (~26 calls); warm + reports = 0. Regression here
  is a hard failure.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo build/test/clippy/fmt
  clean before reporting DONE; orchestrator must independently
  re-verify before accepting.
- **Cumulative regression guard.** Every PR re-runs `cargo test
  -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast` before flipping its checkbox. Run `cargo build
  --release --workspace` first (memory
  `feedback_release_workspace_build_for_polyglot`). Do NOT pipe
  through tail (memory `feedback_no_tail_pipe_for_long_tests`) —
  buffered tail makes a working 99%-CPU process look stuck; let
  stdout pass through.
- **Lints and fmt clean everywhere** (memory feedback_fix_all_lints):
  fix any clippy/rustc warnings and cargo fmt drift encountered,
  even outside the code being touched.
- **Use the toml crate** (memory feedback_toml_parsing) — never
  hand-rolled line scanning of TOML.
- **Use serde_yaml** for all YAML reads/writes.
- **Cross-repo coordination ordering for PR-1.** Atlas PR-1 lands
  FIRST (creates Atlas/crates/{component-ontology,atlas-index}).
  Ravel-Lite Cargo.toml path-edit lands IMMEDIATELY AFTER. Only
  then is ~/Development/atlas-contracts/ safe to remove. Inverting
  the order leaves a build-broken state on at least one repo
  (risk R1).
- **Snapshot copy, not git subtree** (memory
  `feedback_user_low_git_history_value`). The atlas-contracts fold
  is a plain directory copy with a single import commit.
- **No iterator stubs for the singular root** (memory
  `feedback_no_iterator_stubs_for_singletons`). Workspace.root:
  PathBuf has no iterator, no as_slice(), no primary_root(). Every
  consumer reads the field directly. Do not introduce wrapper
  helpers.
- **`atlas-reports` is pure-function only.** No I/O, no Salsa
  mutation. CLI handlers do all I/O. This invariant carries
  forward from Phase 3; Phase 5 does not touch atlas-reports.
- **Editorial tier is fixed at six file types** (Phase 3 design
  §5.2). Phase 5 does not touch the editorial tier; PRs that emit
  outside these bounds are a review-fail.
- **schema_version stays at 1** for every on-disk schema. Phase 5
  introduces zero schema mutations.
- **Worktree base verification.** When dispatching parallel
  subagents (PR-5 alongside PR-2/3/4), confirm each worktree's
  base commit matches current main before the subagent proceeds
  (memory `feedback_worktree_base_verification`).
- **Do not touch mechanisms beyond what the PR's plan §4 task
  authorises.** If implementation pressure suggests a refactor,
  surface the question before doing it. Phase 5's deletion-shaped
  scope is narrow — feature creep here is especially dangerous
  because the audit-grep gates are unforgiving.
- **Commit message convention:** `phase5: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies.

## Step 5 — When the plan and reality disagree

The Phase 5 plan was written before any of its code changes exist.
If a plan instruction doesn't match the codebase (path shifted,
function signature changed, missing dep), prefer the plan's intent
and adapt the code. If the plan is genuinely under-specified or
contradicts itself, stop and surface the question rather than
improvising silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. PR-3 (singularise
Workspace) is the most likely candidate for scope creep — the plan
estimates ~30 call sites but the actual count could grow if new
multi-root references were added since the audit. A 2000-line
surprise diff in PR-3 is not within tolerance.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- Sibling repo (pre-Phase-5): /Users/antony/Development/atlas-contracts
  (path-dep schema crate workspace; folded into Atlas in Phase 5
  PR-1). Post-PR-1 + the manual GitHub archive step in PR-6, this
  repo is read-only forensic context.
- Sibling repo (cross-repo coordination): /Users/antony/Development/
  Ravel-Lite (path-deps the atlas-contracts schema crates today;
  PR-1 of Phase 5 includes a coordinated commit in Ravel-Lite that
  rewrites Cargo.toml lines 51 + 56 to point at ../Atlas/crates/...).
- Workspace members today (pre-Phase-5): crates/atlas-analyzers,
  crates/atlas-engine, crates/atlas-llm, crates/atlas-reports,
  crates/atlas-cli, crates/analyzers/{python,csharp,dart,elixir,
  racket,lispkit}, evaluation/harness. **Phase 5 PR-1 adds
  crates/component-ontology and crates/atlas-index** as the only
  new workspace members for the entire phase.
- Phase 3 polyglot fixture lives at
  `crates/atlas-cli/tests/fixtures/phase3_polyglot/` and the smoke
  test at `crates/atlas-cli/tests/phase3_polyglot_fixture.rs`.
  Phase 5 does NOT mutate either; both are read-only regression
  guards (cumulative cross-phase guard).
- The to-be-deleted multi-root surfaces:
  `crates/atlas-engine/src/root_expansion.rs` (469 LOC, deleted in
  PR-2), `crates/atlas-engine/src/roots.rs` (61 LOC, deleted in
  PR-3), `crates/atlas-engine/tests/multi_root.rs` (154 LOC),
  `crates/atlas-engine/tests/multi_root_path_deps.rs` (742 LOC),
  `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` (593
  LOC, salvaged into contract_edge_in_workspace.rs in PR-4).

## Memory state (project-scoped)

Memories live under
~/.claude/projects/-Users-antony-Development-Atlas/memory/. The
MEMORY.md index lists every entry. Memories referenced in the
Phase 5 design spec / plan that are not present locally are captured
by the design spec / plan itself; treat the doc text as the
constraint and proceed. Save new memories under that directory with
the relevant pointer added to MEMORY.md.

The validated post-Phase-3 phase ordering (Phase 4 = cleanup release
SHIPPED 2026-05-09; Phase 5 = monorepo consolidation part 1, A + C
only; Phase 6–10 = schema cleanups, per-language refinements,
subprocess convergence, LLM analyses, server mode) lives in memory
`project_phase4_plus_roadmap`. PR-5 of Phase 5 retexts the canonical
system-model spec's §10.5 to mark Phase 5 SHIPPED. The Ravel +
Ravel-Lite fold (originally part of Phase 5 in the
project_monorepo_consolidation memory) is deferred to a post-Phase-5
phase, possibly tied to a Bazel migration; this is captured in
memory `project_phase5_split_and_ravel_bazel` and reflected in the
plan's §1 deliverable-restated.

Begin at Step 1.
```
