# Atlas vNext Phase 4 — Status

Companion to `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md`.
This file tracks per-PR completion state across sessions. The
continuation prompt at
`docs/superpowers/prompts/2026-05-09-vnext-continue.md` (Phase-4-shaped)
reads this file (via the `*phase4-plan*` wildcard match) to find the
next PR to dispatch.

**Last updated:** 2026-05-09 (PR-0 landed; PR-7 dropped after Wave 1
pre-flight grep surfaced two real callers of the alias the design
spec classified as orphan — see PR-7 note below; PR-1 landed via
cherry-pick from worktree branch `phase4-pr1`, commits `5e781d9` +
`abb7f44`). PR-2..PR-6 + PR-8 are dispatched by execution sessions
pasting the continuation prompt.

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — LenientBackend extraction (Phase 2 closeout)
- [ ] PR-2 — Decoder consolidation (Phase 2 closeout)
- [ ] PR-3 — L8 phantom-subcomponent fix (Phase 2 closeout)
- [ ] PR-4 — `atomic_write` helper convergence
- [ ] PR-5 — `build_engine_database` / `build_database_for_reports` convergence
- [ ] PR-6 — Sweep-test boilerplate consolidation
- [x] PR-7 — Orphan `pub use save_related_components_atomic` removal (atlas-contracts) — **dropped 2026-05-09**, the alias is not orphan
- [ ] PR-8 — Stale "Phase 4" prose retext + §10 renumbering in canonical system-model spec

When every box is `[x]`, Phase 4 is complete and the continuation
prompt should report success and route to the Phase 5 brainstorm
question (per validated roadmap, Phase 5 = monorepo consolidation).

## Dependency graph (informational; canonical in plan §4 + plan §9)

```
PR-0 (plan + status + continuation prompt)
  │
  ▼
PR-1 (LenientBackend extraction)              ──┐
PR-2 (decoder consolidation)                  ──┤
PR-3 (L8 phantom-subcomponent fix)            ──┤
PR-4 (atomic_write convergence)               ──┼──> PR-6 (sweep-test boilerplate; depends on PR-1 for LenientBackend re-export)
PR-5 (build_engine_database convergence)      ──┤
PR-7 (orphan re-export removal; atlas-contracts)─┤
PR-8 (spec retext + §10 renumbering)          ──┘
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (plan + status + continuation prompt; this commit).
- **Wave 1 (after PR-0):** PR-1, PR-2, PR-3, PR-4, PR-5, PR-7, PR-8 — seven PRs, all on independent surfaces. The widest practical parallel dispatch is ~3 PRs (binding constraint is reviewer attention rather than file conflicts). Suggested pairings:
  - First parallel dispatch: PR-1 (LenientBackend extraction) + PR-7 (orphan re-export deletion) — the two smallest PRs; landing them first removes obstacles for PR-6 and PR-8 sweeps.
  - Second parallel dispatch: PR-4 (atomic_write convergence) + PR-5 (build_engine_database convergence) + PR-8 (spec retext + §10 renumbering) — three medium PRs on disjoint surfaces.
  - Third parallel dispatch: PR-2 (decoder consolidation) + PR-3 (L8 phantom-subcomponent fix). Both are investigation-heavy; surface scope-creep risk early.
- **Wave 2 (after PR-1):** PR-6 (sweep-test boilerplate consolidation) — depends on PR-1 because the consolidated `sweep_support` module re-exports `atlas_engine::testing::LenientBackend`.

The Phase 3 PR-13 polyglot smoke test
(`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative
regression guard for every Phase 4 PR. Each PR's checkbox-flip step
includes a final `cargo test -p atlas-cli --test phase3_polyglot_fixture
--no-fail-fast` invocation; the strict LLM-call-budget assertions
(cold = Phase 2 PR-14 baseline ~26 calls; warm + reports = 0) catch
any drift.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of
what's worth recording: deviations from the plan that the next session
needs to know; surprising fixture quirks; manual verification steps
that succeeded; follow-up cleanup deferred; anything load-bearing for
the cumulative regression guard.

