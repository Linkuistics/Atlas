# Atlas vNext Phase 3 — Status

Companion to `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`.
This file tracks per-PR completion state across sessions. The
continuation prompt at
`docs/superpowers/prompts/2026-05-08-vnext-continue.md` (Phase-3-shaped)
reads this file (via the `*phase3-plan*` wildcard match) to find the
next PR to dispatch.

**Last updated:** 2026-05-08 (Phase 3 COMPLETE). All 14 PRs landed.
Wave 7 closeout: PR-13 polyglot smoke test cherry-picked as
`6520acb` plus a fix-up commit `a1fde5d` to force-add gitignored
fixture override files. Final cargo test/clippy/fmt/release on main
all clean. Phase 3 commits on main (all):
- PR-0a plan + status: `cac1709`
- PR-0b design touch-ups: `986b63e`
- PR-1 atomic_write + gitignore: `31c329d`
- PR-2 surfaces.yaml retrofit: `e5b7828` + `31e2d20`
- PR-3 component.yaml retrofit: `5b2ec1a` + `e158114`
- PR-4 components.yaml retrofit: `a1b9541`
- PR-5 related-components.yaml retrofit: `e146dd4` + `d1b98a0`
- PR-6 overrides extension: atlas-contracts `a0a9a8c` + Atlas
  `1b27827` + `27794d6` (fix-up)
- PR-7 atlas-reports scaffold: `871d75b` + `e0aa5e5`
- PR-8 drift: `1060edb`
- PR-9 impact: `0ec65c5`
- PR-10 modularity: `4ce7245`
- PR-11 divergence: `8ddd3c5`
- PR-12 atomic-write fixtures: `2e8f19d`
- PR-13 polyglot smoke test: `6520acb` + `a1fde5d` (fix-up)

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0a — Plan + status (docs only)
- [x] PR-0b — Design-doc touch-ups in canonical system-model spec (docs only)
- [x] PR-1  — Gitignore mechanism for `<scope>/.atlas/cache/` + atomic_write helper
- [x] PR-2  — Phase 1 retrofit: per-component `surfaces.yaml` → cache
- [x] PR-3  — Phase 1 retrofit: per-component `component.yaml` → cache
- [x] PR-4  — Phase 1 retrofit: top-level `components.yaml` → cache
- [x] PR-5  — Phase 1 retrofit: top-level `related-components.yaml` → cache
- [x] PR-6  — Overrides schema extension: `edges_add` / `edges_suppress` + per-component field overrides
- [x] PR-7  — `atlas-reports` crate scaffold + CLI subcommand framework
- [x] PR-8  — Drift report + `atlas drift` CLI subcommand
- [x] PR-9  — Impact query + `atlas impact <id>` CLI subcommand
- [x] PR-10 — Modularity report + `atlas modularity` CLI subcommand
- [x] PR-11 — Composition divergence + `atlas divergence` CLI subcommand
- [x] PR-12 — Atomic-write fixture suite for stateful files
- [x] PR-13 — Acceptance: Phase 3 polyglot smoke test

When every box is `[x]`, Phase 3 is complete and the continuation
prompt should report success and stop.

## Dependency graph (informational; canonical in plan §4 + plan §9)

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
- **Wave 1 (after PR-0a):** PR-0b (design-doc touch-ups). First task of the next execution session.
- **Wave 2 (after PR-0b):** PR-1, PR-7 — independent surfaces, dispatch concurrently.
- **Wave 3 (after PR-1):** PR-2, PR-3, PR-4, PR-5 — four cache-path retrofits in parallel.
- **Wave 4 (after PR-5):** PR-6 (overrides extension).
- **Wave 5 (after PR-7 + PR-2..PR-5):** PR-8, PR-9, PR-10, PR-11 — four reports concurrently. PR-6 is helpful but not strictly required for the reports to function — they observe whatever edges the engine produces.
- **Wave 6 (after PR-8 + PR-10):** PR-12 (atomic-write fixture suite).
- **Wave 7 (after Wave 6):** PR-13 (Phase 3 polyglot smoke test).

The widest parallel wave is Wave 2 (4 PRs simultaneously). Wave 4 is
also 4 PRs wide. Both waves benefit from
`superpowers:dispatching-parallel-agents` (one Agent tool call per PR,
all in a single message).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of
what's worth recording: deviations from the plan that the next session
needs to know; surprising fixture quirks; manual verification steps that
succeeded; follow-up cleanup deferred; cache-path or schema-mutation
trail (which PR added which field, which PR moved which file).

### PR-0a
2026-05-08 — Landed: the Phase 3 plan
(`docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`) and
this status file
(`docs/superpowers/plans/2026-05-08-phase3-status.md`). Companion to
the Phase 3 design spec
(`docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`,
already on main from commit `02f0914`).

A new Phase-3-shaped continuation prompt at
`docs/superpowers/prompts/2026-05-08-vnext-continue.md` lands in the
same commit and replaces / deprecates the Phase-2-shaped prompt at
`docs/superpowers/prompts/2026-05-07-vnext-continue.md` (which is
moved aside or marked obsolete).

