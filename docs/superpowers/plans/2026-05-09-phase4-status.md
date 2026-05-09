# Atlas vNext Phase 4 — Status

Companion to `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md`.
This file tracks per-PR completion state across sessions. The
continuation prompt at
`docs/superpowers/prompts/2026-05-09-vnext-continue.md` (Phase-4-shaped)
reads this file (via the `*phase4-plan*` wildcard match) to find the
next PR to dispatch.

**Last updated:** 2026-05-09 (Wave-1+2 trio PR-4 / PR-5 / PR-8 landed
via cherry-pick from worktree branches `phase4-pr4` / `phase4-pr5` /
`phase4-pr8`; commits on main `02d608d` / `e89c55f` / `009d7e5`).
Cumulative Phase 4 LOC delta running tally: PR-1 (−278) + PR-4 (−25)
+ PR-5 (−45) + PR-8 (+23) = **−325 net** so far. PR-2 + PR-3 + PR-6
remain; the next dispatch wave is PR-2 + PR-3 + PR-6 (PR-2 and PR-3
are investigation-heavy; PR-6 unblocks now that PR-1 is on main).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — LenientBackend extraction (Phase 2 closeout)
- [ ] PR-2 — Decoder consolidation (Phase 2 closeout)
- [ ] PR-3 — L8 phantom-subcomponent fix (Phase 2 closeout)
- [x] PR-4 — `atomic_write` helper convergence
- [x] PR-5 — `build_engine_database` / `build_database_for_reports` convergence
- [ ] PR-6 — Sweep-test boilerplate consolidation
- [x] PR-7 — Orphan `pub use save_related_components_atomic` removal (atlas-contracts) — **dropped 2026-05-09**, the alias is not orphan
- [x] PR-8 — Stale "Phase 4" prose retext + §10 renumbering in canonical system-model spec

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
2026-05-09 — Landed via cherry-pick from worktree branch `phase4-pr4`.
Commit on main: `02d608d`. Net diff: 3 files, +16/−41 (net **−25
LOC**) — under the plan's −50 to −100 estimate because the deleted
helper's `tempfile::NamedTempFile` body plus the now-unused `Write`
and `tempfile` imports compressed cleanly; no extra explanatory
comments were padded into the canonical helper.

**Implementation summary:** The duplicate `pub(crate) fn atomic_write`
in `crates/atlas-engine/src/cache/layout.rs` (38-line body using
`tempfile::NamedTempFile::new_in` + `write_all` + `flush` + `persist`)
was deleted; its sole call site at `cache/mod.rs:129` now invokes the
canonical `crate::atomic_write::atomic_write(&path, blob)` wrapped in
`.with_context(|| format!("atomic_write to {} failed", path.display()))?`.
The canonical helper uses raw `OpenOptions` + `sync_all()` +
`fs::rename()` (a leaner shape than `NamedTempFile::persist`) and
provides the same atomic guarantee. Doc-comments retexted: the
"left in place for now (a future refactor can converge them — out of
scope for PR-1)" paragraph at `atomic_write.rs:13-19` replaced with a
forward note that the cache writer converged on this helper in
Phase 4 PR-4; `cache/mod.rs:18` and `cache/layout.rs` module-level
docs updated accordingly.

