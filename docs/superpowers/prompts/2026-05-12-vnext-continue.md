# Atlas vNext — Continuation Prompt (Phase 7-shaped)

**Paste the fenced block below into a fresh Claude Code session.** The prompt is idempotent across the Phase 7 arc: re-paste it across as many sessions as it takes; each session detects whether Phase 7 is underway, complete, or done-and-ready-for-Phase-8. When every PR of Phase 7 is complete, the session reports completion and either stops (if no Phase 8 brainstorm exists yet) or routes to brainstorming Phase 8 scope (if the user wants that). The file itself (this header plus the fenced block) is safe to pass to a new session verbatim — the wrapper text is informational and the agent will treat the fenced block as its instructions.

This prompt supersedes `docs/superpowers/prompts/2026-05-11-vnext-continue.md` (Phase-6-shaped, retained for forensic value but not authoritative for Phase 7 sessions). Phases 1, 2, 3, 4, 5, and 6 are all complete; their status files are read-only references for forensic context.

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
final commit a302ce5 on 2026-05-10). Phase 6 is complete (PR-0
through PR-5 landed; PR-1 deferred to Phase 9c; status in
docs/superpowers/plans/2026-05-11-phase6-status.md; final commit
9350735 on 2026-05-11). Phase 7 is the current focus — the LLM-spine
runtime per recast spec §11.1. The Phase 7 brainstorm, plan, status
file, and continuation prompt all exist on main as of 2026-05-12
(brainstorm: f4ea770; PR-0 of Phase 7: <PR-0-COMMIT-SHA>). This
prompt is idempotent: re-paste it across as many sessions as the
Phase 7 arc takes; each session detects the current state and either
drives the next PR or reports Phase 7 complete.

## Step 1 — Orient yourself

1. Run `git log --oneline -20` and `git status` so you know what's
   landed and what's dirty. Note any commits ahead of origin/main —
   this branch's harness blocks direct push to main, so the user
   handles pushes manually.
2. Read docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md
   (the design anchor; brainstorm commit f4ea770). §2 is the 14-item
   decision table (locked pivots; do not relitigate). §4–§8 are the
   per-wave designs (lift verbatim into per-PR briefs). §10 is the
   acceptance criteria. §12 is the open risks.
3. Read docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md
   (the implementation plan with §4 per-PR sub-sections, §5
   acceptance summary, §3 dependency graph, §6 testing surface, §7
   risks). The plan is canonical for sequencing and per-PR scope; the
   brainstorm is canonical for architectural pivots. Where the two
   disagree on scope, the brainstorm wins.
4. Read docs/superpowers/plans/2026-05-12-phase7-status.md (the PR
   checklist + dependency graph + per-PR notes). The next PR to
   dispatch is the lowest-numbered `[ ]` whose dependencies are all
   `[x]`. The only parallel-safe dispatch in Phase 7 is Wave 2 (PR-3,
   three subagents); every other PR is sequential.
5. Optionally read docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md
   for the parent design spec (the architectural inversion). The
   brainstorm imports its non-negotiables (§2) and migration shape
   (§11).
6. Check whether every PR-N checkbox is `[x]`:
   - If YES, Phase 7 is complete. Route to Step 4.
   - If NO, Phase 7 is in-progress. Route to Step 2.

## Step 2 — Execution session (drive the next PR)

Use superpowers:subagent-driven-development. Walk the Phase 7 status
file's PR checklist top-to-bottom. The next PR to dispatch is the
lowest-numbered `[ ]` whose dependencies are all `[x]`.

