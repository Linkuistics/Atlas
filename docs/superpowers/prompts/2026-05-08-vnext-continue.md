# Atlas vNext — Continuation Prompt (Phase 3-shaped)

**Paste the fenced block below into a fresh Claude Code session.** The
prompt is idempotent across the Phase 3 arc: re-paste it across as
many sessions as it takes; each session detects whether Phase 3 is
underway, complete, or done-and-ready-for-Phase-4. When every PR of
Phase 3 is complete, the session reports completion and either stops
(if no Phase 4 design exists yet) or routes to writing the Phase 4
design (if the user wants that). The file itself (this header plus
the fenced block) is safe to pass to a new session verbatim — the
wrapper text is informational and the agent will treat the fenced
block as its instructions.

This prompt supersedes
`docs/superpowers/prompts/2026-05-07-vnext-continue.md`
(Phase-2-shaped, retained for forensic value but not authoritative
for Phase 3 sessions). Phase 1 and Phase 2 are both complete; their
status files are read-only references for forensic context.

---

```
You are continuing the Atlas vNext arc at /Users/antony/Development/Atlas.
Phase 1 is complete (PRs 0–12 landed; status in
docs/superpowers/plans/2026-05-06-phase1-status.md). Phase 2 is
complete (PRs 0–14 landed; status in
docs/superpowers/plans/2026-05-07-phase2-status.md). Phase 3 is the
current focus — its design spec, plan, and status file all exist on
main as of 2026-05-08. This prompt is idempotent: re-paste it across
as many sessions as the Phase 3 arc takes; each session detects the
current state and either drives the next PR or reports Phase 3
complete.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty. Note any commits ahead of origin/main —
   this branch's harness blocks direct push to main, so the user
   handles pushes manually.
2. Read docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md
   (the canonical Phase 3 design spec — committed to main as
   02f0914). Skim the §0 reading-order and §1 summary; deeper
   reading is per-PR.
3. Read docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md
   (the Phase 3 implementation plan with §4 per-PR sub-sections, §5
   acceptance summary, §9 dependency graph). The plan is canonical
   for sequencing; the design spec is canonical for scope and
   schemas. Where the two disagree, the design spec wins.
4. Read docs/superpowers/plans/2026-05-08-phase3-status.md (the PR
   checklist + dependency graph + per-PR notes). The next PR to
   dispatch is the lowest-numbered `[ ]` whose dependencies are all
   `[x]`. The parallel-safe waves are listed in the status file's
   dependency graph; when a wave's PRs are all dispatchable,
   dispatch them concurrently (one Agent tool call per PR, all in a
   single message — use superpowers:dispatching-parallel-agents).
5. Check whether every PR-N checkbox is `[x]`:
   - If YES, Phase 3 is complete. Route to Step 4.
   - If NO, Phase 3 is in-progress. Route to Step 2.

## Step 2 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 3 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered `[ ]` whose dependencies are all `[x]`. The
parallel-safe waves are listed in the status file's dependency graph;
when a wave's PRs are all dispatchable, dispatch them concurrently
(one Agent tool call per PR, all in a single message).

**Special case for PR-0b:** PR-0b lands the design-doc touch-ups in
the canonical
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` per
plan §4 PR-0b enumeration. Docs-only PR; no `cargo test` gates apply
beyond "the canonical spec re-reads coherently" and "no broken
section cross-references." Dispatch as a small subagent with the §4
PR-0b sub-section copy-pasted; review for accuracy of section
numbering and cross-reference fidelity.

**For PR-1 onward:** brief each subagent with:
- The full plan §4 sub-section for that PR, copy-pasted verbatim.
- Pointers to the Phase 3 plan file, the Phase 3 design spec, and
  the canonical system-model design spec (in that priority order).
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.

After each subagent reports DONE, run two-stage review per the
skill:
1. Spec compliance review (against §4 sub-section + §5 acceptance row).
2. Code quality review (only after spec is ✅).

Then independently verify:
- `cargo test --workspace --no-fail-fast`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`

All three must be clean before flipping the checkbox.

When every PR in the Phase 3 status file is `[x]`, route to Step 4.

## Step 3 — Special PR-handling notes

- **PR-2..PR-5 (cache-path retrofits):** Each ships a committed
  grep-audit script that fails CI on any tracked file referencing
  the pre-retrofit path. The script is the authoritative completion
  signal — manual code-review is not sufficient. Two PRs in this
  wave (e.g. PR-2 and PR-3) can race-merge cleanly because they
  touch different files; conflicts are limited to shared writer
  helpers (`pipeline.rs` line ranges) where a careful rebase merges
  cleanly.
- **PR-6 (overrides extension):** Touches atlas-contracts (the
  schema crate at `/Users/antony/Development/atlas-contracts` —
  separate sibling repo). PR-6 has TWO commits: one in atlas-contracts
  for the schema mutation, one in Atlas for the engine integration.
  The atlas-contracts commit must merge first (path-dep crate
  dependency).
- **PR-8..PR-11 (the four reports):** Each PR's CLI handler is the
  *only* place that does I/O for its report; the `atlas-reports`
  crate stays pure. Reviewers must reject any I/O introduced inside
  `atlas-reports/src/`. This is design §3.5 / plan §4 PR-7 invariant
  and load-bearing for Phase 5's Salsa conversion path.
- **PR-12 (atomic-write fixture suite):** Uses a feature-gated panic
  injection hook in `atomic_write.rs`. The hook MUST NOT exist in
  release builds; verify via `cargo build --release` before flipping
  the checkbox.
- **PR-13 (smoke test):** LLM call budget assertions are strict.
  Cold ≈ Phase 2 PR-14 baseline (~26 calls; verify exact via
  `PR14Backend` helper); warm rerun = 0; report runs = 0. A
  subagent that surfaces a >0 LLM-call delta on a report run must
  surface it as a design-spec violation, not bypass it.

## Step 4 — Phase 3 complete; consider Phase 4

When every Phase 3 PR is `[x]`:

1. Verify all three checks one final time on a clean workspace:
   - `cargo test --workspace --no-fail-fast`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
2. Append a closing note to the Phase 3 status file's per-PR notes:
   `### Phase 3 — complete` with a one-paragraph summary of what
   shipped, the cumulative LLM-call savings (if any) on the polyglot
   fixture, and any deferred items the user may want to revisit.
