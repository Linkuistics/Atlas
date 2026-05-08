# Atlas vNext Phase 3 — Status

Companion to `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`.
This file tracks per-PR completion state across sessions. The
continuation prompt at
`docs/superpowers/prompts/2026-05-08-vnext-continue.md` (Phase-3-shaped)
reads this file (via the `*phase3-plan*` wildcard match) to find the
next PR to dispatch.

**Last updated:** 2026-05-08 (PR-4 + PR-5 landed: top-level
`components.yaml` and `related-components.yaml` retrofitted to
`<root>/.atlas/cache/`). Wave 3 (PR-2..PR-5) half complete:
PR-4 + PR-5 on main; PR-2 + PR-3 awaiting sequential redispatch
(initial parallel-dispatch agents hit a session-state worktree-base
bug — see `.claude/memory/feedback_worktree_base_verification.md`).
Wave 2 closed.

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0a — Plan + status (docs only)
- [x] PR-0b — Design-doc touch-ups in canonical system-model spec (docs only)
- [x] PR-1  — Gitignore mechanism for `<scope>/.atlas/cache/` + atomic_write helper
- [ ] PR-2  — Phase 1 retrofit: per-component `surfaces.yaml` → cache
- [ ] PR-3  — Phase 1 retrofit: per-component `component.yaml` → cache
- [x] PR-4  — Phase 1 retrofit: top-level `components.yaml` → cache
- [x] PR-5  — Phase 1 retrofit: top-level `related-components.yaml` → cache
- [ ] PR-6  — Overrides schema extension: `edges_add` / `edges_suppress` + per-component field overrides
- [x] PR-7  — `atlas-reports` crate scaffold + CLI subcommand framework
- [ ] PR-8  — Drift report + `atlas drift` CLI subcommand
- [ ] PR-9  — Impact query + `atlas impact <id>` CLI subcommand
- [ ] PR-10 — Modularity report + `atlas modularity` CLI subcommand
- [ ] PR-11 — Composition divergence + `atlas divergence` CLI subcommand
- [ ] PR-12 — Atomic-write fixture suite for stateful files
- [ ] PR-13 — Acceptance: Phase 3 polyglot smoke test

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