**Wave-by-wave dispatch strategy (from plan §3):**

  - **Wave 1 (after PR-0):** PR-1, then PR-2. Sequential.
  - **Wave 2 (after PR-2):** PR-3a + PR-3b + PR-3c dispatched
    *concurrently* in three pre-created worktrees. Pre-create the
    worktrees yourself via `git worktree add -b phase7-pr3<a|b|c>
    /tmp/atlas-phase7-pr3<a|b|c> main` and dispatch subagents with
    explicit `cwd` pointing at those paths — NOT `isolation:
    "worktree"`, which has been unreliable in this repo (memory
    `feedback_worktree_base_verification`). After dispatch, run
    `git worktree list` and verify each worktree's commit matches
    `git rev-parse origin/main`. After all three subagents report
    DONE, merge to a `phase7-pr3` integration branch (the merge may
    surface trivial conflicts in `crates/atlas-agents/src/tools/mod.rs`
    where each subagent appends to the same re-export list);
    fast-forward `phase7-pr3` to main as PR-3's landing.
  - **Wave 3 (after PR-3):** PR-4, then PR-5. Sequential.
  - **Wave 4 (parallel-safe with Wave 3; depends only on PR-2):**
    PR-6 may be dispatched as soon as PR-2 is `[x]`. The TUI subscriber
    consumes the event bus alone and does not depend on PR-3/PR-4/PR-5
    internals. If session bandwidth allows, dispatch PR-6 alongside
    PR-4 to compress wall time.
  - **Wave 5 (after PR-5 AND PR-6):** PR-7. Final wiring + closeout.

Brief each subagent with:
- The full plan §4 task (Task N for PR-N) for that PR, copy-pasted
  verbatim — the plan's tasks include exact file paths, line numbers,
  and code blocks for every step.
- Pointers to the Phase 7 plan, the Phase 7 brainstorm (the design
  anchor; brainstorm §2 pivots are locked), the LLM-spine recast spec,
  and the canonical system-model design spec (in that priority order).
- The dependency PRs that have already merged (so the subagent can
  read them as reference).
- The non-negotiables below.
- The 5 PR-0 forward-pointers from the Phase 7 status file's PR-0
  note (AgentEvent naming collision; new workspace deps; pre-existing
  files; 27-not-26 wrapper count; PR-1 + PR-5 scope-creep risks).

After each subagent reports DONE, run two-stage review per the skill:
1. Spec compliance review (against the plan §4 task + plan §5 per-PR
   acceptance gate).
2. Code quality review (only after spec is ✅).

Then independently verify on the worktree (or main after merge):
- cargo build --workspace
- cargo test --workspace --no-fail-fast
- cargo clippy --all-targets -- -D warnings
- cargo fmt --check
- cargo build --release --workspace (memory
  feedback_release_workspace_build_for_polyglot: standalone analyser
  [[bin]] targets aren't built by cargo test --release alone; the
  polyglot smoke test discovers them via runtime path lookup)
- cargo test -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast (the cumulative regression guard; cold = ~40
  calibrated baseline per Phase 6 PR-5 closeout; warm + reports = 0).
  Do NOT pipe through tail (memory feedback_no_tail_pipe_for_long_tests).

All checks must be clean before flipping the checkbox.

Each PR follows the two-commit pattern (Phase 4/5/6 precedent):
  1. First commit lands the code/docs changes.
  2. Second commit flips the PR-N status checkbox AND backfills the
     PR-N commit SHA into the status file's per-PR note block.

When every PR in the Phase 7 status file is `[x]`, route to Step 4.

## Step 3 — Special PR-handling notes (Phase 7 specifics)

Phase 7 is the **LLM-spine runtime release** — Atlas's deterministic
engine becomes the fallback; LLM agents become the spine. **No
language analyser retires** in Phase 7; that begins in Phase 8 (Cargo)
and continues across Phase 9 waves. Per-PR specifics:

- **PR-1 (atlas-agents + Tool trait + MCP + async LlmBackend, large):**
  Stacks new crate creation + Tool trait + MCP stdio server with
  multi-client multiplexing + async surface on LlmBackend for 5
  backends + the rename of today's atlas-llm::AgentEvent →
  BackendCallEvent (and AgentObserver → BackendCallObserver) to free
  the names for PR-2's runtime-level AgentEvent + 4 new workspace
  dependencies (tokio, async-trait, ratatui, crossterm). Scope-creep
  candidate per plan §7.7; if subagent hits >2x time/LOC estimate at
  any natural checkpoint (Step 1.3 rename complete; Step 1.5 backend
  impls complete; Step 1.9 MCP server complete; Step 1.13 tests
  green), stop-and-surface.