3. Check whether
   `docs/superpowers/specs/2026-05-*-atlas-vnext-phase4-design.md`
   exists:
   - If YES, the user already drafted Phase 4 design. Stop the
     session and report success; the user pastes a Phase-4-shaped
     continuation prompt next.
   - If NO, the user has not yet decided what Phase 4 contains. Stop
     the session, report Phase 3 success, and surface the question
     "Phase 3 is complete. Phase 4 design candidates per Phase 3
     design §9.1 (convergence + cleanups + LLM analyses): pattern
     detection, subprocess convergence, rust-analyzer integration,
     LLM threshold calibration, etc. Which subset is Phase 4 vs
     Phase 5? Want me to brainstorm Phase 4 scope?"
4. Do NOT auto-write a Phase 4 plan; that requires user-driven
   brainstorm via superpowers:brainstorming.

## Non-negotiables (every session, every subagent)

- **Greenfield.** No on-disk format compatibility with prior phases.
  No migration command. A user upgrading deletes .atlas/ and re-runs.
  This rule was Phase 1's; it carries forward across all phases.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo test/clippy/fmt clean
  before reporting DONE; orchestrator must independently re-verify
  before accepting.
- **Lints and fmt clean everywhere** (memory feedback_fix_all_lints):
  fix any clippy/rustc warnings and cargo fmt drift encountered, even
  outside the code being touched.
- **Use the toml crate** (memory feedback_toml_parsing) — never
  hand-rolled line scanning of TOML.
- **Use serde_yaml** for all YAML reads/writes.
- **Atomic writes for cache files** (design §6.3): all stateful and
  pure-derived cache writes use `atomic_write` (PR-1's helper).
  Non-atomic writes are a review-fail.
- **`atlas-reports` is pure-function only.** No I/O, no Salsa
  mutation. CLI handlers do all I/O. This is the Phase 5 conversion
  invariant.
- **No new LLM call sites.** Every Phase 3 report is a deterministic
  projection of L4–L8 outputs. A subagent introducing a new LLM
  call must surface it for design-spec review, not absorb it
  silently.
- **Editorial tier is fixed at six file types** (design §5.2):
  top-level overrides / external-components / subsystems / analyzers
  / config + per-component overrides. Anything else is derived and
  gitignored under cache/. PRs that emit outside these bounds are a
  review-fail.
- **Do not touch mechanisms beyond what the PR's §4 sub-section
  authorises.** If the implementation pressure suggests a refactor,
  surface the question before doing it. (Phase 1 PR-12 / Phase 2
  PR-1's deviation precedent applies: production-code fixes that are
  intrinsically tied to the integration test the PR exercises, ~37
  LOC, are within scope; broader refactors are not.)
- **Commit message convention:** `phase3: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies. PR-0a / PR-0b commits use the `0a` / `0b` suffix
  literally: `phase3: PR-0a plan + status` and `phase3: PR-0b
  design-doc touch-ups in canonical system-model spec`.

## Step 5 — When the plan and reality disagree

The Phase 3 plan was written before any of its code exists. If a plan
instruction doesn't match the codebase (path shifted, function
signature changed, missing dep), prefer the plan's intent and adapt
the code. If the plan is genuinely under-specified or contradicts
itself, stop and surface the question rather than improvising
silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. Phase 1 PR-12's ~37
LOC of production fixes was within the deviation tolerance; a
4000-line surprise diff is not.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- Sibling repo: /Users/antony/Development/atlas-contracts (path-dep
  schema crate; PR-6 modifies in a separate commit there).
- Workspace members today: crates/atlas-engine, crates/atlas-llm,
  crates/atlas-cli, crates/atlas-analyzers,
  crates/analyzers/{python,csharp,dart,elixir,racket,lispkit},
  evaluation/harness. Phase 3 adds crates/atlas-reports as a new
  workspace member (PR-7).
- Phase 2 polyglot fixture lives at
  `crates/atlas-cli/tests/fixtures/phase2_polyglot/`. Phase 3 PR-13
  adds a sibling `phase3_polyglot/` fixture; do NOT mutate the
  Phase 2 fixture.

## Memory state (project-scoped)

Memories live under `.claude/memory/` in the repo (synced via git
across machines). The MEMORY.md index lists every entry. Memories
referenced in the Phase 3 design spec / plan that are not present
locally (e.g. `feedback_toml_parsing`, `feedback_fix_all_lints`) are
captured by the design spec / plan itself; treat the doc text as the
constraint and proceed. Save new memories under `.claude/memory/`
with the relevant pointer added to MEMORY.md.

Begin at Step 1.
```