### PR-0
2026-05-09 — Landed: the Phase 4 plan
(`docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md`),
this status file
(`docs/superpowers/plans/2026-05-09-phase4-status.md`), and a
Phase-4-shaped continuation prompt at
`docs/superpowers/prompts/2026-05-09-vnext-continue.md`. The Phase 3
prompt at `docs/superpowers/prompts/2026-05-08-vnext-continue.md` is
prefixed with an `**OBSOLETE.** Superseded by …` header so a session
that pastes the wrong prompt self-corrects. Companion to the Phase 4
design spec (`docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`,
already on main from commit `f5a10e3`).

PR-1..PR-8 are dispatched by future sessions pasting the new
continuation prompt. The first execution session can dispatch the
PR-1 + PR-7 pair concurrently (smallest PRs; clears obstacles for
PR-6 and PR-8 sweeps).

Load-bearing context for Wave 1 reviewers:

- **Greenfield carries forward across phases.** No on-disk format
  compatibility with Phases 1 / 2 / 3; no migration commands. A user
  upgrading deletes `.atlas/` and re-runs.
- **No `schema_version` bump in Phase 4.** Every on-disk schema stays
  at `v1`. Phase 4 introduces zero schema mutations.
- **Phase 4 introduces zero new LLM call sites.** Cold polyglot LLM-call
  count must remain at Phase 2 PR-14 baseline; warm + reports = 0.
  Every PR re-runs the Phase 3 polyglot smoke test before flipping its
  checkbox.
- **Six-file editorial tier preserved.** Top-level `overrides`,
  `external-components`, `subsystems`, `analyzers`, `config` +
  per-component `overrides`. Phase 4 does not touch the editorial tier.
- **Atomic writes everywhere.** PR-4 (atomic_write convergence)
  preserves byte-identical durability semantics
  (temp + fsync + rename; mkdirs parent). PR-12 of Phase 3
  (atomic-write fixture suite at
  `crates/atlas-reports/tests/atomic_writes.rs`) is the regression
  guard. Run as `cargo test -p atlas-reports --test atomic_writes
  --no-fail-fast`.
- **`atlas-reports` stays pure-function.** No `fs::*` may be introduced
  inside `crates/atlas-reports/src/*`. PR-5 (build_engine_database
  convergence) touches `pipeline.rs` and `reports.rs` (CLI handlers),
  not `atlas-reports`. The Phase 5 Salsa-conversion invariant outlives
  Phase 4.
- **PR-6 depends on PR-1.** Dispatching PR-6 before PR-1 lands forces
  a temporary inline `LenientBackend` copy that PR-1 then deletes;
  cleaner to sequence them. Wave 1 / Wave 2 split in §9 of the plan
  enforces this.
- **PR-2 and PR-3 are investigation-heavy.** Both should surface scope
  before continuing if the canonical-shape or root-cause turns out
  larger than the LOC estimate (PR-2: -200 to -500; PR-3: ~20-50). Per
  the §5 reconciliation rule in the continuation prompt: a 4000-line
  surprise diff is not within tolerance.
- **PR-4's gate is the PR-12 fixture suite.** The atomic-write fixture
  suite tests durability under crash; it does NOT test error-message
  preservation. The implementer must additionally verify the
  `.with_context(...)` shape preserves the prior anyhow output by
  manual error-injection at a write-path call site. Document the
  result in the PR description.
- **PR-7 is single-line, atlas-contracts only.** The orphan
  `save_related_components_atomic` re-export at
  `atlas-contracts/crates/atlas-index/src/lib.rs:60`. Step 1 grep
  must verify zero callers across BOTH atlas-contracts AND Atlas; if
  any caller exists, STOP — the design assumed orphan status.
- **PR-8 sweeps for missed prose references.** The canonical
  system-model spec at
  `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  carries five known stale "Phase 4" references (lines 502, 981,
  1270, 1314, 1436); §10 expands from §10.4–§10.6 to §10.1–§10.11.
  The PR's step 14 sweep `grep -nE "Phase 4"` against the spec catches
  any leftover; if a missed reference surfaces in review, fix inline
  rather than deferring.

### PR-1
2026-05-09 — Landed via cherry-pick from worktree branch `phase4-pr1`.
Commits on main: `5e781d9` (extraction) + `abb7f44` (review-feedback
fix-up). Net diff: 19 files, +452/-730 (net **−278 LOC**); the +8
extra over the implementer's reported +444 is the two doc-only
review tweaks in the fix-up commit.

**Implementation summary:** `LenientBackend` lives at
`crates/atlas-engine/src/testing.rs` (240 LOC) gated
`#[cfg(any(test, feature = "test-fixtures"))]`. 13 inline duplicates
deleted across `crates/atlas-cli/tests/*.rs` (9 files) and
`crates/atlas-engine/tests/*.rs` (4 files). Three unit tests in
`atlas_engine::testing::tests` cover decline-shape + alternate
classify + per-input call log. `cargo build --release -p atlas-engine`
produces an rlib with zero `LenientBackend` symbols (verified via
`nm`).

