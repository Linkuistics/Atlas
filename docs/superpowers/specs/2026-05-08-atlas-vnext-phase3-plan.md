# Atlas vNext Phase 3 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. The
> Phase 3 status file at
> `docs/superpowers/plans/2026-05-08-phase3-status.md` carries the
> per-PR checkbox state across sessions.

**Status:** Plan (forward-looking; Phase 3 of the Atlas vNext system-model
redesign). Companion to `2026-05-08-atlas-vnext-phase3-design.md` (the
canonical Phase 3 design spec). Sequel to
`2026-05-07-atlas-vnext-phase2-plan.md` (Phase 2 closed; status in
`docs/superpowers/plans/2026-05-07-phase2-status.md`).

**Date:** 2026-05-08.

**Treatment:** Greenfield, carried forward from Phases 1 and 2. No
on-disk format compatibility with prior phases. No migration command. A
user upgrading deletes `.atlas/` and re-runs. Schema number stays at `1`
across the entire phase — no users exist yet, so version-bump ceremony
is unnecessary; the v1 *shape* mutates freely as each PR lands.

**Goal:** Decompose Phase 3 (§10.3 of the design spec, minus pattern
detection — deferred to Phase 4) into an ordered sequence of
independently-mergeable PRs that ship the four LLM-tooling-facing
analyses (drift, impact, modularity, composition divergence) plus the
editorial-vs-derived file classification regime (Phase 1 retrofit + new
overrides schema + gitignore mechanism). Phase 3 still ships as a
one-shot CLI; server mode is Phase 5.

**Architecture:** A new pure-function `crates/atlas-reports/` workspace
member exposes `drift()`, `impact()`, `modularity()`, `divergence()`
over a `ReportInputs<'a> { db, workspace }` view of the engine
database; the CLI handler does all I/O. Four cache-path retrofits move
Phase 1 outputs (`surfaces.yaml`, `component.yaml`, `components.yaml`,
`related-components.yaml`) under `.atlas/cache/`; the editorial tier
collapses to six file types (top-level `overrides`,
`external-components`, `subsystems`, `analyzers`, `config` +
per-component `overrides`). All cache writes are atomic
(temp+fsync+rename). Phase 5 conversion to Salsa-tracked queries is
mechanical because the report functions are already side-effect-free.

**Tech Stack:** Rust workspace; Salsa engine (carried); `serde_yaml`
for all YAML I/O; `toml` crate for any TOML reads (memory
`feedback_toml_parsing`); `sha2` for content hashing; existing
`atlas-engine` / `atlas-analyzers` / `atlas-cli` crates extended; new
`atlas-reports` crate added.

---

## 0. Reading order

Before this plan, read:

1. `2026-05-08-atlas-vnext-phase3-design.md` end-to-end. The design
   spec is the canonical source of scope, schema, formulas, and
   semantics. **This plan operationalises that design; where the two
   disagree, the design spec wins.**
2. `2026-05-06-atlas-system-model-design.md` §4.5–§4.6 (cache
   architecture and data co-location), §6 (file layout — Phase 3
   touches §6.1–§6.4), §10.3 (Phase 3 scope), §11.2 (open questions —
   Phase 3 closes §11.2.9 inline; the rest stay deferred). The Phase 3
   design spec §10 enumerates exactly which sections of this canonical
   spec change.
3. `2026-05-07-atlas-vnext-phase2-plan.md` §3 (mechanisms reused) and
   §4 (per-PR sub-sections — many remain reusable as Phase 3 starting
   points), §7 (Phase 2 deferrals — Phase 3 picks up zero of them; all
   remain Phase 4).
4. `docs/superpowers/plans/2026-05-07-phase2-status.md` per-PR notes
   — especially PR-0 (the Phase 2 schema-mutation trail), PR-2 (the
   subprocess-transport invariants the report code observes only
   indirectly), PR-13 (the L8 phantom fix that the Phase 3 smoke
   fixture must not regress), PR-14 (the polyglot fixture that Phase
   3 extends).
5. Memory entries that constrain Phase 3 (per design §11): if a
   listed memory file is missing locally, treat the design spec's
   one-line summary as the constraint and proceed:
   - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
   - `feedback_fix_all_lints` — every PR runs `cargo clippy
     --all-targets -- -D warnings` and `cargo fmt --check` clean.
   - `project_monorepo_consolidation` — long-term direction; informs
     that Phase 3 should not over-invest in multi-root-specific report
     flavours (the impact partition by deploy-graph is multi-root-aware
     by virtue of using existing engine outputs; no new multi-root
     code paths).

This plan does *not* re-derive the architecture; it sequences it. The
file structure, schemas, formulas, and edge semantics are all in the
design spec.

---

## 1. Phase 3 deliverable, restated

End of Phase 3, an Atlas user running the new report subcommands from a
Phase-2-shaped polyglot workspace shall see:

- **`atlas drift`** writes
  `<root>/.atlas/cache/contract-shas-snapshot.yaml` (baseline, stateful)
  and `<root>/.atlas/cache/reports/drift.yaml` (the report). On first
  run with no prior baseline: baseline captured, report empty,
  exit 0, message printed. On subsequent runs: contracts whose
  `content_sha` changed are listed with their pinned bindings.
- **`atlas impact <id>`** prints to stdout (JSON / YAML / human) the
  transitive consumer set for a contract or component id, with
  language / deploy-graph / lifecycle partition axes. Target-not-found
  exits 2 with Levenshtein-1 candidates on stderr. No file written.
- **`atlas modularity`** writes `<component>/.atlas/cache/modularity.yaml`
  for every component (with up-to-5-entry FIFO history) and a top-level
  `<root>/.atlas/cache/reports/modularity-rollup.yaml` carrying
  per-subsystem aggregates with >2σ outlier flags. Components not in
  any subsystem appear in `unattached_components`.
- **`atlas divergence`** writes
  `<root>/.atlas/cache/reports/composition-divergence.yaml` listing
  divergent component pairs (build-only XOR deploy-only) with severity
  scored against the last drift snapshot. Severity is `null` if no
  drift run has happened.
- Phase 1's `surfaces.yaml`, per-component `component.yaml`, top-level
  `components.yaml`, and `related-components.yaml` live under
  `<scope>/.atlas/cache/` and are gitignored via a one-line
  `.atlas/.gitignore` written on first cache-write.
- Top-level `overrides.yaml` accepts `edges_add` and `edges_suppress`
  (with required `reason` fields); per-component `overrides.yaml`
  accepts `language` / `kind` / `lifecycle` / `subsystem` field
  overrides.
- Every report run is deterministic; the cold and warm LLM call
  budgets match Phase 2's baselines (zero new LLM call sites — design
  §1).

Out of scope for Phase 3, deferred to later phases: pattern detection
(Phase 4); subprocess convergence and the bidirectional LLM callback
channel (Phase 4); rust-analyzer integration (Phase 4 stretch);
contract rename-match (Phase 4); LLM threshold calibration (Phase 4);
cache compression (Phase 4); worktree commit-sha consistency (Phase 4);
all Phase 2 closeout cleanups including `LenientBackend` extraction,
decoder consolidation, `is_manifest_file` extension for Makefile/shell,
L8 phantom-subcomponent fix, per-language Phase 3 refinements (Phase
4); server mode and reactive recomputation (Phase 5);
`--gate`/`--strict` exit-code flags (deferred indefinitely);
pass/fail thresholds for modularity scores (deferred indefinitely);
upstream/subsystem-input variants of impact query (deferred
indefinitely).

---

## 2. Open-question pre-conditions

Phase 3 introduces no new open questions on the design-spec-§11.2
docket and resolves one (§11.2.9) inline:

### 2.1 §11.2.9 — Editorial-vs-derived classification (RESOLVED IN-PHASE)

**Phase 3 resolution:** Editorial = user-asserted only (top-level
`overrides`, `external-components`, `subsystems`, `analyzers`, `config`
+ per-component `overrides` — six file types total). Derived =
everything else, gitignored under `<scope>/.atlas/cache/`. Phase 1's
`surfaces.yaml`, per-component `component.yaml`, top-level
`components.yaml`, and `related-components.yaml` are reclassified as
derived; PR-2..PR-5 of this plan retrofit them to the cache tier.
PR-0's design-doc touch-ups make the classification load-bearing in
the canonical `2026-05-06-atlas-system-model-design.md`.

The `reason` field is required on every `edges_add` and `edges_suppress`
entry in `overrides.yaml`. Per-component `overrides.yaml` field
overrides (`language`, `kind`, `lifecycle`, `subsystem`) supersede
analyser-emitted values for the corresponding fields.

### 2.2 Other open questions

All other §11.2 open questions stay deferred (§11.2.4 contract
rename-match, §11.2.5 query API auth, §11.2.6 LLM threshold
calibration, §11.2.7 cache compression, §11.2.8 worktree consistency)
— design §10.4 (Phase 4) is where they land.

---

## 3. Phase 1 + Phase 2 mechanisms reused as starting points

These mechanisms are extended rather than rewritten. They are *starting
points*, not compatibility constraints — under greenfield treatment,
any Phase 3 PR is free to refactor them when the new code demands it.

