# Atlas vNext — Continuation Prompt (Phase 4-shaped)

**Paste the fenced block below into a fresh Claude Code session.** The
prompt is idempotent across the Phase 4 arc: re-paste it across as
many sessions as it takes; each session detects whether Phase 4 is
underway, complete, or done-and-ready-for-Phase-5. When every PR of
Phase 4 is complete, the session reports completion and either stops
(if no Phase 5 design exists yet) or routes to brainstorming Phase 5
scope (if the user wants that). The file itself (this header plus
the fenced block) is safe to pass to a new session verbatim — the
wrapper text is informational and the agent will treat the fenced
block as its instructions.

This prompt supersedes
`docs/superpowers/prompts/2026-05-08-vnext-continue.md`
(Phase-3-shaped, retained for forensic value but not authoritative
for Phase 4 sessions). Phases 1, 2, and 3 are all complete; their
status files are read-only references for forensic context.

---

```
You are continuing the Atlas vNext arc at /Users/antony/Development/Atlas.
Phase 1 is complete (PRs 0–12 landed; status in
docs/superpowers/plans/2026-05-06-phase1-status.md). Phase 2 is
complete (PRs 0–14 landed; status in
docs/superpowers/plans/2026-05-07-phase2-status.md). Phase 3 is
complete (PRs 0a, 0b, 1–13 landed; status in
docs/superpowers/plans/2026-05-08-phase3-status.md). Phase 4 is the
current focus — its design spec, plan, and status file all exist on
main as of 2026-05-09 (Phase 4 PR-0 commit). This prompt is
idempotent: re-paste it across as many sessions as the Phase 4 arc
takes; each session detects the current state and either drives the
next PR or reports Phase 4 complete.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty. Note any commits ahead of origin/main —
   this branch's harness blocks direct push to main, so the user
   handles pushes manually.
2. Read docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md
   (the canonical Phase 4 design spec — committed to main as
   f5a10e3). Skim §0 (reading order), §1 (summary), §2 (scope),
   and §3 (PR enumeration). Deeper reading is per-PR.
3. Read docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md
   (the Phase 4 implementation plan with §4 per-PR sub-sections, §5
   acceptance summary, §9 dependency graph). The plan is canonical
   for sequencing; the design spec is canonical for scope. Where the
   two disagree, the design spec wins.
4. Read docs/superpowers/plans/2026-05-09-phase4-status.md (the PR
   checklist + dependency graph + per-PR notes). The next PR to
   dispatch is the lowest-numbered `[ ]` whose dependencies are all
   `[x]`. The parallel-safe waves are listed in the status file's
   dependency graph; when a wave's PRs are all dispatchable, dispatch
   them concurrently (one Agent tool call per PR, all in a single
   message — use superpowers:dispatching-parallel-agents).
5. Check whether every PR-N checkbox is `[x]`:
   - If YES, Phase 4 is complete. Route to Step 4.
   - If NO, Phase 4 is in-progress. Route to Step 2.

## Step 2 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 4 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered `[ ]` whose dependencies are all `[x]`. The
parallel-safe waves are listed in the status file's dependency graph;
when a wave's PRs are all dispatchable, dispatch them concurrently
(one Agent tool call per PR, all in a single message).

Brief each subagent with:
- The full plan §4 sub-section for that PR, copy-pasted verbatim.
- Pointers to the Phase 4 plan file, the Phase 4 design spec, the
  canonical system-model design spec, and the Phase 3 plan / status
  files (in that priority order).
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.

After each subagent reports DONE, run two-stage review per the
skill:
1. Spec compliance review (against §4 sub-section + §5 acceptance row).
2. Code quality review (only after spec is ✅).

Then independently verify on the worktree:
- `cargo test --workspace --no-fail-fast`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast`
  (the cumulative regression guard; LLM-call-budget assertions must
  pass — cold = Phase 2 PR-14 baseline ~26 calls; warm + reports = 0)

