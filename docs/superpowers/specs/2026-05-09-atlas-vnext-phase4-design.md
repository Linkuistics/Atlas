# Atlas vNext Phase 4 — Cleanup release (design spec)

Status: brainstormed and approved 2026-05-09. Companion plan + status
file land in PR-0 of Phase 4 itself.

Phase 3 shipped on 2026-05-08 (14 PRs landed across atlas-contracts +
Atlas; final commit `5dd1e92` on Atlas main). This document captures
the canonical scope of Phase 4 — a focused **cleanup release** — and
the post-Phase-4 roadmap covering Phases 5–10.

---

## 0. Reading order

§1 (one-paragraph summary) → §2 (scope) → §3 (PR enumeration) →
§4 (acceptance summary) → §6 (the §10 roadmap update). Skim §5 (risks)
and §7 (references) on demand.

---

## 1. Summary

Phase 4 pays down internal-quality debt accumulated across Phases 1–3
and lands a one-shot documentation retext to align the canonical
system-model design with the new post-Phase-3 roadmap. **No new
user-facing capability, no schema change, no LLM call sites.** Cold
polyglot LLM-call count must remain at the Phase 2 PR-14 baseline; PR-13
of Phase 3 is the regression guard.

Two clusters: **Group A — code-quality cleanups** (~7 PRs touching
engine + CLI internals; mostly delete-duplicates / extract-shared /
fix-known-edge-case) and **Group D — documentation cleanups** (1 PR
retexting four prose locations in the canonical system-model design and
renumbering §10).

Total: ~9 PRs (1 meta + 7 code-quality + 1 docs).

The deferred items from Phase 3 §9.1 that are NOT in Phase 4 — including
subprocess convergence, LLM-driven analyses, user-facing schema
cleanups, per-language refinements — get forward-pointer slots in §10.5
through §10.10 (see §6 of this document).

---

## 2. Scope

### 2.1 In scope (Phase 4)

**Group A — Code-quality cleanups:**

- LenientBackend extraction (Phase 2 closeout).
- Decoder consolidation (Phase 2 closeout).
- L8 phantom-subcomponent fix (Phase 2 closeout).
- `cache::layout::atomic_write` / `atlas_engine::atomic_write`
  convergence — delete the duplicate.
- `build_engine_database` (PR-10) / `build_database_for_reports`
  (PR-11) convergence — extract a shared inner helper.
- Sweep-test boilerplate consolidation across `phase3_retrofit_*.rs`.
- Orphan `pub use save_related_components_atomic` removal in
  atlas-contracts.

**Group D — Documentation cleanups:**

- Stale "Phase 4" prose retext + §10 renumbering in
  `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`.
- Forward-pointer update on Phase 3's §9.1 deferred-list so future
  readers know which phase each deferred item landed in.

### 2.2 Out of scope (deferred to later phases — see §6 for slots)

Each deferred item gets a specific phase number under the new ordering:

- **Phase 5** — monorepo consolidation: atlas-contracts in-tree, fold
  Ravel + Ravel-Lite, delete multi-root machinery.
- **Phase 6** — user-facing schema cleanups: contract rename-match,
  `--strict-overrides` flag, cache compression, worktree commit-sha
  annotations, `is_manifest_file` extension, `subsystem` field wiring,
  `edges_suppress` no-match stderr-capture test.
- **Phase 7** — per-language refinements: full tree-sitter-dart,
  raco-driven Racket dep resolution, Phoenix sub-kinds, Mix umbrella
  decomposition, LispKit `(import …)` symbolic resolution.
- **Phase 8** — subprocess convergence + bidirectional LLM callback +
  rust-analyzer integration (stretch).
- **Phase 9** — LLM-driven analyses: pattern detection + LLM
  confidence threshold calibration.
- **Phase 10** — server mode (file watcher, gRPC + GraphQL API,
  subscription primitives, reactive recomputation).
- **Deferred indefinitely** — gate/strict exit-code flags on reports,
  modularity-score thresholds, upstream / subsystem-input impact
  variants, modularity history depth >5, per-language coupling
  normalisation, multi-tenant SaaS hosting (per Phase 3 §9.3).

### 2.3 Non-negotiables

Carry forward from Phase 3:

- **Greenfield.** No on-disk format compatibility with prior phases;
  no migration commands; users upgrading delete `.atlas/` and re-run.
- **No new LLM call sites.** Cold polyglot LLM-call count must remain
  at Phase 2 PR-14 baseline. PR-13 polyglot test is the regression
  guard.
- **Atomic writes everywhere.** PR-4 (atomic_write helper convergence)
  must preserve byte-identical semantics (temp + fsync + rename;
  mkdirs parent). PR-12 fixture suite is the regression guard.