| Mechanism | Location | How Phase 3 uses it |
|---|---|---|
| `EngineDb` (Salsa workspace) + `Workspace` input | `crates/atlas-engine/src/db.rs` | Read-only by `atlas-reports`. The new `ReportInputs<'a> { db: &'a EngineDb, workspace: &'a Workspace }` borrows from the live engine; reports never mutate or trigger recomputation. |
| Persistent cache (`.atlas/cache/<stage>/<sha>.blob`) | `crates/atlas-engine/src/cache/mod.rs` | Reused unchanged. The new cache files in PR-2..PR-5 land in the same `.atlas/cache/` namespace alongside the existing Salsa-style cache. New report outputs live under `.atlas/cache/reports/`. |
| Atomic write helper | New in this plan (PR-1's gitignore writer is the first user; PR-8's drift-snapshot writer is the second; share via a `crates/atlas-engine/src/atomic_write.rs` util introduced in PR-1) | All stateful writes (drift snapshot, modularity history) and the per-scope `.gitignore` use temp+fsync+rename. |
| `OverridesFile` (top-level + per-component) | `atlas-contracts/crates/atlas-index/src/overrides.rs` (or wherever the existing OverridesFile lives — investigate which file holds the type; likely co-located with `components.rs`) | Extended in PR-6 with `edges_add: Vec<EdgeAdd>`, `edges_suppress: Vec<EdgeSuppress>` (top-level), and `language` / `kind` / `lifecycle` / `subsystem` (per-component). Existing `additions` / `pins` / `suppressions` carry forward unchanged. |
| L4 override merge (Phase 1 PR-6 + Phase 1 PR-0c spec) | `crates/atlas-engine/src/l4_tree.rs` | Extended in PR-6 to handle the new override fields. Existing scoping rules (per-component overrides may only target the component or its sub-components) carry forward to per-component field overrides. |
| L6 edge emission (analyser-discovered → unioned with overrides) | `crates/atlas-engine/src/l6_edges.rs` | Extended in PR-6 to read cached `related-components.yaml` (after PR-5's retrofit), union `edges_add`, subtract `edges_suppress`. The result is the canonical edge set. |
| L9 projections (per-component + top-level YAML writers) | `crates/atlas-engine/src/l9_projections.rs` + `crates/atlas-cli/src/pipeline.rs` | Modified in PR-2..PR-5: each retrofit PR redirects its writer to `<scope>/.atlas/cache/<file>` and updates every reader. |
| Phase 2 polyglot fixture (`phase2_polyglot_fixture.rs`) | `crates/atlas-cli/tests/` | Read-only baseline for PR-13 (Phase 3 smoke test). PR-13 forks/extends, does not mutate the Phase 2 file. |
| `subsystems.yaml` reader | `atlas-contracts/crates/atlas-index/src/subsystems.rs` (or wherever the subsystems schema lives — investigate; Phase 1 introduced it) | Read by PR-10 (modularity rollup) for member→subsystem mapping. Components not in any subsystem appear in `unattached_components`. |
| Contract `content_sha` and Binding `derived_from_contract_sha` | Phase 1 PR-7's surface schema (`atlas-contracts/crates/atlas-index/src/surfaces.rs`) | Read by PR-8 (drift) to compare current vs prior baseline; pinned-binding detection compares each binding's `derived_from_contract_sha` against the current contract `content_sha`. |
| Phase 1 PR-12's multi-root path-dep walker | `crates/atlas-engine/src/root_expansion.rs` | Read-only by `atlas-reports::impact` (the engine emits the union component set; impact traverses `consumes` edges within that set). |

---

## 4. PR sequence

PRs are numbered in dependency order. Sizes are estimates excluding
tests and excluding generated code. Each PR ends with passing
`cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
and `cargo fmt --check`.

### PR-0a — Plan + status (no code)

**Intent:** Land this plan and the Phase 3 status file. Establishes the
planning infrastructure that the Phase-3-shaped continuation prompt
keys off of (`*phase3-plan*` wildcard match → execution mode).

**Files:**
- Create: `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md` (this file).
- Create: `docs/superpowers/plans/2026-05-08-phase3-status.md` (PR checklist + dependency graph + per-PR notes section).

**Acceptance criteria:**
- Both documents land in their respective directories.
- The status file contains 15 PR checkboxes (PR-0a, PR-0b, PR-1..PR-13), all `[ ]` except PR-0a which is `[x]` after this commit.
- The Phase 3 continuation prompt at `docs/superpowers/prompts/2026-05-08-vnext-continue.md` (landing alongside this commit; not gated by plan PR-0a/0b) detects the plan's existence via `*phase3-plan*` and routes future sessions into Step 3 (execution).

**LOC:** 0 code; ~1100-1400 lines of plan + ~80-150 lines of status.

---

### PR-0b — Design-doc touch-ups in canonical system-model spec (no code)

**Intent:** Apply the nine design-doc touch-ups enumerated in Phase 3
design §10. Keeps the canonical
`2026-05-06-atlas-system-model-design.md` in sync with the Phase 3
decisions before any code lands. Lands as a docs-only commit, the
first task of the first execution session after PR-0a.

**Files:**
- Modify: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` per Phase 3 design §10 enumeration:
  1. **§10 renumbering.** Insert new §10.4 "Convergence and cleanups" (Phase 4); renumber old §10.4 "Server mode" → §10.5; renumber §10.5 "Migration from v1" → §10.6 (mark OBSOLETE — superseded by Phase 1's greenfield non-negotiable).
  2. **§10.3 scope clarification.** Replace the current bullet list with the four-analysis canonical scope (drift / impact / modularity / divergence). Move pattern detection out of §10.3 into the new §10.4. Add: *"§10.3 introduces no new LLM call sites; all analyses are pure aggregations over L4–L8 outputs."*
  3. **§10.4 (new — "Convergence and cleanups").** Phase 4 scope as enumerated in design §9.1.
  4. **§4.5 cache-architecture clarification.** Append: *"Cache files are local-only and gitignored by convention. `atlas-cli` writes a one-line `.gitignore` at each `.atlas/` scope on first cache-write, containing `cache/`. Cache portability across hosts is via explicit `atlas cache export/import` commands (deferred); cache is not shared via git."*
  5. **§4.6 data-co-location clarification.** Append: *"Co-located means same directory tree as source, not git-tracked alongside source. Editorial files are git-tracked; derived files (cache, reports) are gitignored."*
  6. **§6 file-layout sections** — add a "Git status" column. Editorial tier files marked `tracked`; derived tier marked `gitignored (under cache/)`. Update §6.1 (`components.yaml`), §6.2 (`component.yaml`), §6.3 (`surfaces.yaml`), §6.4 (`related-components.yaml`) to reflect new cache locations.
  7. **§11.2 open questions.** Add closed entry: *"§11.2.9 Editorial-vs-derived classification of on-disk files. **RESOLVED in Phase 3:** editorial = user-asserted only (overrides / external-components / subsystems / analyzers / config + per-component overrides); everything else is derived and gitignored under `cache/`. Phase 1 files surfaces.yaml, component.yaml, components.yaml, related-components.yaml are retrofit to derived tier in Phase 3."*
  8. **§11.2.5 renumber.** Update body text from "Phase 4 query API auth" → "defer to Phase 5 design" (server mode is now Phase 5).
  9. **§12 risks table.** Add row: *"Phase 3 retrofit (4 file-path moves + overrides schema extensions) leaves dangling readers. | Medium | High | Sweep tests + grep audit during each retrofit PR; greenfield rule means no migration path."*

**Acceptance criteria:**
- The canonical design spec is updated with all nine touch-ups in a single commit.
- Re-reading the canonical design spec end-to-end shows the §10 renumbering coherent (no broken cross-references), the new §10.4 enumerated, the §11.2.9 entry present, and the §12 risk row added.
- A grep for `"§10.4"` in the canonical design spec yields hits in the new "Convergence and cleanups" section, not in the old "Server mode" section.
- A grep for `"§10.5"` finds the renumbered "Server mode" content (formerly §10.4); a grep for `"§10.6"` finds the OBSOLETE-marked "Migration from v1" section (formerly §10.5).
- No new content is added to the canonical spec beyond the nine enumerated touch-ups; the rest of the spec is byte-stable.
- The Phase 3 status file's PR-0b checkbox flips to `[x]` after this commit.

**LOC:** 0 code; ~250-400 lines of design-doc touch-ups (counted as total inserted/modified lines across the canonical spec).

---

### PR-1 — Gitignore mechanism for `<scope>/.atlas/cache/`

**Intent:** Ship the per-scope `.gitignore` writer that PR-2..PR-5
need before they can write retrofit cache paths. The writer is
idempotent: file written iff absent; if present with different
content, respect-and-warn. Also introduces the shared atomic-write
helper that PR-8 (drift snapshot) and PR-10 (modularity history)
depend on.

**Files (in `crates/atlas-engine/src/`):**
- Create: `atomic_write.rs` — `pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()>` performing temp-file + fsync + rename. Temp file name: `<final>.tmp.<pid>.<rand-u64>`. On any error mid-write, the temp is best-effort cleaned up; the destination is never partially overwritten.
- Modify: `lib.rs` — re-export `atomic_write`.
- Create: `gitignore.rs` — `pub fn ensure_atlas_gitignore(scope: &Path) -> io::Result<EnsureGitignoreOutcome>`. The function:
  - Computes `scope.join(".atlas/.gitignore")`.
  - If the file does not exist: writes `cache/\n` via `atomic_write`. Returns `EnsureGitignoreOutcome::Wrote`.
  - If the file exists and contains a line `cache/` (trimmed): no-op. Returns `EnsureGitignoreOutcome::AlreadyPresent`.
  - If the file exists but does not contain a `cache/` line: leaves it alone, logs `eprintln!("warning: .atlas/.gitignore at <scope>/.atlas/.gitignore exists but does not list cache/; cache files may be tracked unintentionally")`. Returns `EnsureGitignoreOutcome::CustomisedWithoutCacheLine`.
- Modify: `lib.rs` — re-export `ensure_atlas_gitignore` and `EnsureGitignoreOutcome`.

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs` — at the point where the engine first writes any file under `<scope>/.atlas/`, call `ensure_atlas_gitignore(scope)` for each scope (top-level workspace `.atlas/` and every per-component `.atlas/`). The gitignore call itself uses `atomic_write` so a kill mid-write doesn't leave a half-written `.gitignore`. Emit at most one warning line per session per scope.

**Acceptance criteria:**
- New unit test (`atomic_write::tests`): `atomic_write_creates_destination` — call against an absent path, assert the file exists with the expected bytes after the call returns.
- New unit test: `atomic_write_overwrites_existing` — pre-populate the destination, call again, assert new bytes won.
- New unit test: `atomic_write_kill_during_write_leaves_destination_intact` — simulate kill mid-write (e.g. by injecting a `panic!` between temp-write and rename via a feature-gated hook) and assert the destination is either fully-old or absent, never half-written.
- New unit test (`gitignore::tests`): `ensure_writes_when_absent` — temp dir, call once, assert `.atlas/.gitignore` exists with content `cache/\n`.
- New unit test: `ensure_no_op_when_already_present` — write `cache/\n` first, call, assert outcome `AlreadyPresent` and file unchanged byte-for-byte.
- New unit test: `ensure_warns_when_customised_without_cache_line` — write `*.log\n` first, call, assert outcome `CustomisedWithoutCacheLine` and file unchanged.
- New integration test: a fresh-checkout fixture run through `pipeline.rs` writes `.atlas/.gitignore` at the top-level scope.
- Idempotency test: a second invocation of `pipeline.rs` against the same fixture does NOT rewrite `.atlas/.gitignore` (verified by mtime comparison or content-byte comparison).

**LOC:** ~250-400 (atomic-write + gitignore + tests).

---

### PR-2 — Phase 1 retrofit: per-component `surfaces.yaml` → cache

**Intent:** Move `<component>/.atlas/surfaces.yaml` to
`<component>/.atlas/cache/surfaces.yaml`. First of four cache-path
retrofits; the design spec (§5.4) sequences these as standalone
migration PRs because each is independently auditable. Greenfield
rule applies: no migration command. A user upgrading deletes `.atlas/`
and re-runs.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l9_projections.rs` — `surfaces_yaml_snapshot` is unchanged; only the writer's path changes.
- Modify: any reader that reads `<component>/.atlas/surfaces.yaml` for engine-internal use. Likely none in the engine itself (the engine works off in-memory state); investigate `crates/atlas-engine/tests/` and integration tests for hardcoded paths.

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs` — the `<component>/.atlas/surfaces.yaml` writer (introduced in Phase 1 PR-7) writes to `<component>/.atlas/cache/surfaces.yaml` instead. Ensure parent directory exists (`mkdir -p <component>/.atlas/cache/`). Use `atomic_write` from PR-1.
- Modify: every CLI integration test under `crates/atlas-cli/tests/` that reads or asserts on `<component>/.atlas/surfaces.yaml`. Update to the new path.

**Files (in `atlas-contracts/`):**
- Investigate: any consumer in atlas-contracts crates that reads per-component `surfaces.yaml`. If any exists, update the path. (Phase 1 PR-7's surfaces.yaml is the canonical projection of an engine-side invariant; downstream consumers like ravel-lite read it through `atlas-contracts` exports if at all.)

**Acceptance criteria:**
- New committed grep-audit script: `crates/atlas-cli/tests/grep_no_old_surfaces_path.sh`, exits 1 if any tracked file matches `\.atlas/surfaces\.yaml` (escaping the `.` properly). PR runs the script in CI.
- New end-to-end sweep test: `crates/atlas-cli/tests/phase3_retrofit_surfaces.rs` — runs `atlas index` on the Phase 2 polyglot fixture, asserts every component has a non-empty `<component>/.atlas/cache/surfaces.yaml` and zero `<component>/.atlas/surfaces.yaml` files exist.
- Phase 2 fixture-based tests continue to pass with the path update applied.
- New regression test: cache-hit on no-op rerun (verifies the path move did not break Phase 1's L5 cache invariant).
- The `.atlas/.gitignore` file at each scope contains `cache/` (verified by reading the file post-run).

**LOC:** ~350-550 (writer change + reader sweep + test fixture updates).

---

### PR-3 — Phase 1 retrofit: per-component `component.yaml` → cache

**Intent:** Move `<component>/.atlas/component.yaml` to
`<component>/.atlas/cache/component.yaml`. Second of four cache-path
retrofits. Independent of PR-2 (different file type, different writer
codepath); can land in parallel.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l9_projections.rs` — `per_component_yaml_snapshot` is unchanged; only the writer's path changes.
- Investigate: per-component `component.yaml` readers in the engine (Phase 1 PR-6 introduced this writer; readers may exist in `crates/atlas-cli` for verification flows).

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs` — the `<component>/.atlas/component.yaml` writer (Phase 1 PR-6) writes to `<component>/.atlas/cache/component.yaml`. Use `atomic_write`.
- Modify: every CLI integration test that reads per-component `component.yaml`. Update path.

**Files (in `atlas-contracts/`):**
- Investigate per-component `component.yaml` consumers (likely fewer than `surfaces.yaml` consumers; per-component `component.yaml` is a single-component projection of `components.yaml`).

**Acceptance criteria:**
- New committed grep-audit script: exits 1 if any tracked file matches `\.atlas/component\.yaml` (NOT `components.yaml` — anchor on the singular). Use a regex that excludes the plural form.
- New end-to-end sweep test: every component has `<component>/.atlas/cache/component.yaml` populated; zero `<component>/.atlas/component.yaml` files exist.
- Phase 2 fixture tests continue to pass with path updates applied.
- The per-component `component.yaml`'s `analyser_id` / `analyser_version` fields (Phase 2 PR-4) are still populated correctly.

**LOC:** ~350-550.

---

### PR-4 — Phase 1 retrofit: top-level `components.yaml` → cache

**Intent:** Move `<root>/.atlas/components.yaml` to
`<root>/.atlas/cache/components.yaml`. Third of four cache-path
retrofits. Independent of PR-2 / PR-3 (different file, different
scope); can land in parallel.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l9_projections.rs` — `components_yaml_snapshot` is unchanged; only the writer's path changes.

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs` — the top-level `<root>/.atlas/components.yaml` writer writes to `<root>/.atlas/cache/components.yaml`. Use `atomic_write`.
- Modify: every CLI integration test that reads `components.yaml` from the top-level scope.

**Files (in `atlas-contracts/`):**
- Investigate downstream `components.yaml` consumers. The Phase 1 PR-12 smoke-test pattern (atlas-contracts visible in Ravel-Lite) reads `components.yaml`; update its expected path.

**Acceptance criteria:**
- New committed grep-audit script: exits 1 if any tracked file matches `\.atlas/components\.yaml` outside the cache subdirectory.
- New end-to-end sweep test: top-level `<root>/.atlas/cache/components.yaml` populated; zero `<root>/.atlas/components.yaml` files exist.
- Phase 1 PR-12-style smoke test (atlas-contracts components visible in Ravel-Lite consumer) continues to pass with the path update.
- The `components.yaml` content is bit-for-bit identical to Phase 2's recorded output (modulo any header `path:` field that records the file's location).

**LOC:** ~350-550.

---

### PR-5 — Phase 1 retrofit: top-level `related-components.yaml` → cache

**Intent:** Move `<root>/.atlas/related-components.yaml` to
`<root>/.atlas/cache/related-components.yaml`. Fourth and final
cache-path retrofit. Settles the Phase 1 retrofit before PR-6
(overrides schema extension) lands the engine code that unions
`edges_add` against this cache file.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l9_projections.rs` — `related_components_yaml_snapshot` unchanged; writer path updated.
- Modify: any L6 reader that reads `related-components.yaml` from the previous run for forensics or rename-match. (Phase 1 PR-8 introduced this writer; rename-match operates on in-memory state, so likely no readers.)

**Files (in `crates/atlas-cli/src/`):**
- Modify: `pipeline.rs` — top-level `<root>/.atlas/related-components.yaml` writer writes to `<root>/.atlas/cache/related-components.yaml`. Use `atomic_write`.
- Modify: every CLI integration test that reads `related-components.yaml` from the top-level scope.

**Files (in `atlas-contracts/`):**
- Investigate downstream `related-components.yaml` consumers and update.

**Acceptance criteria:**
- New committed grep-audit script: exits 1 if any tracked file matches `\.atlas/related-components\.yaml` outside the cache subdirectory.
- New end-to-end sweep test: top-level `<root>/.atlas/cache/related-components.yaml` populated; zero `<root>/.atlas/related-components.yaml` files exist.
- Phase 2 fixture tests continue to pass with the path update.
- Edge-emission-from-Dockerfiles (Phase 1 PR-9) and edge-emission-from-Compose (Phase 2 PR-11) tests continue to pass.

**LOC:** ~350-550.

---

### PR-6 — Overrides schema extension: edges_add / edges_suppress + per-component field overrides

**Intent:** Extend `OverridesFile` (top-level + per-component) per
design §5.5. Top-level gains `edges_add` and `edges_suppress` (with
required `reason`); per-component gains four field overrides
(`language`, `kind`, `lifecycle`, `subsystem`). The L6 edge-emission
path (`crates/atlas-engine/src/l6_edges.rs`) reads
`<root>/.atlas/cache/related-components.yaml` (post-PR-5), unions
`edges_add`, subtracts `edges_suppress`. Per-component field overrides
flow through the L4 override merge.

**Files (in `atlas-contracts/crates/atlas-index/src/`):**
- Modify: `overrides.rs` (or wherever the existing `OverridesFile` lives — investigate; Phase 1 PR-1's PR notes name the file). Add types:
  ```rust
  #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
  pub struct EdgeAdd {
      pub kind: String,            // edge-kind name (e.g. "bundled-into")
      pub from: String,            // component or contract id
      pub to: String,
      pub reason: String,          // required; deserialisation rejects empty/missing
  }
  #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
  pub struct EdgeSuppress {
      pub kind: String,
      pub from: String,
      pub to: String,
      pub reason: String,
  }
  ```
  Top-level `OverridesFile` gains `pub edges_add: Vec<EdgeAdd>` and `pub edges_suppress: Vec<EdgeSuppress>` (both default to empty via `#[serde(default)]`).
- Modify: per-component `OverridesFile` shape gains `pub language: Option<String>`, `pub kind: Option<String>`, `pub lifecycle: Option<String>`, `pub subsystem: Option<String>` (each `#[serde(default)]`).
- Modify: round-trip serde tests already required by Phase 1's pattern — extend with one round-trip test per new field.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l4_tree.rs` — extend the per-component override merge to apply `language`, `kind`, `lifecycle`, `subsystem` field overrides. Each overrides the analyser-emitted value for the corresponding component-descriptor field. The existing scoping rule (per-component override may only target its own component or sub-components, per Phase 1 PR-0c) carries forward unchanged.
- Modify: `l6_edges.rs` — after the analyser-discovered edge set is computed, union with `edges_add` and subtract `edges_suppress`. Both are matched by `(kind, from, to)` triple. Edge-suppress that matches no analyser-discovered edge logs `eprintln!("warning: edges_suppress entry [<kind> <from> -> <to>] matched no analyser-discovered edge")`. Suppress-after-add semantics: if the same `(kind, from, to)` appears in both, the suppress wins (both reasons logged for forensics).
- Modify: relevant per-component-descriptor projection (`l9_projections.rs::per_component_yaml_snapshot`) — the projected `language` / `kind` / `lifecycle` / `subsystem` fields reflect the post-override values.

**Acceptance criteria:**
- Round-trip serde tests: each new field round-trips through `serde_yaml::to_string` then `from_str`.
- Schema-rejection test: an `edges_add` entry without `reason` deserialises to a clear error (not silent default-empty-string).
- Engine integration test: a fixture with `edges_add: [{ kind: bundled-into, from: a, to: b, reason: "manual annotation" }]` produces an `(a, b, bundled-into)` edge in the final `related-components.yaml`.
- Engine integration test: a fixture with an `edges_suppress` entry against an analyser-discovered edge eliminates that edge from the output.
- Engine integration test: a fixture where `edges_suppress` matches no analyser-discovered edge logs the documented warning to stderr (capture and assert) AND leaves the analyser-discovered set unchanged.
- Engine integration test: a per-component `overrides.yaml` with `language: rust` overrides an analyser-detected `language: python` (or whatever's relevant given the fixture) — the projected `component.yaml`'s language reflects the override.
- Engine integration test: per-component override scoping rule (Phase 1 PR-0c) — a per-component `overrides.yaml` whose `language` / `kind` field references a different component is rejected with a hard error matching the Phase 1 scoping-rejection error.
- A new memory or per-PR status note records that `edges_add` / `edges_suppress` exist as canonical mechanisms — useful for the Phase 4 LLM analysers that may emit candidate edges as add-suggestions.

**LOC:** ~700-1100 (atlas-contracts schema + engine merge logic + l6_edges union/suppress + tests).

---

### PR-7 — `atlas-reports` crate scaffold + CLI subcommand framework

**Intent:** Create the new `crates/atlas-reports/` workspace member
with the four pure-function signatures and accompanying types from
design §3.2. Add four CLI subcommands (`atlas drift`, `atlas impact`,
`atlas modularity`, `atlas divergence`) that dispatch into the crate;
each handler initially returns a `ReportError::NotImplemented`. PR-7
ships only the scaffolding — actual report logic lands in PR-8..PR-11.
This decoupling lets PR-8..PR-11 ship in parallel.

**Files (new crate):**
- Create: `crates/atlas-reports/Cargo.toml` — workspace member; deps: `atlas-engine` (path), `atlas-contracts/crates/atlas-index` (path), `serde`, `serde_yaml`, `serde_json`, `sha2`, `chrono` (for `Utc::now()` timestamps), `thiserror`.
- Create: `crates/atlas-reports/src/lib.rs` — re-exports.
- Create: `crates/atlas-reports/src/types.rs`:
  ```rust
  pub struct ReportInputs<'a> {
      pub db: &'a EngineDb,
      pub workspace: &'a Workspace,
  }

  pub enum ImpactTarget {
      Contract(ContractId),
      Component(ComponentId),
  }

  pub enum ReportError {
      NotImplemented,                          // returned from PR-7 stubs
      TargetNotFound { needle: String, candidates: Vec<String> },  // PR-9
      Io(io::Error),                           // PR-8/10/11/12 file writers
      // additional variants added by PR-8..PR-11 as the report logic lands
  }
  ```
- Create: `crates/atlas-reports/src/snapshot.rs` — `pub struct ContractShaSnapshot { pub schema_version: u32, pub captured_at: DateTime<Utc>, pub contract_shas: BTreeMap<ContractId, Sha256Hex> }`. Round-trip serde tests.
- Create: `crates/atlas-reports/src/drift.rs`:
  ```rust
  pub fn drift(
      _inputs: ReportInputs,
      _prev_snapshot: Option<ContractShaSnapshot>,
  ) -> Result<(DriftReport, ContractShaSnapshot), ReportError> {
      Err(ReportError::NotImplemented)
  }

  pub struct DriftReport { /* shape from design §4.1 */ }
  ```
  Plus the report struct with full schema (`schema_version`, `generated_at`, `baseline_captured_at`, `contracts_changed`, `contracts_added`, `contracts_removed`, `summary`). Round-trip serde tests for the report struct.
- Create: `crates/atlas-reports/src/impact.rs`:
  ```rust
  pub fn impact(
      _inputs: ReportInputs,
      _target: ImpactTarget,
  ) -> Result<ImpactReport, ReportError> {
      Err(ReportError::NotImplemented)
  }

  pub struct ImpactReport { /* shape from design §4.2 */ }
  ```
- Create: `crates/atlas-reports/src/modularity.rs`:
  ```rust
  pub fn modularity(
      _inputs: ReportInputs,
      _prior_per_component: HashMap<ComponentId, ModularityHistory>,
  ) -> Result<ModularityReport, ReportError> {
      Err(ReportError::NotImplemented)
  }

  pub struct ModularityReport { /* per-component map + rollup; design §4.3 */ }
  pub struct ModularityHistory { /* design §4.3 */ }
  ```
- Create: `crates/atlas-reports/src/divergence.rs`:
  ```rust
  pub fn divergence(
      _inputs: ReportInputs,
      _drift_baseline: Option<&ContractShaSnapshot>,
  ) -> Result<DivergenceReport, ReportError> {
      Err(ReportError::NotImplemented)
  }

  pub struct DivergenceReport { /* shape from design §4.4 */ }
  ```

**Files (workspace + CLI):**
- Modify: top-level `Cargo.toml` — add `crates/atlas-reports` to `members`.
- Modify: `crates/atlas-cli/Cargo.toml` — add `atlas-reports` (path).
- Modify: `crates/atlas-cli/src/main.rs` — register four new subcommands. Use `clap`'s subcommand pattern (already used by `atlas index`). Each handler:
  - Loads the engine database (cached or recomputed; same path as `atlas index`).
  - Calls the corresponding `atlas_reports::*` function.
  - On `Err(ReportError::NotImplemented)`, prints `"<subcommand> is not yet implemented"` to stderr and exits 1.
- Add: `--json | --yaml | --human` flag to all four subcommands (clap parses; routing is no-op in PR-7).
- Add: `--no-write` flag to `drift`, `modularity`, `divergence` (rejected with clear error for `impact`, which never persists).
- Add: per-subcommand argument parsing — `impact` takes `<id>` positional.

**Acceptance criteria:**
- `cargo build --workspace` succeeds with the new crate.
- `cargo test -p atlas-reports` passes (round-trip serde tests for snapshot + each report struct).
- `atlas drift --help` / `atlas impact --help` / `atlas modularity --help` / `atlas divergence --help` print usage including the format flags.
- `atlas drift` invocation against a fixture exits 1 with the documented stderr message; the same for the other three.
- `atlas impact --no-write foo` returns a clear error rejecting the flag.
- A round-trip test loads a hand-written `contract-shas-snapshot.yaml` exemplar from design §4.1 and serialises it back identically (modulo timestamp formatting).
- The new `atlas-reports` crate's README / lib.rs docstring documents the Phase 5 conversion path (the `#[salsa::tracked]` migration) per design §3.5.

**LOC:** ~900-1300 (crate scaffold + types + 4 stubbed functions + CLI dispatch + tests).

---

### PR-8 — Drift report + `atlas drift` CLI subcommand

**Intent:** First report implementation. Computes drift between the
current engine output and the prior `contract-shas-snapshot.yaml`
baseline. The CLI handler writes both the report and the new snapshot
atomically (using PR-1's helper). On first run with no baseline,
captures and exits clean with a guidance message.

**Files (in `crates/atlas-reports/src/`):**
- Modify: `drift.rs` — implement `drift()` per design §4.1:
  - Iterate every contract in `inputs.db` — collect `(contract_id, current_content_sha)`.
  - Iterate every binding in `inputs.db` — collect `(component_id, contract_id, binding_content_sha, derived_from_contract_sha, language)`.
  - If `prev_snapshot.is_none()`: return `(DriftReport { baseline_captured_at: None, contracts_changed: [], contracts_added: [], contracts_removed: [], summary: Summary { total_contracts, changed: 0, added: 0, removed: 0, pinned_bindings_count: 0 } }, ContractShaSnapshot { ... captured from current })`.
  - Otherwise:
    - `contracts_changed`: for each contract in both prev and current with `prev != current`, walk every binding consuming that contract; for each, if `binding.derived_from_contract_sha == prev` add it to `pinned_bindings`.
    - `contracts_added`: in current but not prev.
    - `contracts_removed`: in prev but not current.
    - Compute `summary.pinned_bindings_count` as the sum across all changed contracts.
  - Return `(report, new_snapshot)` — even when the baseline already exists, the new snapshot is the fresh capture.

**Files (in `crates/atlas-cli/src/`):**
- Modify: the `atlas drift` handler:
  1. Resolve the workspace primary root (same way as `atlas index`).
  2. Load engine database; if not cached, run the index pipeline (`pipeline::run` or equivalent).
  3. Construct `ReportInputs { db: &engine, workspace: &ws }`.
  4. Read prior snapshot from `<root>/.atlas/cache/contract-shas-snapshot.yaml`, if present; deserialise; on parse error, print warning and treat as `None`.
  5. Call `atlas_reports::drift(inputs, prev_snapshot)`.
  6. Render output to stdout in the requested format (default human).
  7. Unless `--no-write`: write `<root>/.atlas/cache/reports/drift.yaml` (atomic) and `<root>/.atlas/cache/contract-shas-snapshot.yaml` (atomic). Mkdir-p as needed.
  8. Print a one-line summary (counts of changed/added/removed/pinned).
  9. First-run UX: if no prior snapshot, print `"No prior baseline found. Captured baseline of N contracts. Run \`atlas drift\` again after changes to see drift."`.
  10. Exit 0.

**Acceptance criteria (unit tests in `crates/atlas-reports/`):**
- `drift_first_run_no_baseline` — `prev_snapshot = None`, fixture with 3 contracts → report has empty change arrays, snapshot captures all 3.
- `drift_baseline_unchanged` — `prev_snapshot` matches current → empty change arrays.
- `drift_baseline_changed` — one contract's `content_sha` differs → that contract is in `contracts_changed` with the prior and current shas.
- `drift_contract_added` — fixture adds a contract → it's in `contracts_added`.
- `drift_contract_removed` — fixture removes a contract → it's in `contracts_removed`.
- `drift_pinned_binding_detected` — a binding whose `derived_from_contract_sha == prior` and the contract changed → binding appears under `pinned_bindings` for that contract.
- `drift_pinned_binding_up_to_date` — a binding whose `derived_from_contract_sha == current` → NOT in `pinned_bindings`.

**Acceptance criteria (CLI integration tests in `crates/atlas-cli/tests/`):**
- `atlas_drift_first_run_writes_snapshot_and_empty_report` — fresh fixture, run `atlas drift`, assert both cache files exist; report has empty change arrays.
- `atlas_drift_second_run_after_contract_change_reports_drift` — first run captures baseline, mutate one contract, second run reports it.
- `atlas_drift_no_write_flag_skips_writes` — `atlas drift --no-write` after a previous run does NOT mutate the existing snapshot or report files (verified by mtime).
- `atlas_drift_kill_during_snapshot_write_leaves_file_intact` — kill-during-write fixture (using PR-1's atomic-write helper); snapshot is either fully-old or fully-new, not half-written.

**LOC:** ~1000-1400 (drift logic + CLI handler + atomic-write integration + tests).

---

### PR-9 — Impact query + `atlas impact <id>` CLI subcommand

**Intent:** Stdout-only report. Walk `consumes` edges from a target
contract or component to the transitive consumer set, partitioned by
language / deploy-graph / lifecycle. Cycle-safe via a seen-set.
Target-not-found returns `ReportError::TargetNotFound` with
Levenshtein-1 candidates; CLI translates to exit code 2 with stderr
suggestions.

**Files (in `crates/atlas-reports/src/`):**
- Modify: `impact.rs` — implement `impact()` per design §4.2:
  - Resolve the target id: try contract-namespace match, then component-namespace match. If neither resolves, return `Err(ReportError::TargetNotFound { needle: id, candidates: levenshtein_distance_1_candidates(id, &all_ids) })`.
  - For contract input: walk `consumes-contract` edges from the target contract; for each consumer component, recurse through every contract it provides and walk consumers transitively.
  - For component input: union of impact sets across each contract the component provides.
  - Use a seen-set (`BTreeSet<ComponentId>`) for cycle safety.
  - Direction: downstream consumers only (no upstream).
  - Edge type: `consumes-contract` only. `depends-on` build edges are NOT walked (per design §4.2).
  - Build three independent partitions: by language, by deploy graph, by lifecycle. Each maps every component in `transitive_consumers` to its value on that axis. Components with no value on an axis appear under a `null` / `unknown` bucket.

**Files (in `crates/atlas-cli/src/`):**
- Modify: the `atlas impact` handler:
  1. Parse positional `<id>` argument.
  2. Load engine database.
  3. Construct `ReportInputs`.
  4. Call `atlas_reports::impact(inputs, target)`.
  5. On `Ok(report)`: render to stdout (default human; `--json` / `--yaml` produce structured output). Exit 0.
  6. On `Err(ReportError::TargetNotFound { needle, candidates })`: print `"target not found: <needle>"` to stderr; if candidates non-empty, print `"did you mean:\n  - <c1>\n  - <c2>\n  ..."`. Exit 2.
  7. `--no-write` is rejected at clap level for this subcommand.

**Acceptance criteria (unit tests in `crates/atlas-reports/`):**
- `impact_direct_only_consumer_returned` — fixture with one contract consumed by exactly one component → that component is the only consumer.
- `impact_transitive_consumer_returned` — A consumes B's contract; B consumes C's contract; impact on C → both A and B in `transitive_consumers`.
- `impact_cycle_safe` — A consumes B; B consumes A → impact on either returns the cycle members exactly once each (no infinite loop).
- `impact_partition_by_language_correct` — three consumers, mixed languages → each lists under its language partition.
- `impact_partition_by_deploy_graph_correct` — fixture with two compose orchestrations covering different consumers → partition reflects deploy-graph membership.
- `impact_partition_by_lifecycle_correct` — runtime / build-time / test-only consumers each in their bucket.
- `impact_target_not_found_returns_levenshtein_candidates` — query `"ravel-lit"` against fixture containing `"ravel-lite/api"` → candidates include `"ravel-lite/api"`.
- `impact_empty_result_for_unconsumed_contract` — contract no one consumes → empty `direct_consumers` and `transitive_consumers`.
- `impact_contract_input_vs_component_input` — same contract via contract id and via providing-component id produce the same consumer set (assuming the component provides only that contract).

**Acceptance criteria (CLI integration tests):**
- `atlas_impact_known_target_human_format` — output contains an indented tree of consumers.
- `atlas_impact_json_format` — output parses as JSON matching the schema.
- `atlas_impact_target_not_found_exits_2` — exit code 2 + stderr contains `"did you mean"`.
- `atlas_impact_no_write_flag_rejected` — `atlas impact --no-write foo` exits non-zero with a clear flag-not-applicable error.

**LOC:** ~900-1200.

---

### PR-10 — Modularity report + `atlas modularity` CLI subcommand

**Intent:** Per-component metrics (Ca / Ce / Instability / Cohesion /
Surface stability / Surface complexity) per design §4.3, plus a
top-level rollup with subsystem aggregates and >2σ outlier flags.
Stateful: per-component `modularity.yaml` carries up to 5 history
entries (FIFO). The CLI handler reads each component's prior history
before invocation; `atlas-reports::modularity()` is pure over its
`prior_per_component` input.

**Files (in `crates/atlas-reports/src/`):**
- Modify: `modularity.rs` — implement six metric formulas and history rotation:
  - **Afferent coupling (Ca)**: count distinct components consuming any contract this component provides. `consumes-contract` edges only. Self-loops excluded.
  - **Efferent coupling (Ce)**: count distinct components whose contracts this component consumes. Self-loops excluded.
  - **Instability (I)** = `Ce / (Ca + Ce)`. When `Ca + Ce == 0`, I = 0.0.
  - **Cohesion** = `1 - ((distinct_consumer_sets - 1) / (num_provided_contracts - 1))`. With 0 or 1 provided contracts, cohesion = 1.0 (vacuous).
  - **Surface stability** = `matching_adjacent_pairs / total_adjacent_pairs` over up to last 5 history entries. With <2 entries, stability = 1.0 (no pairs possible).
  - **Surface complexity** = `provided_contracts × avg_bindings_per_contract` (integer; raw count).
  - History rotation: if current `surface_fingerprint` matches the most-recent history entry's fingerprint → no append (no duplicate). Otherwise prepend; if total >5, drop oldest. History entries are immutable once written.
- Implement subsystem aggregates: read `subsystems.yaml` from the engine state; for each subsystem, compute `mean` and `stddev` of each metric across members. Flag any member whose value is `>2σ` from the subsystem mean as an outlier *for that metric* (multiple outlier rows possible per component if it's outlier on multiple metrics). Components not in any subsystem appear in `unattached_components` with their ids and a count.

**Files (in `crates/atlas-cli/src/`):**
- Modify: the `atlas modularity` handler:
  1. Load engine database.
  2. Walk every component; for each, read `<component>/.atlas/cache/modularity.yaml` if present; deserialise to `ModularityHistory`. Assemble `HashMap<ComponentId, ModularityHistory>`.
  3. Call `atlas_reports::modularity(inputs, prior_per_component)`.
  4. Render output (`--json` / `--yaml` / `--human`).
  5. Unless `--no-write`: write each component's `<component>/.atlas/cache/modularity.yaml` (atomic) AND `<root>/.atlas/cache/reports/modularity-rollup.yaml` (atomic). Mkdir-p as needed.
  6. Print summary: number of components, number of subsystems, number of outliers, number of unattached.

**Acceptance criteria (unit tests in `crates/atlas-reports/`):**
- One unit test per formula with a hand-crafted fixture and a hand-computed expected value:
  - `ca_counts_distinct_consumers`
  - `ce_counts_distinct_provided-by`
  - `instability_zero_when_no_couplings`
  - `instability_correct_for_balanced_couplings`
  - `cohesion_one_for_zero_or_one_contract`
  - `cohesion_decreases_with_disjoint_consumer_sets`
  - `surface_stability_one_with_lt_2_history_entries`
  - `surface_stability_correct_with_4_entries`
  - `surface_complexity_zero_for_no_contracts`
- `history_rotation_no_duplicate_when_fingerprint_matches` — simulate same input twice; second run does not append.
- `history_rotation_drops_oldest_at_5_entries` — six entries with distinct fingerprints; final history has the most recent 5, oldest dropped.
- `subsystem_aggregate_mean_stddev_correct` — three-member subsystem with hand-computed mean and stddev → asserted exact.
- `subsystem_outlier_flagged_at_2_sigma` — one member at 2.5σ from mean → flagged.
- `subsystem_no_outliers_when_all_within_2_sigma` — three members tightly clustered → empty `outliers`.
- `unattached_components_listed_correctly` — workspace with two subsystem-tagged and one untagged → `unattached_components.count == 1`, ids match.
- `empty_subsystems_yaml_produces_unattached_only_rollup` — graceful fallback when no `subsystems.yaml` exists.

**Acceptance criteria (CLI integration tests):**
- `atlas_modularity_first_run_writes_per_component_files` — fresh run; every component has `<component>/.atlas/cache/modularity.yaml` with one history entry.
- `atlas_modularity_second_run_with_no_changes_no_history_append` — second run; per-component files still have one entry (no duplicate).
- `atlas_modularity_second_run_with_surface_change_appends_history` — mutate a component's surface; per-component file has 2 entries.
- `atlas_modularity_writes_rollup_at_top_level` — `<root>/.atlas/cache/reports/modularity-rollup.yaml` exists with subsystem aggregates.
- `atlas_modularity_no_write_skips_writes` — `--no-write` does not mutate any cache file.

**LOC:** ~1300-1700 (formulas + history + aggregates + CLI handler + tests).

---

### PR-11 — Composition divergence + `atlas divergence` CLI subcommand

**Intent:** Pair classification between every component pair in the
workspace. A pair is **divergent** iff exactly one of build-coupled
(direct `depends-on`) or deploy-coupled (any composition edge) is
true. Severity = count of contracts the pair shares whose `content_sha`
changed since the drift baseline. No drift baseline → severity is
`null`.

**Files (in `crates/atlas-reports/src/`):**
- Modify: `divergence.rs` — implement `divergence()` per design §4.4:
  - Compute build edges = direct `depends-on` edges from L4–L8.
  - Compute deploy edges = any composition edge (`bundled-into`, `co-deployed-with`, `orchestrated-by`, etc. — see Phase 1 PR-9 + Phase 2 PR-11).
  - For each unordered pair `{A, B}`:
    - `build_coupled = direct_edge_exists(A, B, "depends-on") || direct_edge_exists(B, A, "depends-on")`.
    - `deploy_coupled = any_composition_edge_between(A, B)`.
    - Pair is **divergent** iff `build_coupled XOR deploy_coupled`.
  - For each divergent pair: compute shared contracts = intersection of `(consumes ∪ provides)` for A and B. Severity = count of shared contracts where `baseline.contract_shas[id]` is missing OR `current_content_sha != baseline.contract_shas[id]`. If `drift_baseline.is_none()`, severity is `None` for all pairs and the report header notes "drift baseline absent".
  - Sort divergent pairs lexicographically by `(min(A, B), max(A, B))` for deterministic output.

**Files (in `crates/atlas-cli/src/`):**
- Modify: the `atlas divergence` handler:
  1. Load engine database.
  2. Read drift snapshot from `<root>/.atlas/cache/contract-shas-snapshot.yaml` if present (read-only — divergence does NOT modify the drift snapshot).
  3. Call `atlas_reports::divergence(inputs, drift_baseline.as_ref())`.
  4. Render output.
  5. Unless `--no-write`: write `<root>/.atlas/cache/reports/composition-divergence.yaml` (atomic).
  6. Print summary: total pairs examined, divergent count, by-severity histogram.

**Acceptance criteria (unit tests in `crates/atlas-reports/`):**
- `divergence_pair_classification_build_only` — fixture with `depends-on` but no composition → divergent, coupling `build_only`.
- `divergence_pair_classification_deploy_only` — composition edge but no `depends-on` → divergent, coupling `deploy_only`.
- `divergence_pair_classification_both` — both edges present → NOT divergent.
- `divergence_pair_classification_neither` — no edges → NOT divergent.
- `divergence_severity_zero_when_no_shared_contracts_drifted` — divergent pair, shared contracts all unchanged since baseline → severity 0.
- `divergence_severity_counts_drifted_shared_contracts` — divergent pair, two shared contracts changed since baseline → severity 2.
- `divergence_severity_counts_added_shared_contracts` — contract added since baseline → counts toward severity.
- `divergence_severity_null_without_baseline` — `drift_baseline = None` → severity is `None` for all pairs; report header reflects baseline absence.
- `divergence_empty_when_no_divergent_pairs` — fixture with consistent build+deploy coupling → empty `divergent_pairs`.

**Acceptance criteria (CLI integration tests):**
- `atlas_divergence_after_drift_writes_severity_aware_report` — first run drift (captures baseline), then divergence → report has severity values.
- `atlas_divergence_without_prior_drift_writes_null_severity_report` — divergence run with no prior drift baseline → header notes baseline absent, severity null everywhere.
- `atlas_divergence_no_write_skips_writes` — `--no-write` honored.
- `atlas_divergence_does_not_modify_drift_snapshot` — divergence run after drift; snapshot mtime/content unchanged.

**LOC:** ~900-1200.

---

### PR-12 — Atomic-write fixture suite for stateful files

**Intent:** Stress-test the atomic-write semantics for the two
stateful files Phase 3 introduces: drift snapshot (PR-8) and
modularity per-component history (PR-10). Design §6.3 explicitly
requires kill-during-write fixtures. PR-1's atomic-write helper has
its own basic test; this PR exercises it under the realistic
serialise+write pattern of the report pipelines.

**Files (in `crates/atlas-reports/tests/`):**
- Create: `atomic_writes.rs` — kill-during-write fixtures for both stateful patterns:
  - `drift_snapshot_kill_during_write_leaves_file_intact`: pre-populate snapshot with state S1; invoke a wrapper that simulates kill between `atomic_write`'s temp-write and rename phases (e.g. via a cargo feature gate `atomic_write_panic_after_temp` that's only enabled in this test); re-read snapshot after recovery; assert content equals S1 (still the old state).
  - `drift_snapshot_kill_after_rename_succeeds`: same fixture, kill simulated after rename; re-read snapshot; assert content equals new state.
  - `modularity_history_kill_during_write_preserves_prior_5_entries`: per-component file with 5 prior entries; kill simulated mid-write of the rotated 6th entry; re-read; assert all 5 prior entries intact.
  - `modularity_history_kill_after_rename_persists_rotation`: same, kill after rename; re-read; assert rotation persisted (oldest dropped, new entry first).
  - Both stateful files: stress-test by running the atomic-write through 10 simulated kills at random points in the write sequence; re-read each time; assert the file contents are always one of {prior, new}, never partial.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `atomic_write.rs` — add a `#[cfg(test)]` (or feature-gated) hook that lets the test inject a panic-after-temp-write or panic-before-rename. The hook must NOT exist in release builds.

**Acceptance criteria:**
- All five named fixtures pass.
- The 10-iteration random-kill stress test passes deterministically (seed the RNG so failures are reproducible).
- A code-quality check: the panic-injection hook is `#[cfg(test)]` or feature-gated and absent from `cargo build --release` (asserted via a build-time check or a manual review note in the PR description).

**LOC:** ~400-700.

---

### PR-13 — Phase 3 polyglot smoke test

**Intent:** End-to-end acceptance test that exercises all four reports
plus the Phase 3 retrofit/overrides extensions against an extension of
Phase 2's polyglot fixture. The fixture is hermetic (checked-in), runs
end-to-end without network, and asserts on structured outputs (not
serialised YAML strings).

**Files (in `crates/atlas-cli/tests/`):**
- Create: `phase3_polyglot_fixture.rs` — extends `crates/atlas-cli/tests/fixtures/phase2_polyglot/` (read-only baseline) into a new `crates/atlas-cli/tests/fixtures/phase3_polyglot/`:
  - **Drift trigger.** One contract whose `content_sha` is rewritten between two engine runs. Achieved by a fixture-helper that mutates the binding source after the first run.
  - **Divergence trigger #1.** Two components deploy-coupled-only via `co-deployed-with` (compose service pair) with NO `depends-on` edge.
  - **Divergence trigger #2.** Two components build-coupled-only via `depends-on` with NO composition edge.
  - **Modularity outlier.** One component with deliberate ~10× efferent coupling vs its subsystem peers (drives the `>2σ` outlier flag).
  - **Subsystem fixture.** One subsystem with three members defined in `subsystems.yaml`.
  - **Edges_add fixture.** One user-asserted edge in `<root>/.atlas/overrides.yaml::edges_add` (with `reason`).
  - **Edges_suppress fixture.** One user-suppressed edge in `<root>/.atlas/overrides.yaml::edges_suppress` (with `reason`).
- Run order:
  1. `atlas index` (cold). Assert L4 cache populated with PR-2..PR-5 retrofit paths; gitignore present.
  2. `atlas drift` (first run). Assert baseline captured; report empty change arrays; first-run UX message printed.
  3. Mutate one contract.
  4. `atlas index` (warm + delta). Assert exactly the affected component re-classifies.
  5. `atlas drift` (second run). Assert one entry in `contracts_changed` with the expected pinned-binding entries.
  6. `atlas modularity`. Assert per-component files written; rollup written; the deliberate-outlier component is in the subsystem's `outliers` for `efferent_coupling`.
  7. `atlas divergence`. Assert two divergent pairs (one `deploy_only`, one `build_only`); severity reflects the contract mutated in step 3 if shared.
  8. `atlas impact <known-id>` for a contract chosen so that the transitive consumer set is non-empty. Assert partition axes correctly populated.
- LLM call budget assertions:
  - Cold (step 1): same as Phase 2's PR-14 baseline (~26; verify exact via the `PR14Backend` helper). Phase 3 introduces zero new LLM call sites.
  - Warm rerun (re-running step 1 with no source mutations): 0 LLM calls.
  - Steps 2/5/6/7/8: 0 LLM calls (every report is deterministic).
- Cache-discipline assertions:
  - All cache files live under `<scope>/.atlas/cache/`, NEVER outside.
  - `<scope>/.atlas/.gitignore` exists at every scope and contains `cache/`.
  - All eight new Phase 3 cache files are populated by end of run order:
    - `<root>/.atlas/cache/contract-shas-snapshot.yaml`
    - `<root>/.atlas/cache/reports/drift.yaml`
    - `<root>/.atlas/cache/reports/modularity-rollup.yaml`
    - `<root>/.atlas/cache/reports/composition-divergence.yaml`
    - `<root>/.atlas/cache/components.yaml` (PR-4 retrofit)
    - `<root>/.atlas/cache/related-components.yaml` (PR-5 retrofit)
    - `<component>/.atlas/cache/modularity.yaml` per component (PR-10)
    - `<component>/.atlas/cache/{surfaces.yaml,component.yaml}` per component (PR-2 + PR-3 retrofit)
- Override fixture assertions:
  - `edges_add` entry materialises in `<root>/.atlas/cache/related-components.yaml`.
  - `edges_suppress` entry eliminates the matching analyser-discovered edge from the same file.
  - Both PR-6 warning paths (suppress-matches-nothing) tested elsewhere; this fixture exercises the happy path.

**Files (in `crates/atlas-cli/tests/fixtures/phase3_polyglot/`):**
- Copy and extend the Phase 2 fixture's contents (do not mutate the Phase 2 fixture). Retain the `*.buildkite` Dockerfile suffix from Phase 2 PR-14.
- Add: `<root>/.atlas/overrides.yaml` with the `edges_add` and `edges_suppress` entries.
- Add: `<root>/.atlas/subsystems.yaml` with the three-member subsystem.
- Add: a deliberate-outlier component (could be a tiny extra Rust crate that consumes ~10 contracts to push its efferent coupling above 2σ).

**Acceptance criteria:**
- The integration test runs to completion under `cargo test --workspace --no-fail-fast`.
- All structured assertions pass.
- LLM-budget assertions pass (cold ≤ Phase 2 baseline; warm + reports = 0).
- The Phase 2 fixture file (`phase2_polyglot_fixture.rs`) is unchanged after this PR (verified by diff in code review).

**LOC:** ~800-1200 fixture files + ~600-900 test code.

---

## 5. Acceptance criteria summary (per-PR table)

The following table is the canonical acceptance gate. A PR may not
land until every row in its column is green.

| PR | Tests pass | New unit/integration tests | Smoke test contributes to |
|---|---|---|---|
| PR-0a | n/a (docs)        | n/a                                                                                        | n/a |
| PR-0b | n/a (docs)        | n/a                                                                                        | n/a |
| PR-1  | atlas-engine      | atomic-write happy/kill, gitignore absent/present/customised, integration: pipeline writes gitignore | PR-13 (`.gitignore` exists at every scope) |
| PR-2  | workspace         | grep-audit zero hits, sweep test: no surfaces.yaml outside cache, cache-hit rerun           | PR-13 (per-component cache/surfaces.yaml populated) |
| PR-3  | workspace         | grep-audit zero hits, sweep test: no component.yaml outside cache                          | PR-13 (per-component cache/component.yaml populated) |
| PR-4  | workspace         | grep-audit zero hits, sweep: no top-level components.yaml outside cache                    | PR-13 (top-level cache/components.yaml populated) |
| PR-5  | workspace         | grep-audit zero hits, sweep: no top-level related-components.yaml outside cache            | PR-13 (top-level cache/related-components.yaml populated) |
| PR-6  | workspace         | round-trip serde, edges_add union, edges_suppress subtract + warning, per-component field overrides, scoping rejection | PR-13 (edges_add+suppress fixture asserts) |
| PR-7  | atlas-reports     | round-trip serde for snapshot + each report struct, all four CLI subcommands print help    | PR-13 (CLI subcommands route into atlas-reports) |
| PR-8  | workspace         | first-run, baseline-changed, contract-added/removed, pinned-binding detected/up-to-date, kill-during-write atomicity | PR-13 step 2/5 (drift first-run + drift after change) |
| PR-9  | workspace         | direct/transitive/cycle, partition-by-{language/deploy/lifecycle}, target-not-found candidates, empty result | PR-13 step 8 (impact on known contract) |
| PR-10 | workspace         | each formula has a unit test, history rotation FIFO, subsystem aggregate, outlier flag, unattached_components | PR-13 step 6 (modularity outlier flagged) |
| PR-11 | workspace         | pair classification (4 cases), severity (with/without baseline), divergence-does-not-modify-snapshot | PR-13 step 7 (two divergent pairs) |
| PR-12 | atlas-reports     | drift-snapshot kill-during-write/after-rename, modularity-history kill-during-write/after-rename, 10-iteration stress | PR-13 (all stateful writes are kill-safe) |
| PR-13 | e2e               | polyglot Phase 3 fixture: 4 reports + retrofit + overrides; LLM budget + cache discipline | this *is* the smoke test |

---

## 6. Risks (Phase 3 specific)

These are operational risks for the Phase 3 implementation,
supplementing design §8 (architectural risks) and the Phase 1/2 plan
risks (which carry forward).

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| One of PR-2..PR-5's retrofit sweeps misses a hardcoded path reference, leaving the engine reading from one location and the writer producing another. | High | High | Each retrofit PR ships a committed grep-audit script that fails CI on any hit; PR-13's smoke test exercises end-to-end so a missed reader trips the cache-discipline assertion. |
| PR-6's edges_add / edges_suppress merge logic conflicts with Phase 1's `additions` / `pins` / `suppressions` (semantics overlap). | Medium | Medium | PR-6 documents that `edges_add` / `edges_suppress` are *strictly orthogonal* to `additions` (which adds component nodes) and `pins` (which pins component fields). The new fields target edges only. PR-6's tests include a backwards-compat sweep against Phase 2's existing override fixture usage. |
| Modularity cohesion formula (LCOM4-adapted) produces useless numbers in real workspaces — pegged near 1.0 or 0.0. | Medium | Medium | Design spec §4.3 ships the v1 formula with explicit "subject to revisit after dull/Linkuistics" calibration note; revision is a docs+formula change with no schema change. Phase 3 ships honest numbers; Phase 4+ may calibrate. |
| Stateful file (drift snapshot, modularity history) corruption from non-atomic writes. | Medium | Medium (data) | All stateful writes use `atomic_write` (PR-1). PR-12's kill-during-write fixture stress-tests the helper under both report patterns. |
| Impact-query cycle on a contract graph with mutual consumption produces infinite traversal. | Low | Medium | Cycle-safe via seen-set; specific test fixture in PR-9. |
| Severity rating in divergence requires a drift baseline; first-run divergence has `severity: null` everywhere, possibly confusing tooling that scripts on the field. | Medium | Low | Documented in design §4.4; CLI prints "drift baseline absent — run drift first for severity ratings". Test asserts the report header reflects baseline absence. |
| PR-6's overrides extension fights with Phase 1 `additions` semantics in subtle ways the design spec does not enumerate. | Medium | Medium | PR-6's test suite includes round-trip tests against Phase 2's existing override usage in fixtures; any conflict surfaces as a Phase 2 fixture failure. |
| Gitignore mechanism (PR-1) writes `.atlas/.gitignore` in a user-customised gitignore that the user expects to control. | Low | Low | Idempotent: write iff absent. If present without `cache/` line, respect-and-warn. PR-1's test suite covers all three branches. The mechanism does NOT modify a present file. |
| Modularity per-component file growth across long-running workspaces. | Low | Low | Hard cap at 5 history entries with FIFO rotation. Cap is enforced in PR-10 logic; covered by `history_rotation_drops_oldest_at_5_entries` test. |
| Phase 3 PR count blowup beyond ~14 PRs from retrofit complexity. | Medium | Low | Each retrofit PR is bounded by its grep-audit script; if a retrofit's actual diff exceeds 2× the LOC estimate, surface and split (Phase 1 PR-12 / Phase 2 PR-1 deviation precedents apply). |
| Subsystem aggregates when `subsystems.yaml` is empty / absent. | Medium | Low | `modularity()` gracefully reports `subsystems: []` and populates `unattached_components` with every component. PR-10 has a dedicated `empty_subsystems_yaml_produces_unattached_only_rollup` test. |
| `atlas impact` Levenshtein-1 candidate suggestion is too narrow (typoed inputs differing by 2+ chars yield empty candidates). | Low | Low | Phase 3 ships Levenshtein-1; broader fuzzy-match is a Phase 4+ tooling refinement. Empty candidates is acceptable. |
| `atlas-reports` crate's pure-function design leaks Salsa-input shape into its API such that the Phase 5 conversion needs a wider rewrite than design §3.5 promises. | Low | Medium | `ReportInputs<'a> { db, workspace }` is intentionally minimal. PR-7's PR description re-iterates the Phase 5 conversion path; reviewers flag any API addition that breaks it. |
| Two PRs land out of dependency order due to merge timing (especially PR-2..PR-5 in parallel, where the wave is 4 PRs wide). | Low | High | Each PR's description explicitly lists `Depends on: PR-N`; CI / orchestrator should refuse to merge a PR whose dependency target has not yet landed. |

---

## 7. Out of scope for Phase 3

These items are deferred to later phases. A reviewer flagging them as
missing should redirect to the relevant phase.

### 7.1 Deferred to Phase 4 (convergence + cleanups + LLM analyses)

- **Pattern detection** (originally design §10.3 in pre-Phase-3 form). Needs the bidirectional LLM callback channel that Phase 4 introduces.
- **Subprocess convergence**: migrate Cargo / Dockerfile / RustSurface / LlmClassify / TS-as-subprocess from in-process to subprocess.
- **Bidirectional LLM callback channel** for subprocess analysers.
- **`rust-analyzer` integration** replacing `syn` (Phase 4 stretch).
- **LLM confidence threshold calibration** (design §11.2.6).
- **Contract rename-match** (design §11.2.4).
- **`--strict-overrides` flag**.
- **Cache compression** (design §11.2.7).
- **Worktree commit-sha consistency annotations** (design §11.2.8).
- **Phase 2 closeout cleanups**: `LenientBackend` test-helper extraction, decoder consolidation onto `decode_subprocess_surface_payload`, `is_manifest_file` extension for Makefile/shell auto-discovery, L8 phantom-subcomponent fix.
- **Per-language Phase 3-style refinements** that the brainstorm wave deferred: full tree-sitter-dart, raco-driven Racket dep resolution, Phoenix sub-kinds for Elixir, Mix umbrella decomposition, LispKit `(import …)` symbolic resolution.

### 7.2 Deferred to Phase 5 (server mode)

- **File watcher + Salsa input updates.**
- **gRPC + HTTP+GraphQL query API.**
- **Subscription primitives** (contract sha, surface sha).
- **Server lifecycle** (start, restart, GC).
- **CLI as thin client** to co-located server.
- **Optional Grafeo derived index** for ad-hoc Cypher / GQL / SPARQL queries.
- **Reactive recomputation of reports.**
- **Phase 5 query API authentication and authorisation** (design §11.2.5).
- **Salsa-tracking the report queries.** `atlas-reports` ships as pure functions; Phase 5 converts each to `#[salsa::tracked]` per design §3.5.

### 7.3 Deferred indefinitely

- **`--gate` / `--strict` exit-code flags** for CI integration on reports. Low priority; users can script on `--json` output.
- **Pass/fail thresholds for modularity scores.** Downstream tooling concern.
- **Upstream / subsystem-input variants of impact query.** Design-minimum per design §10.3.
- **Modularity history depth >5 entries.** Hard cap is intentional; v2 if data demands it.
- **Per-language coupling normalisation.** No principled formula across language ecosystems; ships raw.
- **Multi-tenant / SaaS hosting.** Design §11.3 explicit non-goal.

---

## 8. References

- **Design spec (Phase 3):** `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`. Canonical scope; this plan operationalises it.
- **Project design spec:** `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` (especially §10.3, §11.2, §12, plus the §10/§4/§6/§11/§12 touch-ups landing in PR-0).
- **Phase 1 plan:** `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`. PR-6 (scattered `.atlas/` writers), PR-7 (surfaces.yaml), PR-8 (related-components.yaml + edge kinds), PR-9 (Dockerfile composition edges) are the Phase 1 mechanisms PR-2..PR-5 retrofit.
- **Phase 2 plan:** `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`. PR-11 (Compose composition edges) is part of the deploy-graph that PR-11-of-this-plan (divergence) and PR-9-of-this-plan (impact partition) read.
- **Phase 1 status (per-PR notes):** `docs/superpowers/plans/2026-05-06-phase1-status.md`.
- **Phase 2 status (per-PR notes):** `docs/superpowers/plans/2026-05-07-phase2-status.md`.
- **Phase 3 status (per-PR notes — this plan's companion, PR checklist + dependency graph + per-PR notes):** `docs/superpowers/plans/2026-05-08-phase3-status.md`.
- **Open-question resolutions from prior phases:**
  - `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md` (Phase 1).
  - `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` (Phase 1).
- **Continuation prompt** (rewritten Phase-3-shaped at session-handoff time): `docs/superpowers/prompts/2026-05-08-vnext-continue.md` (or whatever the new prompt's filename ends up — the older Phase-2-shaped prompt at `2026-05-07-vnext-continue.md` is replaced/deprecated alongside this plan landing).
- **Memory entries that constrain Phase 3** (any missing entries are not load-bearing; the design spec captures the same constraints):
  - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
  - `feedback_fix_all_lints` — every PR runs `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
  - `project_monorepo_consolidation` — long-term direction; informs that Phase 3 should not over-invest in multi-root-specific report flavours.

---

## 9. Dependency graph (canonical)

```
PR-0a (plan + status)
  │
  ▼
PR-0b (design-doc touch-ups in canonical system-model spec)
  │
  ├──> PR-1 (gitignore mechanism + atomic_write helper)
  │     │
  │     ├──> PR-2 (retrofit surfaces.yaml)        ──┐
  │     ├──> PR-3 (retrofit per-component component.yaml)  ──┤
  │     ├──> PR-4 (retrofit top-level components.yaml)      ──┤
  │     └──> PR-5 (retrofit related-components.yaml)        ──┤
  │                                                          │
  │                                                          ▼
  │                                                  PR-6 (overrides ext)
  │                                                          │
  └──> PR-7 (atlas-reports scaffold + CLI framework)         │
            │                                                │
            │   ┌─── (PR-2..PR-5 must have landed)  ◀────────┘
            │   │
            ▼   ▼
        PR-8  (drift)         ──┐
        PR-9  (impact)         ─┤
        PR-10 (modularity)    ──┤───> PR-12 (atomic-write fixture suite)
        PR-11 (divergence)    ──┘            │
                                              ▼
                                      PR-13 (Phase 3 polyglot smoke test)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0a (plan + status; this commit).
- **Wave 1 (after PR-0a):** PR-0b (design-doc touch-ups). Docs-only; first task of the first execution session. Strictly sequential to PR-0a because both touch the planning-artefact tier.
- **Wave 2 (after PR-0b):** PR-1 and PR-7 concurrently. They touch independent surfaces (PR-1 = atomic-write + gitignore; PR-7 = new crate + CLI subcommand framework). Could in principle land in parallel with PR-0b, but execution sessions land docs-PRs sequentially for cleaner per-PR review.
- **Wave 3 (after PR-1):** PR-2, PR-3, PR-4, PR-5 — four cache-path retrofits in parallel. Each has its own grep-audit script so a missed reader in one PR does not block the others.
- **Wave 4 (after PR-5):** PR-6 (overrides extension) — depends on PR-5 because the engine code reads cached `related-components.yaml` then unions `edges_add`. Could land in parallel with the wave-5 reports if the team wants extra parallelism.
- **Wave 5 (after PR-7 + PR-2..PR-5; PR-6 helpful but not strictly required for PR-8/PR-9/PR-10/PR-11 to function — they observe whatever edges the engine produces):** PR-8, PR-9, PR-10, PR-11 — four reports concurrently. Each ships its CLI subcommand handler atop PR-7's framework.
- **Wave 6 (after PR-8 + PR-10):** PR-12 (atomic-write fixture suite). Tests the kill-during-write semantics of both stateful patterns.
- **Wave 7 (after Wave 6):** PR-13 (Phase 3 polyglot smoke test).

The widest parallel wave is Wave 2 (4 PRs simultaneously). Wave 4 is also 4 PRs wide. Both waves benefit from `superpowers:dispatching-parallel-agents` (one Agent tool call per PR, all in a single message).