PR-0b (design-doc touch-ups in
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`) is
the first task of the next execution session. The touch-ups are
enumerated in plan §4 PR-0b and in the Phase 3 design spec §10.

Load-bearing context for Wave 2 reviewers (after PR-0b lands):

- **Greenfield carries forward across phases.** No on-disk format
  compatibility with Phase 1 / Phase 2; no migration. A user upgrading
  deletes `.atlas/` and re-runs.
- **No schema_version bump in Phase 3.** All on-disk schemas (drift
  snapshot, drift report, impact, modularity per-component +
  rollup, divergence) ship as `schema_version: 1`. The integer is
  reserved for a future breaking-change phase if it ever happens.
- **Phase 3 introduces zero new LLM call sites.** Every report is a
  deterministic projection of L4–L8 engine outputs. The cold and warm
  LLM call budgets in PR-13's polyglot smoke test must match Phase 2's
  PR-14 baseline (~26 cold, 0 warm). A subagent that introduces a new
  LLM call must surface this immediately for design review.
- **Editorial tier is fixed at six file types.** Top-level
  `overrides`, `external-components`, `subsystems`, `analyzers`,
  `config` + per-component `overrides`. PR-2..PR-5 retrofit moves
  Phase 1's `surfaces.yaml`, `component.yaml`, `components.yaml`,
  `related-components.yaml` to the cache (gitignored) tier. Anything
  not in the six editorial files is derived.
- **All cache writes are atomic** (temp+fsync+rename via PR-1's
  helper). Stateful-file writes (drift snapshot in PR-8; modularity
  history in PR-10) are particularly load-bearing — corruption from
  non-atomic writes was an explicit design-spec §6.3 concern.
- **PR-1 must land before PR-2..PR-5 dispatch.** PR-2..PR-5 each
  call `ensure_atlas_gitignore` from PR-1. Wave 2's parallel
  dispatch is conditional on PR-1 having merged.
- **PR-7 is independent of PR-1..PR-6.** It scaffolds the
  `atlas-reports` crate and CLI subcommand framework with stubbed
  `Err(NotImplemented)` handlers. Dispatching PR-7 concurrent with
  PR-1 in Wave 1 is safe.
- **The `atlas-reports` crate is intentionally pure-function**
  (design §3.5 / plan §4 PR-7). Reviewers must reject any I/O or
  Salsa-mutation introduced inside the crate; CLI handlers do all
  I/O. This keeps the Phase 5 conversion path mechanical.
- **Each retrofit PR ships a committed grep-audit script** that
  fails CI on any tracked file referencing the old (non-cache) path.
  These scripts are the canonical guard against missed readers.
- **Per-component `modularity.yaml` is hard-capped at 5 history
  entries** (FIFO). The cap is in plan §4 PR-10 and design §4.3.
  Any subagent attempting to make this configurable must surface
  the question — "modularity history depth >5" is a deferred-
  indefinitely scope item per plan §7.3.

### PR-0b
2026-05-08 — Landed: nine design-doc touch-ups to
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`.
Commit `986b63e` (+70 / −8 lines, single file). Spec-reviewed against
plan §4 PR-0b enumeration; all nine touch-ups verbatim, all five
acceptance criteria satisfied, byte-stability outside touched
sections confirmed.

Two interpretive calls, both sound and consistent with existing spec
conventions:

- §11.2.9 was added as item `9.` in the existing Markdown ordered
  list rather than as a `### 11.2.9` heading. §11.2 has always used
  ordered-list items (1.–8.); the implementer correctly matched the
  convention. Inbound `§11.2.9` cross-references in §6 stanzas
  resolve positionally.
- §6 sub-sections (§6.1–§6.4) did not previously have per-file
  tables, so the Git-status touch-up landed as a one-line
  `**Path:** … **Git status:** …` stanza per the touch-up's explicit
  fallback guidance. Each stanza records the post-retrofit cache
  path (e.g. `<scope>/.atlas/cache/components.yaml`).

Out-of-scope coherence drift surfaced by the spec reviewer (NOT
fixed in PR-0b — outside the nine-touch-up enumeration): four prose
references where "Phase 4" still implicitly means "server mode"
remain in the canonical spec at:

- §5.6 "Server mode (eventual)" header — references "Phase 4".
- §9 introduction — "Server mode is the Phase 4 target".
- §11.4 "Once Phase 4 ships and the server has concrete polyglot
  consumers".
- Glossary line ~1436 — "deferred to Phase 4 and beyond".

These are prose, not §10.X cross-references, so they don't break the
PR-0b acceptance criterion "no broken cross-references." But they're
semantically stale post-renumbering (server mode is now Phase 5 per
new §10.5). **Follow-up: a small docs-only retext PR could land
these later — not blocking on Phase 3, since none of the Phase 3
code reads §5.6 / §9 / §11.4 prose.** Recording here so the next
phase's continuation prompt can decide whether to bundle the retext.

Wave 2 ready: PR-1 (gitignore + atomic_write helper) and PR-7
(`atlas-reports` crate scaffold + CLI subcommand framework) are
independent surfaces and dispatchable concurrently. Use
`superpowers:dispatching-parallel-agents` (one Agent tool call per
PR, both in a single message).

### PR-1
2026-05-08 — Landed: per-scope `.gitignore` writer + shared
`atomic_write` helper. Commit `31c329d` (5 files, +536 lines net).

**Code surface:**
- New: `crates/atlas-engine/src/atomic_write.rs` — `pub fn atomic_write(&Path, &[u8]) -> io::Result<()>` performing temp + `sync_all` + rename. Temp filename `<final>.tmp.<pid>.<rand-u64-hex>`. Best-effort temp cleanup on `io::Error`. Test-only (`#[cfg(test)]`) panic-injection hook between temp-write and rename — verified absent from release binary symbols.
- New: `crates/atlas-engine/src/gitignore.rs` — `pub fn ensure_atlas_gitignore(&Path) -> io::Result<EnsureGitignoreOutcome>` with three variants (`Wrote`, `AlreadyPresent`, `CustomisedWithoutCacheLine`). Exact-match-after-trim on `cache/` (NOT substring or glob). Wrote path uses `atomic_write` (eats own dogfood).
- Modified: engine `lib.rs` (mod decls + re-exports), CLI `pipeline.rs` (call sites + `GitignoreSession` dedup struct), CLI integration test file.

**Interpretive call (sound, ratified by spec review):** the dedup mechanism is a `GitignoreSession { visited: BTreeSet<PathBuf> }` struct threaded through `write_per_component_files` as `&mut`, with canonicalised-path keying (input-path fallback when canonicalisation fails on a not-yet-existent dir). Plan does not prescribe a specific dedup mechanism; this is the smallest signature change consistent with "at most one warning line per session per scope."

**Load-bearing details for downstream PRs (PR-2..PR-5, PR-8, PR-10):**
- Re-exports for downstream callers: `atlas_engine::atomic_write` (function), `atlas_engine::ensure_atlas_gitignore`, `atlas_engine::EnsureGitignoreOutcome`.
- `GitignoreSession::ensure_for(scope)` is the per-scope ensure call sites in PR-2..PR-5 should use; lives in `crates/atlas-cli/src/pipeline.rs`. Each retrofit PR's writer should call into the existing session rather than constructing a new one.
- Existing private `cache::layout::atomic_write` (different signature, returns `anyhow::Result`) was left in place to keep PR-1 within scope. A future refactor can converge them; not blocking Phase 3.
- POSIX 1s mtime granularity: integration test for idempotency sleeps 1100ms between runs. Future cache-related integration tests with mtime checks should follow this pattern.

Wave 2 still has PR-7 (`atlas-reports` crate scaffold) outstanding.
PR-7 is independent of PR-1 (no shared files; PR-7 only stubs and
returns `Err(NotImplemented)`, no `atomic_write` use yet — that
lands in PR-8/PR-10/PR-11). Once PR-7 lands, Wave 3 (PR-2..PR-5
retrofits) becomes dispatchable in parallel; each retrofit calls
`ensure_atlas_gitignore` from PR-1 via the `GitignoreSession`.

### PR-7
2026-05-08 — Landed: `atlas-reports` crate scaffold + CLI subcommand
framework. New crate `crates/atlas-reports/` (workspace member) with
seven modules (`lib`, `types`, `snapshot`, `drift`, `impact`,
`modularity`, `divergence`); four CLI subcommands (`atlas drift`,
`atlas impact`, `atlas modularity`, `atlas divergence`) wired
through a new `crates/atlas-cli/src/reports.rs`; new workspace dep
`chrono = "0.4"` (with `serde` feature) added to top-level
`[workspace.dependencies]`.

**Code surface:**
- All four `pub fn` entry-points in `atlas-reports` return
  `Err(ReportError::NotImplemented)`. PR-8..PR-11 land the actual
  bodies. The crate is intentionally I/O-free; reviewers should
  reject any `fs::*` introduced inside `crates/atlas-reports/src/*`.
- Type shapes for the four reports + the snapshot follow design spec
  §4.1–§4.4 verbatim. Every report struct ships with
  `schema_version: u32` defaulting to `1`. All structs derive
  `Serialize`, `Deserialize`, `Debug`, `Clone`, `PartialEq` (`Eq`
  added where every nested field supports it; `ModularityMetrics`
  and friends only get `PartialEq` because they hold `f64`).
- 19 unit tests (round-trip serde for snapshot, drift report,
  impact report, modularity report + per-component payload + rollup
  + history, divergence report; plus the canonical exemplar test
  for `contract-shas-snapshot.yaml` from design §4.1).
- CLI: `OutputFormat` is a `clap::ValueEnum` (`yaml | json | human`,
  default `yaml`). `--no-write` flag on `drift`, `modularity`,
  `divergence`. `impact` deliberately omits `--no-write` so
  `atlas impact --no-write foo` produces clap's own
  `error: unexpected argument '--no-write'` message; no custom
  rejection code needed.
- Phase 5 conversion path documented in `crates/atlas-reports/src/lib.rs`'s
  module-level doc comment per design §3.5.

**Spec interpretive calls (recorded for PR-8..PR-11):**

1. **`ContractShaSnapshot.contract_shas` is `Vec<ContractShaEntry>`,
   not `BTreeMap<ContractId, Sha256Hex>`.** The plan §4 PR-7 prompt
   prescribed a `BTreeMap`, but the design spec §4.1 wire format is
   a sequence of `{id, content_sha}` records. The continuation
   prompt instructs "design spec wins on type shapes", so the
   in-memory form mirrors the YAML (sorted-by-id sequence).
   `ContractShaSnapshot::as_map()` provides a `BTreeMap<&ContractId,
   &Sha256Hex>` lookup view for drift's diff loop in PR-8.

2. **Modularity history-entry field set follows design §4.3, not the
   plan prompt's `{contracts_emitted, contracts_consumed, fan_in,
   fan_out, churn}`.** Design §4.3 names the metrics
   `afferent_coupling`, `efferent_coupling`, `instability`,
   `cohesion`, `surface_complexity` (history entries omit
   `surface_stability` because that metric is computed *from*
   history rather than stored per-entry). The plan's metric names
   were placeholders; "design spec wins on type shapes" disposes of
   the conflict.

3. **`ContractId` is a `pub type ContractId = String;` alias (per the
   continuation prompt).** PR-9 (impact) is free to upgrade this to
   a newtype if traversal needs richer behaviour. Existing
   `atlas-index` `surfaces.yaml` records `contract_id: String`, so
   the alias matches usage everywhere else.

4. **`AtlasDatabase` is borrowed by `ReportInputs<'a>` but never
   accessed by PR-7's stubs.** Each handler short-circuits before
   the engine load — no `IndexConfig` construction, no
   `run_index_cmd`-style backend wiring. PR-8..PR-11 will replace
   the stub bodies with the real load-database flow; the design
   §3.3 sequence diagram is the canonical wiring.

5. **`DivergencePair.components` is `[String; 2]`, not `[ComponentId;
   2]`.** Design §4.4 wire format has the pair as a list-of-strings
   for YAML readability; the in-memory type follows the wire format
   verbatim. PR-11 can convert to `ComponentId` internally during
   the pair-classification loop.

**Verification:** `cargo build --workspace`, `cargo test --workspace`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all
clean. Manual smoke checked all four `--help` outputs include the
`--format` flag (and `--no-write` where applicable), all four stubs
print `"<subcommand> is not yet implemented"` to stderr and exit 1,
and `atlas impact --no-write foo` rejects the flag with clap's
unexpected-argument error.

**Schema fix (commit `e0aa5e5`, follow-up to `871d75b`):** the
initial PR-7 `ImpactReport` had a richer in-memory shape (`direct:
Vec<ImpactNode>`, `transitive: Vec<ImpactNode>`) that disagreed with
design §4.2's documented wire format. The spec reviewer caught this
as a substantive type-shape deviation. Refactored `impact.rs` to
match the design spec verbatim:

- `direct: Vec<ImpactNode>` → `direct_consumers: Vec<String>`.
- `transitive: Vec<ImpactNode>` → `transitive_consumers: Vec<String>`.
- New `target: ImpactReportTarget { kind: ImpactReportTargetKind, id: String }`.
- New `partitions: ImpactPartitions { by_language, by_deploy_graph,
  by_lifecycle: BTreeMap<String, Vec<String>> }` — three independent
  axes per design §4.2's "three independent partitions, not a 3D
  grid" mandate.
- `summary: ImpactSummary { direct_count: u32, transitive_count: u32 }`
  — verified against design spec field names.
- Obsolete `ImpactNode` / `ImpactNodeKind` / `ImpactTargetView` types
  removed entirely (zero refs anywhere in the workspace).
- Two new tests added: `impact_report_matches_design_spec_exemplar`
  (parses the verbatim §4.2 YAML exemplar) and
  `impact_report_target_kind_serialises_lowercase`. Total
  atlas-reports test count is now 20 (was 19).