- **PR-2 (transcript cache + event bus + JSON-Lines, medium):**
  Extends `crates/atlas-engine/src/llm_cache.rs` with multi-shot
  `call_agent_cached`; extends `crates/atlas-engine/src/atomic_write.rs`
  with two-file `atomic_write_pair` primitive (do NOT propose to
  *create* either file — both already exist). Defines the new
  `atlas-agents::AgentEvent` enum (different variants from the renamed
  `atlas-llm::BackendCallEvent`). EventBus = Tokio
  `broadcast::channel<AgentEvent>` capacity 1024; lagged-receiver =
  error-and-log (not silent-drop). Drain handshake before
  `AgentRuntime::run()` returns is structurally enforced by the
  RuntimeComplete sentinel + per-subscriber done_tx oneshot.

- **PR-3 (27 tool wrappers, three parallel subagents, medium):**
  ONLY multi-subagent PR. Wrapper count is **27, not 26** (brainstorm
  §5 rounded; plan §4 Task 3 locks 27 across 9 manifests + 4 mature +
  6 mid-tier + 8 weak-tooling). Each wrapper is a pure pass-through;
  no LLM, no new reasoning, no behaviour change. Per-wrapper
  behavioural-parity unit tests assert wrapper output == direct-call
  output. Pre-create the three worktrees yourself per
  feedback_worktree_base_verification; verify base SHA before
  dispatching each subagent. Subagents commit to phase7-pr3a/b/c
  branches; orchestrator merges to phase7-pr3 integration branch
  after all three DONE; fast-forward to main.

- **PR-4 (runtime single-iteration + Lane A, large):** Lands the
  AgentRuntime struct, deterministic-only dispatch (mandatory
  override files; PR-5 relaxes), HTTP tool-use loop, MCP tool-loop
  observation, Lane A schema validation. PR-4 does NOT wire the
  runtime into `atlas index` — that's PR-7. The single-iteration
  smoke test (`agent_runtime_single_iteration.rs`) exercises the full
  path via test_backend with canned responses.

- **PR-5 (fixedpoint + LLM dispatch + Lane B, large):** Three
  subsystems stacked in one PR. Scope-creep candidate per plan §7.6;
  if subagent hits >2x time/LOC estimate, candidate split is PR-5a
  (fixedpoint + LLM dispatch) → PR-5b (Lane B). Brainstorm §2 row 11
  locks cross-provider Lane B (Anthropic↔OpenAI mapping); same-
  provider fallback emits `AuditDegraded` event.

- **PR-6 (TUI + replay, medium):** `ratatui` + `crossterm`
  subscribers. Activates when stdout is a TTY AND `--no-tui` is not
  set. `--log-events events.jsonl` is *parallel* to TUI (file
  subscriber active alongside TUI). `--replay-from-cache` is
  single-transport (the recorded cache's `transport_flavour` must
  match the configured backend) — emit a helpful error on mismatch
  rather than rendering an empty TUI.

- **PR-7 (end-to-end wiring + closeout, large):** Wires AgentRuntime
  into `atlas index` via the SINGLE `Handle::block_on` boundary at
  the CLI entry point. Extends polyglot smoke with cross-transport
  parity check (claude_code vs codex). Runs Atlas-on-Atlas (no
  override file → dispatch fires) to calibrate the dispatch-overhead
  baseline; records cold token total + iteration count + wall time
  in the closeout note. Verifies polyglot fixture has full override
  coverage as Step 7.1 pre-flight (if not, surface before wiring).
  **No canonical-spec retext needed** — Phase 6 PR-5 already shipped
  the §4.3/§7/§8/§10 retext per recast §13. PR-7 only touches canon
  if Phase 7's shipped scope deviates from §10.7's headline; current
  scope matches.