- **`atlas-reports` stays pure-function.** Even if Phase 4 PRs touch
  the helpers feeding into reports, no `fs::*` may be introduced
  inside `crates/atlas-reports/src/*`. Phase 5-Salsa conversion
  invariant.
- **Six-file editorial tier preserved.** No new editorial files; no
  demotions to cache.
- **`schema_version` stays at 1** for every Phase 3 on-disk schema.
- **Cargo test/clippy/fmt clean per PR.** Orchestrator independently
  re-verifies before flipping the checkbox.

---

## 3. PR enumeration

Each entry sketches files touched, intent, and acceptance criteria.
Detailed line-level execution lives in the Phase 4 plan (PR-0).

### PR-0 — Plan + status (no code)

**Intent:** Land the Phase 4 plan
(`docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md`), the
status file (`docs/superpowers/plans/2026-05-09-phase4-status.md`), and
a Phase-4-shaped continuation prompt at
`docs/superpowers/prompts/2026-05-09-vnext-continue.md` that supersedes
the Phase 3 prompt.

**Acceptance:** All three docs land in one commit; the continuation
prompt's wildcard `*phase4-plan*` matches the new plan file.

**LOC:** ~600 (plan) + ~80 (status skeleton) + ~250 (continuation
prompt).

---

### PR-1 — `LenientBackend` extraction

**Intent:** Phase 2 closeout. The `LenientBackend` test stub is
duplicated inline across multiple integration test files. Extract to a
single shared location (likely `crates/atlas-engine/src/testing.rs`
under `#[cfg(any(test, feature = "test-fixtures"))]`, OR a new
`atlas-test-fixtures` workspace crate if the engine doesn't already have
a `testing` module).

**Files (sketch):**
- New: `crates/atlas-engine/src/testing.rs` (or equivalent).
- Modified: every test file currently constructing an inline
  `LenientBackend`.

**Acceptance:**
- All existing tests that previously used inline `LenientBackend`
  import the shared one.
- Engine binary release build clean (the testing module must NOT exist
  in release builds — verify via `cargo build --release`).
- `cargo test --workspace` passes byte-identically.

**LOC:** ~200-400.

---

### PR-2 — Decoder consolidation

**Intent:** Phase 2 closeout. Multiple analyser decoders (per-language)
share substantial logic that was duplicated during Phase 2's parallel
analyser work. Pick a canonical decoder shape and migrate per-language
implementations.

**Files (sketch):**
- Modified: `crates/atlas-analyzers/src/*` per-language decoder
  modules.
- Possibly new: a shared `decoder.rs` with the canonical type signatures
  and helper functions.

**Acceptance:**
- Per-language test fixtures pass byte-identically (analyser output
  has not changed).
- `cargo clippy --all-targets -- -D warnings` clean.
- LOC reduction in the per-language modules; net workspace LOC
  decreases.

**LOC:** -200 to -500 (refactor, expect net deletion).

---

### PR-3 — L8 phantom-subcomponent fix

**Intent:** Phase 2 closeout. A known L8 (subsystem composition) issue
where phantom subcomponents are emitted under specific edge cases.
The implementer reproduces via a hand-written failing test before
diagnosing — Phase 3 §9.1 listed this as a one-liner without root-cause
detail, so PR-3's first task is to find a minimal failing fixture.

**Files (sketch):**
- Modified: the specific L8 module(s).
- New: a unit test reproducing the previously-broken edge case.

**Acceptance:**
- New unit test passes; previously failed without the fix.
- Existing L8 tests continue to pass.
- A code comment at the fix site references the prior incident.

**LOC:** ~20-50 (small focused fix).

---

### PR-4 — `atomic_write` helper convergence

**Intent:** Two atomic-write helpers exist:
- `atlas_engine::atomic_write` — public, `io::Result`, mkdirs parent
  internally, used by PR-1 and Phase 3 retrofits.
- `cache::layout::atomic_write` (private to the engine cache module),
  `anyhow::Result`, different API shape.

Pick `atlas_engine::atomic_write` as canonical (it's already the
publicly-exported one used by Phase 3); migrate the cache-internal call
sites; delete the duplicate.

**Files (sketch):**
- Modified: `crates/atlas-engine/src/cache/layout.rs` (delete the
  private helper).
- Modified: every call site of the private helper.

**Acceptance:**
- Workspace compiles clean.
- `cargo test --workspace` passes byte-identically.
- PR-12's atomic-write fixture suite passes (before-rename and
  after-rename hooks unchanged).
- LOC net negative.

**LOC:** -50 to -100.

---

### PR-5 — `build_engine_database` / `build_database_for_reports` convergence

**Intent:** PR-10 (modularity, Phase 3) added
`atlas_cli::pipeline::build_engine_database` (the L4–L6 fixedpoint +
L5 pre-warm without writes). PR-11 (divergence, Phase 3) added a
private `build_database_for_reports` helper inside `reports.rs` doing
similar work via a different path. Extract a single shared inner helper
in `pipeline.rs`; have both `run_modularity` and `run_divergence` call
it.