**Lesson for downstream PRs:** when a plan-prompt sub-section
references "shape from design §X" but doesn't quote the wire format
inline, the implementer (and reviewer) must read §X verbatim — the
plan-prompt is intentionally less detailed than the design spec on
type-shape minutiae and is not a substitute for reading the design
spec. The "design spec wins on type shapes" rule applies whether or
not the implementer recognised the conflict.

PR-2..PR-5 (retrofits) and PR-8..PR-11 (report bodies) are now
unblocked. PR-9 (impact body) inherits the corrected wire-format-
matching `ImpactReport` shape; no follow-up renderer projection
needed.

### PR-4
2026-05-08 — Landed: top-level `components.yaml` retrofit to
`<root>/.atlas/cache/components.yaml`. Commit `a1b9541` on main
(cherry-picked from worktree commit `48f6beb`). 9 files changed,
+401 / −19. Spec compliance ✅ (4/4 acceptance criteria); code
quality review verdict "Ready to merge: Yes" (no Critical / Important
issues; six Minor nice-to-haves on comment density and DRY,
non-blocking).

**Code surface:**
- `crates/atlas-cli/src/pipeline.rs`: `prior_components_path` reader
  (line 229) and writer call site (line 682) both updated to
  `cache/components.yaml`. Added explicit `create_dir_all(cache/)`
  before the writer to handle the `PersistentCache::open`-fell-back
  case (the existing `save_components_atomic` helper does NOT create
  parent dirs).
- 6 CLI integration test files updated with path-only changes
  (`agent_observer_e2e.rs`, `atlas_contracts_in_ravel_lite.rs`,
  `byte_identity_l5_demand.rs`, `phase2_polyglot_fixture.rs`,
  `pipeline_integration.rs`, `scattered_atlas_layout.rs`).
- New: `crates/atlas-cli/tests/grep_no_old_components_path.sh`
  (chmod +x) — Perl negative-lookahead regex
  `\.atlas/(?!cache/)components\.yaml`; excludes `docs/` and
  `evaluation/results/`. Wire it into CI by invoking from a `#[test]`
  shim (PR-13 may consolidate the four retrofit grep-audit invocations).
- New: `crates/atlas-cli/tests/phase3_retrofit_components.rs` —
  4 tests calling `run_index` end-to-end against the `tiny` fixture
  with `SweepBackend`; covers cache-path populated, content sanity,
  byte-identity on no-op rerun (content-equality based, not mtime,
  so no `sleep 1100ms` needed), and recursive sweep for stray
  non-cache files.

**Load-bearing details for downstream PRs:**
- **`atlas-contracts/crates/atlas-index/src/yaml_io.rs`'s
  `save_components_atomic` is the canonical writer.** It wraps
  `write_atomic` (temp-file + rename) but does NOT create parent
  directories. When PR-2/PR-3/PR-5/etc. write to `<scope>/.atlas/cache/`
  paths, they must `create_dir_all(cache/)` before the save call OR
  rely on `atlas_engine::atomic_write` (PR-1's helper) which DOES
  create parent dirs. PR-4 used the explicit `create_dir_all` form;
  PR-5's PR uses `atomic_write` directly. Either is acceptable; the
  *invariant* is that the `cache/` subdir must exist before the
  rename target lands.
- **PersistentCache fallback.** `PersistentCache::open` creates the
  `cache/` dir on success but may fall back to in-memory; in that
  case the `cache/` dir does not yet exist. Hence the explicit mkdir.
  This is a Phase 3 invariant: every cache-tier writer must tolerate
  the in-memory-fallback case.
- **Worktree-local target/ cache produced phantom polyglot test
  failures.** During PR-4's spec-review verification, the
  `phase2_polyglot_fixture` tests failed on the worktree but passed
  3/3 on main with the commit cherry-picked. Root cause was the
  worktree's stale `target/` artefacts (the worktree had been
  iterating across multiple cargo runs). Lesson for future
  retrofit reviews: when "regression on the worktree" appears, verify
  by cherry-picking the diff onto a clean main and re-running there.
- **Greenfield holds.** No compatibility shim, no migration code.
  Reader and writer move in lockstep; old-path consumers that survive
  in `git grep` would be caught by the grep-audit script.

PR-5 (related-components.yaml) is committed on its worktree branch
(`f9ab087`) awaiting integration onto main. PR-2 (surfaces.yaml) and
PR-3 (component.yaml) need redispatch — PR-2's initial agent ran
on a stale `c6ddb67` worktree base and tried to manually duplicate
PR-1's `atomic_write` / `gitignore` helpers (contamination); PR-3's
initial agent reported BLOCKED for the same root cause and its
worktree was auto-cleaned. The worktree-base issue is documented in
the Atlas project memory at
`.claude/memory/feedback_worktree_base_verification.md`; future
sessions verifying parallel worktree dispatch should run
`git worktree list` immediately and confirm each new worktree's
HEAD matches current main.

### PR-5
2026-05-08 — Landed: top-level `related-components.yaml` retrofit
to `<root>/.atlas/cache/related-components.yaml`. Two commits on
main: `e146dd4` (cherry-pick from worktree commit `f9ab087`) and
`d1b98a0` (orchestrator fix-up — strip `.atlas/` prefix from two
literal old-path mentions in the new sweep test docstring +
assertion message so the PR-5 grep-audit exits 0). Spec compliance
✅ (4/4 acceptance criteria); code quality review verdict
"Ready to merge: Yes" (no Critical / Important issues; six Minor
nice-to-haves on writer-comment clarification + orphan re-export
tracking, all non-blocking).

**Code surface:**
- `crates/atlas-cli/src/pipeline.rs`: import `atlas_engine::atomic_write`,
  remove `save_related_components_atomic`. Update `prior_related_path`
  reader (line ~231) to `cache/related-components.yaml`. Replace
  the writer call with an inline `serde_yaml::to_string` +
  `atomic_write` block — `atomic_write` `create_dir_all`s the
  `cache/` parent before the rename target lands, which the existing
  `save_related_components_atomic` helper does NOT.
- 4 CLI integration test files updated with path-only changes
  (`agent_observer_e2e.rs`, `atlas_contracts_in_ravel_lite.rs`,
  `phase2_polyglot_fixture.rs`, `pipeline_integration.rs`). Two of
  these (`agent_observer_e2e.rs`, `pipeline_integration.rs`)
  required cherry-pick conflict resolution because PR-4 touched
  the same files; resolutions union both PRs' assertions correctly.
- New: `crates/atlas-cli/tests/grep_no_old_related_path.sh`
  (chmod +x) — Perl negative-lookahead regex
  `\.atlas/(?!cache/)related-components\.yaml`; excludes `docs/`,
  `LLM_STATE/`, and `evaluation/results/`. PR-5's exclusion of
  `LLM_STATE/` differs from PR-4's audit (no `LLM_STATE/` excluded
  there); both are correct because `LLM_STATE/` only contains
  matching strings for the related-components case in this session.
- New: `crates/atlas-cli/tests/phase3_retrofit_related.rs` —
  3 tests calling `run_index` end-to-end against the `tiny`
  fixture with `LenientBackend`; covers cache-path populated +
  parseable, dry-run no-write, byte-identity on no-op rerun.

**Load-bearing details for downstream PRs:**
- **`atlas_engine::atomic_write` is the preferred cache-tier writer.**
  PR-4 used the existing `save_components_atomic` helper plus an
  explicit `create_dir_all(cache/)`. PR-5 used `atomic_write`
  directly because it `create_dir_all`s parents internally. PR-2 +
  PR-3 should pick the simpler idiom (direct `atomic_write`) for
  consistency, OR continue using the existing save-helper plus
  explicit mkdir. Either is acceptable; the *invariant* is that the
  `cache/` subdir must exist before the rename target lands.
- **Orphan `pub use save_related_components_atomic` in
  `atlas-contracts/crates/atlas-index/src/lib.rs` line 60.**
  After PR-5, this re-export has zero callers in either repo.
  Rust does NOT warn on orphan public re-exports, so the dead
  code persists silently. **Tracking item: a future Phase 3 / 4
  cleanup PR should remove this re-export from atlas-contracts.**
  Not blocking PR-5; defer to a sibling-repo cleanup PR.
- **PR-2's redispatch.** PR-2's worktree at
  `.claude/worktrees/agent-a33068c44f27fca4d` is contaminated
  (uncommitted duplicates of `atomic_write.rs` and `gitignore.rs`
  + modifications to `lib.rs` to register them, all on a stale
  `c6ddb67` base). The orchestrator should remove that worktree
  via `git worktree remove --force` after PR-2's initial agent
  finishes its current run (likely a long workspace-test pass that
  may eventually succeed on the contaminated tree but produces a
  diff that's a mix of PR-1 + PR-2 work — review-fail). Then
  redispatch PR-2 fresh on a verified main-base worktree.

PR-2 (surfaces.yaml) and PR-3 (component.yaml) remain the only
Wave 3 PRs outstanding. Wave 4 (PR-6 overrides extension) and
Wave 5 (PR-8..PR-11 reports) become dispatchable once Wave 3
completes.

### PR-3
2026-05-08 — Landed: per-component `component.yaml` retrofit to
`<component>/.atlas/cache/component.yaml`. Two commits on main:
`5b2ec1a` (cherry-pick from worktree commit `6c74049`) + `e158114`
(orchestrator fix-up — rephrased two doc-comment lines in the
PR-3 grep-audit script that referenced the literal plural-form
path, tripping PR-4's grep-audit). Spec compliance ✅ (4/4
acceptance criteria met; polyglot 3/3 still passing on main + PR-3);
code quality review verdict "Ready to merge: Yes" (no Critical /
Important issues; three Minor nice-to-haves on sweep-test
boilerplate consolidation, comment density, comment redundancy).

**Code surface (6 files changed, +403 / −28):**
- `crates/atlas-cli/src/pipeline.rs`: writer path → `target_dir.join("cache").join("component.yaml")`
  via existing `write_per_component_atomic` helper (which delegates
  to `write_yaml_atomic` and `create_dir_all`s the target dir).
  Mirrors PR-4's idiom (existing save-helper + explicit cache-dir
  argument) rather than PR-5's idiom (direct `atomic_write` which
  mkdirs internally).