## Step 4 — Phase 7 complete; consider Phase 8

When every Phase 7 PR is `[x]`:

1. Verify all checks one final time on a clean main:
   - cargo build --workspace
   - cargo test --workspace --release --no-fail-fast
   - cargo clippy --all-targets -- -D warnings
   - cargo fmt --check
   - cargo build --release --workspace
   - cargo test -p atlas-cli --test phase3_polyglot_fixture --release
     --no-fail-fast
2. Confirm PR-7's closeout note in the Phase 7 status file is
   populated with: cumulative LOC delta per PR; Atlas-on-Atlas
   baseline numbers (cold token total + iteration count + wall time);
   final commit SHAs for PR-0 through PR-7; Phase 7 → Phase 8 handoff
   text.
3. Check whether
   docs/superpowers/brainstorms/2026-05-*-atlas-vnext-phase8-brainstorm.md
   exists:
   - If YES, the user already brainstormed Phase 8. Stop the session
     and report success; the user pastes a Phase-8-shaped continuation
     prompt next.
   - If NO, the user has not yet brainstormed Phase 8. Stop the
     session, report Phase 7 success, and surface the question
     "Phase 7 is complete. Phase 8 (Cargo retirement per recast spec
     §11.2) is the next phase. Want me to brainstorm Phase 8 scope?"
4. Do NOT auto-write a Phase 8 plan; that requires user-driven
   brainstorm via superpowers:brainstorming. The recast spec captures
   architectural intent at the phase level but per-PR scope still
   needs design.

## Non-negotiables (every session, every subagent)

- **Greenfield + hard upgrade discipline.** No on-disk format
  compatibility with prior phases. No migration command. A user
  upgrading deletes .atlas/ and re-runs.
- **Brainstorm §2 decision table is canonical.** Do not relitigate
  pivots during Phase 7; surface dissent as a question to the user.
- **Tests are the gate.** Acceptance criteria in plan §5 are
  non-negotiable. Subagent must run cargo build/test/clippy/fmt clean
  before reporting DONE; orchestrator must independently re-verify
  before accepting.
- **Cumulative regression guard.** Every PR-1+ re-runs `cargo test
  -p atlas-cli --test phase3_polyglot_fixture --release
  --no-fail-fast` before flipping its checkbox. Run `cargo build
  --release --workspace` first. Do NOT pipe through tail. Cold = ~40
  (calibrated baseline); warm + reports = 0.
- **Lints and fmt clean everywhere:** fix any clippy/rustc warnings
  and cargo fmt drift encountered, even outside the code being
  touched.
- **Use the toml crate and serde_yaml** — never hand-rolled line
  scanning.
- **Workspace path-deps carry path only, no `version` field** (memory
  `feedback_no_version_on_workspace_path_deps`).
- **No iterator stubs for singletons** (memory
  `feedback_no_iterator_stubs_for_singletons`).
- **Worktree base verification.** When dispatching parallel subagents
  in Wave 2 (PR-3), pre-create the three worktrees yourself via
  `git worktree add -b ... main` and dispatch with explicit `cwd`,
  NOT `isolation: "worktree"`. After dispatch, run `git worktree
  list` and verify each new worktree's commit matches current main
  HEAD (memory `feedback_worktree_base_verification`).
- **Sync→async boundary discipline.** Engine code is sync; agent
  code is async; the sync→async boundary is the engine's call-out to
  the agent runtime in atlas-cli (single `Handle::block_on`); no
  nested block_on. PR-1 adds a clippy::disallowed_methods rule on
  `tokio::runtime::Handle::block_on` for `atlas-engine` and
  `atlas-agents/src/runtime/`.
- **No new LLM call sites outside the override-shortcircuit pattern.**
  PR-5's LLM-decided dispatch IS a new LLM call site; the
  override-file shortcircuit + polyglot fixture's full override
  coverage are the load-bearing protections that keep the cumulative
  regression guard green.
