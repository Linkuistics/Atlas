# Atlas vNext Phase 5 — Status

Companion to `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-10-vnext-continue.md` (Phase-5-shaped) reads this file (via the `*phase5-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-10 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — Fold A: atlas-contracts in-tree (structural)
- [x] PR-2 — Drop discovery (deletion + CLI surface change)
- [x] PR-3 — Singularise `Workspace` (type + call-site refactor)
- [x] PR-4 — Salvage tests (test suite surgery)
- [x] PR-5 — Retext canonical system-model design (docs only)
- [ ] PR-6 — Acceptance + closeout (verification only)

When every box is `[x]`, Phase 5 is complete and the continuation prompt should report success and route to the Phase 6 brainstorm question (per validated roadmap; Phase 6 = user-facing schema cleanups; canonical §10.6).

## Dependency graph (informational; canonical in plan §3)

```
PR-0 ──► PR-1 ──► PR-2 ──► PR-3 ──► PR-4 ──► PR-6
                                              ▲
                              PR-5 ───────────┘  (docs-only; parallel-safe with PR-2/3/4)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (this commit).
- **Wave 1 (after PR-0):** PR-1 alone. The atlas-contracts fold is structural and gates everything downstream — both because the in-tree schema crates are workspace members from this point forward and because the Ravel-Lite cross-repo path-edit commit pairs with Atlas PR-1.
- **Wave 2 (after PR-1):** PR-2 alone. The CLI surface change + `expand_roots` deletion. Sequential — PR-2 must precede PR-3 because PR-3 collapses the type whose call sites PR-2 stops *populating*.
- **Wave 3 (after PR-2):** PR-3 alone. The Workspace type collapse + ~30 call-site rewrites. Sequential — PR-4 depends on the post-PR-3 singular API.
- **Wave 4 (after PR-3):** PR-4 alone. The salvaged single-root test.
- **Parallel branch — PR-5:** docs-only, surface disjoint from PR-1..PR-4. May dispatch concurrent with any of waves 2–4 in a separate worktree. Must merge before PR-6 (which contains the SHA-backfill step).
- **Wave 5 (final):** PR-6 — acceptance + closeout. Depends on PR-5 being in (the `<PR-6-COMMIT-SHA>` placeholder in canonical §10.5 backfills inside PR-6's two-commit window).

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard for every Phase 5 PR. Each PR's checkbox-flip step includes a final `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` invocation; the strict LLM-call-budget assertions (cold = Phase 2 PR-14 baseline ~26 calls; warm + reports = 0) catch any drift.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; cross-repo coordination outcomes (Ravel-Lite path-edit commit sha); manual verification steps that succeeded; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0
2026-05-10 — Landed: the Phase 5 plan, this status file, and the continuation prompt. Commit: `<sha>` on main. Plan: `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-10-vnext-continue.md`.

### PR-1
2026-05-10 — Atlas-contracts folded in-tree as Atlas workspace members. Atlas commit `42db86e342f287bde585f969ad80fe6123a90dd9` on main; coordinated Ravel-Lite path-edit commit `820c083ae34fff837476f5aa11d507eeba1e2504` on Ravel-Lite main (Atlas first, Ravel-Lite second per design spec §5 R1). All cargo gates clean: `cargo build --workspace`, `cargo test --workspace --release --no-fail-fast` (78 test result lines, 0 failures), `cargo clippy --all-targets -- -D warnings` (0), `cargo fmt --check` (0), `cargo build --release --workspace`, polyglot smoke test (`phase3_polyglot_fixture` ok in 89s — strict cold/warm-call assertions held).

**Website-merge layout:** Option A — copied `~/Development/atlas-contracts/website/{index.md,meta.yml}` into `website/docs/schema/`. Atlas's static site generator is directory-driven (no nav field in top-level `meta.yml`), so no top-level `meta.yml` change required; the new `docs/schema/` directory is auto-discovered.

**defaults/ diff resolution:** `ontology.yaml` was unique to atlas-contracts and copied across to `defaults/ontology.yaml`. This was forced earlier than the plan ordering anticipated because `component-ontology/src/defaults.rs` does `include_str!("../../../defaults/ontology.yaml")` — without the file, `cargo build --workspace` fails. No deviation from intent, just resequencing.

**`cargo publish --dry-run` outputs:**

- `component-ontology`: `defaults/ontology.yaml` relocated from workspace root into the crate (`crates/component-ontology/defaults/ontology.yaml`) in follow-up commit `98ee68d`; `include_str!` updated from `"../../../defaults/ontology.yaml"` to `"../defaults/ontology.yaml"`. Tarball now captures the file. `cargo publish --dry-run -p component-ontology` clean: "Packaged 13 files, 94.2KiB (23.9KiB compressed)" + Verifying succeeded.
- `atlas-index`: dry-run fails at the manifest-validation step with `dependency component-ontology does not specify a version`. This is the **expected pre-publish state** — workspace path-deps (whether at workspace-root `[workspace.dependencies]` or in per-crate `Cargo.toml` files like `atlas-index/Cargo.toml:13`) intentionally carry **path only, no version**, matching the atlas-contracts upstream convention. The publish-time mechanism (cargo-release rewriting, just-in-time `version` injection, or equivalent) injects the version externally to source so the source tree never carries an out-of-band version anchor. (Follow-up commit `98ee68d` initially added `version = "0.1.0"` here; reverted in commit `c784050` per user direction.)

Phase 5 design spec §6.4 PR-1 acceptance gate: `component-ontology` dry-run clean as required; `atlas-index` dry-run failure is the convention-honouring pre-publish state, not a regression.

**Atlas root `release.toml`:** Existed pre-fold (plan said it didn't); preserved its existing `cargo-release` config (push=false, tag-name conventions) and just augmented the `publish = false` comment to reference the new per-crate overrides. Minor deviation from plan step 1.8 (edit instead of create); content-equivalent.

**Cargo.lock:** Updated as a side effect of adding workspace members (records `tempfile`/`regex` dev-deps for the schema crates). Staged and committed alongside `Cargo.toml`.

### PR-2
2026-05-10 — Commit: `24a4c6a` on main. All cargo gates clean: `cargo build --workspace`, `cargo test --workspace --release --no-fail-fast` (0 failures), `cargo clippy --all-targets -- -D warnings` (0), `cargo fmt --check` (0), `cargo build --release --workspace`, polyglot smoke test (`phase3_polyglot_fixture` ok in 94.74s — strict cold/warm-call assertions held).

**`--additional-root` clap error text:**
```
error: unexpected argument '--additional-root' found

  tip: to pass '--additional-root' as a value, use '-- --additional-root'

Usage: atlas index <ROOT>
```

**Scope-bleed — files deleted in PR-2:**
- `crates/atlas-engine/tests/multi_root_path_deps.rs` (742 LOC): directly imported `expand_roots` + `expand_roots_with_warnings` — blocked test compilation.
- `crates/atlas-engine/tests/l5_elixir_surface.rs` (291 LOC): imported and called `expand_roots` in two test functions — blocked test compilation.
- `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` (593 LOC): was scheduled for PR-4 deletion; pulled forward because the pipeline no longer auto-discovers the peer root via `expand_roots`, so the two-root fixture broke at runtime (unresolved contract participant error).
- `crates/atlas-cli/tests/l6_participant_surface_sha.rs`: `cross_tree_cache_invalidates_on_peer_root_serde_struct_edit` test function deleted (relied on peer-root discovery); `write_plain_crate_with_path_dep` helper removed (now dead code). The remaining two test functions still pass.

PR-4 now becomes purely adding `contract_edge_in_workspace.rs` (the salvaged single-root replacement for the deleted AC#1–5 tests) plus deleting `crates/atlas-engine/tests/multi_root.rs`.

**Follow-up (commit `bc10a9c`):** Spec-compliance review identified two over-deletions: (1) the elixir-surface coverage tests in `l5_elixir_surface.rs` were lost when the file was deleted whole-file; restored as a surgical edit with only the two `expand_roots`-using tests removed. (2) Three stale `root_expansion`/`expand_roots` doc references in `manifest_parse.rs:47`, `manifest_parse.rs:377`, and `manifest_patterns.rs:21` were retexted (plan step 2.10 sweep was incomplete in the original PR-2 commit, missing files outside `pipeline.rs`). All cargo gates re-verified clean post-fix; polyglot smoke test still green.

**Follow-up (commit `f5b4148`):** Code-quality review surfaced four prose artefacts that survived PR-2's deletion sweep and now misdescribed the post-PR-2/PR-3 single-root system. Renamed `persist_discovered_roots` → `persist_workspace_root` and retexted `build_engine_database` doc, `manifest_patterns.rs:20` comment, and `manifest_parse.rs:375–383` doc. No behaviour change; cargo + polyglot smoke test re-verified clean.

### PR-3
2026-05-10 — Commit: `f6caa18` on main; status checkbox + scope-bleed note in `8e7c6c0`. `Workspace.roots: Vec<PathBuf>` → `Workspace.root: PathBuf`; deleted `roots.rs` (61 LOC) + `best_root_for`; rewrote ~30 call sites across L2/L3/L4/L5/L6/L8/L9. Net diff: −622 LOC. All cargo gates clean: build, tests (workspace + polyglot smoke), clippy, fmt.

**Scope-bleed — tests deleted in PR-3:**
- `crates/atlas-engine/tests/multi_root.rs` (154 LOC): authorised by plan §3.19 — referenced `Workspace.roots` plurally, no compile path post-collapse.
- One test (`peer_root_with_empty_segment_does_not_phantom_emit_primary_subdirs`) from `crates/atlas-engine/tests/l7_l8_fixedpoint.rs` and one inline test (`empty_segment_with_no_manifests_does_not_phantom_emit_primary_subdirs`) from `crates/atlas-engine/src/l8_recurse.rs`: not in plan, pulled forward (referenced `Workspace.roots` plurally; would have blocked compilation).

**Audit grep (plan §3.21):** `git grep -E 'multi.root|multi-root|workspace\.roots' crates/atlas-engine/src/ crates/atlas-cli/src/` returns 6 surviving "multi-root" hits in source-tree doc comments (post-Issue-1 fix). Each is either `#[allow(dead_code)]`-annotated forward-compat scaffolding (`l4_tree.rs:382` `owning_root` field, `l4_tree.rs:567` reserved `roots` parameter, `l4_tree.rs:795` `scoping_prefixes` field) or explanatory historical context inside live code (`l8_recurse.rs:64`/`:95`/`:278` rationales for absolute-path storage). All retained intentionally; none describe currently-active multi-root behaviour.

**Follow-up (commit `d2e8381`):** Code-quality review surfaced three cheap cleanups: retexted the misleading "Multi-root: each path segment..." comment in `l8_recurse.rs:~443`, the stale "covers all roots" prose at `db.rs:12`, and removed a vestigial brace scope in `l9_projections.rs:~310`. Two findings deferred: I-1 (l6_paths.rs:51 dead branch — pre-existing) and I-2 (build_engine_database public API still Vec<PathBuf> — plan did not authorise return-type collapse).

**PR-4 scope clarification:** `multi_root.rs` was already deleted in PR-3, so PR-4 is now purely adding `contract_edge_in_workspace.rs` (the salvaged single-root replacement for the AC#1–5 contract-edge tests deleted in PR-2 + tests deleted in PR-3).

### PR-4
2026-05-10 — Commit: `62517d9` on main. New file `crates/atlas-cli/tests/contract_edge_in_workspace.rs` (554 LOC). Single-root rewrite of `atlas_contracts_in_ravel_lite.rs` (593 LOC; deleted in PR-2 scope-bleed). All cargo gates clean: `cargo build --workspace`, `cargo test --workspace --release --no-fail-fast` (0 failures), `cargo clippy --all-targets -- -D warnings` (0), `cargo fmt --check` (0), `cargo build --release --workspace`, polyglot smoke test (`phase3_polyglot_fixture` ok in 90.51s).

**Key fixture change:** The original fixture created two tempdirs (one per root) connected by a cross-directory path-dep. The new fixture creates one tempdir with two sibling crates (`consumer/` and `schema-crate/`) directly under the root. No workspace-level `Cargo.toml` is written at the root — adding one would cause the cargo-classifier to emit a `kind: workspace` component for the tempdir itself (with a random tempdir-name ID), which would break the contract-id assertion.

**AC#1–5 mapping table:**

| AC | Original assertion (lines in `atlas_contracts_in_ravel_lite.rs`) | New assertion (lines in `contract_edge_in_workspace.rs`) | Single-root translation notes |
|----|------------------------------------------------------------------|----------------------------------------------------------|-------------------------------|
| 1 | Lines 294–364: `components.yaml` must list both `consumer-crate` and `atlas-contracts` components; additionally asserts both dirs appear in `components.yaml.roots` (multi-root: two root entries). | Lines 285–328: `components.yaml` must list both `consumer` and `schema-crate` components. | **`roots` assertion dropped**: in single-root mode there is only one root (the tempdir itself). Asserting that both component IDs appear in `components` is the correct single-root equivalent; the dual-root `roots[]` assertion has no analogue and is explicitly not preserved (not redundant — different semantics; dropped because the invariant it tested no longer exists). |
| 2 | Lines 366–417: `related-components.yaml` carries a `consumes-contract` edge from `consumer-crate` to a contract under `atlas-contracts/`. | Lines 331–381: `related-components.yaml` carries a `consumes-contract` edge from `consumer` to a contract under `schema-crate/`. | Component IDs changed: `consumer-crate` → `consumer`, `atlas-contracts` → `schema-crate`. Assertion structure identical. |
| 3 | Lines 419–468: `atlas-contracts/.atlas/cache/surfaces.yaml` lists the contract under `contracts_defined`; asserts `kind == DataFormat` and `definition_binding.symbol == "Foo"`. | Lines 383–430: `schema-crate/.atlas/cache/surfaces.yaml` lists the contract under `contracts_defined`; asserts `kind == DataFormat` and `definition_binding.symbol == "Foo"`. | Path change: `parent.path().join("atlas-contracts/.atlas/cache/surfaces.yaml")` → `tmp.path().join("schema-crate/.atlas/cache/surfaces.yaml")`. All assertion fields preserved. |
| 4 | Lines 498–512: no-op re-run makes zero LLM calls; persistent cache root exists. | Lines 459–474: no-op re-run makes zero LLM calls; persistent cache root exists. | Identical assertion structure; component IDs not referenced directly in this AC. |
| 5 | Lines 515–592: after editing `atlas-contracts/src/lib.rs`: (a) `Stage2Edges` called ≥1 (L6 batch miss); (b) `surface_calls_for(CONTRACTS_ID) >= 1` (schema crate L5 miss); (c) `surface_calls_for(CONSUMER_ID) == 0` (consumer L5 hit); (d) `Classify == 0`; (e) `edited.total() < cold_total`. | Lines 476–554: after editing `schema-crate/src/lib.rs`: identical five assertions with `SCHEMA_ID` / `CONSUMER_ID` / `edited` substituted. | Full assertion set preserved. Only the path and constant names changed. |

### PR-5
2026-05-10 — Commit: `57fb124` on main (cherry-picked from worktree branch `phase5-pr5` commit `1d794eb`). Docs-only retext of canonical system-model design.

- `2026-05-06-atlas-system-model-design.md`: §5.3 "Multi-root workspace" deleted; §5.4→§5.3, §5.5→§5.4, §5.6→§5.5 renumbered; §10.5 marked SHIPPED 2026-05-10 with full §7 retext (literal `<PR-6-COMMIT-SHA>` placeholder); §10.1 multi-root architectural-seam bullet deleted (section kept — multi-root was one of several bullets, not the only); glossary "Multi-root workspace" entry deleted.
- §6.5 (line 748) had a live functional reference to "multi-root workspace" that wasn't in the plan; agent retexted to "single-root workspace" since it described current behaviour, not historical Phase 1 design. Out-of-plan but in-spirit; flagged for spec review.
- Override-scoping spec edits actually spanned lines 14–20 / 173–176 / 249–252 (slightly shifted from the plan's nominal 19/177/253 estimates).
- Historical references retained: §10.1 goal text, §10.5 SHIPPED text, §11.1 decision table ("Multi-root over federation"), and the Federation glossary entry. Specs are time-snapshots.

`<PR-6-COMMIT-SHA>` placeholder in §10.5 to be backfilled by a follow-up commit after PR-6 lands.

**Follow-up (commit `aa4c646`):** Code-quality review surfaced four editorial issues: (Critical) §8.3's cross-reference `§5.5` was updated to `§5.4` post-renumber — the renumber sweep missed this internal back-reference. (Important) Federation glossary entry retexted to anchor the historical chain (multi-root workspace is itself retired). (Important) §10.1 Goal line: option A applied — `[retired Phase 5]` bracketing note added inline; the canonical design is a living document so flagging retired concepts inline is the correct editorial register. (Minor) §6.5 line 748 phrasing: `within the single-root workspace` qualifier dropped — redundant given surrounding context.
