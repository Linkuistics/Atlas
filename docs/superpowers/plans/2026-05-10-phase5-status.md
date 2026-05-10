# Atlas vNext Phase 5 — Status

Companion to `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-10-vnext-continue.md` (Phase-5-shaped) reads this file (via the `*phase5-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-10 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [x] PR-1 — Fold A: atlas-contracts in-tree (structural)
- [x] PR-2 — Drop discovery (deletion + CLI surface change)
- [x] PR-3 — Singularise `Workspace` (type + call-site refactor)
- [ ] PR-4 — Salvage tests (test suite surgery)
- [ ] PR-5 — Retext canonical system-model design (docs only)
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

**PR-3 (commit `f6caa18`, 2026-05-10):** `Workspace.roots: Vec<PathBuf>` → `Workspace.root: PathBuf`; deleted `roots.rs` (61 LOC) + `best_root_for`; rewrote ~30 call sites across L2/L3/L4/L5/L6/L8/L9; deleted scope-bleed tests (`multi_root.rs`, two in `l7_l8_fixedpoint.rs`, one inline in `l8_recurse.rs`). PR-4 scope note: `multi_root.rs` was already deleted in this PR, so PR-4 is purely adding `contract_edge_in_workspace.rs`.