**Files (sketch):**
- Modified: `crates/atlas-cli/src/pipeline.rs` (canonical helper).
- Modified: `crates/atlas-cli/src/reports.rs` (delete the duplicate;
  call into the canonical helper).

**Acceptance:**
- Workspace compiles clean.
- `cargo test -p atlas-cli` passes byte-identically (modularity and
  divergence tests still pass; output unchanged).
- LOC net negative.

**LOC:** -50 to -100.

---

### PR-6 — Sweep-test boilerplate consolidation

**Intent:** Phase 3 PRs 2–5 each shipped a `phase3_retrofit_*.rs` test
file with ~100 LoC of fixture-build boilerplate (`materialise_fixture`,
`base_config`, `LenientBackend` / `SweepBackend`, `tiny_fixture_root`,
`copy_dir_all`, `run_with`). Extract to
`crates/atlas-cli/tests/common/sweep_support.rs`; update the four
`phase3_retrofit_*.rs` files to import.

**Files (sketch):**
- New: `crates/atlas-cli/tests/common/sweep_support.rs`.
- Modified: `phase3_retrofit_surfaces.rs`,
  `phase3_retrofit_component.rs`, `phase3_retrofit_components.rs`,
  `phase3_retrofit_related.rs`.

**Acceptance:**
- All four `phase3_retrofit_*.rs` tests pass.
- LOC net negative across the four files (boilerplate moved out).
- The shared module compiles in isolation (`cargo test -p atlas-cli
  --test sweep_support` if appropriate).

**LOC:** -100 to -200 across the four test files; +120 in the new
shared module = net negative.

---

### PR-7 — Orphan re-export removal

**Intent:** After Phase 3 PR-5 (related-components.yaml retrofit),
`atlas-contracts/crates/atlas-index/src/lib.rs:60` carries a
`pub use save_related_components_atomic` with zero callers in either
repo. Rust does not warn on orphan public re-exports; delete it.

**Files (sketch):**
- Modified: `atlas-contracts/crates/atlas-index/src/lib.rs` (single
  line deletion).

**Acceptance:**
- atlas-contracts `cargo build --workspace` clean.
- Atlas `cargo build --workspace` clean (proves no consumer used the
  orphan).
- atlas-contracts test suite passes.

**LOC:** -1 line + small commit message.

---

### PR-8 — Stale "Phase 4" prose retext + §10 renumbering

**Intent:** Phase 3's PR-0b surfaced four prose locations in
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` where
"Phase 4" still implicitly means "server mode" (post the
post-Phase-3-PR-0b §10.4 update). Plus this PR retexts §10 with the
new phase numbering (see §6 of this document).

**Files (sketch):**
- Modified: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  — §10 renumbering (table-form update); §5.6, §9, §11.4, glossary
  line ~1436 retexted to refer to "server mode" or "Phase 10"
  consistently.
- Modified:
  `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`
  §9.1 — forward-pointer update so each deferred item lists its
  destination phase.

**Acceptance:**
- No broken `§10.X` cross-references (`grep -nE "§10\.[1-6]"` in the
  spec yields only valid references after the update).
- Spec re-reads coherently — no contradictions between §10's table
  and the prose elsewhere.
- The byte-stability of the canonical spec OUTSIDE the four touched
  prose paragraphs and §10 is preserved (verified by `git diff
  --stat` ratio: only the touched sections show diffs).

**LOC:** +30 to +60 net (table update + prose retext).

---

## 4. Acceptance summary (per-PR table)

| PR | Title | Verifier |
|---|---|---|
| 0 | Plan + status (docs only) | Three docs land; continuation prompt's wildcard matches |
| 1 | `LenientBackend` extraction | Workspace tests pass; release build clean (no test-fixture symbols leaked) |
| 2 | Decoder consolidation | Per-language fixtures byte-identical; clippy clean; LOC net negative |
| 3 | L8 phantom-subcomponent fix | New unit test passes; existing L8 tests pass; comment cites prior incident |
| 4 | `atomic_write` convergence | Workspace tests pass byte-identically; PR-12 fixtures pass; LOC net negative |
| 5 | `build_engine_database` / `build_database_for_reports` convergence | atlas-cli tests pass byte-identically; LOC net negative |
| 6 | Sweep-test boilerplate consolidation | Four retrofit tests pass; LOC net negative |
| 7 | Orphan re-export removal | Workspace builds clean in both repos |
| 8 | §10 retext + renumbering | No broken cross-references; coherent re-read; §9.1 forward-pointers updated |

The Phase 3 polyglot smoke test (`phase3_polyglot_fixture.rs`) runs to
completion under `cargo test --workspace --no-fail-fast` after EVERY
Phase 4 PR — its strict LLM-call-budget assertions are the cumulative
regression guard (cold = Phase 2 baseline, warm + reports = 0).

---

## 5. Risks (Phase 4 specific)

- **PR-2 (decoder consolidation) scope creep.** Per-language decoders
  may accumulate special-cases that don't fit a clean canonical shape.
  Mitigation: if a language's decoder doesn't fit cleanly, leave it
  alone in PR-2 and surface for a Phase 7 (per-language refinements)
  follow-up.
- **PR-4 (atomic_write convergence) silent regression.** The
  cache::layout helper's API shape (`anyhow::Result`) differs from
  `atlas_engine::atomic_write` (`io::Result`); migration must preserve
  the error-context that anyhow provides at every call site.
  Mitigation: at each call site, wrap with `.with_context(...)` after
  migrating to `io::Result`. PR-12 fixture suite runs as the final
  gate.
- **PR-8 (spec retext) prose drift.** Other prose paragraphs in the
  canonical spec may also reference "Phase 4" implicitly for "server
  mode" — sweep more thoroughly than PR-0b's enumeration. Mitigation:
  `grep -niE "phase 4|server mode"` across the canonical spec before
  committing; classify each match.
- **No regression in cold LLM-call count.** Each PR re-runs PR-13's
  polyglot test; any drift surfaces immediately.

---

## 6. §10 roadmap update (Phase 4 PR-8 lands this verbatim)

The canonical
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` §10
is updated to:

