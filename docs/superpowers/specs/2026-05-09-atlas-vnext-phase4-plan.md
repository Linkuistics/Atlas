# Atlas vNext Phase 4 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. The
> Phase 4 status file at
> `docs/superpowers/plans/2026-05-09-phase4-status.md` carries the
> per-PR checkbox state across sessions.

**Status:** Plan (forward-looking; Phase 4 of the Atlas vNext system-model
redesign). Companion to `2026-05-09-atlas-vnext-phase4-design.md` (the
canonical Phase 4 design spec, on main as commit `f5a10e3`). Sequel to
`2026-05-08-atlas-vnext-phase3-plan.md` (Phase 3 closed 2026-05-08;
status in `docs/superpowers/plans/2026-05-08-phase3-status.md`).

**Date:** 2026-05-09.

**Treatment:** Greenfield, carried forward from Phases 1, 2, and 3. No
on-disk format compatibility with prior phases. No migration command. A
user upgrading deletes `.atlas/` and re-runs. `schema_version` stays at
`1` across the entire phase — Phase 4 introduces no schema mutations.

**Goal:** Decompose Phase 4 (§10.4 of the design spec, the **cleanup
release**) into an ordered sequence of independently-mergeable PRs that
land seven internal-quality cleanups (Phases 2 and 3 closeouts +
helper convergence + sweep-test consolidation + orphan-symbol removal)
and a one-shot canonical-spec retext that aligns the system-model
design with the validated post-Phase-3 phase ordering. **No new
user-facing capability, no schema change, no LLM call sites.** Cold
polyglot LLM-call count must remain at the Phase 2 PR-14 baseline; PR-13
of Phase 3 is the regression guard, re-run after every Phase 4 PR.

**Architecture:** Phase 4 is a *cleanup release*, not a feature release.
Eight code/docs PRs touching disjoint surfaces, mostly delete-duplicates
or extract-shared-helpers; net LOC is negative. The Phase 3 polyglot
smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the
cumulative regression guard — every Phase 4 PR re-runs it before
flipping the status checkbox. PR-12 of Phase 3 (atomic-write fixture
suite) is the regression guard for PR-4 (atomic_write convergence)
specifically.

**Tech Stack:** Rust workspace; Salsa engine (carried unchanged);
`serde_yaml` for all YAML I/O; `toml` crate for any TOML reads
(memory `feedback_toml_parsing`); existing `atlas-engine` /
`atlas-analyzers` / `atlas-cli` / `atlas-reports` crates extended; no
new crates introduced.

---

## 0. Reading order

Before this plan, read:

1. `2026-05-09-atlas-vnext-phase4-design.md` end-to-end. The design
   spec is the canonical source of scope, PR boundaries, acceptance,
   and the §6 roadmap table that PR-8 lands verbatim. **This plan
   operationalises that design; where the two disagree, the design
   spec wins.**
2. `2026-05-08-atlas-vnext-phase3-plan.md` §3 (mechanisms reused) and
   §4 PR-12 / PR-13 sub-sections (atomic-write fixture suite + polyglot
   smoke test — both are Phase 4 regression guards), and §5 acceptance
   summary (Phase 4 mirrors its per-PR-table discipline).
3. `docs/superpowers/plans/2026-05-08-phase3-status.md` — particularly
   the closeout notes (the `## Phase 3 — complete` section starting
   line ~1300) which enumerate the Phase 4 cleanup candidates with
   surrounding context: the four `phase3_retrofit_*.rs` boilerplate
   sites, the `build_engine_database` / `build_database_for_reports`
   near-duplicate, the orphan `save_related_components_atomic`
   re-export, and the four stale "Phase 4" prose references in the
   canonical system-model spec.