**Plan deviations the implementer surfaced as concerns** (all triaged
before merge; recorded for forensic value):

- **Plan claimed the 13 inline copies were byte-identical except for
  whitespace — they weren't.** Copies varied semantically:
  `pipeline_integration.rs` carried unused `overrides` / `force_error`
  fields with zero call sites (verified via grep before deletion);
  `persistent_cache_lifecycle.rs` logged `Vec<(PromptId, String)>`
  (tuple form) where most logged `Vec<PromptId>`; the four
  atlas-engine polyglot tests (`l5_csharp_surface`, `l5_elixir_surface`,
  `l5_python_surface`, `multi_root_path_deps`) used non-Rust default
  classifications (`csharp-project`, `python-package`, etc.).
  Implementer canonicalised a configurable form — `new(fp)` for the
  Rust-library default + `with_classify(fp, value)` for the four
  polyglot sites + both `calls()` (returning `Vec<PromptId>`) and
  `calls_with_inputs()` (returning `Vec<(PromptId, String)>`) for
  the two consumption shapes. Both extra constructors and the second
  inspection method are *used* by ≥1 migrated test (verified during
  spec-compliance review); they are not API bloat.
- **Plan's trait-API references (`LlmBackend::classify`,
  `Outcome::Decline`) were wrong.** The actual trait is
  `LlmBackend::call(&LlmRequest) -> Result<Value, LlmError>`; the
  "decline" shape lives at `PromptId::Subcarve` returning
  `{ should_subcarve: false, sub_dirs: [], rationale: "policy declined" }`.
  Implementer mapped "decline shape" to the Subcarve canned response
  and wrote three tests instead of the spec's one (covering decline +
  classify-override + canonical-input logging).
- **Polyglot smoke test debug-mode runs were unusably slow** (>9 min
  on this machine, killed twice). Implementer ran the test in
  `--release` mode where it finished in 87.27s with all budget
  assertions passing (`cold = 40 LLM calls` within the test's
  pre-existing `< 100` bound; warm = 0; modularity = 0; divergence =
  0; impact = 0). Orchestrator confirmed via diff that PR-1's only
  change to `phase3_polyglot_fixture.rs` was rustfmt-only reflow
  (no logic, no assertion changes), so debug-mode slowness is a
  property of the polyglot test itself (multi-language tree-sitter
  + L0–L8 fixedpoint + 8-step report flow), not a PR-1 regression.
- **Plan's "~26 cold LLM calls" was stale prose.** The actual
  pre-PR-1 polyglot test asserts `cold > 0 && cold < 100` (see doc
  comment at lines 67-70 of pre-PR-1 `phase3_polyglot_fixture.rs`).
  The current cold count of 40 was the pre-PR-1 baseline; this is
  not a PR-1 regression. Future Phase 4 sessions should treat the
  test's `< 100` assertion as canonical and ignore the plan/prompt's
  "~26" number. (PR-8 may want to refresh this prose; tracking as a
  PR-8 sub-task is out of scope for this note.)

**Two-stage review:** spec-compliance reviewer ran independent
verification (read all 13 migrated files; ran `nm` on release rlib;
re-ran the polyglot smoke test in release mode; confirmed `cold=40`,
all zero-budget invariants hold) — ✅ compliant, no issues. Code-
quality reviewer found zero Critical / zero Important issues; flagged
six Minor doc/comment polish items (M1–M6) plus one hidden-coupling
note (H1, the LenientBackend-is-infallible boundary). Two of the
seven (M6, H1) were applied as the fix-up commit `abb7f44`; the
other five are forensic notes.