All four must be clean before flipping the checkbox.

When every PR in the Phase 4 status file is `[x]`, route to Step 4.

## Step 3 — Special PR-handling notes (Phase 4 specifics)

Phase 4 is a *cleanup release* — net LOC negative, no new user-facing
capability, no schema change, no LLM call sites. Per-PR specifics:

- **PR-1 (LenientBackend extraction):** ~14+ test files duplicate the
  `struct LenientBackend` + `impl LlmBackend for LenientBackend`. The
  shared definition lives at `crates/atlas-engine/src/testing.rs`
  gated `#[cfg(any(test, feature = "test-fixtures"))]`. The release
  build MUST NOT contain the symbol — verify via `cargo build
  --release -p atlas-engine` and a grep on the resulting rlib. Run
  `grep -rln "^struct LenientBackend" crates/` immediately before the
  edit pass to detect any new test files added since the plan was
  written.

- **PR-2 (decoder consolidation):** Investigation-heavy. Step 1 of
  the PR is an enumeration of every per-language decoder site
  classified as `(canonical-shape | language-specific |
  should-stay-separate)`. Languages that don't fit the canonical shape
  STAY UNTOUCHED in PR-2 and surface as Phase 7 (per-language
  refinements) follow-ups. Do not absorb language-specific complexity
  into the shared helper. The PR description must list the
  "intentionally not migrated" languages with one-line rationale per
  site.

- **PR-3 (L8 phantom-subcomponent fix):** TDD-first. Write a
  hand-crafted unit test that triggers the phantom emission BEFORE
  diagnosing. The design spec's "cause TBD by implementer" wording
  means the implementer reproduces, diagnoses, and fixes — in that
  order. The fix should be small (~20-50 LOC); if the diagnosis
  surfaces a structural issue, surface and split the PR rather than
  expanding scope.

- **PR-4 (atomic_write convergence):** PR-12 of Phase 3
  (`crates/atlas-reports/tests/atomic_writes.rs`) is the
  durability regression guard. The PR-12 suite tests kill-during-write
  and after-rename hooks; it does NOT test error-message preservation.
  The implementer MUST additionally verify the `.with_context(...)`
  shape at `cache/mod.rs:129` preserves the prior anyhow error chain
  — manually trigger an error (e.g. write to a read-only directory)
  and document the resulting error chain pre/post in the PR
  description. The PR-12 fixture suite passing byte-identically is a
  hard gate.