4. `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
   — particularly §10 (current state: §10.1–§10.6, with §10.4 carrying
   the *old* "Phase 4 — Convergence and cleanups" definition and §10.5
   carrying *server mode*); PR-8 of this plan lands the new §10
   structure (§10.1–§10.11) verbatim from the Phase 4 design spec §6.
5. Memory entries that constrain Phase 4 (per design §11): if a listed
   memory file is missing locally, treat the design spec's one-line
   summary as the constraint and proceed:
   - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
   - `feedback_fix_all_lints` — every PR runs `cargo clippy
     --all-targets -- -D warnings` and `cargo fmt --check` clean.
   - `project_phase4_plus_roadmap` — the validated phase ordering
     Phase 4 slots into; informs §10.5–§10.10 ordering in the PR-8
     retext.

This plan does *not* re-derive scope; it sequences and grounds it. The
PR boundaries, acceptance criteria, and roadmap table are all in the
design spec.

---

## 1. Phase 4 deliverable, restated

End of Phase 4, the Atlas codebase shall exhibit the following
internal-quality properties without changing any user-observable
behaviour:

- **No duplicate `LenientBackend` test stub.** Every Atlas integration
  test that previously declared an inline `LenientBackend` struct
  (13 files in `crates/atlas-cli/tests/` and
  `crates/atlas-engine/tests/`, verified 2026-05-09) imports a single
  shared definition.
  The shared definition is gated so it does not exist in release
  builds (`cargo build --release` clean, no test-fixture symbols
  leaked).
- **No duplicate decoder shape across language analysers.** Per-language
  decoders share a single canonical helper. Net LOC across
  `crates/atlas-analyzers/` is reduced; per-language fixture outputs
  remain byte-identical.
- **L8 phantom-subcomponent fix landed.** A previously-broken edge
  case in subsystem composition (where phantom subcomponents are
  emitted) is fixed and pinned with a regression test. Existing L8
  tests continue to pass.
- **One canonical `atomic_write` helper.** `atlas_engine::atomic_write`
  (the `pub` helper at `crates/atlas-engine/src/atomic_write.rs:40`,
  `io::Result`) is the only atomic-write helper in the engine. The
  duplicate `cache::layout::atomic_write` (the `pub(crate)` helper at
  `crates/atlas-engine/src/cache/layout.rs:84`, `anyhow::Result`) is
  deleted, with its sole call site (`cache/mod.rs:129`) migrated to
  `crate::atomic_write::atomic_write` wrapped with `.with_context(...)`
  to preserve anyhow's error context. The PR-12 atomic-write fixture
  suite continues to pass byte-identically.
- **One canonical engine-database build helper.** `pipeline.rs::
  build_engine_database` (`crates/atlas-cli/src/pipeline.rs:761`) and
  the private `reports.rs::build_database_for_reports`
  (`crates/atlas-cli/src/reports.rs:978`) are converged onto a single
  shared inner helper in `pipeline.rs`. Both `run_modularity` (PR-10
  of Phase 3) and `run_divergence` (PR-11 of Phase 3) call it. atlas-cli
  test outputs are byte-identical pre/post.
- **Sweep-test boilerplate centralised.** The four
  `phase3_retrofit_*.rs` integration tests
  (`crates/atlas-cli/tests/phase3_retrofit_{surfaces,component,components,related}.rs`)
  no longer carry inline copies of the fixture-build boilerplate
  (`materialise_fixture`, `base_config`, `LenientBackend` /
  `SweepBackend`, `tiny_fixture_root`, `copy_dir_all`, `run_with`).
  A shared `crates/atlas-cli/tests/common/sweep_support.rs` (or
  equivalent module path under `tests/common/`) is the canonical home;
  the four retrofit tests import from it. Net LOC negative across the
  five files.
- **No orphan `pub use save_related_components_atomic`.** The orphan
  re-export at `atlas-contracts/crates/atlas-index/src/lib.rs:60` is
  deleted. Both `cargo build --workspace` (atlas-contracts) and
  `cargo build --workspace` (Atlas) remain clean.
- **Canonical system-model spec aligned with the validated phase
  ordering.** §10 of `2026-05-06-atlas-system-model-design.md` reads as
  the new §10.1–§10.11 table (per Phase 4 design spec §6), and the
  four stale "Phase 4 = server mode" prose references (§5.6 line 502;
  §9 / §10.5 prose around line 981; §11.4 line ~1314; glossary line
  ~1436) are retexted to either "Phase 10" or "server mode" without
  the obsolete phase number. Inbound cross-references to old §10.4 /
  §10.5 are updated in the same commit. The Phase 3 design's §9.1
  deferred-list gets a forward-pointer update so each deferred item
  lists its destination phase.

Every report runs deterministically; the cold and warm LLM call
budgets match Phase 2's PR-14 baseline (zero new LLM call sites —
design §1, non-negotiable). `schema_version` remains at `1` for every
on-disk schema. The six-file editorial tier is preserved.

Out of scope for Phase 4, deferred to later phases per the validated
roadmap (memory `project_phase4_plus_roadmap`, design §6):

- **Phase 5** — monorepo consolidation: atlas-contracts in-tree, fold
  Ravel + Ravel-Lite, delete multi-root machinery.
- **Phase 6** — user-facing schema cleanups: contract rename-match,
  `--strict-overrides`, cache compression, worktree commit-sha
  annotations, `is_manifest_file` extension for Makefile/shell,
  `subsystem` field wiring (deferred from Phase 3 PR-9), `edges_suppress`
  no-match stderr-capture test (deferred from Phase 3 PR-10).
- **Phase 7** — per-language refinements: full tree-sitter-dart,
  raco-driven Racket dep resolution, Phoenix sub-kinds for Elixir,
  Mix umbrella decomposition, LispKit `(import …)` symbolic resolution.
- **Phase 8** — subprocess convergence: migrate Cargo / Dockerfile /
  RustSurface / LlmClassify / TS-as-subprocess to subprocess;
  bidirectional LLM callback channel; rust-analyzer integration
  (stretch).
- **Phase 9** — LLM-driven analyses: pattern detection (recurring
  component / edge shapes); LLM confidence threshold calibration
  (§11.2.6).
- **Phase 10** — server mode: file watcher + Salsa input updates;
  gRPC + GraphQL API; subscription primitives; reactive recomputation;
  CLI as thin client.
- **Deferred indefinitely** — gate/strict exit-code flags, modularity-
  score thresholds, upstream / subsystem-input impact variants,
  modularity history depth >5, per-language coupling normalisation,
  multi-tenant SaaS hosting (per Phase 3 §9.3).

---

## 2. Open-question pre-conditions

Phase 4 introduces no new open questions on the design-spec-§11.2
docket and resolves none. The §11.2.5 entry (Phase 5 query API auth) is
retexted in PR-8 to refer to the new Phase 10 (server mode is now §10.10
under the new ordering); the open question itself remains open and
deferred to Phase 10.

The §11.2.9 entry (editorial-vs-derived classification) was resolved in
Phase 3 PR-0b and remains resolved; no Phase 4 work touches it.

---

## 3. Phase 1 + Phase 2 + Phase 3 mechanisms reused as starting points

These mechanisms are extended or simplified, not rewritten. Greenfield
applies: any Phase 4 PR is free to delete, rename, or restructure when
the cleanup demands it.

| Mechanism | Location | How Phase 4 uses it |
|---|---|---|
| `atomic_write` (canonical) | `crates/atlas-engine/src/atomic_write.rs` (`pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()>` at line 40) | PR-4 makes this the *only* atomic-write helper in the engine: deletes the duplicate `cache::layout::atomic_write` and migrates its sole caller. |
| `cache::layout::atomic_write` (duplicate) | `crates/atlas-engine/src/cache/layout.rs` (`pub(crate) fn atomic_write(target: &Path, blob: &[u8]) -> anyhow::Result<()>` at line 84) | Deleted in PR-4; its single caller (`cache/mod.rs:129`) is rewritten to call the canonical helper with `.with_context(...)` for anyhow error preservation. |
| Atomic-write fixture suite | `crates/atlas-reports/tests/atomic_writes.rs` (Phase 3 PR-12; housed in `atlas-reports` because that crate's `dev-dependencies` carry the `atomic_write_panic_after_temp` feature flag) | Re-runs as PR-4's gate: kill-during-write and after-rename hooks must continue to pass byte-identically. PR-4 does NOT extend the suite. Run as `cargo test -p atlas-reports --test atomic_writes --no-fail-fast`. |
| `build_engine_database` | `crates/atlas-cli/src/pipeline.rs:761` (Phase 3 PR-10's pipeline helper — public, used by `run_modularity`) | PR-5 either keeps this name as the canonical helper or extracts a shared inner `build_database_inner(...)` and re-implements `build_engine_database` as a one-line wrapper; implementer's choice. |
| `build_database_for_reports` | `crates/atlas-cli/src/reports.rs:978` (Phase 3 PR-11's private helper — used by `run_divergence`) | PR-5 deletes this in favour of calling into the canonical helper from `pipeline.rs`. |
| `phase3_retrofit_*.rs` | `crates/atlas-cli/tests/phase3_retrofit_{surfaces,component,components,related}.rs` (Phase 3 PR-2..PR-5 sweep tests) | PR-6 leaves the test bodies in place and lifts the ~100-LoC fixture-build boilerplate (`materialise_fixture`, `base_config`, `LenientBackend` / `SweepBackend`, `tiny_fixture_root`, `copy_dir_all`, `run_with`) into a shared `crates/atlas-cli/tests/common/sweep_support.rs` module. |
| `LenientBackend` test stub | Inline in 13 test files (verified 2026-05-09): `crates/atlas-cli/tests/{atlas_drift,atlas_modularity,divergence_cli,persistent_cache_lifecycle,phase3_overrides_edges,phase3_retrofit_components,pipeline_integration,scattered_atlas_layout,surfaces_emission_rust}.rs` and `crates/atlas-engine/tests/{l5_csharp_surface,l5_elixir_surface,l5_python_surface,multi_root_path_deps}.rs` | PR-1 extracts to a single shared definition under `crates/atlas-engine/src/testing.rs` gated `#[cfg(any(test, feature = "test-fixtures"))]`; every duplicating test imports from there. PR-6's sweep_support module re-exports for convenience (the four `phase3_retrofit_*.rs` files reference `LenientBackend` via shared boilerplate that PR-6 lifts). |
| `save_atomic as save_related_components_atomic` | `atlas-contracts/crates/atlas-index/src/lib.rs:60` (orphan re-export from Phase 3 PR-5 closeout) | PR-7 deletes this single line. The underlying `save_atomic` symbol stays; only the renamed re-export is removed. |
| Phase 3 polyglot smoke test | `crates/atlas-cli/tests/phase3_polyglot_fixture.rs` (Phase 3 PR-13) | Cumulative regression guard for every Phase 4 PR. Each PR runs `cargo test -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast` before flipping its status checkbox. The strict LLM-call-budget assertions (cold = Phase 2 baseline; warm + reports = 0) catch any drift. |
| `2026-05-06-atlas-system-model-design.md` §10 | `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` lines 1161–1222 (current §10.4–§10.6) | PR-8 expands §10.4 to carry only the new "Phase 4 — Cleanup release" entry, inserts new §10.5–§10.9 (consolidation / schema cleanups / per-language refinements / subprocess / LLM analyses), renumbers old §10.5 → §10.10 (server mode) and old §10.6 → §10.11 (migration; OBSOLETE marker preserved). All §6 roadmap-table content lands verbatim. |

---

## 4. PR sequence