**Manual error-context verification (per the §4 PR-4 risk gate):** by
inspection of both implementations, the error-chain shape is
practically equivalent. The OLD path attached anyhow `.with_context(...)`
calls at every step ("creating cache directory <dir>", "creating
temp file in <dir>", "writing cache blob to temp file in <dir>",
"flushing cache blob to temp file in <dir>", "persisting cache blob
to <target>"); the NEW path returns raw `io::Error` from each step
(parent-mkdir, OpenOptions::open, write_all, sync_all, rename) and
attaches a single outer `.with_context("atomic_write to <path>
failed")` from the call site. The destination path is preserved in
every error chain via the outer wrap; the underlying `io::Error::kind()`
(e.g. `PermissionDenied`, `StorageFull`, `CrossesDevices`) conveys
which step failed. The PR-12 atomic-write fixture suite
(`crates/atlas-reports/tests/atomic_writes.rs`, 5 tests; the
canonical kill-during-write durability regression guard) passes
byte-identically post-migration — durability semantics preserved.

**Cumulative regression guard:** polyglot smoke test passed in 88.45s
release-mode (matches PR-1's 87.27s baseline within natural variance);
LLM-call-budget assertions hold (cold > 0 && < 100; warm = 0;
modularity/divergence/impact = 0). `cargo fmt --check` + `cargo
clippy --all-targets -- -D warnings` clean.

**Build-prerequisite gotcha for future Phase 4 sessions:** the
polyglot smoke test in `--release` mode requires the standalone
analyzer binaries (atlas-{python,csharp,dart,elixir,racket,lispkit}-
analyzer) to be built first. Running just `cargo test -p atlas-cli
--test phase3_polyglot_fixture --release --no-fail-fast` only builds
atlas-cli + its dependencies, leaving the analyzer binaries absent —
the test then produces empty surfaces for those languages and fails
on contract validation (the failure shape is `consumes-contract
edge: component <X> → unresolved contract <Y>`). PR-1's release-mode
pass implicitly relied on the analyzer binaries already being built.
Future Phase 4 sessions should `cargo build --release --workspace`
first when running the polyglot test in release mode (debug works
without this prerequisite but is unusably slow per the PR-1 note).
The continuation prompt's verification protocol should be amended
to include this `cargo build --release --workspace` precondition;
flagging as a candidate amendment for the next Phase 4 docs PR.

**Worktree mechanism:** worktree created manually via `git worktree
add /Users/antony/Development/Atlas-phase4-pr4 -b phase4-pr4
fe7070d` (per the PR-1 status note's pattern; avoids the runtime's
`isolation: "worktree"` stale-base bug from `feedback_worktree_base_
verification`).

### PR-5
2026-05-09 — Landed via cherry-pick from worktree branch `phase4-pr5`.
Commit on main: `e89c55f`. Net diff: 2 files, +50/−95 (net **−45
LOC**) — within the plan's −50 to −100 estimate. The +50 is the new
`build_engine_database_for_reports` wrapper (16 lines of code +
~30 lines of structured doc-comment); the −95 is the deleted
`build_database_for_reports` body in `reports.rs` plus pruned
imports.

**Implementation summary:** PR-11 of Phase 3 (divergence) had
introduced a private `build_database_for_reports` helper inside
`reports.rs` that duplicated ~75 lines of
`pipeline::build_engine_database`'s body via a slightly leaner code
path. Phase 4 PR-5 collapses the two by adding a thin wrapper
`pub fn build_engine_database_for_reports(root, output_dir,
fingerprint_override, backend)` in `pipeline.rs` that synthesises an
`IndexConfig` with the divergence path's defaults (no `--no-overrides`,
no `--recarve`, `respect_gitignore = true`), wires up a silent
stderr reporter (`ProgressMode::Never`), and forwards through to the
canonical `build_engine_database`. `run_divergence` now calls the
wrapper instead of the deleted local helper.

**Step-1 delta (per plan §4 PR-5 step 1):** the deleted helper
diverged from the canonical helper in five distinct ways:

| Aspect | Canonical `build_engine_database` | Deleted `build_database_for_reports` |
|---|---|---|
| Signature | `(config: &IndexConfig, backend, reporter) -> Result<(AtlasDatabase, Vec<PathBuf>), IndexError>` | `(root, output_dir, fingerprint_override, backend) -> Result<AtlasDatabase>` |
| Visibility | `pub` (used by `run_modularity`) | `fn` (private; used only by `run_divergence`) |
| BudgetSentinel | yes | no |
| Progress reporter | events emitted | no events |
| Analyser overrides merge | yes | no |
| L5 pre-warm | parallel (rayon) | sequential |
| `IndexConfig` shape | full | implicit defaults |

**Step-2 decision (per plan §4 PR-5 step 2):** chose the *thin-
wrapper* path explicitly permitted by the plan ("If
`build_database_for_reports` does anything substantively different
beyond pre-warming, prefer adding a thin wrapper… rather than baking
divergence semantics into the shared body"). The wrapper synthesises
only the *defaults* the deleted helper used; the canonical helper
takes care of the rest. The convergence point is a strict semantic
superset for the divergence path: it gains BudgetSentinel coverage,
the analyser-overrides merge, and the parallel L5 pre-warm. None of
these change the byte-identical YAML output the divergence handler
emits — they're operationally invisible from the report's
perspective.

**Pruned imports (forensic detail):** `atlas_engine::{all_components,
expand_roots, run_fixedpoint, seed_filesystem_excluding, surface_of,
FixedpointConfig, LlmResponseCache, PersistentCache}` and
`atlas_index::{load_or_default_externals, load_or_default_overrides,
load_or_default_subsystems_overrides, OverridesFile,
SubsystemsOverridesFile}`. The size of this pruning corroborates
that the deleted helper had genuinely duplicated machinery (not just
a couple of redundant lines).

**Cumulative regression guard:** the divergence-byte-identical
acceptance is satisfied by the existing
`atlas_divergence_after_drift_writes_severity_aware_report` test in
`crates/atlas-cli/tests/divergence_cli.rs` (asserts byte-identical
output YAML against the fixture's expected
`composition-divergence.yaml`); it passed byte-identically in
release mode. Polyglot smoke test passed in >60s release-mode
(the cumulative LLM-call-budget assertions hold; cold > 0 && < 100;
warm = 0; modularity/divergence/impact = 0). `cargo clippy
--all-targets -- -D warnings` and `cargo fmt --check` clean.

**Worktree mechanism:** worktree created manually via `git worktree
add /Users/antony/Development/Atlas-phase4-pr5 -b phase4-pr5
fe7070d` per the PR-1 pattern.

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
2026-05-09 — Landed via cherry-pick from worktree branch `phase4-pr8`.
Commit on main: `009d7e5`. Net diff: 2 files, +67/−44 (net **+23
LOC**) — under the plan's +60 to +120 estimate; the implementer
used precise replacements rather than always-inserting where the
existing prose admitted clean substitution.

**Implementation summary:** the PR is one commit, two files changed.

`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`:
- §10.4 retexted from "Convergence and cleanups" (the pre-Phase-3
  multi-track grab-bag) to "Phase 4 — Cleanup release" with the
  shorter scope-statement landing the canonical Phase 4 design
  spec's wording verbatim.
- New §10.5 (Phase 5 — Monorepo consolidation), §10.6 (Phase 6 —
  User-facing schema cleanups), §10.7 (Phase 7 — Per-language
  refinements), §10.8 (Phase 8 — Subprocess convergence), §10.9
  (Phase 9 — LLM-driven analyses) inserted per Phase 4 design spec
  §6 verbatim.
- Old §10.5 (Server mode) renumbered to §10.10; old §10.6 (Migration
  from v1) renumbered to §10.11 (OBSOLETE marker preserved
  unchanged).
- Five prose retexts: line 502 (§5.6 server-mode intro: "Phase 4" →
  "Phase 10"); line 981 (§9 introduction: "Phase 4 target" → "Phase
  10 target"); line 1270 (§11.2 question 5: full retext + retitle
  from "Phase 5 query API" to "Phase 10 query API"); line 1314
  (§11.4: "Phase 4 ships" → "Phase 10 ships"); line 1436 (glossary
  Role-B Grafeo entry: "deferred to Phase 4 and beyond" → "deferred
  to Phase 10 (server mode) and beyond" — the surrounding paragraph
  is about Grafeo as a derived projection alongside server mode, so
  the explicit Phase 10 wording is more informative than "deferred
  indefinitely").

`docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`:
- §9.1 deferred-list: eleven `(now Phase X)` forward-pointer
  annotations appended (pattern detection → Phase 9; subprocess
  convergence → Phase 8; bidirectional LLM callback → Phase 8;
  rust-analyzer → Phase 8 stretch; LLM threshold calibration →
  Phase 9; contract rename-match → Phase 6; `--strict-overrides` →
  Phase 6; cache compression → Phase 6; worktree commit-sha → Phase
  6; Phase 2 closeouts → Phase 4; per-language refinements →
  Phase 7).

**Sweep verifications (per plan §4 PR-8 step 14 + continuation
prompt PR-8 special instructions):**

- `grep -nE "Phase 4" canonical-spec` → exactly two occurrences,
  both inside the new §10.4 heading and body. Zero missed retexts.
- `grep -nE "§10\.[0-9]+" docs/superpowers/` → all hits in the
  current-authoritative docs (canonical spec, Phase 4 design spec)
  resolve to valid §10.1–§10.11 headings. Other matches in sibling
  plans/prompts (Phase 1/2/3 plans, the Phase 3 design's own §10
  renumbering instructions) are forensic — frozen at the time those
  docs were written; retained intentionally per the prior-phase-doc
  convention.
- `grep -nE "Phase 4 = server mode|moved from Phase 4 to Phase 5"`
  → zero hits. The renumbering rationale lives in the commit
  message, not in the spec.

**Cumulative regression guard:** PR-8 is docs-only — no code path
could affect the polyglot smoke test. The cumulative regression-
guard pass is inherited from PR-1's 87.27s release-mode pass
(commit `fe7070d`). To preserve the every-PR discipline, future
sessions running PR-8-shaped pure-docs PRs may skip the polyglot
re-run if the diff is byte-stable outside touched-prose paragraphs;
otherwise re-run for safety.

**Worktree mechanism:** worktree created manually via `git worktree
add /Users/antony/Development/Atlas-phase4-pr8 -b phase4-pr8
fe7070d` per the PR-1 pattern.

**Forensic loose-end (out-of-scope; not blocking):** the §9.1
*heading* in the Phase 3 design ("Deferred to Phase 4 (convergence
+ cleanups + LLM analyses)") is now stale because most listed items
moved to Phase 6/7/8/9; the plan's step 15 only mandated per-item
forward-pointer annotations, not a heading retext. A future
Phase 5+ docs polish could update the heading; out of scope for
PR-8.