- **PR-5 (build_engine_database / build_database_for_reports
  convergence):** Step 1 diff note in the PR description enumerates
  what the two helpers do differently. If the deltas are non-trivial
  (e.g. one runs L5 pre-warm and the other doesn't), the canonical
  helper accepts a parameter to drive that behaviour, NOT silently
  bake one behaviour as the new default. Output YAMLs from
  `run_modularity` and `run_divergence` must be byte-identical
  pre/post on a fresh fixture run.

- **PR-6 (sweep-test boilerplate consolidation):** Sequenced AFTER
  PR-1. The shared module at `crates/atlas-cli/tests/common/sweep_support.rs`
  re-exports `pub use atlas_engine::testing::LenientBackend;` from
  PR-1. Cargo's `tests/common/mod.rs` idiom prevents the shared module
  from being compiled as its own test binary; verify by running
  `cargo test -p atlas-cli` and confirming no "no tests in `common`"
  warning. The four `phase3_retrofit_*.rs` tests still pass after
  import-rewrite.

- **PR-7 (orphan re-export removal):** SINGLE-LINE deletion at
  `atlas-contracts/crates/atlas-index/src/lib.rs:60` (the `as
  save_related_components_atomic` rename). atlas-contracts only —
  there is no Atlas-side commit because no Atlas code references the
  alias (this is the design rationale for "orphan"). Step 1 grep MUST
  verify zero callers across BOTH atlas-contracts AND Atlas; if a
  caller exists, STOP — the design assumed orphan status. The
  underlying `save_atomic` symbol stays exported under its real name.

- **PR-8 (spec retext + §10 renumbering):** The canonical
  system-model spec at
  `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  carries five known stale "Phase 4" references (lines 502, 981,
  1270, 1314, 1436). §10 expands from §10.4–§10.6 to §10.1–§10.11
  (the new §10 table is in Phase 4 design §6, lands verbatim). Plus
  the Phase 3 design's §9.1 deferred-list gets eleven forward-pointer
  annotations (`(now Phase X)`). Final step is a sweep `grep -nE
  "Phase 4" docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  — expected: no occurrence outside the new §10.4 heading and body.
  Any other occurrence is a missed retext; fix it inline. Plus a
  cross-doc sweep `grep -rnE "§10\.[0-9]+" docs/superpowers/` to
  catch dangling cross-references in sibling design docs.

## Step 4 — Phase 4 complete; consider Phase 5

When every Phase 4 PR is `[x]`:

1. Verify all four checks one final time on a clean workspace:
   - `cargo test --workspace --no-fail-fast`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
   - `cargo test -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast`
2. Append a closing note to the Phase 4 status file's per-PR notes:
   `### Phase 4 — complete` with a one-paragraph summary of what
   shipped, the cumulative LOC delta (expected: net negative across
   PR-1..PR-7; PR-8 is small positive), and any deferred items the
   user may want to revisit.
3. Check whether
   `docs/superpowers/specs/2026-05-*-atlas-vnext-phase5-design.md`
   exists:
   - If YES, the user already drafted Phase 5 design. Stop the
     session and report success; the user pastes a Phase-5-shaped
     continuation prompt next.
   - If NO, the user has not yet brainstormed Phase 5. Stop the
     session, report Phase 4 success, and surface the question
     "Phase 4 is complete. Phase 5 (monorepo consolidation) is the
     next phase per the validated roadmap (memory
     `project_phase4_plus_roadmap`; canonical §10.5). Phase 5 scope:
     fold atlas-contracts in-tree, fold Ravel + Ravel-Lite, delete
     multi-root machinery. Want me to brainstorm Phase 5 scope?"
4. Do NOT auto-write a Phase 5 plan; that requires user-driven
   brainstorm via superpowers:brainstorming.

## Non-negotiables (every session, every subagent)

- **Greenfield.** No on-disk format compatibility with prior phases.
  No migration command. A user upgrading deletes .atlas/ and re-runs.
  This rule has carried forward from Phase 1; it carries forward
  through Phase 4.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo test/clippy/fmt clean
  before reporting DONE; orchestrator must independently re-verify
  before accepting.
- **Cumulative regression guard.** Every PR re-runs `cargo test
  -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast` before
  flipping its checkbox. The LLM-call-budget assertions (cold =
  Phase 2 PR-14 baseline ~26 calls; warm + reports = 0) are strict.
  A subagent that surfaces a >0 LLM-call delta on any phase MUST
  surface it as a Phase 4 invariant violation, not bypass it.
- **No new LLM call sites.** Phase 4 is a cleanup release; every
  report stays a deterministic projection of L4–L8 outputs. A
  subagent introducing a new LLM call must surface it for design-spec
  review, not absorb it silently.
- **Lints and fmt clean everywhere** (memory feedback_fix_all_lints):
  fix any clippy/rustc warnings and cargo fmt drift encountered, even
  outside the code being touched.
- **Use the toml crate** (memory feedback_toml_parsing) — never
  hand-rolled line scanning of TOML.
- **Use serde_yaml** for all YAML reads/writes.
- **Atomic writes for cache files.** PR-4 preserves byte-identical
  durability semantics. PR-12 of Phase 3 fixture suite is the
  regression guard.
- **`atlas-reports` is pure-function only.** No I/O, no Salsa
  mutation. CLI handlers do all I/O. This is the Phase 5 conversion
  invariant; it outlives Phase 4. PR-5 (build_engine_database
  convergence) touches `pipeline.rs` and `reports.rs` (CLI
  handlers), NOT `atlas-reports`.
- **Editorial tier is fixed at six file types** (Phase 3 design
  §5.2): top-level overrides / external-components / subsystems /
  analyzers / config + per-component overrides. Phase 4 does not
  touch the editorial tier; PRs that emit outside these bounds are
  a review-fail.
- **schema_version stays at 1** for every on-disk schema. Phase 4
  introduces zero schema mutations.
- **Do not touch mechanisms beyond what the PR's §4 sub-section
  authorises.** If the implementation pressure suggests a refactor,
  surface the question before doing it. (Phase 1 PR-12 / Phase 2
  PR-1's deviation precedent applies: production-code fixes that
  are intrinsically tied to the integration test the PR exercises,
  ~37 LOC, are within scope; broader refactors are not.)
- **Commit message convention:** `phase4: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies.

## Step 5 — When the plan and reality disagree

The Phase 4 plan was written before any of its code exists. If a plan
instruction doesn't match the codebase (path shifted, function
signature changed, missing dep), prefer the plan's intent and adapt
the code. If the plan is genuinely under-specified or contradicts
itself, stop and surface the question rather than improvising
silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. PR-2 (decoder
consolidation) and PR-3 (L8 phantom-subcomponent fix) are the most
likely candidates for scope creep — both are investigation-heavy and
the design's LOC estimates assume the canonical shape / minimal fix
discipline holds. A 4000-line surprise diff is not within tolerance.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- Sibling repo: /Users/antony/Development/atlas-contracts (path-dep
  schema crate; PR-7 modifies it in a single one-line atlas-contracts
  commit; no Atlas-side commit because no Atlas code references the
  orphan alias).
- Workspace members today: crates/atlas-engine, crates/atlas-llm,
  crates/atlas-cli, crates/atlas-analyzers,
  crates/analyzers/{python,csharp,dart,elixir,racket,lispkit},
  crates/atlas-reports (Phase 3 PR-7), evaluation/harness. Phase 4
  introduces no new workspace members.
- Phase 3 polyglot fixture lives at
  `crates/atlas-cli/tests/fixtures/phase3_polyglot/` and the smoke
  test at `crates/atlas-cli/tests/phase3_polyglot_fixture.rs`. Phase 4
  does NOT mutate either; both are read-only regression guards.
- Phase 3 PR-12 atomic-write fixture suite lives at
  `crates/atlas-reports/tests/atomic_writes.rs` (housed in
  `atlas-reports` because that crate's `dev-dependencies` carry the
  `atomic_write_panic_after_temp` feature flag). Phase 4 PR-4 uses
  this suite as its kill-during-write durability regression guard;
  run as `cargo test -p atlas-reports --test atomic_writes
  --no-fail-fast`.

## Memory state (project-scoped)

Memories live under `.claude/memory/` in the repo (synced via git
across machines). The MEMORY.md index lists every entry. Memories
referenced in the Phase 4 design spec / plan that are not present
locally (e.g. `feedback_toml_parsing`, `feedback_fix_all_lints`) are
captured by the design spec / plan itself; treat the doc text as the
constraint and proceed. Save new memories under `.claude/memory/`
with the relevant pointer added to MEMORY.md.

The validated post-Phase-3 phase ordering (Phase 4 = cleanup release;
Phase 5 = monorepo consolidation; Phase 6–10 = schema cleanups,
per-language refinements, subprocess convergence, LLM analyses,
server mode) lives in memory `project_phase4_plus_roadmap`. PR-8 of
Phase 4 retexts the canonical system-model spec's §10 with this
ordering verbatim from design §6.

Begin at Step 1.
```
