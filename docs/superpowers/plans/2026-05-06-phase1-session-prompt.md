# Atlas vNext Phase 1 — Session Prompt

**Paste the fenced block below into a fresh Claude Code session.** The
prompt is idempotent: re-paste it across as many sessions as Phase 1
takes; each session reads `docs/superpowers/plans/2026-05-06-phase1-status.md`
to find the next pending PR and proceeds from there. When every PR is
marked `[x]`, the session reports completion and stops.

---

```
You are continuing Phase 1 implementation of the Atlas vNext system-model
redesign at /Users/antony/Development/Atlas. Phase 1 is multi-session
work; you are picking up from wherever the prior session left off.
Use subagent-driven development (superpowers:subagent-driven-development):
dispatch a fresh subagent per PR with two-stage review between tasks.

## Step 1 — Orient yourself (always do this first)

1. Read docs/superpowers/plans/2026-05-06-phase1-status.md. Each PR
   is a checkbox: [ ] = pending, [~] = in progress (subagent
   dispatched, not yet merged), [x] = done. Per-PR notes at the
   bottom of that file may carry load-bearing context from prior
   sessions — read every non-empty section before acting.
2. Run `git log --oneline -30` to confirm the status file matches
   reality. Commit messages should follow `phase1: PR-N <title>`.
   If the status file disagrees with git, trust git and fix the
   status file as your first commit.
3. If every PR in the status file is [x], Phase 1 is complete:
   report success, summarise what shipped, and stop.

## Step 2 — Read the canonical sources

These do not change between sessions; re-read the relevant sections
each time so you stay grounded:

- docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md —
  the 13-PR plan you are executing. Read in full at session start;
  re-read the §4 sub-section for the PR you are about to dispatch.
- docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
  — §0, §3, §4, §5, §6, §8, §10.1, §11.2 are load-bearing for
  Phase 1.
- Auto-memory at ~/.claude/projects/-Users-antony-Development-Atlas/
  memory/MEMORY.md and the entries it indexes — particularly
  project_atlas_vnext_phase1_plan, project_atlas_vnext_system_model_design,
  feedback_toml_parsing, feedback_fix_all_lints,
  tombstone_emit_once_design, all_components_not_salsa_tracked,
  project_distribution_brew_bottles.
- Companion specs (created during PR-0; read once they exist):
  - docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md
  - docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md

## Step 3 — Pick the next PR(s)

Walk the status file's PR list top-to-bottom. The next PR to dispatch
is the lowest-numbered [ ] PR whose dependencies are all [x]. The
parallel-safe waves are listed in the status file's dependency graph;
when a wave's PRs are all dispatchable, dispatch them concurrently
(one Agent tool call per PR, all in a single message).

PR-0 is docs-only and you should execute it yourself in the main
session — subagent isolation buys nothing for docs and the main
session needs to internalise the canonicalisation and scoping rules
because it reviews every downstream PR against them.

PR-1 onwards are subagent dispatches. Brief each subagent with:
  - The full §4 sub-section for that PR, copy-pasted verbatim.
  - Pointers to the plan file and the design spec.
  - The dependency PRs that have already merged (so the subagent
    can read them as reference).
  - The non-negotiables in Step 4 below.

## Step 4 — Critical context (every subagent must hear this)

- **Greenfield treatment.** No on-disk format compatibility with v1.
  No migration command. No transition window. No dual-read paths,
  no schema-version bridging, no fallback logic for old layouts.
  A user upgrading deletes .atlas/ and re-runs.
- **§3 of the plan lists v1 mechanisms reused as starting points** —
  these are NOT byte-for-byte invariants. They are working code to
  extend or refactor as the new shape demands.
- **Tests are the gate.** Each PR's acceptance criteria in plan §5
  are non-negotiable. Subagent must run `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`, and
  `cargo fmt --check` before reporting success.
- **Lints and fmt clean everywhere** (memory feedback_fix_all_lints):
  fix any clippy/rustc warnings and cargo fmt drift encountered, even
  outside the code being touched.
- **Use the toml crate** (memory feedback_toml_parsing) — never
  hand-rolled line scanning of TOML.
- **Do not touch v1 mechanisms beyond what the PR's §4 sub-section
  authorises.** If the implementation pressure suggests refactoring
  one of the §3 starting-point mechanisms, surface the question
  before doing it.
- **Commit message convention:** `phase1: PR-N <short title>`.
  Body should reference the plan section and list the acceptance
  criteria the PR satisfies.

## Step 5 — After each PR completes

1. Review the subagent's diff against the plan's §4 PR sub-section
   and §5 acceptance row. Reject if any criterion is unmet.
2. Verify `cargo test --workspace`, clippy, fmt all clean (run them
   yourself; do not trust subagent self-report alone).
3. If the PR introduces a non-obvious decision the next session
   needs to know, append a note to the relevant section of
   2026-05-06-phase1-status.md.
4. Flip the checkbox to [x] in the status file. Commit the status
   update either with the PR or as a follow-up.
5. Loop back to Step 3 to pick the next PR(s).

## Step 6 — When the plan and reality disagree

The plan was written before any of this code exists. If a plan
instruction doesn't match the codebase (path shifted, function
signature changed, missing dep), prefer the plan's intent and adapt
the code. If the plan is genuinely under-specified or contradicts
itself, stop and surface the question to me rather than improvising
silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. Don't ship a 4000-line
diff just because the plan said 1000.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- Sibling repos used by integration tests:
  - /Users/antony/Development/atlas-contracts (path-dep target;
    contains the atlas-index, component-ontology crates that
    PR-1 modifies).
  - /Users/antony/Development/Ravel-Lite (consumer for the PR-12
    end-to-end smoke test).
- Workspace members today: crates/atlas-engine, crates/atlas-llm,
  crates/atlas-cli, evaluation/harness. PR-5 adds crates/atlas-analyzers.
- Cross-repo edits: PR-1 modifies files inside
  /Users/antony/Development/atlas-contracts. That repo is a sibling,
  not a sub-tree; commit there separately and ensure local path-deps
  resolve before testing the Atlas-side changes.

Begin at Step 1.
```

---

## Maintenance notes (not part of the prompt)

If the plan structure changes (new PRs added, scope splits), update
both this prompt's PR count references and `2026-05-06-phase1-status.md`'s
PR list. The status file's per-PR notes section is the durable
log that survives session rotation; treat it as a structured commit
message stream.

If you want a per-PR reset (e.g., a PR landed but you discovered a
regression), un-check the box, append a note explaining the regression
and what work is needed, and re-paste the prompt — the next session
will pick up the now-pending PR.