- `crates/atlas-cli/tests/scattered_atlas_layout.rs`: 5 path
  references + 4 doc-comment touch-ups.
- `crates/atlas-engine/src/ingest.rs`: comment update (the
  `.atlas/` prune logic's inline comment).
- `crates/atlas-engine/src/l9_projections.rs`: doc comment on
  `per_component_yaml_snapshot` updated to reference new cache path.
- New: `crates/atlas-cli/tests/grep_no_old_component_path.sh`
  (chmod +x) — Perl negative-lookahead regex
  `\.atlas/(?!cache/)component\.yaml(?!s)`; the `(?!s)` distinguishes
  singular from plural form. Excludes self, `docs/`, `LLM_STATE/`,
  `evaluation/results/`.
- New: `crates/atlas-cli/tests/phase3_retrofit_component.rs` —
  2 tests calling `run_index` end-to-end against the `tiny` fixture;
  asserts `analyser_id` + `analyser_version` populated (Phase 2 PR-4
  invariant) and zero `component.yaml` files outside `cache/`
  (recursive-walk guard).

**Load-bearing details for downstream PRs:**
- **The session worktree-base bug is a real reproducible issue.**
  After PR-3's first redispatch produced another stale-base
  worktree (BLOCKED in 16s by the prompt's STEP-0 base-verification
  guard), the orchestrator pre-created the worktree manually via
  `git worktree add -b phase3-pr3-impl /Users/antony/Development/Atlas-phase3-pr3-impl main`
  and dispatched the agent without `isolation: "worktree"`, with
  the absolute worktree path embedded in every Bash/Edit/Read
  instruction. This mitigation worked. PR-2 was redispatched the
  same way and is currently running.
- **Plural vs singular grep-audit collision.** PR-3's grep-audit
  doc-comment trip-wire on PR-4's grep-audit is a small footgun
  worth knowing: any new grep-audit script in this codebase that
  references *another* grep-audit's forbidden path literally
  (even in comments) will trip the other audit. Use rephrased
  doc-comments (e.g. "plural-form components.yaml" rather than
  ".atlas/components.yaml") to avoid this.
- **Sweep-test boilerplate is now duplicated 3× (PR-3 / PR-4 /
  PR-5).** ~100 LoC of `materialise_fixture` + `base_config` +
  `LenientBackend`/`SweepBackend` + `tiny_fixture_root` + `copy_dir_all`
  + `run_with` per file. Code reviewer recommends consolidation
  after PR-2 lands so the refactor folds all four files in one
  pass. Track as Phase 3 cleanup; not blocking.

### PR-2
2026-05-08 — Landed: per-component `surfaces.yaml` retrofit to
`<component>/.atlas/cache/surfaces.yaml`. Two commits on main:
`e5b7828` (cherry-pick from worktree commit `4eb339e`, with
one orchestrator-resolved conflict in `pipeline.rs:892-895`
unioning PR-3's and PR-2's doc-comment mentions) + `31e2d20`
(orchestrator fix-up — rephrased the sweep test docstring's literal
`<component>/.atlas/surfaces.yaml` mention to dodge PR-2's own
grep-audit). Spec compliance ✅ (5/5 acceptance criteria met,
including AC4 cache-hit fingerprint-equality on rerun and AC5
`.atlas/.gitignore` `cache/` line at every scope); code quality
review verdict "Ready to merge: Yes" (no Critical / Important
issues; five Minor nice-to-haves on path-join idiom + tech-debt
consolidation, all non-blocking).

**Code surface (8 files changed, +414 / −43):**
- `crates/atlas-engine/src/l9_projections.rs`: `surfaces_path`
  field default in `PerComponentFile` changed from `"surfaces.yaml"`
  to `"cache/surfaces.yaml"`. Doc comment updated.
- `crates/atlas-cli/src/pipeline.rs`: writer path →
  `target_dir.join("cache/surfaces.yaml")`; switched from
  `write_yaml_atomic(&target_dir, ...)` (which mkdirs target_dir)
  to direct `atomic_write` (which mkdirs the parent of the full
  path). Mirrors PR-5's idiom rather than PR-3/PR-4's
  save-helper-plus-mkdir pattern; both are functionally equivalent.
- 4 CLI integration test files updated with path-only changes
  (`surfaces_emission_rust.rs` 8 refs, `atlas_contracts_in_ravel_lite.rs`,
  `phase2_polyglot_fixture.rs` `surfaces_path_for` helper,
  `scattered_atlas_layout.rs` `surfaces_path` field-value assertion).
- New: `crates/atlas-cli/tests/grep_no_old_surfaces_path.sh`
  (chmod +x) — Perl negative-lookahead regex
  `\.atlas/(?!cache/)surfaces\.yaml`. Excludes self, `docs/`,
  `LLM_STATE/`, `evaluation/results/`.
- New: `crates/atlas-cli/tests/phase3_retrofit_surfaces.rs` —
  4 tests calling `run_index` end-to-end against the `tiny`
  fixture; covers AC(a) cache-path populated, AC(b) zero
  stragglers (full recursive tree walk), AC(c) cache-hit
  fingerprint-equality on rerun, AC(d) `.atlas/.gitignore`
  contains `cache/` at each component scope.

**Plan deviation (legitimate):** `scattered_atlas_layout.rs` was
not in the plan §4 PR-2 enumeration of consumer-test files, but
required updating because its `assert_eq!(parsed.surfaces_path,
"surfaces.yaml")` assertion was tied to the now-changed default
value of the `surfaces_path` field. Implementer flagged this
transparently. Future plan PRs that change a field's default
should pre-enumerate every test asserting on that default.

## Wave 3 closeout — 2026-05-08

**Wave 3 is complete.** All four cache-path retrofits (PR-2
surfaces.yaml + PR-3 component.yaml + PR-4 components.yaml +
PR-5 related-components.yaml) are on main with spec compliance
✅ and code quality reviews ✅. The Phase 1 editorial
on-disk shape (six file types) is preserved — the four retrofits
moved their pre-Phase-3 editorial sit under cache/, leaving the
editorial tier as: top-level `overrides`, `external-components`,
`subsystems`, `analyzers`, `config` + per-component `overrides`
(per design §5.2).

**Cumulative session learnings** captured for the next session:

- **The session worktree-base bug is reproducible.** The harness's
  `isolation: "worktree"` mechanism creates worktrees off a stale
  ref. Mitigation: pre-create worktrees via
  `git worktree add -b <branch> <abs-path> main` and dispatch agents
  without `isolation`, with absolute worktree paths embedded in
  every Bash/Edit/Read instruction. STEP-0 base-verification at
  the top of every retrofit prompt detects regressions in 16
  seconds. Documented in `.claude/memory/feedback_worktree_base_verification.md`.

- **Cross-grep-audit collision pattern.** Every retrofit ships a
  grep-audit forbidding its own pre-retrofit path literal. Doc
  comments and assertion messages in retrofit artifacts must AVOID
  literal `.atlas/<old-path>.yaml` strings (otherwise they trip
  another retrofit's audit). Rephrase with the prefix-less form
  (e.g., "plural-form components.yaml", "pre-PR-2 surfaces.yaml
  directly under .atlas/"). Each of PR-2, PR-3, PR-5's sweep tests
  required a fix-up commit for this; PR-4 happened to avoid it
  by writing the docstring carefully.

- **Worktree-local target/ caches produce phantom test failures.**
  Three separate spec-compliance reviews (PR-3 / PR-4 / PR-5)
  reported `phase2_polyglot_fixture` failing on the worktree but
  passing 3/3 on main with the diff cherry-picked. ALWAYS verify
  cross-tree before believing an implementer's "pre-existing
  failure" claim — cherry-pick onto main and re-run.

- **Writer-idiom asymmetry across retrofits is acceptable.**
  PR-3 + PR-4 use save-helper + explicit `create_dir_all`; PR-2 +
  PR-5 use `atomic_write` directly (which mkdirs internally). The
  invariant is "cache/ exists before the rename target lands";
  enforcement is per-call-site, not codebase-wide. Code reviewer's
  recommendation (deferred to a Phase 3 / 4 cleanup): converge to
  a single idiom, which would also let `cache::layout::atomic_write`
  (the duplicate atomic-write helper from Phase 1) be deleted.

- **Sweep-test boilerplate consolidation candidate.** All four
  `phase3_retrofit_*.rs` sweep tests share ~100 LOC of fixture
  setup + canned-backend boilerplate. Code reviewer recommends
  extracting to `crates/atlas-cli/tests/common/sweep_support.rs`
  in a post-Wave-3 cleanup PR. Track as Phase 3 cleanup; not
  blocking.

- **Orphan `pub use save_related_components_atomic` re-export.**
  In `atlas-contracts/crates/atlas-index/src/lib.rs:60`, this
  re-export has zero callers in either repo after PR-5. Rust does
  NOT warn on orphan public re-exports. Track for sibling-repo
  cleanup PR; not blocking.

### PR-6
2026-05-08 — Landed: overrides schema gains `edges_add` /
`edges_suppress` (top-level only) and per-component `overrides:`
block (`language` / `kind` / `lifecycle` / `subsystem` fields). Three
commits across two repos:

- atlas-contracts `a0a9a8c` — `phase3: PR-6 atlas-contracts —
  overrides schema gains edges_add/edges_suppress + per-component
  field overrides`. 4 files (+~210 / −1).
- Atlas `1b27827` — `phase3: PR-6 engine — l4 field-override merge
  + l6 edges_add/edges_suppress + l9 post-override projection`.
  8 files (+~580 / −13). The atlas-contracts schema commit must land
  first because atlas-index is path-dep.
- Atlas `27794d6` — `phase3: PR-6 fix-up — reject edges_overrides
  at per-component scope; pub(crate) merged_overrides; warn on bad
  lifecycle; symmetric suppress test`. 3 files (+228 / −13).
  Closes four Important issues from the code-quality review (see
  below).

**Spec compliance ✅** (8/8 acceptance criteria after fix-up; one
narrow deviation on AC-5 documented below). **Code quality ✅** post
fix-up (no Critical issues; four Important issues were closed by
commit `27794d6`).

**Schema-ambiguity resolution.** Plan §4 PR-6 prescribed flat fields
on `OverridesFile`; design §5.5's per-component YAML example showed
them grouped under `overrides:`. Per "design spec wins on schemas",
adopted the design's nested form: a new `ComponentFieldOverrides`
struct with serde rename `#[serde(rename = "overrides", default,
skip_serializing_if = ...)]`. `OverridesFile.field_overrides:
ComponentFieldOverrides` is the in-memory name; the YAML key is
`overrides:`. Empty blocks omit the YAML key entirely (keeps existing
fixtures byte-stable).

**Code surface (commit `1b27827` + `27794d6`):**
- `atlas-contracts/crates/atlas-index/src/schema.rs` — added
  `EdgeAdd`, `EdgeSuppress`, `ComponentFieldOverrides`. Extended
  `OverridesFile` with three new fields (`edges_add`,
  `edges_suppress`, `field_overrides`). `reason` is non-optional
  on EdgeAdd / EdgeSuppress, so missing-reason rejects with a clear
  serde error.
- `atlas-contracts/crates/atlas-index/src/lib.rs` — re-exported the
  three new types.
- `atlas-contracts/crates/atlas-index/src/yaml_io.rs` — extended
  `sample_overrides_file()` and added 7 round-trip / rejection /
  default-shape tests.
- `atlas-engine/src/l4_tree.rs` — `MergedOverrides` aggregating
  pins + additions + edges + per-component field overrides;
  `apply_per_component_field_overrides` post-id-allocation;
  `pub(crate) fn merged_overrides(db)` exposed intra-crate; new
  `validate_per_component_scope` early-rejects edges_add /
  edges_suppress at per-component scope (design §5.5 says these
  are top-level only); warning emission when
  `LifecycleScope::parse` returns `None` for an authored
  lifecycle override (the user's intent surfaces as a stderr-style
  warning, not a silent skip).
- `atlas-engine/src/l6_edges.rs` — `apply_user_edge_overrides`
  unions `edges_add` then subtracts `edges_suppress` (suppress wins
  on same triple). Symmetric edge participants canonicalised
  (sorted) before matching for non-directed kinds. Eight new unit
  tests including the symmetric-suppress regression test.
- `atlas-engine/src/lib.rs` — `merged_overrides` is **not**
  re-exported from the public crate API (Phase 4 LLM analysers must
  emit candidate edges via the sanctioned `edges_add` channel, not
  inspect the override set directly).
- `atlas-cli/tests/phase3_overrides_edges.rs` — 8 file-system
  integration tests covering edges_add insertion, edges_suppress
  no-match (no-op), suppress-after-add (suppress wins), four
  field-override paths (language / kind / lifecycle / subsystem),
  and the per-component scope-violation rejection.
- `.claude/memory/project_phase3_overrides_edges.md` — new
  project-scoped memory recording that `edges_add` / `edges_suppress`
  are canonical user-authoring seams; useful for Phase 4 LLM
  analysers (which should emit candidate edges as `edges_add`
  suggestions, not via a side channel).

**Documented narrow deviation (AC-5):** the no-match suppress
warning text is emitted in production via `eprintln!` at
`l6_edges.rs:304-308`, but the existing CLI integration test
(`edges_suppress_no_match_leaves_set_unchanged`) does not capture
stderr to assert on the warning text. The in-process `run()`
harness used by the test doesn't currently plumb stderr capture;
adding it is broader than PR-6 scope. The behaviour is verified by
the unit test at the helper layer plus the acceptance test that
the edge set is unchanged. Recording here so a future Phase 3 / 4
test-infra cleanup PR can close the gap.

**Documented forward-compatibility no-op:** the `subsystem` field
on `ComponentFieldOverrides` has no destination on `ComponentEntry`
(subsystem membership is tracked via `SubsystemsFile` /
`SubsystemsOverridesFile` schemas with their own pipeline). The
field is captured in the schema for Phase 4 / 5 wiring. Test
`field_override_subsystem_is_captured_but_does_not_panic` confirms
the engine accepts the field without crashing.

**Load-bearing details for downstream PRs (PR-8..PR-11, PR-12):**
- `merged_overrides` is `pub(crate)` — reports / CLI handlers must
  read overrides through the existing engine projection paths
  (`per_component_yaml_snapshot`, `all_components`, etc.), not via
  this internal helper. Phase 5's Salsa conversion will replace
  this with `#[salsa::tracked]` queries.
- `edges_add` / `edges_suppress` are top-level-only. Per-component
  files declaring them are hard-rejected at validation with the
  new `TreeAssemblyError::EdgesOverridesAtPerComponentScope`
  variant. Reports that read the post-override edge set get the
  unioned-then-subtracted result.
- The post-override `language` / `kind` / `lifecycle` values flow
  through `l9_projections::per_component_yaml_snapshot` to the
  cached `<component>/.atlas/cache/component.yaml`. Modularity
  (PR-10) and Divergence (PR-11) reports that read these fields
  will see the post-override values.

**Verification:** `cargo test --workspace --no-fail-fast` 1129
passing in Atlas, 186 in atlas-contracts. `cargo clippy
--all-targets -- -D warnings` clean in both. `cargo fmt --check`
clean in both. `cargo build --release` clean (the PR-1
panic-injection invariant is preserved — PR-6 doesn't touch
atomic_write).

Wave 5 (PR-8 drift / PR-9 impact / PR-10 modularity / PR-11
divergence) is now dispatchable in parallel. PR-6 was helpful but
not strictly required for the reports to function; they observe
whatever edges the engine produces.

### PR-8
2026-05-08 — Landed: drift report + `atlas drift` CLI subcommand.
Cherry-picked onto main as `1060edb` from worktree branch
`phase3-pr8` commit `e5f1524` (10 files, +1457 / −33). The
implementing agent did the work but did not generate a DONE report
or commit — the orchestrator inspected the worktree, ran cargo
verification, committed on the branch, and cherry-picked.

**Code surface:**
- `crates/atlas-reports/src/drift.rs` — `pub fn drift(inputs,
  prev_snapshot)` returns `(DriftReport, ContractShaSnapshot)`.
  Pure-function: iterates contracts and bindings via a
  `DriftEngineView` trait abstraction (over `&AtlasDatabase` in
  production, over a hand-built fixture trait impl in unit tests).
  First-run case returns empty change arrays + fresh snapshot.
  `pinned_bindings` are sorted by `(component, contract_id)` for
  determinism.
- `crates/atlas-reports/src/lib.rs` — re-exports.
- `crates/atlas-cli/src/reports.rs` — `atlas drift` handler:
  reads `<root>/.atlas/cache/contract-shas-snapshot.yaml` if
  present, calls `atlas_reports::drift`, renders to stdout, then
  unless `--no-write` writes both `.atlas/cache/reports/drift.yaml`
  and `.atlas/cache/contract-shas-snapshot.yaml` atomically via
  PR-1's `atlas_engine::atomic_write`. First-run UX prints
  `"No prior baseline found. Captured baseline of N contracts.
  Run \`atlas drift\` again after changes to see drift."`.
- `crates/atlas-cli/src/{lib,main}.rs` — wiring for the new
  handler (drift was previously stubbed by PR-7).
- `crates/atlas-cli/Cargo.toml` — adds atlas-reports dep with the
  `atomic_write_panic_after_temp` cargo feature on atlas-engine
  (gated to test runs only).
- `crates/atlas-cli/tests/atlas_drift.rs` — 4 CLI integration
  tests including the kill-during-write fixture that exercises
  the panic-injection hook.

**PR-12 scope-creep (acknowledged, not blocking):** PR-8's AC
required a `kill-during-snapshot-write` integration test. The
agent satisfied the AC by introducing a feature-gated
panic-injection hook in `crates/atlas-engine/src/atomic_write.rs`
and a public `test_hooks_pub::arm_panic_before_rename()` helper
gated behind the `atomic_write_panic_after_temp` cargo feature.
The hook is one-shot, thread-local, and absent from release
builds — verified via `cargo build --release` (clean). This is
infrastructure that PR-12 was nominally going to add; PR-12's
remaining scope is therefore narrowed to "exercise the hook with
the broader fixture suite across both stateful files (drift +
modularity history)" rather than "add the hook AND the
fixtures."

**Acceptance criteria:**
- 7 unit tests in drift.rs (drift_first_run_no_baseline,
  drift_baseline_unchanged, drift_baseline_changed,
  drift_contract_added, drift_contract_removed,
  drift_pinned_binding_detected, drift_pinned_binding_up_to_date)
  plus 4 bonus tests (sorting, null-derived-from, summary
  aggregation across changed contracts, round-trip yaml/json).
- 4 CLI integration tests
  (atlas_drift_first_run_writes_snapshot_and_empty_report,
  atlas_drift_second_run_after_contract_change_reports_drift,
  atlas_drift_no_write_flag_skips_writes,
  atlas_drift_kill_during_snapshot_write_leaves_file_intact).

**Verification on main (post-cherry-pick):**
- `cargo test --workspace --no-fail-fast`: clean (the worktree-
  local `phase2_polyglot_fixture` failures were the documented
  Wave-3 worktree-target/ artifact pattern; passing 3/3 on main).
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release`: clean (panic-injection hook absent
  from release binary symbols).

**Load-bearing details for downstream PRs (PR-11, PR-12):**
- The drift snapshot file path is
  `<root>/.atlas/cache/contract-shas-snapshot.yaml`. PR-11
  (divergence) reads this file read-only to compute severity. The
  schema is `ContractShaSnapshot` from `atlas-reports::snapshot`.
- The atomic-write panic-injection hook lives at
  `atlas_engine::atomic_write::test_hooks_pub` (gated behind the
  `atomic_write_panic_after_temp` cargo feature on atlas-engine).
  PR-12's broader fixture suite reuses this surface; it should NOT
  re-introduce the hook.

**Worktree-base bug observation:** PR-8 (and the other Wave 5
agents) did not generate DONE reports — the implementing agent
did substantive work in the pre-created worktree but appears to
have hit a token / time budget before issuing a final summary or
committing on the branch. The orchestrator (this session) handled
the commit + cherry-pick + verification cycle. Work product is
sound; the orchestration overhead is a fact of life with multi-PR
parallel agent dispatch and large per-PR LOC budgets.

### PR-9
2026-05-08 — Landed: impact query + `atlas impact <id>` CLI
subcommand. Cherry-picked onto main as `0ec65c5` from worktree
branch `phase3-pr9` commit `d6484a8` (6 files, +1283 / −39). The
implementing agent generated a clean DONE report; orchestrator
made one fix-up during the cherry-pick (see below).

**Code surface:**
- `crates/atlas-reports/src/impact.rs` — `pub fn impact(inputs,
  target)` returns `Result<ImpactReport, ReportError>`. Resolves
  contract-id-vs-component-id via two-pass lookup, walks
  `consumes-contract` edges with a seen-set (cycle-safe), unions
  impact across contracts when target is a component, builds three
  independent partitions (language / deploy_graph / lifecycle), and
  returns Levenshtein-1 candidates on TargetNotFound. Includes an
  inline `levenshtein_distance_1_candidates` helper (no external
  crate).
- `crates/atlas-reports/Cargo.toml` — adds `salsa` workspace dep.
- `crates/atlas-cli/src/reports.rs` — `atlas impact` handler reads
  from `Workspace::prior_components` / `Workspace::prior_related_components`
  (the YAMLs `atlas index` already wrote) via a hard-error
  `ReportsBackend` that fails loudly on any LLM call attempt. Renders
  to stdout in YAML/JSON/human format. Exit 0 on success; exit 2 +
  "did you mean: ..." on TargetNotFound.
- `crates/atlas-cli/Cargo.toml` — adds `salsa` workspace dep.
- `crates/atlas-cli/tests/atlas_impact_cli.rs` — 4 CLI integration
  tests.

**Orchestrator fix-up during cherry-pick:**
- Replaced `atlas_cli::DEFAULT_OUTPUT_SUBDIR` with
  `crate::DEFAULT_OUTPUT_SUBDIR` at `reports.rs:570`. The original
  symbol path works from `main.rs` (binary referencing the lib
  externally) and from `tests/`, but not from inside the lib's own
  `src/`.
- Header docstring + imports union-merged with PR-8 (drift) which
  landed on main earlier as `1060edb`.

**Architectural deviation (significant):** PR-9 reads from on-disk
YAMLs rather than calling `all_components(db)` / `all_proposed_edges(db)`
on a live engine database. Plan §4 PR-9 prescribed `ReportInputs {
db, workspace }`; PR-9 instead populates `Workspace::prior_*` slots
from `<output>/cache/*.yaml` and uses a hard-error backend. Per
design §3.1 / §3.3 ("reports observe what the engine has already
produced"), this is a sound interpretation, but it differs from
PR-10 / PR-11's choice (which build a full database and run the
fixedpoint, relying on the persistent LLM cache). PR-13's polyglot
smoke test is the ground-truth verifier (cold = Phase 2 baseline,
warm = 0, report-runs = 0 LLM calls).

**Other deviations:**
- Deploy-graph partition keys: design exemplar shows
  `compose:dev` / `compose:ops` synthetic labels; PR-9 uses the
  orchestration component's id (e.g. `ravel-lite/compose-dev`)
  derived from `bundled-into` edges. Wire format
  (`BTreeMap<String, Vec<String>>`) accepts any string key.
- Lifecycle partition keys: design exemplar shows
  `runtime` / `build-time` / `test-only`; PR-9 uses the canonical
  `LifecycleScope::as_str()` values (`runtime` / `build` / `test` /
  `deploy` / etc.).

**Acceptance criteria:** 9 unit tests in impact.rs +
4 CLI integration tests in atlas_impact_cli.rs.

### PR-11
2026-05-08 — Landed: composition divergence report + `atlas
divergence` CLI subcommand. Cherry-picked onto main as `8ddd3c5`
from worktree branch `phase3-pr11` commit `4c0d927` (4 files,
+1268 / −35). The implementing agent generated a clean DONE
report; orchestrator made three fix-ups during the cherry-pick
(see below).

**Code surface:**
- `crates/atlas-reports/src/divergence.rs` — `pub fn divergence(
  inputs, drift_baseline)` returns `Result<DivergenceReport,
  ReportError>`. Builds build-graph (direct `depends-on`) and
  deploy-graph (any composition edge), iterates unordered
  component pairs, classifies each as `build_coupled XOR
  deploy_coupled`, computes severity from drift baseline. Sorts
  divergent pairs lexicographically by `(min, max)` for
  determinism.
- `crates/atlas-cli/src/reports.rs` — `run_divergence_cmd` builds a
  full `AtlasDatabase`, runs the fixedpoint (relying on the
  persistent LLM cache for warm-no-cost), reads the optional drift
  snapshot read-only from
  `<output>/.atlas/cache/contract-shas-snapshot.yaml`, calls
  `atlas_reports::divergence`, atomically writes
  `<output>/.atlas/cache/reports/composition-divergence.yaml` via
  `atlas_engine::atomic_write` unless `--no-write`.
- `crates/atlas-cli/src/lib.rs` — promoted `pub mod reports` so
  integration tests can call `run_divergence` directly. PR-9
  already had this on its branch; main reflects it from PR-9's
  cherry-pick.
- `crates/atlas-cli/Cargo.toml` — adds `chrono` dev-dep.
- `crates/atlas-cli/tests/divergence_cli.rs` — 4 CLI integration
  tests including the `does_not_modify_drift_snapshot` regression
  guard (asserts both bytes-equality and mtime-equality of the
  snapshot before/after the divergence run).

**Orchestrator fix-ups during cherry-pick:**
- Header docstring + imports union-merged with PR-8 (drift) +
  PR-9 (impact).
- `print_human(&DivergenceReport)` renamed to
  `print_divergence_human` to avoid collision with PR-9's
  `print_human(&ImpactReport)` (Rust does not have function
  overloading).
- Removed duplicate local `const DEFAULT_OUTPUT_SUBDIR: &str =
  ".atlas";` (the import from `crate::pipeline` is canonical).

**Architectural call (acknowledged):** PR-11 builds a full
`AtlasDatabase` and runs the fixedpoint, mirroring PR-10's
approach. This is a different mechanism than PR-8/PR-9's
read-from-YAMLs. Both satisfy the no-new-LLM-calls invariant; the
mechanism difference is load-bearing for future
converge-or-keep-divergent cleanup.

**PR-8 dependency handling:** divergence's CLI tests hand-craft the
drift snapshot via `serde_yaml::to_string` of an in-memory
`ContractShaSnapshot`, decoupling PR-11 verification from PR-8's
landing order.

**Code-duplication tracking item:** PR-11 introduces a private
`build_database_for_reports` helper in `reports.rs`. PR-10 added a
parallel `build_engine_database` in `pipeline.rs` (better location).
Future cleanup PR should converge the two through a shared inner
helper. Track as Phase 3 / 4 cleanup; not blocking.

**Acceptance criteria:** 9 unit tests in divergence.rs +
4 CLI integration tests in divergence_cli.rs.

### PR-10
2026-05-08 — Landed: modularity report + `atlas modularity` CLI
subcommand. Cherry-picked onto main as `4ce7245` from worktree
branch `phase3-pr10` commit `36e7242` (6 files, +2078 / −42 — the
largest single PR in Phase 3). The implementing agent generated a
clean DONE report; orchestrator made three fix-ups during the
cherry-pick (see below).

**Code surface:**
- `crates/atlas-reports/src/modularity.rs` — six metric formulas
  (Ca / Ce / Instability / Cohesion / Surface stability / Surface
  complexity), history rotation (FIFO, hard cap 5 entries),
  subsystem aggregates with `>2σ` outlier flagging. Pure-function
  over `ReportInputs` + `prior_per_component: HashMap<ComponentId,
  ModularityHistory>`. The `HISTORY_CAP` is a private `const usize
  = 5`, **not configurable** — its docstring cites plan §7.3's
  deferred-indefinitely list.
- `crates/atlas-cli/src/reports.rs` — `run_modularity_cmd`
  (production entry) + `run_modularity` (library entry,
  integration-testable) + `ModularityRunOptions` +
  `render_modularity_human`. The handler walks every component,
  reads each prior `<component>/.atlas/cache/modularity.yaml` if
  present, calls `atlas_reports::modularity`, then unless
  `--no-write` writes per-component files atomically AND the top-
  level `<root>/.atlas/cache/reports/modularity-rollup.yaml`
  atomically.
- `crates/atlas-cli/src/pipeline.rs` — new `pub fn
  build_engine_database` (the L4–L6 fixedpoint + L5 pre-warm
  without writes) and `pub fn resolve_component_dir`. Parallel to
  `run_index`; convergence deferred. The CLI handler carries its
  own `--budget` / `--no-budget` posture (mirrors `atlas index`).
- `crates/atlas-cli/src/lib.rs` — exposes the new module + helpers.
- `crates/atlas-cli/Cargo.toml` — adds `atlas-reports`,
  `chrono`, `serde_yaml` dev-deps.
- `crates/atlas-cli/tests/atlas_modularity.rs` — 5 CLI integration
  tests.

**Orchestrator fix-ups during cherry-pick:**
- Header docstring + imports union-merged with PR-8 / PR-9 / PR-11.
- Removed PR-10's duplicate `use atlas_cli::reports;` (PR-9 already
  added it on main as `main.rs:157`).
- Auto-merged Cargo.toml's atlas-reports dev-dep addition.

**Deviations:**
- `subsystem_outlier_flagged_at_2_sigma` fixture: plan called for
  "one member at 2.5σ from mean → flagged" but with three
  sample-stddev members the maximum achievable z-score is √2 ≈
  1.414σ, structurally below 2σ. The test uses 8 members (seven at
  0.0, one at 1.0) which produces ≈2.475σ — still satisfies
  ">2σ flagged" semantics. Inline comment walks through the
  arithmetic.
- Subsystem aggregates use `surfaces.contracts_defined.len()` for
  `total_bindings` (one defining binding per contract today). If
  Phase 4+ adds multi-binding contracts, this becomes a
  sum-of-bindings; the helper signature
  `compute_surface_complexity(provided, total_bindings)` already
  accommodates both shapes.
- Test name typography: plan listed `ce_counts_distinct_provided-by`
  with a hyphen; renamed to `ce_counts_distinct_provided_by`
  (Rust identifiers do not allow hyphens).

**Architectural call:** PR-10 follows PR-11's pattern (full engine
database + fixedpoint with cached LLM responses) rather than
PR-8/PR-9's read-from-YAMLs. PR-13's smoke test is the
ground-truth verifier.

**Acceptance criteria:** 9 metric tests + 2 history-rotation tests
+ 5 subsystem tests + 5 CLI integration tests = 21 of 21.

## Wave 5 closeout — 2026-05-08

**Wave 5 is complete.** All four reports (PR-8 drift / PR-9 impact /
PR-10 modularity / PR-11 divergence) are on main with passing tests.
Verification on main with all four landed:
- `cargo test --workspace --no-fail-fast` — all green.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.
- `cargo build --release` — clean (panic-injection hook absent).

**Cumulative session learnings** captured for future sessions:

- **Parallel agent dispatch with large per-PR LOC budgets is high
  variance.** PR-8's agent did the work but didn't generate a DONE
  report or commit (likely token-budget exhaustion at 240k tokens /
  130 tool uses). Other three agents reported cleanly. The
  orchestrator (this session) ran cargo verification on the
  worktree, committed on the branch, and cherry-picked onto main
  for all four PRs. Rule of thumb: don't trust agents to commit
  cleanly across multi-thousand-LOC PRs; always verify the worktree
  state independently.

- **Status file is orchestrator-owned.** Two of four Wave 5 agents
  (PR-9 + PR-11) included status-file edits in their commits despite
  the prompt instructing otherwise. Cherry-pick conflicts on the
  status file are easy to resolve (`git checkout HEAD -- ...`) but
  the cleaner fix is a stronger upfront prompt: explicit "DO NOT
  MODIFY docs/superpowers/plans/*.md" line.

- **Architectural divergence between report PRs.** PR-8/PR-9 read
  from on-disk YAMLs (no engine recomputation); PR-10/PR-11 build a
  full database and rely on the persistent LLM cache for warm-no-
  cost. Both satisfy the no-new-LLM-calls invariant but the
  mechanism difference is real. Convergence to a single approach
  is a Phase 3 / 4 cleanup; PR-13's smoke test verifies the
  invariant holds across both.

- **`reports.rs` cherry-pick conflict pattern.** All four Wave 5
  PRs modified `crates/atlas-cli/src/reports.rs` to fill in
  different stub bodies left by PR-7. Cherry-picking onto main with
  prior PRs landed required:
  1. Header docstring union-merge (each PR adds its description).
  2. Imports union-merge (each PR adds disjoint import sets).
  3. Function-name collision fix (`print_human` was used by both
     PR-9 and PR-11 with different parameter types — Rust has no
     overloading; one had to be renamed).
  4. `DEFAULT_OUTPUT_SUBDIR` const dedup (PR-9 added a local const;
     PR-11 imported the canonical one from `crate::pipeline`).
  None of these is hard, but they accumulate. Sequential
  cherry-pick with cargo check after each is the right cadence.

- **Phantom polyglot failures recurred.** As in Wave 3, the
  worktree-local `target/` artifacts caused
  `phase2_polyglot_fixture` to fail on the worktree but pass 3/3
  on main with the diff cherry-picked. Cherry-pick-then-verify is
  the canonical resolution; do not trust worktree test output
  alone for the polyglot suite.

Wave 6 (PR-12 atomic-write fixture suite) is now dispatchable. PR-8
already brought forward the panic-injection hook
(`atlas_engine::atomic_write::test_hooks_pub`, gated behind the
`atomic_write_panic_after_temp` cargo feature), so PR-12's scope
narrows to "exercise the hook with the broader fixture suite across
both stateful files (drift snapshot + modularity history)" rather
than "add the hook AND the fixtures."

### PR-12
2026-05-08 — Landed: atomic-write fixture suite for stateful files.
Cherry-picked onto main as `2e8f19d` from worktree branch
`phase3-pr12` commit `a90f4068` (4 files, +673 / −3). Clean DONE
report from the implementing agent; cherry-pick was conflict-free.

**Code surface:**
- `crates/atlas-engine/src/atomic_write.rs` — added a SYMMETRIC
  `panic_after_rename` hook to the existing `test_hooks_pub`
  module (`arm_panic_after_rename`, `disarm_panic_after_rename`,
  `maybe_panic_after_rename`). One-shot, thread-local, mirrors
  PR-8's `panic_before_rename` discipline. Wired AFTER
  `fs::rename(...)` so a panic from this hook leaves the
  destination fully-new.
- `crates/atlas-reports/Cargo.toml` — adds dev-deps `atlas-engine`
  with `atomic_write_panic_after_temp` feature, `rand 0.8`,
  `tempfile`.
- `crates/atlas-reports/tests/atomic_writes.rs` — new file (599
  lines): five named fixture tests + 10-iteration random-kill
  stress test (`StdRng::seed_from_u64(12345)` for reproducibility).

**Acceptance criteria — all pass:**
- `drift_snapshot_kill_during_write_leaves_file_intact`
- `drift_snapshot_kill_after_rename_succeeds`
- `modularity_history_kill_during_write_preserves_prior_5_entries`
- `modularity_history_kill_after_rename_persists_rotation`
- `drift_and_modularity_atomic_writes_are_kill_safe_under_random_stress`
  (10 iterations seeded)

**Hook-absence verification (release build):**
- `cargo build --release` clean (no warnings).
- `nm target/release/atlas | grep -ci 'panic_before_rename\|panic_after_rename'` → 0 matches.
- `nm target/release/libatlas_engine.rlib | grep -ci 'panic_before_rename\|panic_after_rename'` → 0 matches.
- `nm target/release/libatlas_reports.rlib | grep -ci 'panic_before_rename\|panic_after_rename'` → 0 matches.
- The feature gate `atomic_write_panic_after_temp` is opt-in
  (default-off); enabled only on dev-deps in `atlas-cli` (PR-8) and
  `atlas-reports` (PR-12).

**Documented narrow deviation:** Plan §4 PR-12 says
"invoke `atlas_reports::modularity(...)` to compute the per-component
output". The agent instead constructed `ComponentModularity` fixture
values directly because `atlas_reports::modularity()` requires a
real `&AtlasDatabase` and constructing one from a tempdir tree
crosses into "the CLI handler does additional work irrelevant to
atomic-write semantics" the prompt warned against. The serialise +
atomic_write call sequence is identical to the CLI handler's. The
local re-declaration of `HISTORY_CAP = 5` is acceptable because the
cap is fixed by design spec (not configurable; drift between values
would itself be a spec violation).

**Wave 7 ready:** PR-13 (Phase 3 polyglot smoke test) is the final
PR. It depends on every prior wave: cache layout (PR-2..PR-5),
overrides extension (PR-6), reports framework (PR-7), all four
report bodies (PR-8..PR-11), and atomic-write fixtures (PR-12 — for
the kill-during-write reliability story). LLM call budget assertions
must match Phase 2's PR-14 baseline (~26 cold, 0 warm, 0 reports).

### PR-13
2026-05-08 — Landed: Phase 3 polyglot smoke test. Two commits on
main:
- `6520acb` — main implementation (44 files, +1474). Cherry-picked
  from worktree branch `phase3-pr13` commit `582b71c`. The agent
  did not generate a DONE report (token-budget exhaustion
  pattern, same as PR-8); orchestrator inspected the worktree, ran
  cargo verification (1 passed in ~17.5 min), and committed.
- `a1fde5d` — fix-up: force-add `.atlas/components.overrides.yaml`
  and `.atlas/subsystems.overrides.yaml` (2 files, +131). The repo's
  top-level `.gitignore:19 .atlas/` rule had filtered them out of
  the orchestrator's `git add -A` during cherry-pick, leaving the
  fixture incomplete on main even though it was complete on the
  worktree. Polyglot test asserted on `edges_add` materialisation
  and surfaced this as a data-missing failure.

**Code surface:**
- `crates/atlas-cli/tests/phase3_polyglot_fixture.rs` — single
  `polyglot_phase3_acceptance` integration test (~1133 lines).
  Pattern follows Phase 2's `phase2_polyglot_fixture.rs`. Reuses
  `PR14Backend` for LLM-call counting.
- `crates/atlas-cli/tests/fixtures/phase3_polyglot/` — verbatim
  copy of Phase 2 fixture plus:
  - `outlier_cluster/` (6 peer + 1 outlier crates) — drives the
    `>2σ` efferent-coupling outlier flag for modularity.
  - `compose-proxy/` + `compose/` updates — deploy-only divergence
    trigger via `co-deployed-with`.
  - `.atlas/components.overrides.yaml` — `edges_add` (depends-on,
    flutter-app, dart-lib) + `edges_suppress` (one analyser-
    discovered edge), each with required `reason`.
  - `.atlas/subsystems.overrides.yaml` — three-member subsystem
    fixture with the outlier as a member.

**Phase 2 fixture and `phase2_polyglot_fixture.rs` are byte-
identical pre/post** (verified via `git diff main` returning 0
lines on the worktree).

**Test coverage (8-step run order, all assertions pass):**
1. `atlas index` cold — L4 cache populated with PR-2..PR-5
   retrofit paths; gitignore present at every `.atlas/` scope.
2. `atlas drift` first run — baseline captured; first-run UX
   guidance message printed.
3. Mutate one contract via fixture-helper.
4. `atlas index` warm + delta — exactly the affected component
   re-classifies.
5. `atlas drift` second run — one entry in `contracts_changed`
   with expected pinned-binding entries.
6. `atlas modularity` — per-component files written; rollup
   written; deliberate-outlier component in subsystem outliers
   for `efferent_coupling`.
7. `atlas divergence` — two divergent pairs (one `deploy_only`,
   one `build_only`).
8. `atlas impact` — partition axes correctly populated.

**LLM call budget invariants (strict, all pass):**
- Cold (step 1): matches Phase 2 PR-14 baseline.
- Warm rerun: 0.
- Drift run 1: 0 (read-only handler).
- Drift run 2: 0.
- Modularity: 0 (fixedpoint cache-hit-only).
- Divergence: 0 (fixedpoint cache-hit-only).
- Impact: 0 (read-only handler).
- Each report-run assertion message ends "A non-zero count is a
  Phase 3 invariant violation — do not relax."

**Cache-discipline assertions (all pass):**
- All cache files under `<scope>/.atlas/cache/`.
- `<scope>/.atlas/.gitignore` exists at every scope and contains
  `cache/`.
- All eight new Phase 3 cache files populated by end of run order
  (drift snapshot, drift report, modularity rollup, divergence
  report, components.yaml, related-components.yaml, per-component
  modularity.yaml + surfaces.yaml + component.yaml).

**Override fixture assertions (all pass):**
- `edges_add (depends-on, flutter-app, dart-lib)` materialises in
  `<root>/.atlas/cache/related-components.yaml`.
- `edges_suppress` entry eliminates the matching analyser-
  discovered edge.

**Final cross-tree verification:**
- `cargo test --workspace --no-fail-fast`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build --release`: clean (panic-injection hooks absent).

## Phase 3 — complete

2026-05-08 — Phase 3 ships. All 14 PRs landed, all four reports
working end-to-end against the polyglot fixture, every Phase 3
invariant verified by tests:

- **Greenfield maintained.** No on-disk format compatibility with
  Phase 1 / Phase 2; no migration commands; users upgrading delete
  `.atlas/` and re-run.
- **Six-file editorial tier preserved** (top-level `overrides`,
  `external-components`, `subsystems`, `analyzers`, `config` +
  per-component `overrides`). Phase 1's surfaces / component /
  components / related-components moved to the cache (gitignored)
  tier. PR-13's polyglot fixture exercises the editorial files.
- **Schema_version stays at 1.** All Phase 3 on-disk schemas (drift
  snapshot, drift report, impact, modularity per-component +
  rollup, divergence) ship as v1.
- **Zero new LLM call sites.** Cold polyglot LLM-call count matches
  Phase 2 PR-14 baseline; every report run is 0 LLM calls. Verified
  by PR-13's strict assertions.
- **All cache writes are atomic.** PR-1's helper covers everything;
  PR-12's fixture suite stress-tests the kill-during-write
  semantics for the two stateful files (drift snapshot, modularity
  history).
- **Reports crate is pure-function.** `crates/atlas-reports/` has
  zero `fs::*` calls; all I/O lives in the CLI handlers. This is
  the Phase 5 Salsa-conversion invariant.

**Architectural notes for Phase 4 / 5 reviewers:**

- **PR-8 / PR-9 read-from-YAMLs vs PR-10 / PR-11 fixedpoint-with-
  cache.** The four reports take two different approaches to
  reading engine state. Both satisfy the no-new-LLM-calls invariant
  (PR-13 verifies). Convergence to a single approach is a future
  cleanup; PR-13's smoke test is the regression guard if convergence
  changes the budget numbers.
- **`build_engine_database` (PR-10's pipeline helper) and
  `build_database_for_reports` (PR-11's reports helper) are near-
  duplicates.** Both build a fresh `AtlasDatabase` and run the
  fixedpoint. Future PR can converge through a shared inner
  helper; not blocking.
- **Orphan `pub use save_related_components_atomic` in
  atlas-contracts.** From PR-5 closeout. Untouched in Phase 3.
- **`.atlas/` gitignore + fixture interaction.** PR-13's fix-up
  commit (`a1fde5d`) documents that fixture editorial files need
  `git add -f` because the repo's top-level `.gitignore` excludes
  `.atlas/`. Future fixtures with editorial files must follow this
  pattern.

**Cumulative LLM-call savings on the polyglot fixture:** Phase 3
introduces zero new LLM call sites, so cold cost matches Phase 2's
PR-14 baseline. Warm reruns and report runs are all 0. The savings
of Phase 3 are **operational** (drift / impact / modularity /
divergence reports become available without engine recomputation
cost) rather than **token-budget** — this is the design intent
("reports observe what the engine has already produced", design
§3.1).

**Deferred items the user may want to revisit (none blocking):**

- Stale "Phase 4" prose references in canonical system-model spec
  (§5.6, §9, §11.4, glossary line ~1436). PR-0b's spec review
  surfaced these as semantically stale post-renumbering. Docs-only
  retext PR; not blocking Phase 3 since no Phase 3 code reads the
  prose. Recorded in PR-0b's per-PR notes.
- Sweep-test boilerplate consolidation (PR-2..PR-5). ~100 LoC
  duplicated across four `phase3_retrofit_*.rs` files; reviewer
  recommended `crates/atlas-cli/tests/common/sweep_support.rs`.
  Not blocking.
- Phase 3 cache-tier writer-idiom convergence: some retrofits use
  `atlas_engine::atomic_write` directly (mkdirs internally), some
  use `save_*_atomic` helpers + explicit `create_dir_all`. Both
  are correct; convergence to a single idiom would also let
  `cache::layout::atomic_write` (the duplicate atomic-write helper
  from Phase 1) be deleted.
- Convergence of the `build_engine_database` (PR-10) and
  `build_database_for_reports` (PR-11) helpers; ~50 LoC
  duplicated.
- `subsystem` field on `ComponentFieldOverrides` is captured in
  the schema but applied as a no-op at L4 (because `ComponentEntry`
  has no `subsystem` field). Future Phase 4+ wiring point.
- AC-5 narrow deviation in PR-6: the `edges_suppress` no-match
  warning is emitted in production via `eprintln!` but not
  captured in CLI tests because the in-process `run()` harness
  doesn't plumb stderr capture. Verifiable manually; closing the
  gap is a small test-infra cleanup.

Phase 3 is **ready for the next session's continuation prompt to
report success and stop, OR for a Phase 4 brainstorm**. The Phase
3 design spec §9.1 lists Phase 4 candidates: pattern detection,
subprocess convergence, rust-analyzer integration, LLM threshold
calibration. The user has not yet drafted a Phase 4 design spec
(verified `ls docs/superpowers/specs/2026-05-*-atlas-vnext-phase4-design.md`
returns no matches). Per the continuation prompt's Step 4: "the
user has not yet decided what Phase 4 contains. Stop the session,
report Phase 3 success, and surface the question 'Phase 3 is
complete. Phase 4 design candidates per Phase 3 design §9.1
(convergence + cleanups + LLM analyses): pattern detection,
subprocess convergence, rust-analyzer integration, LLM threshold
calibration, etc. Which subset is Phase 4 vs Phase 5? Want me to
brainstorm Phase 4 scope?'"