**Cumulative regression guard:** polyglot smoke test passes (87s
release-mode); cold = 40, warm = 0, modularity = 0, divergence = 0,
impact = 0. `cargo fmt --check` + `cargo clippy --all-targets -- -D
warnings` clean on main after cherry-pick.

**Wave-1 follow-on:** Wave 1's recommended next pairing is PR-4
(`atomic_write` convergence) + PR-5 (`build_engine_database`
convergence) + PR-8 (spec retext + §10 renumbering) — three medium
PRs on disjoint surfaces (atlas-engine cache, atlas-cli pipeline +
reports, docs). PR-2 (decoder consolidation) and PR-3 (L8 phantom-
subcomponent fix) are investigation-heavy and should land last to
avoid scope-creep blocking the rest. PR-6 (sweep-test boilerplate)
sequences after PR-1 (this commit) since `sweep_support.rs`
re-exports `atlas_engine::testing::LenientBackend`.

**Worktree mechanism note for future Phase 4 sessions:** the runtime's
`isolation: "worktree"` parameter created the first PR-1 attempt's
worktree off sha `1b3d7cd` (14 commits stale) — the failure mode
memory `feedback_worktree_base_verification` warns about. The fix
was to create the worktree manually via
`git worktree add /Users/antony/Development/Atlas-phase4-prN -b
phase4-prN <current-main-sha>` and brief the implementer to work
from that path without `isolation: "worktree"`. Adopt this pattern
for subsequent Phase 4 PRs until the runtime's stale-base bug is
resolved.

### PR-2
*Awaiting dispatch.*

### PR-3
*Awaiting dispatch.*

### PR-4
*Awaiting dispatch.*

### PR-5
*Awaiting dispatch.*

### PR-6
*Awaiting dispatch (sequenced after PR-1).*

### PR-7
2026-05-09 — **Dropped.** The Phase 4 design spec §3 PR-7 and plan §4
PR-7 classified `save_related_components_atomic` as an orphan
re-export with zero callers in either repo. PR-7's mandated step-1
sweep `grep -rn "save_related_components_atomic"
/Users/antony/Development/atlas-contracts/
/Users/antony/Development/Atlas/` (run before any edit) found two real
Atlas-side consumers introduced by Phase 3 PR-9 (impact query, commit
`0ec65c5`):

- `crates/atlas-cli/tests/atlas_impact_cli.rs:15`
  `use atlas_index::{save_components_atomic, save_related_components_atomic};`
- `crates/atlas-cli/tests/atlas_impact_cli.rs:111`
  `save_related_components_atomic(&cache_dir.join("related-components.yaml"), &related).unwrap();`

The renamed alias is a deliberate, descriptive disambiguator at the
test call-site (`save_atomic` is the generic helper exported under
the same `pub use` block from `component_ontology`; the renamed
alias makes "this saves related-components.yaml specifically"
locally obvious). It is not orphan — Phase 3 PR-9 added these
callers after the Phase 3 §9.1 deferred-list was frozen, and the
Phase 4 design (drafted 2026-05-09) inherited the stale "orphan"
classification without re-running the grep.

Per the Phase 4 plan's PR-7 step 1 ("If any caller exists, STOP and
surface it — the design assumed zero callers") and the continuation
prompt §5 ("If a plan instruction doesn't match the codebase…"),
PR-7 is dropped from Phase 4 rather than expanding scope to rewrite
the Atlas-side callers. The alias stays; nothing is deleted; nothing
is renamed.

Acceptance: PR-7 contributes zero LOC delta to Phase 4. Phase 4
ships 7 code/docs PRs (PR-1..PR-6 + PR-8) instead of 8. The Phase 4
closeout note in this file should record the final cumulative LOC
delta accordingly.

No design-spec retext is required for this drop — the Phase 4 design
spec (§3 PR-7 + §6 §10.4 row) and plan (§4 PR-7 + §5 acceptance
table) become forensically inaccurate from 2026-05-09 onward; future
readers should treat this status-file note as the canonical
disposition. If a Phase 5+ session wants to retext the Phase 4 spec,
that's a one-line strikethrough at design §3 PR-7 — out of scope for
the current arc.

### PR-8
*Awaiting dispatch.*
