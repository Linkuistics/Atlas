# Atlas vNext — Continuation Prompt (Phase 2-shaped — DEPRECATED 2026-05-08)

> **Deprecated.** Phase 2 is complete; this prompt is retained for
> forensic value only. For continuing the Phase 3 arc, use
> `docs/superpowers/prompts/2026-05-08-vnext-continue.md` instead.

---

# Atlas vNext — Continuation Prompt (original Phase-2-shaped wrapper text below)

**Paste the fenced block below into a fresh Claude Code session.** The
prompt is idempotent across the entire vNext arc: re-paste it across
as many sessions as it takes; each session detects the current phase,
detects whether that phase has a plan, and either writes the plan or
dispatches the next PR. When every PR of every phase is complete, the
session reports completion and stops. The file itself (this header
plus the fenced block) is safe to pass to a new session verbatim —
the wrapper text is informational and the agent will treat the fenced
block as its instructions.

---

```
You are continuing the Atlas vNext arc at /Users/antony/Development/Atlas.
Phase 1 is complete (PRs 0–12 landed on main; see the Phase 1 status
file for forensic per-PR notes). The next step in the arc is Phase 2 —
"Pluggability and polyglot" per design §10.2 — but Phase 2 has no plan
yet. This prompt is idempotent: re-paste it across as many sessions as
the Phase 2 arc takes; each session detects whether the plan exists
and either writes it or drives its execution.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty.
2. Read docs/superpowers/plans/2026-05-06-phase1-status.md (the §
   "Phase 1 — complete" note at the bottom is the canonical "yes,
   Phase 1 is done" signal). Confirm Phase 1 is closed before
   proceeding.
3. Check whether docs/superpowers/specs/2026-05-06-atlas-vnext-phase2-plan.md
   (or any file matching `*phase2-plan*`) exists.
   - If it does NOT exist, your job this session is **planning** —
     follow Step 2.
   - If it DOES exist, your job this session is **execution** —
     follow Step 3.

## Step 2 — Planning session (write the Phase 2 plan)

Re-read these canonical sources before drafting anything:
- docs/superpowers/specs/2026-05-06-atlas-system-model-design.md §10.2
  (Phase 2 scope), §11.2 (open questions whose Phase-2 sub-bullets are
  load-bearing), §12 (the Phase 1 schema-churn risk row — Phase 2's
  first non-Rust analyser is the abstraction-confirmation milestone).
- docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md §3 (v1
  starting points — many remain reusable as Phase 2 starting points),
  §7 (out-of-scope-for-Phase-1 list — the Phase 2 backlog).
- docs/superpowers/plans/2026-05-06-phase1-status.md per-PR notes
  (especially PR-1 for the analyzer schema, PR-5 for the dispatcher,
  PR-7 for the rust-surface-analyzer template, PR-12 for the multi-root
  cross-tree fixes you'll likely have to extend).

Use superpowers:brainstorming first (mandatory for creative work) to
align on Phase 2's scope, abstraction-confirmation analyser, and PR
sequence. Then use superpowers:writing-plans to draft the plan.

The new plan file MUST mirror the Phase 1 plan's shape:
- Header note explaining greenfield treatment / non-negotiables.
- §3 "v1 mechanisms reused as starting points" table (carry forward
  Phase 1 mechanisms that Phase 2 extends).
- §4 per-PR sub-sections with **Intent**, **Files**, **Acceptance
  criteria**, **LOC** for each PR.
- §5 acceptance-criteria summary table (one row per PR, with the
  smoke-test PR as the integration gate).
- §6 risks specific to Phase 2.
- §7 out-of-scope (defer to Phase 3 / 4).
- Dependency graph drawing the parallel-safe waves.

Save it as
`docs/superpowers/specs/2026-05-06-atlas-vnext-phase2-plan.md` (or use
today's date if you prefer; either works as long as it's discoverable
by the wildcard match in Step 1).

After the plan exists, ALSO create
`docs/superpowers/plans/<date>-phase2-status.md` with:
- A PR checklist (one `- [ ]` per PR, in the same shape as the Phase 1
  status file's "PR status" section).
- A dependency-graph block (informational; canonical in plan §4).
- An empty "Per-PR notes" section.

Commit both files in a single commit:
`phase2: PR-0 plan + status file`. Do not start implementing PRs in
the planning session — the next paste of this prompt enters Step 3.

## Step 3 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 2 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered [ ] PR whose dependencies are all [x]. The
parallel-safe waves are listed in the status file's dependency graph;
when a wave's PRs are all dispatchable, dispatch them concurrently
(one Agent tool call per PR, all in a single message).

Brief each subagent with:
- The full §4 sub-section for that PR, copy-pasted verbatim.
- Pointers to the Phase 2 plan file and the design spec.
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.

After each subagent reports DONE, run two-stage review per the skill:
1. Spec compliance review (against §4 sub-section + §5 acceptance row).
2. Code quality review (only after spec is ✅).

Then independently verify:
- `cargo test --workspace --no-fail-fast`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`

All three must be clean before flipping the checkbox.

When every PR in the Phase 2 status file is [x], Phase 2 is complete:
report success, summarise what shipped, and stop. The next paste of
this prompt then enters Step 1 again, detects that Phase 2 is closed
and Phase 3 has no plan, and the cycle repeats for Phase 3.

## Non-negotiables (every session, every subagent)

- **Greenfield.** No on-disk format compatibility with prior phases.
  No migration command. A user upgrading deletes .atlas/ and re-runs.
  This rule was Phase 1's; it carries forward.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo test/clippy/fmt clean before
  reporting DONE; orchestrator must independently re-verify before
  accepting.
- **Lints and fmt clean everywhere** (memory feedback_fix_all_lints):
  fix any clippy/rustc warnings and cargo fmt drift encountered, even
  outside the code being touched.
- **Use the toml crate** (memory feedback_toml_parsing) — never
  hand-rolled line scanning of TOML.
- **Do not touch mechanisms beyond what the PR's §4 sub-section
  authorises.** If the implementation pressure suggests a refactor,
  surface the question before doing it. (Phase 1 PR-12 is the
  acceptable-deviation precedent: production-code fixes that are
  intrinsically tied to the integration test the PR exercises, ~37
  LOC, are within scope; broader refactors are not.)
- **Commit message convention:** `phase2: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies.

## Step 4 — When the plan and reality disagree

The Phase 2 plan is written before any of its code exists. If a plan
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
  schema crate; modify with separate commits in that repo when
  required by a Phase 2 PR).
- Workspace members today: crates/atlas-engine, crates/atlas-llm,
  crates/atlas-cli, crates/atlas-analyzers, evaluation/harness.
  Phase 2 may add new analyser crates per the plan.

Begin at Step 1.
```