- **Do not touch mechanisms beyond what the PR's plan §4 task
  authorises.** If implementation pressure suggests a refactor,
  surface the question before doing it.
- **Commit message convention:** `phase7: PR-N <short title>`. Body
  references the plan section and lists the acceptance criteria the
  PR satisfies.
- **Two-commit PR pattern:** first commit lands code/docs; second
  commit flips status checkbox AND backfills PR-N commit SHA.

## Step 5 — When the plan and reality disagree

The Phase 7 plan was written from the brainstorm, which was written
without touching code. If a plan instruction doesn't match the
codebase (path shifted since brainstorm, function signature changed,
dep missing), prefer the brainstorm's intent and adapt the plan. If
the brainstorm is genuinely under-specified or contradicts itself,
stop and surface the question rather than improvising silently.

If you discover that a PR's scope is materially larger than the LOC
estimate (more than 2x), stop, surface the discovery, and consider
whether the PR should split before continuing. PR-1 (atlas-agents +
Tool trait + MCP + async surface) and PR-5 (fixedpoint + LLM
dispatch + Lane B) are the most likely candidates for scope creep
per plan §7.6 + §7.7.

## Workspace state

- Repo: /Users/antony/Development/Atlas (branch main).
- Phase 7 brainstorm: docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md
  (commit f4ea770 on main; the design anchor).
- Phase 7 plan: docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md
  (committed as part of PR-0; see status file for PR-0 commit SHA).
- LLM-spine recast spec: docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md
  (commit 409dcc5; the parent design spec).
- Workspace members (post-Phase-5): crates/atlas-analyzers,
  crates/atlas-engine, crates/atlas-llm, crates/atlas-reports,
  crates/atlas-cli, crates/component-ontology, crates/atlas-index,
  crates/analyzers/{python,csharp,dart,elixir,racket,lispkit},
  evaluation/harness. Phase 7 adds crates/atlas-agents (PR-1).
- Phase 3 polyglot fixture lives at
  crates/atlas-cli/tests/fixtures/phase3_polyglot/ and the smoke test
  at crates/atlas-cli/tests/phase3_polyglot_fixture.rs. Phase 7
  EXTENDS the smoke test in PR-7 (cross-transport parity check) but
  does not retire it.

## Memory state (project-scoped)

Memories live under
~/.claude/projects/-Users-antony-Development-Atlas/memory/ (which is
symlinked to .claude/memory/ in-repo per
`feedback_atlas_memory_in_repo`). The MEMORY.md index lists every
entry. Memories load-bearing for Phase 7:
- `feedback_atlas_llm_spine_intent` — strategic preference; Phase 7
  begins this inversion.
- `project_phase4_plus_roadmap` — phase ordering (Phase 7 in-progress
  → Phase 8 next).
- `feedback_cross_provider_llm_audit` — Lane B design rationale
  (PR-5).
- `project_atlas_common_backend_config` — default BackendRouter
  config (claude_code + codex) + MCP server multi-client multiplexing
  requirement (PR-1).
- `feedback_worktree_base_verification` — Wave 2 (PR-3) dispatch
  discipline.
- `feedback_no_tail_pipe_for_long_tests`,
  `feedback_release_workspace_build_for_polyglot`,
  `feedback_atlas_memory_in_repo`,
  `feedback_no_iterator_stubs_for_singletons`,
  `feedback_no_version_on_workspace_path_deps` —
  execution-discipline constraints carried forward.

Memory updates for Phase 7 land in PR-7 closeout: mark Phase 7
SHIPPED in `project_phase4_plus_roadmap`; advance Phase 8 (Cargo
retirement) to next-up; refresh MEMORY.md index entries. PR-7 may
add new memories if the empirical Atlas-on-Atlas calibration or the
cross-transport parity work surfaces a load-bearing finding.

Begin at Step 1.
```