| Number | Title | Status |
|---|---|---|
| §10.1 | Phase 1 — Architectural seam | done |
| §10.2 | Phase 2 — Pluggability and polyglot | done |
| §10.3 | Phase 3 — Drift, impact, modularity | done |
| §10.4 | Phase 4 — **Cleanup release** | this phase — code-quality + docs (~9 PRs) |
| §10.5 | Phase 5 — **Monorepo consolidation** | atlas-contracts in-tree; fold Ravel + Ravel-Lite; delete multi-root machinery |
| §10.6 | Phase 6 — **User-facing schema cleanups** | contract rename-match (§11.2.4); `--strict-overrides`; cache compression (§11.2.7); worktree commit-sha annotations (§11.2.8); `is_manifest_file` Makefile/shell extension; `subsystem` field wiring; `edges_suppress` no-match warning stderr-capture test |
| §10.7 | Phase 7 — **Per-language refinements** | full tree-sitter-dart; raco-driven Racket dep resolution; Phoenix sub-kinds; Mix umbrella decomposition; LispKit `(import …)` symbolic resolution |
| §10.8 | Phase 8 — **Subprocess convergence** | migrate Cargo / Dockerfile / RustSurface / LlmClassify / TS-as-subprocess to subprocess; bidirectional LLM callback channel; rust-analyzer integration (stretch) |
| §10.9 | Phase 9 — **LLM-driven analyses** | pattern detection (recurring component / edge shapes); LLM confidence threshold calibration (§11.2.6) |
| §10.10 | Phase 10 — **Server mode** | file watcher + Salsa input updates; gRPC + GraphQL API; subscription primitives; reactive recomputation; CLI as thin client |
| §10.11 | Migration from v1 | obsolete (greenfield non-negotiable, Phase 1) |

Prose retext targets within the canonical spec:

- **§5.6 "Server mode (eventual)"** — references "Phase 4"; rewrite to
  "Phase 10" or use "server mode" without a phase number.
- **§9 introduction** — "Server mode is the Phase 4 target" → "Server
  mode is the Phase 10 target".
- **§11.4** — "Once Phase 4 ships and the server has concrete polyglot
  consumers" → "Once Phase 10 ships …".
- **Glossary line ~1436** — "deferred to Phase 4 and beyond" →
  "deferred to Phase 10 (server mode)" or similar wording matching the
  immediate context.

The Phase 3 design's §9.1 forward-pointer update (also in PR-8): every
deferred item gets a "(now Phase X)" annotation so future readers know
where it landed.

---

## 7. References

- Phase 3 design:
  `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`
  (canonical Phase 3 scope; §9.1 deferred-list is the source for
  Phase 4 candidates).
- Phase 3 plan:
  `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`.
- Phase 3 status:
  `docs/superpowers/plans/2026-05-08-phase3-status.md` (per-PR notes
  carry the candidate items for Phase 4: orphan re-exports, helper
  duplications, sweep-test boilerplate).
- Canonical system-model design:
  `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  §10 is the canonical roadmap (this Phase 4's PR-8 updates it).
- Memory:
  `.claude/memory/project_monorepo_consolidation.md` (long-term goal:
  fold atlas-contracts + Ravel + Ravel-Lite into Atlas; matches
  Phase 5 in this roadmap).