PRs are numbered in dependency order. Sizes are estimates excluding
tests and excluding generated code. Each PR ends with passing
`cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
and `cargo fmt --check`. Each PR additionally re-runs the Phase 3
polyglot smoke test (`cargo test -p atlas-cli --test
phase3_polyglot_fixture --no-fail-fast`) and asserts the LLM-call
budgets are unchanged from the Phase 2 PR-14 baseline (cold ≈ 26 calls;
warm and report runs = 0).

### PR-0 — Plan + status + continuation prompt (no code)

**Intent:** Land this plan, the Phase 4 status file, and a
Phase-4-shaped continuation prompt at
`docs/superpowers/prompts/2026-05-09-vnext-continue.md` that supersedes
the Phase 3 prompt at `docs/superpowers/prompts/2026-05-08-vnext-continue.md`.
Establishes the planning infrastructure that the new continuation
prompt keys off of (`*phase4-plan*` wildcard match → execution mode).

**Files:**
- Create: `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md` (this file).
- Create: `docs/superpowers/plans/2026-05-09-phase4-status.md` (PR checklist + dependency graph + per-PR notes section, seeded with PR-0's note only).
- Create: `docs/superpowers/prompts/2026-05-09-vnext-continue.md` (Phase-4-shaped continuation prompt; mirrors the structure of `2026-05-08-vnext-continue.md` with paths/scope rewritten for Phase 4).
- Modify: `docs/superpowers/prompts/2026-05-08-vnext-continue.md` — prepend a one-paragraph "Obsolete; superseded by `2026-05-09-vnext-continue.md`" header so a future operator pasting it gets routed to the current prompt. Do NOT delete the file (forensic value preserved per Phase 3's treatment of the Phase-2 prompt).

**Acceptance criteria:**
- All four file changes land in a single commit with message `phase4: PR-0 plan + status + continuation prompt`.
- The status file contains 9 PR checkboxes (PR-0..PR-8), all `[ ]` except PR-0 which is `[x]` after this commit.
- The new continuation prompt's wildcard matcher at Step 1 detects `*phase4-plan*` and routes to execution.
- The Phase 3 prompt's first non-frontmatter paragraph reads "**OBSOLETE.** Superseded by …" so a session that pastes the wrong prompt self-corrects.
- `git diff --stat` for this commit lists exactly four files: the three new docs + the one-paragraph touch-up to the Phase 3 prompt.

**LOC:** 0 code; ~700-900 lines (plan ~600; status ~80; continuation prompt ~250; Phase 3 prompt obsolescence header ~5).

---

### PR-1 — `LenientBackend` extraction

**Intent:** Phase 2 closeout. The `LenientBackend` test stub is
duplicated inline across 13 integration test files (enumerated in §3,
verified 2026-05-09).
Extract to a single shared location so adding a new test doesn't
require copy-pasting the struct + impl + LlmBackend trait
implementation.

**Files (in `crates/atlas-engine/src/`):**
- Create: `testing.rs` — `pub struct LenientBackend { ... }` plus its
  inherent `impl LenientBackend` (constructor) and `impl LlmBackend
  for LenientBackend`. The exact type signature is copy-and-canonicalise
  from any of the existing duplicates (they are byte-identical except
  for whitespace; pick the most recent one,
  `crates/atlas-cli/tests/atlas_drift.rs`'s copy, as canonical).
- Modify: `lib.rs` — at the bottom of the file, add:
  ```rust
  #[cfg(any(test, feature = "test-fixtures"))]
  pub mod testing;
  ```
  The feature gate ensures `LenientBackend` does not exist in release
  builds. Add `test-fixtures = []` to `Cargo.toml`'s `[features]`
  section.

**Files (in `crates/atlas-engine/Cargo.toml`):**
- Modify: under `[features]`, add `test-fixtures = []`.

**Files (in `crates/atlas-cli/Cargo.toml`):**
- Modify: under `[dev-dependencies]`, add the `test-fixtures` feature
  to the existing `atlas-engine` dev-dep entry:
  ```toml
  atlas-engine = { path = "../atlas-engine", features = ["test-fixtures"] }
  ```

**Files (deleting inline duplicates):**
- Modify: each of the following test files. Delete the inline
  `struct LenientBackend`, the `impl LenientBackend`, and the `impl
  LlmBackend for LenientBackend`. Add `use atlas_engine::testing::
  LenientBackend;` to the use-block. Verify no other test in the file
  references inline-only state of the struct (none do — the struct is
  stateless aside from the constructor).

  Verified list (13 files; obtained 2026-05-09 via
  `grep -rln "^struct LenientBackend" crates/atlas-cli/tests/ crates/atlas-engine/tests/`):
  - `crates/atlas-cli/tests/atlas_drift.rs`
  - `crates/atlas-cli/tests/atlas_modularity.rs`
  - `crates/atlas-cli/tests/divergence_cli.rs`
  - `crates/atlas-cli/tests/persistent_cache_lifecycle.rs`
  - `crates/atlas-cli/tests/phase3_overrides_edges.rs`
  - `crates/atlas-cli/tests/phase3_retrofit_components.rs`
  - `crates/atlas-cli/tests/pipeline_integration.rs`
  - `crates/atlas-cli/tests/scattered_atlas_layout.rs`
  - `crates/atlas-cli/tests/surfaces_emission_rust.rs`
  - `crates/atlas-engine/tests/l5_csharp_surface.rs`
  - `crates/atlas-engine/tests/l5_elixir_surface.rs`
  - `crates/atlas-engine/tests/l5_python_surface.rs`
  - `crates/atlas-engine/tests/multi_root_path_deps.rs`

  Re-run `grep -rln "^struct LenientBackend" crates/` immediately
  before starting the edit pass to detect any new test files added
  since this plan was written.

- Investigate (likely-call-sites that don't currently declare a
  struct but may import `LenientBackend` indirectly): the four
  `phase3_retrofit_*.rs` test files (`surfaces`, `component`,
  `components`, `related`) — `components` declares its own struct;
  the other three reference `LenientBackend` via shared boilerplate
  that PR-6 lifts into `tests/common/sweep_support.rs`. PR-1 may
  leave the three retrofit files alone (they pick up the canonical
  `LenientBackend` automatically once PR-6 has the shared module
  re-export it from `atlas_engine::testing`). Verify with `grep -n
  "LenientBackend" crates/atlas-cli/tests/phase3_retrofit_*.rs`
  before deciding whether to touch them in PR-1 or defer to PR-6.

**Acceptance criteria:**
- New unit test in `crates/atlas-engine/src/testing.rs::tests`:
  `lenient_backend_constructs_and_returns_decline` — assert the
  constructor returns the expected default and that
  `LlmBackend::classify(...)` returns `Outcome::Decline` (or whatever
  the decline-shape is for the trait; pattern-match the existing
  duplicates).
- `cargo build --release -p atlas-engine` is clean and the resulting
  binary contains no `LenientBackend` symbol (verify via `nm
  target/release/libatlas_engine.rlib | grep LenientBackend` returning
  no hits, OR equivalent for the binary form actually produced).
- `cargo test --workspace --no-fail-fast` passes with output
  byte-identical to before the PR (LLM-call counts unchanged).
- `grep -rln "^struct LenientBackend" crates/` returns zero hits after
  the PR (every site now uses the imported one).
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**LOC:** ~250-450 (new shared module ~80; deletions across ~19 test
files ~400; net ~250-400 deletion).

---

### PR-2 — Decoder consolidation

**Intent:** Phase 2 closeout. Multiple analyser decoders (per-language)
share substantial logic that was duplicated during Phase 2's parallel
analyser work. Pick a canonical decoder shape and migrate per-language
implementations.

**Files (investigation pass — do this first):**
- Read: `crates/atlas-engine/src/l5_surface.rs` and identify the
  decode-related functions (`decode_subprocess_surface_payload` and
  any siblings).
- Read: `crates/atlas-analyzers/src/*` per-language decoder modules
  to enumerate which ones share the canonical shape and which have
  language-specific divergences.
- Output of investigation: a 5–10-line note in the PR description
  listing every decoder site, classified as `(canonical-shape |
  language-specific | should-stay-separate)`. This grounds the
  consolidation scope and lets reviewers verify nothing gets
  accidentally homogenised.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l5_surface.rs` if the canonical helper lands here (likely
  if the canonical decoder is closest to `decode_subprocess_surface_payload`'s
  shape). Otherwise create `crates/atlas-engine/src/decoder.rs` and
  re-export from `lib.rs`.

**Files (in `crates/atlas-analyzers/src/` and per-analyser crates):**
- Modify: each per-language module that currently carries a
  decode-shape duplicate. Migrate to call the canonical helper.
  Language-specific divergences (e.g. analyser-specific error wrapping
  or extra validation) are preserved as wrappers around the canonical
  call, not absorbed into the shared helper.

**Acceptance criteria:**
- `cargo test --workspace --no-fail-fast` passes byte-identically.
  Per-language fixture outputs (under
  `crates/analyzers/{python,csharp,dart,elixir,racket,lispkit}/tests/`)
  are unchanged.
- `cargo clippy --all-targets -- -D warnings` clean.
- Net workspace LOC decreases (verified via `git diff --stat`).
- The PR description lists every per-language decoder site touched
  and explicitly states which sites were intentionally LEFT as
  language-specific (with one-line rationale per site).
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**Risk gate:** If during investigation a language's decoder doesn't
fit cleanly under the canonical shape (substantial language-specific
state, divergent error handling, etc.), the implementer SHOULD leave
that language alone in PR-2 and surface it as a Phase 7 (per-language
refinements) follow-up. Do not absorb language-specific complexity
into the shared helper. Per design §5 risk row: "PR-2 (decoder
consolidation) scope creep" — the mitigation is to leave non-fitting
languages alone.

**LOC:** -200 to -500 (refactor; expect net deletion).

---

### PR-3 — L8 phantom-subcomponent fix

**Intent:** Phase 2 closeout. A known L8 (subsystem composition) issue
where phantom subcomponents are emitted under specific edge cases.
The Phase 3 §9.1 deferred-list named the bug without specifying the
root cause, so PR-3's first task is to find a minimal failing fixture.

**Step 1 — Reproduce (TDD discipline):**
1. Read `crates/atlas-engine/src/l8_*.rs` (or wherever the L8 / subsystem
   composition logic lives — discover via `grep -rn "phantom\|subsystem
   composition\|emit_subcomponent" crates/atlas-engine/src/`).
2. Read existing L8 tests to understand the test idiom.
3. Write a hand-crafted unit test in the L8 module's `#[cfg(test)]`
   section that constructs a minimal subsystem-composition state
   triggering the phantom emission. Assert the expected (correct)
   output. The test must fail before the fix.
4. Run the test: `cargo test -p atlas-engine <test_name>`. Confirm it
   fails with a phantom-subcomponent assertion mismatch.

**Step 2 — Diagnose:**
- Inspect the failing test's actual output. Read the L8 emission
  code path. Identify where the phantom is introduced (likely a
  missing dedup pass, an off-by-one in a parent-link traversal, or a
  scope leak in a recursive descent).
- Add a one-paragraph diagnosis to the PR description: input shape →
  emission step → failure → root cause.

**Step 3 — Fix:**
- Apply the minimal fix. If the fix introduces non-obvious logic, add
  a code comment at the fix site referencing "Phase 4 PR-3
  (L8 phantom subcomponent fix)" so future readers can locate the
  diagnosis in this commit's message.

**Step 4 — Verify:**
- New test now passes.
- Existing L8 tests pass.
- `cargo test --workspace --no-fail-fast` passes.

**Files (sketch — actual files determined by Step 1):**
- Modify: the specific L8 module(s) containing the emission bug.
- Modify: the same module's `#[cfg(test)]` section with the new
  regression test.

**Acceptance criteria:**
- New unit test asserts the bug is fixed and would fail without the
  fix (verify by `git stash`-ing the production change and running
  the test — expected: FAIL).
- Existing L8 tests pass; no regressions in `crates/atlas-engine/tests/`.
- A code comment at the fix site cites the prior incident and PR
  number so future readers locate the diagnosis.
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**Risk gate:** If the bug turns out to be larger than the design's
"~20-50 LOC" estimate (e.g. requires reshaping a data structure or
touching analyser interfaces), the implementer SHOULD stop at the
diagnosis step and surface the scope before continuing. Per the
plan/reality reconciliation rule in §5 of the continuation prompt: a
4000-line surprise diff is not within tolerance.

**LOC:** ~20-50 (small focused fix + ~20-line regression test).

---

### PR-4 — `atomic_write` helper convergence

**Intent:** Two atomic-write helpers exist in the engine. Pick the
canonical one, migrate the sole caller of the duplicate, delete the
duplicate. The duplicate has been documented in
`crates/atlas-engine/src/atomic_write.rs:17` as deferred since PR-1
of Phase 3; Phase 4 closes the gap.

**Concrete state pre-PR (verified 2026-05-09):**
- Canonical: `crates/atlas-engine/src/atomic_write.rs:40` — `pub fn
  atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()>`. mkdirs
  parent internally (line 50-52). Test coverage in PR-12 fixture
  suite.
- Duplicate: `crates/atlas-engine/src/cache/layout.rs:84` —
  `pub(crate) fn atomic_write(target: &Path, blob: &[u8]) -> anyhow::
  Result<()>`. Used at exactly one call site:
  `crates/atlas-engine/src/cache/mod.rs:129` —
  `layout::atomic_write(&path, blob)`.

**Files (in `crates/atlas-engine/src/cache/`):**
- Modify: `mod.rs:129` — change the call from
  `layout::atomic_write(&path, blob)` to:
  ```rust
  crate::atomic_write::atomic_write(&path, blob)
      .with_context(|| format!("atomic_write to {} failed", path.display()))?;
  ```
  The `.with_context(...)` re-establishes the anyhow error context that
  the existing `cache::layout::atomic_write` provided implicitly
  through its `anyhow::Result` return type.
- Modify: `cache/mod.rs:18` — update the doc-comment that references
  `layout::atomic_write` to reference the canonical
  `crate::atomic_write::atomic_write` instead.
- Modify: `cache/layout.rs` — delete the `pub(crate) fn atomic_write`
  function (lines 84–end-of-fn). Verify no other intra-crate caller
  exists (`grep -n "layout::atomic_write" crates/atlas-engine/src/`
  should return zero hits after deletion). If `cache/layout.rs` had
  any private helpers used only by the deleted `atomic_write`, delete
  those too.
- Modify: `crates/atlas-engine/src/atomic_write.rs:17` — update the
  doc-comment that says "the existing `cache::layout::atomic_write`
  is left in place for now" to remove that paragraph (the deferred
  refactor is now done).

**Acceptance criteria:**
- `cargo build --workspace` clean.
- `cargo test --workspace --no-fail-fast` passes byte-identically.
- `grep -n "fn atomic_write" crates/atlas-engine/src/cache/layout.rs`
  returns zero hits.
- `grep -rn "layout::atomic_write" crates/` returns zero hits.
- The PR-12 atomic-write fixture suite
  (`crates/atlas-reports/tests/atomic_writes.rs`) passes
  byte-identically — this is the regression guard for the kill-during-
  write semantics that must be preserved. Run as
  `cargo test -p atlas-reports --test atomic_writes --no-fail-fast`.
- LOC net negative (verified via `git diff --stat`).
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**Risk gate:** Per design §5, "PR-4 silent regression" — the cache
flow is anyhow::Result; if the migration drops any error context, the
PR-12 fixture suite will not catch it (the suite tests durability
under crash, not error-message preservation). The implementer MUST
verify the `.with_context(...)` shape matches the prior anyhow output
by manually triggering an error (e.g. write to `/dev/full` or to a
read-only directory) and inspecting the resulting error chain pre/post.
Document the result in the PR description.

**LOC:** -50 to -100 (net deletion).

---

### PR-5 — `build_engine_database` / `build_database_for_reports` convergence

**Intent:** PR-10 of Phase 3 (modularity) added
`atlas_cli::pipeline::build_engine_database` (the L4–L6 fixedpoint +
L5 pre-warm without writes). PR-11 of Phase 3 (divergence) added a
private `build_database_for_reports` helper inside `reports.rs` doing
similar work via a slightly different path. Extract a single shared
inner helper in `pipeline.rs`; have both `run_modularity` and
`run_divergence` call it.

**Concrete state pre-PR (verified 2026-05-09):**
- `crates/atlas-cli/src/pipeline.rs:761` — `pub fn
  build_engine_database(...)`. Used by `run_modularity`.
- `crates/atlas-cli/src/reports.rs:978` — `fn
  build_database_for_reports(...)` (private). Used by
  `run_divergence`.

**Step 1 — Diff the two helpers:**
- Read both functions end-to-end. Note signature differences (input
  types, return types) and body differences (which engine stages
  each invokes, what each pre-warms).
- Output of step 1: a 5–10-line note in the PR description listing
  the deltas. If the deltas are non-trivial (e.g. one runs L5
  pre-warm and the other doesn't), the canonical helper must accept
  a parameter to drive that behaviour, not silently bake one
  behaviour as the new default.

**Step 2 — Define the canonical helper:**
- The canonical helper lives in `crates/atlas-cli/src/pipeline.rs`.
  Suggested signature (refine in step 1's diff):
  ```rust
  pub fn build_engine_database(
      workspace: &Workspace,
      // additional parameters that capture the deltas from step 1,
      // e.g. `prewarm_l5: bool` if one helper pre-warms and the other doesn't
  ) -> anyhow::Result<EngineDatabase>;
  ```
- If `build_database_for_reports` does anything substantively
  different beyond pre-warming, prefer adding a thin wrapper
  `fn build_engine_database_for_reports(...) -> ...` in `pipeline.rs`
  that calls the canonical helper, rather than baking divergence
  semantics into the shared body.

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs:761-…` — `build_engine_database` becomes the
  canonical helper. Adjust signature if step 1's diff requires.
- Modify: `reports.rs:978-…` — delete `build_database_for_reports`.
- Modify: `reports.rs::run_divergence` — call into
  `pipeline::build_engine_database` (or the thin wrapper from step 2)
  instead of the now-deleted `build_database_for_reports`.

**Acceptance criteria:**
- `cargo build --workspace` clean.
- `cargo test -p atlas-cli --no-fail-fast` passes byte-identically:
  modularity tests still pass; divergence tests still pass; output
  YAML files are byte-identical pre/post (verified via
  `diff -u <pre>/composition-divergence.yaml <post>/composition-divergence.yaml`
  on a fresh fixture run).
- `cargo test --workspace --no-fail-fast` passes byte-identically.
- LOC net negative.
- The PR description includes the step 1 diff note + step 2's
  signature decision so reviewers can verify no behavioural drift.
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**LOC:** -50 to -100.

---

### PR-6 — Sweep-test boilerplate consolidation

**Intent:** Phase 3 PRs 2–5 each shipped a `phase3_retrofit_*.rs` test
file with ~100 LoC of fixture-build boilerplate (`materialise_fixture`,
`base_config`, `LenientBackend` / `SweepBackend`, `tiny_fixture_root`,
`copy_dir_all`, `run_with`). Extract to
`crates/atlas-cli/tests/common/sweep_support.rs`; update the four
`phase3_retrofit_*.rs` files to import.

**Sequencing note:** PR-6 SHOULD land after PR-1, because PR-1 already
extracts `LenientBackend` into a shared location; the sweep_support
module then re-exports `pub use atlas_engine::testing::LenientBackend;`
rather than holding its own copy. If PR-6 lands before PR-1, it must
declare a temporary inline copy that PR-1 then deletes — extra churn.

**Concrete state pre-PR (verified 2026-05-09):**
The four files are:
- `crates/atlas-cli/tests/phase3_retrofit_surfaces.rs`
- `crates/atlas-cli/tests/phase3_retrofit_component.rs`
- `crates/atlas-cli/tests/phase3_retrofit_components.rs`
- `crates/atlas-cli/tests/phase3_retrofit_related.rs`

The shared module path under `tests/common/` follows the standard
Cargo idiom for test-suite shared code: a file at
`crates/atlas-cli/tests/common/mod.rs` is automatically excluded
from being compiled as its own integration test. (Cargo treats every
top-level `.rs` file under `tests/` as a separate test crate, but a
file inside a subdirectory, declared via `mod common;` from a top-level
test, is compiled into that test's binary.)

**Files (in `crates/atlas-cli/tests/common/`):**
- Create: `mod.rs` — declare `pub mod sweep_support;`. (If
  `tests/common/mod.rs` already exists from prior phases, add the
  `pub mod sweep_support;` line.)
- Create: `sweep_support.rs` — the shared module. Lift the following
  helpers from the four `phase3_retrofit_*.rs` files (they are
  byte-identical or near-byte-identical across the four):
  - `pub fn materialise_fixture(...)`.
  - `pub fn base_config(...) -> Config`.
  - `pub struct SweepBackend` + `impl SweepBackend` + `impl LlmBackend
    for SweepBackend` (a sibling of `LenientBackend` used by the sweep
    tests).
  - `pub fn tiny_fixture_root() -> PathBuf` (or `TempDir`).
  - `pub fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()>`.
  - `pub fn run_with(args: &[&str]) -> ExitCode` (or whatever the
    invocation idiom is in the existing files).
  - Re-export: `pub use atlas_engine::testing::LenientBackend;` (from
    PR-1).
  - The exact helper enumeration is "every `fn` and `struct` defined
    in any of the four `phase3_retrofit_*.rs` files outside the
    `#[test]` body". Read the four files end-to-end before extracting.

**Files (in `crates/atlas-cli/tests/`):**
- Modify: `phase3_retrofit_surfaces.rs` — delete every helper now in
  `common::sweep_support`. Add at the top:
  ```rust
  mod common;
  use common::sweep_support::*;
  ```
  Verify the `#[test]` body still compiles and asserts unchanged.
- Modify: `phase3_retrofit_component.rs` — same pattern.
- Modify: `phase3_retrofit_components.rs` — same pattern.
- Modify: `phase3_retrofit_related.rs` — same pattern.

**Acceptance criteria:**
- All four `phase3_retrofit_*.rs` tests pass under `cargo test
  -p atlas-cli --no-fail-fast`.
- LOC net negative across the four `phase3_retrofit_*.rs` files
  (boilerplate moved out).
- The shared module compiles cleanly when imported. (Cargo's
  `tests/common/mod.rs` idiom does NOT compile `common` as its own
  test binary; verify by running `cargo test -p atlas-cli` and
  confirming no "no tests in `common`" warning.)
- `cargo clippy --all-targets -- -D warnings` clean across the four
  files (the helpers may previously have had `#[allow(dead_code)]`
  hints in some files but not others; the consolidated module should
  not need any).
- `cargo fmt --check` clean.
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**LOC:** -100 to -200 across the four test files; +120 in the new
shared module = net negative.

---

### PR-7 — Orphan re-export removal

**Intent:** After Phase 3 PR-5 (related-components.yaml retrofit),
`atlas-contracts/crates/atlas-index/src/lib.rs:60` carries:
```rust
pub use ... save_atomic as save_related_components_atomic, ...
```
with zero callers in either repo. Rust does not warn on orphan public
re-exports; delete the renamed alias.

**Concrete state pre-PR (verified 2026-05-09):**
- File: `/Users/antony/Development/atlas-contracts/crates/atlas-index/src/lib.rs`.
- Line 60 currently reads (approximately, verify exact text before
  editing): `    save_atomic as save_related_components_atomic, ComponentId, ComponentIdError, Edge, EdgeKind,`.
- The `save_atomic` symbol itself remains exported under its real name
  via the same `pub use` block; only the `as save_related_components_atomic`
  rename is removed.

**Step 1 — Verify zero callers:**
- Run, against both Atlas and atlas-contracts checkouts:
  ```bash
  grep -rn "save_related_components_atomic" /Users/antony/Development/atlas-contracts/ /Users/antony/Development/Atlas/
  ```
- Expected: only the single `pub use` line. If any caller exists, STOP
  and surface it — the design assumed zero callers.

**Files (in `atlas-contracts/crates/atlas-index/src/`):**
- Modify: `lib.rs:60` — remove the `as save_related_components_atomic`
  rename. The line becomes:
  ```rust
      save_atomic, ComponentId, ComponentIdError, Edge, EdgeKind,
  ```

**Acceptance criteria:**
- atlas-contracts `cargo build --workspace` clean.
- atlas-contracts `cargo test --workspace` passes.
- Atlas `cargo build --workspace` clean (proves no Atlas consumer used
  the renamed alias).
- Atlas `cargo test --workspace --no-fail-fast` passes.
- `grep -rn "save_related_components_atomic" /Users/antony/Development/atlas-contracts/ /Users/antony/Development/Atlas/`
  returns zero hits after the PR.
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged.

**Cross-repo commit ordering:** PR-7 ships as ONE atlas-contracts
commit. There is no Atlas-side commit because no Atlas code references
the renamed alias — this is the design rationale for declaring the
re-export "orphan". After the atlas-contracts commit lands, the next
Atlas-side `cargo build` will pick up the change via the path-dep
without any Atlas-side edit.

**LOC:** -1 line (atlas-contracts only).

---

### PR-8 — Stale "Phase 4" prose retext + §10 renumbering

**Intent:** Phase 3 PR-13's closeout surfaced four prose locations in
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` where
"Phase 4" still implicitly means "server mode" (post the
post-Phase-3-PR-0b §10.4 update). Plus this PR retexts §10 with the
new phase numbering from the validated post-Phase-3 roadmap (memory
`project_phase4_plus_roadmap`; design §6).

**Concrete state pre-PR (verified 2026-05-09 via `grep -nE 'Phase 4|server mode|Phase 5|Phase 10'` against the canonical spec):**

Current §10 structure (lines 1142–1222):
- §10.3 (line 1142) — Phase 3 — Drift, impact, modularity (DONE).
- §10.4 (line 1161) — Phase 4 — Convergence and cleanups (lists
  pattern detection, subprocess, rust-analyzer, LLM thresholds,
  contract rename-match, strict-overrides, cache compression, worktree
  commit-sha, Phase 2 closeouts, per-language refinements).
- §10.5 (line 1187) — Phase 5 — Server mode.
- §10.6 (line 1199) — Migration from v1 (OBSOLETE marker).

Stale prose references to retext:
- Line 502 (§5.6 server-mode intro): `"Server mode (Phase 4) makes
  Atlas long-running:"`.
- Line 981 (§9 introduction): `"Server mode is the Phase 4 target. The
  CLI continues to work as a degenerate ..."`.
- Line 1270 (§11.2 open question 5): `"Defer to Phase 5 design (server
  mode moved from Phase 4 to Phase 5; see §10.5)."`.
- Line 1314 (§11.4): `"Once Phase 4 ships and the server has concrete
  polyglot consumers issuing ..."`.
- Line 1436 (glossary): `"deferred to Phase 4 and beyond."`.

Plus references inside §10.4 itself (line 1169: `"Needs LLM machinery
that Phase 4 introduces."`) become self-reference if §10.4 retexts to
"Phase 4 — Cleanup release"; per design §6 the §10.4 body shrinks to
"this phase — code-quality + docs (~9 PRs)" and the LLM-machinery
forward-pointer moves to §10.9 (Phase 9 — LLM-driven analyses).

**Files (in `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`):**

The PR is one commit, all changes in this single file.

1. **Replace §10.4 (lines 1161–1185).** The new §10.4 body is short:
   ```markdown
   ### 10.4 Phase 4 — Cleanup release

   **Goal:** Pay down internal-quality debt accumulated across
   Phases 1–3 and align canonical documentation with the validated
   post-Phase-3 phase ordering. No new user-facing capability, no
   schema change, no LLM call sites.

   **Scope (~9 PRs):**
   - LenientBackend extraction; decoder consolidation; L8 phantom-
     subcomponent fix (Phase 2 closeouts).
   - `atomic_write` helper convergence; `build_engine_database` /
     `build_database_for_reports` convergence; sweep-test boilerplate
     consolidation; orphan `save_related_components_atomic` removal.
   - This §10 retext + Phase 3 design §9.1 forward-pointer update.

   See `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`
   for the canonical Phase 4 scope.
   ```

2. **Insert new §10.5 after the new §10.4.** Per design §6:
   ```markdown
   ### 10.5 Phase 5 — Monorepo consolidation

   **Goal:** Fold atlas-contracts + Ravel + Ravel-Lite into Atlas;
   delete multi-root machinery.
   ```
   (Brief; the canonical scope lives in the eventual Phase 5 design
   spec, not here.)

3. **Insert new §10.6.** Per design §6 (user-facing schema cleanups):
   ```markdown
   ### 10.6 Phase 6 — User-facing schema cleanups

   **Scope:** Contract rename-match (§11.2.4); `--strict-overrides`
   flag; cache compression (§11.2.7); worktree commit-sha
   annotations (§11.2.8); `is_manifest_file` Makefile/shell
   extension; `subsystem` field wiring (Phase 3 PR-9 deferral);
   `edges_suppress` no-match warning stderr-capture test (Phase 3
   PR-10 deferral).
   ```

4. **Insert new §10.7.** Per design §6 (per-language refinements):
   ```markdown
   ### 10.7 Phase 7 — Per-language refinements

   **Scope:** Full tree-sitter-dart; raco-driven Racket dep
   resolution; Phoenix sub-kinds for Elixir; Mix umbrella
   decomposition; LispKit `(import …)` symbolic resolution.
   ```

5. **Insert new §10.8.** Per design §6 (subprocess convergence):
   ```markdown
   ### 10.8 Phase 8 — Subprocess convergence

   **Scope:** Migrate Cargo / Dockerfile / RustSurface / LlmClassify /
   TS-as-subprocess to subprocess; bidirectional LLM callback channel;
   rust-analyzer integration (stretch).
   ```

6. **Insert new §10.9.** Per design §6 (LLM-driven analyses):
   ```markdown
   ### 10.9 Phase 9 — LLM-driven analyses

   **Scope:** Pattern detection (recurring component / edge shapes);
   LLM confidence threshold calibration (§11.2.6).
   ```

7. **Renumber old §10.5 → §10.10.** The "Server mode" body (currently
   lines 1187–1198) keeps its existing scope text; only the heading
   changes to:
   ```markdown
   ### 10.10 Phase 10 — Server mode
   ```

8. **Renumber old §10.6 → §10.11.** The "Migration from v1" body keeps
   its OBSOLETE marker and historical body unchanged; only the heading
   changes to:
   ```markdown
   ### 10.11 Migration from v1
   ```

9. **Retext line 502 (§5.6).** Change `"Server mode (Phase 4) makes
   Atlas long-running:"` to `"Server mode (Phase 10) makes Atlas
   long-running:"`.

10. **Retext line 981 (§9 introduction).** Change `"Server mode is the
    Phase 4 target."` to `"Server mode is the Phase 10 target."`. The
    "degenerate CLI" sentence remains.

11. **Retext line 1270 (§11.2 open question 5).** Change `"Defer to
    Phase 5 design (server mode moved from Phase 4 to Phase 5; see
    §10.5)."` to `"Defer to Phase 10 design (server mode is now
    §10.10 under the validated post-Phase-3 ordering; see §10.10)."`.
    Also update the open question's title prefix from "Phase 5 query
    API authentication and authorisation" to "Phase 10 query API
    authentication and authorisation".

12. **Retext line 1314 (§11.4).** Change `"Once Phase 4 ships and the
    server has concrete polyglot consumers"` to `"Once Phase 10 ships
    and the server has concrete polyglot consumers"`.

13. **Retext line 1436 (glossary).** Change `"deferred to Phase 4 and
    beyond"` to a wording that matches the immediate context — likely
    `"deferred to Phase 10 (server mode) and beyond"` or simply
    `"deferred indefinitely"` if the surrounding context refers to a
    deferred-indefinitely item per Phase 3 §9.3. Read the surrounding
    paragraph before deciding; document the choice in the PR
    description.

14. **Sweep for missed references.** Run, after applying changes 1–13:
    ```bash
    grep -nE "Phase 4" docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
    ```
    Expected after the PR: no occurrence outside the new §10.4 heading
    and the new §10.4 body. Any other occurrence is a missed retext;
    fix it in the same commit.

**Files (in `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`):**

15. **Forward-pointer update to §9.1.** For each item in the §9.1
    deferred-list, append `"(now Phase X)"` matching the validated
    roadmap. Specifically:
    - Pattern detection → `(now Phase 9)`.
    - Subprocess convergence → `(now Phase 8)`.
    - Bidirectional LLM callback channel → `(now Phase 8)`.
    - rust-analyzer integration → `(now Phase 8 stretch)`.
    - LLM threshold calibration → `(now Phase 9)`.
    - Contract rename-match → `(now Phase 6)`.
    - `--strict-overrides` → `(now Phase 6)`.
    - Cache compression → `(now Phase 6)`.
    - Worktree commit-sha annotations → `(now Phase 6)`.
    - Phase 2 closeouts (LenientBackend, decoder, manifest-file, L8) → `(now Phase 4)`.
    - Per-language refinements → `(now Phase 7)`.

**Acceptance criteria:**
- `grep -nE "§10\.[0-9]+" docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  returns valid references only — every match resolves to an existing
  heading. No dangling `§10.4` reference points at the *old* "Phase 4
  — Convergence and cleanups" content; no `§10.5` reference points at
  the *old* "Phase 5 — Server mode" content (server mode is now
  §10.10).
- The spec re-reads coherently end-to-end. No contradictions between
  §10's structure and the prose elsewhere.
- The byte-stability of the canonical spec OUTSIDE the touched prose
  paragraphs and §10 is preserved (verified by `git diff --stat`:
  exactly one file modified for the canonical spec; the `+`/`-` line
  counts are bounded by the §10 expansion + ~5 retext lines elsewhere
  + the new §10 forward-pointer in §11.2).
- The Phase 3 design's §9.1 deferred-list shows the eleven forward-
  pointer annotations.
- `grep -nE "Phase 4 = server mode|moved from Phase 4 to Phase 5"`
  against the canonical spec returns zero hits (the renumbering
  rationale is in this commit message, not in the spec).
- Phase 3 polyglot smoke test passes; LLM-call budget unchanged. (The
  smoke test does not read the canonical spec, but running it
  preserves the every-PR discipline.)

**Risk gate:** Per design §5, "PR-8 (spec retext) prose drift" — other
prose paragraphs may also reference "Phase 4" implicitly. The sweep at
step 14 is the canonical guard; if a missed reference surfaces during
review, fix it inline rather than deferring.

**LOC:** +60 to +120 net (the §10 expansion adds ~50 lines of new
section bodies; retext changes are ~10 lines net).

---

## 5. Acceptance criteria summary (per-PR table)

The following table is the canonical acceptance gate. A PR may not
land until every row in its column is green.

| PR | Tests pass | New unit/integration tests | Smoke test contributes to | LLM-call budget verifier |
|---|---|---|---|---|
| PR-0 | n/a (docs) | n/a | n/a | n/a |
| PR-1 | workspace + release | `lenient_backend_constructs_and_returns_decline` in `atlas-engine::testing::tests` | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-2 | workspace | per-language fixture outputs byte-identical (existing tests) | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-3 | workspace | new L8 phantom-subcomponent regression test | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-4 | workspace + PR-12 atomic-write fixtures | (none new; PR-12 fixture suite is the regression guard) | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-5 | atlas-cli | byte-identical output YAMLs from `run_modularity` and `run_divergence` | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-6 | atlas-cli | the four `phase3_retrofit_*.rs` tests pass after import-rewrite | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-7 | atlas-contracts + Atlas | (none; the orphan was untested) | Phase 3 polyglot test must pass byte-identically | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |
| PR-8 | n/a (docs) | (none — docs-only) | Phase 3 polyglot test must pass byte-identically (run as discipline check, even though the spec retext does not affect code) | Cold = Phase 2 PR-14 baseline; warm = 0; reports = 0 |

The Phase 3 polyglot smoke test
(`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) runs to
completion under `cargo test --workspace --no-fail-fast` after EVERY
Phase 4 PR — its strict LLM-call-budget assertions are the cumulative
regression guard. If any Phase 4 PR causes the budget assertions to
fail, the PR has introduced a behavioural regression and must be
reworked before landing.

---

## 6. Risks (Phase 4 specific)

These are operational risks for the Phase 4 implementation,
supplementing design §5 and the Phase 1/2/3 plan risks (which carry
forward).

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| PR-2 (decoder consolidation) scope creep into per-language refinements. | High | Medium | Step 1 investigation pass enumerates every decoder site BEFORE migration; languages whose decoders don't fit the canonical shape stay untouched and surface as Phase 7 follow-ups. PR description must list the "intentionally not migrated" languages. |
| PR-4 (atomic_write convergence) silent regression in error context. | Medium | Medium | Manual error-injection check at a write-path call site (e.g. write to a read-only directory) pre/post; PR description records the error-chain output. PR-12 fixture suite is the durability regression guard but does not test error-message preservation. |
| PR-5 (build_engine_database convergence) silent behavioural drift. | Medium | High (output YAMLs change) | Step 1 diff note in PR description enumerates what the two helpers do differently; reviewers verify the canonical helper preserves both behaviours via per-handler parameters. Output YAMLs verified byte-identical pre/post on a fresh fixture run. |
| PR-6 (sweep-test boilerplate) duplicates `LenientBackend` if landed before PR-1. | Low | Low | PR-6 sequenced AFTER PR-1 in §9 dependency graph. If a parallel-dispatch session ignores the order, the orchestrator catches it at the dependency-check step. |
| PR-8 (spec retext) prose drift in untouched paragraphs. | Medium | Low | Step 14 sweep `grep -nE "Phase 4"` against the canonical spec catches any leftover reference. Reviewer second-pass after the PR lands. |
| PR-3 (L8 fix) larger than 50 LOC because the bug is structural. | Medium | Medium | TDD discipline enforces minimal-fix discipline: write failing test → diagnose → minimal fix. If the diagnosis surfaces a structural issue, surface and split the PR per the §5 reconciliation rule in the continuation prompt. |
| Cumulative LLM-call budget drift across multiple Phase 4 PRs. | Low | High | Every PR re-runs the Phase 3 polyglot smoke test before flipping its checkbox. Drift surfaces immediately on the offending PR; later PRs do not mask earlier drift because each measures cumulative state. |
| PR-7 (orphan re-export) accidentally deletes a non-orphan symbol. | Low | High | Step 1 grep verifies zero callers across BOTH atlas-contracts AND Atlas. If a caller exists, STOP — the design assumed orphan status. |
| The §10 renumbering in PR-8 creates dangling cross-references in *other* design docs (Phase 1/2/3 specs may cite §10.5 or §10.4 by old number). | Medium | Low | Step 14 sweep includes a `grep -rnE "§10\.[0-9]+"` across `docs/superpowers/` to catch dangling refs in sibling docs. Updates land in the same PR-8 commit if any are found. |
| Phase 3 polyglot test execution time (~17 minutes per Phase 3 PR-13 closeout note) makes the every-PR re-run gate expensive. | High | Low | Accept the cost. Phase 4 has only 8 code/docs PRs; total Phase-4-runtime overhead is ~2.5 hours wall-clock for the cumulative regression-guard invocations. The cost is structural and matches Phase 3's discipline. |

---

## 7. Out of scope for Phase 4

These items are deferred to later phases per the validated roadmap
(memory `project_phase4_plus_roadmap`; design §6). A reviewer flagging
them as missing should redirect to the relevant phase.

### 7.1 Deferred to Phase 5 (monorepo consolidation)

- Folding `/Users/antony/Development/atlas-contracts` into
  `/Users/antony/Development/Atlas/crates/atlas-contracts/` (or
  similar in-tree path).
- Folding Ravel + Ravel-Lite into Atlas.
- Deleting the multi-root machinery in
  `crates/atlas-engine/src/root_expansion.rs` (or whatever portion of
  it serves cross-repo path-deps; the in-Atlas multi-root semantics
  per design §6 may stay).

### 7.2 Deferred to Phase 6 (user-facing schema cleanups)

- Contract rename-match (design §11.2.4).
- `--strict-overrides` flag.
- Cache compression (design §11.2.7).
- Worktree commit-sha consistency annotations (design §11.2.8).
- `is_manifest_file` extension for Makefile / shell.
- `subsystem` field wiring on `ComponentEntry` (Phase 3 PR-9
  deferral; the field is captured in the override schema but applied
  as a no-op at L4).
- `edges_suppress` no-match warning stderr-capture test (Phase 3
  PR-10 deferral; the warning fires correctly in production but is
  not yet test-asserted because the in-process `run()` harness
  doesn't plumb stderr capture).

### 7.3 Deferred to Phase 7 (per-language refinements)

- Full tree-sitter-dart adoption.
- raco-driven Racket dep resolution.
- Phoenix sub-kinds for Elixir.
- Mix umbrella decomposition.
- LispKit `(import …)` symbolic resolution.

### 7.4 Deferred to Phase 8 (subprocess convergence)

- Migrating Cargo / Dockerfile / RustSurface / LlmClassify /
  TS-as-subprocess from in-process to subprocess.
- Bidirectional LLM callback channel.
- `rust-analyzer` integration replacing `syn` (stretch).

### 7.5 Deferred to Phase 9 (LLM-driven analyses)

- Pattern detection (recurring component shapes, recurring edge
  shapes).
- LLM confidence threshold calibration (design §11.2.6).

### 7.6 Deferred to Phase 10 (server mode)

- File watcher + Salsa input updates.
- gRPC + HTTP+GraphQL query API.
- Subscription primitives (contract sha, surface sha).
- Server lifecycle (start, restart, GC).
- CLI as thin client to co-located server.
- Optional Grafeo derived index for ad-hoc Cypher / GQL / SPARQL.
- Reactive recomputation of reports.
- Phase 10 query API authentication and authorisation (design
  §11.2.5; renumbered from "Phase 5" by PR-8 of this plan).
- Salsa-tracking the report queries (mechanical conversion, design
  §3.5).

### 7.7 Deferred indefinitely (per Phase 3 §9.3)

- `--gate` / `--strict` exit-code flags for CI integration on reports.
- Pass/fail thresholds for modularity scores.
- Upstream / subsystem-input variants of impact query.
- Modularity history depth >5 entries.
- Per-language coupling normalisation.
- Multi-tenant / SaaS hosting.

---

## 8. References

- **Design spec (Phase 4):** `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`. Canonical scope; this plan operationalises it.
- **Project design spec:** `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`. PR-8 of this plan retexts §5.6, §9, §10 (full §10.1–§10.11 re-shape), §11.2.5, §11.4, and the glossary.
- **Phase 3 design spec:** `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`. PR-8 of this plan adds forward-pointer annotations to §9.1.
- **Phase 1 plan:** `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`. Forensic context for L8 / surfaces / components mechanisms.
- **Phase 2 plan:** `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`. Forensic context for the LenientBackend / decoder / manifest-file mechanisms PR-1, PR-2, PR-3 close out.
- **Phase 3 plan:** `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`. Source for the cleanup candidates Phase 4 lands; particularly §4 PR-2..PR-5 (sweep-test boilerplate sites) and §4 PR-10 / PR-11 (build_engine_database / build_database_for_reports duplication).
- **Phase 1 status (per-PR notes):** `docs/superpowers/plans/2026-05-06-phase1-status.md`.
- **Phase 2 status (per-PR notes):** `docs/superpowers/plans/2026-05-07-phase2-status.md`.
- **Phase 3 status (per-PR notes):** `docs/superpowers/plans/2026-05-08-phase3-status.md` — particularly the `## Phase 3 — complete` closeout (lines ~1300–1400) which documents the Phase 4 cleanup candidates.
- **Phase 4 status (per-PR notes — this plan's companion, PR checklist + dependency graph + per-PR notes):** `docs/superpowers/plans/2026-05-09-phase4-status.md`.
- **Continuation prompt (Phase-4-shaped):** `docs/superpowers/prompts/2026-05-09-vnext-continue.md`. Supersedes `docs/superpowers/prompts/2026-05-08-vnext-continue.md`.
- **Memory entries that constrain Phase 4** (any missing entries are not load-bearing; the design spec captures the same constraints):
  - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
  - `feedback_fix_all_lints` — every PR runs `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
  - `project_phase4_plus_roadmap` — the validated post-Phase-3 phase ordering; PR-8 lands the canonical §10 retext that surfaces this ordering in the system-model spec.
  - `project_monorepo_consolidation` — long-term direction; informs that Phase 4 should not introduce any new multi-root-specific code paths (Phase 5 is where consolidation happens).

---

## 9. Dependency graph (canonical)

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
- **Wave 1 (after PR-0):** PR-1, PR-2, PR-3, PR-4, PR-5, PR-7, PR-8 — seven PRs, all on independent surfaces. Up to ~3 PRs can be dispatched concurrently in practice; the binding constraint is reviewer attention rather than file conflicts.
  - PR-1 + PR-7 are the smallest (a focused extraction; a one-line deletion); pair these as the first parallel dispatch if the orchestrator wants to land cleanups quickly.
  - PR-4 should be paired with the PR-12 fixture-suite re-run as its own gate; the implementer runs `cargo test -p atlas-reports --test atomic_writes --no-fail-fast` before reporting DONE.
  - PR-8 is docs-only and has no `cargo test` gate beyond the every-PR Phase 3 polyglot test; it can land at any point in Wave 1.
- **Wave 2 (after PR-1):** PR-6 (sweep-test boilerplate) — depends on PR-1 because the consolidated `sweep_support` module re-exports `atlas_engine::testing::LenientBackend`. Could land in Wave 1 with a temporary inline `LenientBackend` copy that PR-1 then deletes, but this introduces churn; the cleaner path is sequencing PR-6 after PR-1.

The widest practical parallel wave is ~3 PRs (e.g. PR-1 + PR-4 + PR-7 dispatched concurrently). Phase 4 is *not* as parallel as Phase 3's Wave 5 (4-wide); the win is "many small disjoint PRs" rather than "wide fan-out". Both wave sets benefit from `superpowers:dispatching-parallel-agents` (one Agent tool call per PR, all in a single message).

The Phase 3 PR-13 polyglot smoke test is the cumulative regression guard for Phase 4. Every PR's checkbox-flip step includes a final `cargo test -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast` invocation; the LLM-call-budget assertions catch any drift.
