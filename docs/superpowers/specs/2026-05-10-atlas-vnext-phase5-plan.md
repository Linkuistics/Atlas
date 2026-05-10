# Atlas vNext Phase 5 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The Phase 5 status file at `docs/superpowers/plans/2026-05-10-phase5-status.md` carries the per-PR checkbox state across sessions.

**Goal:** Retire Atlas's multi-root architectural seam by folding `atlas-contracts` into the Atlas workspace and collapsing every `Vec<PathBuf>` root to a singular `PathBuf` root, end-to-end, with no user-facing capability change and no LLM-call-budget regression.

**Architecture:** Phase 5 is a *deletion-shaped consolidation release*. Seven PRs (PR-0 plan + PR-1 fold + PR-2 drop discovery + PR-3 singularise Workspace + PR-4 salvage tests + PR-5 retext canonical design + PR-6 acceptance). Net: ~−600 LOC production code, ~−1,000 LOC test code, ~−30 lines design prose. The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard — every Phase 5 PR re-runs it before flipping its status checkbox. Cold polyglot LLM-call count must remain at the Phase 2 PR-14 baseline (~26 calls); warm + reports = 0.

**Tech Stack:** Rust workspace (Atlas + the freshly-folded `component-ontology` and `atlas-index` schema crates); Salsa engine (`Workspace` Salsa input collapses from `roots: Vec<PathBuf>` to `root: PathBuf`); existing `atlas-cli` / `atlas-engine` / `atlas-analyzers` / `atlas-reports` crates edited in place; cross-repo coordination with `~/Development/Ravel-Lite/Cargo.toml` (path-dep rewrite, separate-repo commit). No new crates introduced.

---

## 0. Reading order

Before this plan, read:

1. `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md` end-to-end. The design spec is the canonical source of scope, PR boundaries, acceptance criteria, post-Phase-5 architecture, and the §7 canonical design.md retext that PR-5 lands verbatim. **This plan operationalises that design; where the two disagree, the design spec wins.**
2. `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-plan.md` §0 (reading order) and §1 (deliverable, restated) to see the prior-phase plan structure this plan follows; and `docs/superpowers/plans/2026-05-09-phase4-status.md` for the status-file shape PR-0 reproduces.
3. `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` §5.3 (Multi-root workspace; deleted in PR-5), §10 (current state: §10.1–§10.10), §10.5 (Phase 5 entry that PR-5 marks SHIPPED), and the glossary entry "Multi-root workspace" (line ~1450).
4. `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` lines 19, 177, 253 (three §5.3 references retexted in PR-5).
5. Memory entries that constrain Phase 5:
   - `feedback_user_low_git_history_value` — the atlas-contracts fold is a plain snapshot copy, single import commit. No `git subtree` or `git filter-repo`.
   - `feedback_no_iterator_stubs_for_singletons` — `Workspace.root: PathBuf` with no iterator/slice accessor; no `roots()` method that returns a length-1 slice.
   - `feedback_no_tail_pipe_for_long_tests` — never `tail`-pipe `cargo test` invocations of the polyglot smoke test; let stdout pass through.
   - `feedback_release_workspace_build_for_polyglot` — before any release polyglot test, `cargo build --release --workspace` first; standalone analyser `[[bin]]` targets are not built by `cargo test --workspace --release` alone.
   - `project_phase5_split_and_ravel_bazel` — Phase 5 scope is A + C only; folding Ravel + Ravel-Lite is a deferred later phase, possibly tied to Bazel.
   - `feedback_worktree_base_verification` — when dispatching parallel subagents via `isolation:"worktree"`, verify each worktree's base commit matches current main before the subagent proceeds.

This plan does *not* re-derive scope; it sequences and grounds it. The PR boundaries, acceptance criteria, and design retext are all in the design spec.

---

## 1. Phase 5 deliverable, restated

End of Phase 5, the Atlas codebase shall exhibit the following properties without changing any user-observable behaviour beyond the removal of the never-used `--additional-root` CLI flag:

- **Schema crates in-tree.** `crates/component-ontology/` and `crates/atlas-index/` are workspace members of Atlas. They retain independent `0.1.0` versioning and continue publishing to crates.io from inside Atlas via per-crate `release.toml` overrides. The `[workspace.dependencies]` lines that previously read `path = "../atlas-contracts/crates/..."` now read `path = "crates/..."`.
- **Ravel-Lite path-deps updated.** `~/Development/Ravel-Lite/Cargo.toml` lines 51 + 56 read `path = "../Atlas/crates/{component-ontology,atlas-index}"`. Commit lands in Ravel-Lite's main branch immediately after Atlas PR-1.
- **Multi-root machinery deleted.** `crates/atlas-engine/src/root_expansion.rs`, `crates/atlas-engine/src/roots.rs`, `crates/atlas-engine/tests/multi_root.rs`, `crates/atlas-engine/tests/multi_root_path_deps.rs`, and the original `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` do not exist. The `--additional-root` CLI flag, `IndexConfig.additional_roots: Vec<PathBuf>`, `IndexConfig::all_roots()`, `Workspace::primary_root()`, `expand_roots`, `expand_roots_with_warnings`, and `best_root_for` are gone.
- **Workspace Salsa input is singular.** `Workspace.root: PathBuf` (no `roots`, no `Vec<PathBuf>`, no plural accessor). Every layer that needs path-relativisation calls `path.strip_prefix(workspace.root(db))` directly.
- **Salvaged contract-edge test green.** `crates/atlas-cli/tests/contract_edge_in_workspace.rs` exercises the full Phase 1 PR-12 acceptance flow against a single-root two-crate fixture (consumer + schema-crate stand-in). AC#1–5 of the deleted `atlas_contracts_in_ravel_lite.rs` map verbatim into assertions in the new test.
- **Canonical design retexted.** `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` §5.3 deleted, §5.4–§5.6 renumbered to §5.3–§5.5, §10 marks Phase 5 SHIPPED with PR-6's commit SHA, §10.1 multi-root architectural seam callout deleted, glossary "Multi-root workspace" entry deleted. `2026-05-06-override-scoping-scattered-atlas.md` lines 19, 177, 253 retexted to describe single-root layout.
- **Audit greps clean.** `git grep -E 'multi.root|multi-root|additional_root|expand_roots|best_root_for' crates/` returns zero non-test, non-deleted-file hits at PR-6 close. Surviving prose may exist only in *historical* spec/status files (Phases 1–4); those are intentionally left as-is per the convention "specs are time-snapshots, not living documents."
- **Cumulative regression guard green.** `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` is green at every PR boundary (PR-1 through PR-6), with cold-call count at the Phase 2 PR-14 baseline (~26) and warm-call count at 0.

---

## 2. File structure (what each PR touches)

This map locks the decomposition. Each task below produces self-contained changes that make sense independently and pass the cumulative regression guard on landing.

### Files **created** in Phase 5

| Path | PR | Purpose |
|---|---|---|
| `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md` | PR-0 | This plan. |
| `docs/superpowers/plans/2026-05-10-phase5-status.md` | PR-0 | Per-PR checkbox status file. |
| `docs/superpowers/prompts/2026-05-10-vnext-continue.md` | PR-0 | Cross-machine session-resume prompt (Phase-5-shaped). |
| `crates/component-ontology/` (entire subtree, snapshot copy) | PR-1 | Schema crate folded in-tree. |
| `crates/atlas-index/` (entire subtree, snapshot copy) | PR-1 | Schema crate folded in-tree. |
| `crates/component-ontology/release.toml` | PR-1 | Per-crate override so workspace `cargo release` skips it. |
| `crates/atlas-index/release.toml` | PR-1 | Per-crate override so workspace `cargo release` skips it. |
| `crates/atlas-cli/tests/contract_edge_in_workspace.rs` | PR-4 | Salvaged single-root rewrite of `atlas_contracts_in_ravel_lite.rs`. |

### Files **modified** in Phase 5

| Path | PR | Change |
|---|---|---|
| `Cargo.toml` (workspace root) | PR-1 | Add two `members`; rewrite `[workspace.dependencies]` paths; drop the cross-repo comment block. |
| `release.toml` (workspace root) | PR-1 | Document deferral to per-crate overrides for the two schema crates. |
| `website/` | PR-1 | Atlas-contracts website content relocated under `website/docs/schema/` (or PR-1-proposed alternative). |
| `defaults/` | PR-1 | Fold any unique content from `~/Development/atlas-contracts/defaults/`. |
| `~/Development/Ravel-Lite/Cargo.toml` | PR-1 (separate repo) | Rewrite path-dep lines 51 + 56 to point at `../Atlas/crates/...`. |
| `crates/atlas-cli/src/main.rs` | PR-2 | Delete `additional_roots` field + `--additional-root` arg + canonicalisation block. |
| `crates/atlas-cli/src/pipeline.rs` | PR-2 | Delete `IndexConfig.additional_roots`, `IndexConfig::all_roots()`, the two `manual_iter` chains, and update doc comments. |
| `crates/atlas-engine/src/lib.rs` | PR-2 + PR-3 | Delete `mod root_expansion;`, `mod roots;`, the `pub use` re-exports of `expand_roots`, `expand_roots_with_warnings`, `best_root_for`. |
| `crates/atlas-engine/src/db.rs` | PR-3 | `Workspace.roots: Vec<PathBuf>` → `root: PathBuf`; delete `Workspace::primary_root()`; update `AtlasDatabase::new` and `from_workspace_input` signatures. |
| `crates/atlas-engine/src/l2_candidates.rs` | PR-3 | Rewrite `workspace.roots(db)` site at line 66. |
| `crates/atlas-engine/src/l3_classify.rs` | PR-3 | Rewrite two sites at lines 117 + 131 + 941; remove `use crate::roots::best_root_for`. |
| `crates/atlas-engine/src/l4_tree.rs` | PR-3 | Rewrite three sites at lines 149 + 195 + 299 + 1037. |
| `crates/atlas-engine/src/l5_surface.rs` | PR-3 | Rewrite ~10 sites at lines 274 + 282 + 285 + 432 + 535 + 609 + 624 + 692 + 792 + 1138 + 1224 + 1230 + 1315 + 1321 + 1480 + 1552 + 1558; remove `use crate::roots::best_root_for`. |
| `crates/atlas-engine/src/l6_compose_edges.rs` | PR-3 | Rewrite site at line 72. |
| `crates/atlas-engine/src/l6_composition.rs` | PR-3 | Rewrite site at line 83. |
| `crates/atlas-engine/src/l6_paths.rs` | PR-3 | Rewrite site at line 53; remove `use crate::roots::best_root_for`. |
| `crates/atlas-engine/src/l8_recurse.rs` | PR-3 | Rewrite sites at lines 252 + 293; remove `use crate::roots::best_root_for`. |
| `crates/atlas-engine/src/l9_projections.rs` | PR-3 | Rewrite sites at lines 51 + 200 + 325 + 415 + 417 + 436; remove `use crate::roots::best_root_for`. |
| `crates/atlas-engine/tests/l7_l8_fixedpoint.rs` | PR-3 | Comment update at line 86. |
| `crates/atlas-cli/tests/l6_participant_surface_sha.rs` | PR-3 | Update stale `expand_roots` comment at line 287. |
| `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` | PR-5 | Delete §5.3, renumber §5.4→§5.3 etc., mark Phase 5 SHIPPED in §10, delete §10.1 multi-root callout, delete glossary entry. |
| `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` | PR-5 | Retext lines 19, 177, 253. |
| `docs/superpowers/plans/2026-05-10-phase5-status.md` | PR-1..PR-6 | Flip per-PR checkboxes; PR-6 lands closeout note + Upgrade notes. |
| `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase4_plus_roadmap.md` | PR-6 | Mark Phase 5 SHIPPED; advance Phase 6 to next-up. |
| `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_monorepo_consolidation.md` | PR-6 | Mark "atlas-contracts in-tree" complete; remaining Ravel/Ravel-Lite fold deferred. |
| `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase5_split_and_ravel_bazel.md` | PR-6 | Update with shipped state. |

### Files **deleted** in Phase 5

| Path | PR | LOC |
|---|---|---|
| `crates/atlas-engine/src/root_expansion.rs` | PR-2 | 469 |
| `crates/atlas-engine/src/roots.rs` | PR-3 | 61 |
| `crates/atlas-engine/tests/multi_root.rs` | PR-4 | 154 |
| `crates/atlas-engine/tests/multi_root_path_deps.rs` | PR-4 | 742 |
| `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` | PR-4 | 593 |

---

## 3. Dependency graph

```
PR-0 (plan + status + continuation prompt)
  │
  ▼
PR-1 (atlas-contracts fold) ────────┐
  │                                  │
  ▼                                  │
PR-2 (drop discovery)                │
  │                                  │
  ▼                                  │
PR-3 (singularise Workspace)         │
  │                                  │
  ▼                                  │
PR-4 (salvage tests) ────────────────┤
                                     │
PR-5 (retext canonical design) ──────┤  (parallel-safe with PR-2/3/4 — docs-only, disjoint surface)
                                     │
                                     ▼
                            PR-6 (acceptance + closeout)
```

**Parallel dispatch opportunities:**

- **PR-5 is parallel-safe with PR-2 / PR-3 / PR-4.** It edits only `docs/superpowers/specs/*.md`; the code-side PRs do not touch those files. PR-5 may be dispatched concurrently with the deletion sequence to compress the phase. PR-5 still must land *before* PR-6 (PR-6's audit greps include the canonical design spec, but only crate-tree hits gate; design-spec retext is informational at PR-6).
- **PR-2 and PR-3 are sequentially ordered.** PR-3 collapses the type whose call sites PR-2 stops *manually populating*. Inverting the order leaves PR-2 unable to compile against an already-singular `Workspace.root`.
- **PR-4 follows PR-3.** PR-4 deletes the three multi-root tests; once PR-3 lands the tests cannot compile (they reference the deleted `expand_roots`, `Workspace.roots`, etc.). PR-4 also writes the salvaged single-root replacement, which depends on the post-PR-3 singular API.
- **PR-1 must precede PR-2/3/4** because the schema crates live in-tree before any `Cargo.toml` consumer can reference them as workspace members. (In practice nothing in PR-2/3/4 references the schema crates directly — they're indirectly used through `[workspace.dependencies]` — but PR-1's coordinated Ravel-Lite commit is a phase-defining step that happens early.)

---

## 4. Tasks

### Task 0: PR-0 — Plan + status + continuation prompt *(docs only)*

**Files:**
- Create: `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md` (this file)
- Create: `docs/superpowers/plans/2026-05-10-phase5-status.md`
- Create: `docs/superpowers/prompts/2026-05-10-vnext-continue.md`

- [ ] **Step 0.1: Verify clean working tree on Phase 5 worktree branch**

```bash
git status
git log --oneline -5
```

Expected: clean working tree; HEAD on a Phase 5 worktree branch (likely `phase5-pr0` or similar) branched from current `main` (`579a809` or later).

- [ ] **Step 0.2: This plan file already exists from this writing-plans session — skip recreation**

The plan you are reading lives at `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md`. PR-0 includes it as one of three deliverables. The remaining two — status file and continuation prompt — are written next.

- [ ] **Step 0.3: Create the Phase 5 status file**

Write `docs/superpowers/plans/2026-05-10-phase5-status.md`:

```markdown
# Atlas vNext Phase 5 — Status

Companion to `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-10-vnext-continue.md` (Phase-5-shaped) reads this file (via the `*phase5-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-10 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [ ] PR-1 — Fold A: atlas-contracts in-tree (structural)
- [ ] PR-2 — Drop discovery (deletion + CLI surface change)
- [ ] PR-3 — Singularise `Workspace` (type + call-site refactor)
- [ ] PR-4 — Salvage tests (test suite surgery)
- [ ] PR-5 — Retext canonical system-model design (docs only)
- [ ] PR-6 — Acceptance + closeout (verification only)

When every box is `[x]`, Phase 5 is complete and the continuation prompt should report success and route to the Phase 6 brainstorm question (per validated roadmap; Phase 6 = user-facing schema cleanups).

## Dependency graph (informational; canonical in plan §3)

```
PR-0 ──► PR-1 ──► PR-2 ──► PR-3 ──► PR-4 ──► PR-6
                                              ▲
                              PR-5 ───────────┘  (docs-only; parallel-safe with PR-2/3/4)
```

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; cross-repo coordination outcomes (Ravel-Lite path-edit commit sha); manual verification steps that succeeded; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0
2026-05-10 — Landed: the Phase 5 plan, this status file, and the continuation prompt. Worktree branch: `phase5-pr0`. Commit: `<sha>`.
```

- [ ] **Step 0.4: Create the continuation prompt (Phase-5-shaped)**

Write `docs/superpowers/prompts/2026-05-10-vnext-continue.md`. Model it on `docs/superpowers/prompts/2026-05-09-vnext-continue.md` (Phase 4) verbatim, with these substitutions:

- Replace every "Phase 4" reference with "Phase 5" except the historical-context paragraph at the top, which records that Phase 4 is complete.
- The fenced block lists Phases 1–4 as complete with their status-file paths, then reads the Phase 5 design (`2026-05-10-atlas-vnext-phase5-design.md`), plan (`2026-05-10-atlas-vnext-phase5-plan.md`), and status (`2026-05-10-phase5-status.md`).
- The "next-up brainstorm question" routes to Phase 6 (user-facing schema cleanups) per `project_phase4_plus_roadmap`.
- The "supersedes" line at the top points at `docs/superpowers/prompts/2026-05-09-vnext-continue.md`.

The exact text-substitution recipe means a copy-and-edit of the Phase 4 prompt; do not handcraft from scratch.

- [ ] **Step 0.5: Run `cargo build --workspace` and confirm clean (sanity baseline)**

```bash
cargo build --workspace
```

Expected: clean build. PR-0 changes no code; this is a pre-flight to confirm the worktree is in a known-good state before later PRs.

- [ ] **Step 0.6: Commit PR-0**

```bash
git add docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md \
        docs/superpowers/plans/2026-05-10-phase5-status.md \
        docs/superpowers/prompts/2026-05-10-vnext-continue.md
git commit -m "$(cat <<'EOF'
phase5: PR-0 plan + status + continuation prompt

No code changes. Lays the per-PR scaffolding for the
multi-root retirement work (PR-1 through PR-6). Companion
to 2026-05-10-atlas-vnext-phase5-design.md (on main as f80e179).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit on the Phase 5 worktree branch.

---

### Task 1: PR-1 — Fold A: atlas-contracts in-tree *(structural)*

**Files:**
- Create: `crates/component-ontology/` (snapshot copy from `~/Development/atlas-contracts/crates/component-ontology/`)
- Create: `crates/atlas-index/` (snapshot copy from `~/Development/atlas-contracts/crates/atlas-index/`)
- Create: `crates/component-ontology/release.toml`
- Create: `crates/atlas-index/release.toml`
- Modify: `Cargo.toml:1-60` (workspace root)
- Modify: `release.toml` (workspace root, if present; create otherwise)
- Modify: `website/` (relocate atlas-contracts website content)
- Modify: `defaults/` (fold unique content from atlas-contracts)
- **Cross-repo:** `~/Development/Ravel-Lite/Cargo.toml:51,56`

**Pre-flight constraint:** PR-1 cannot land until `~/Development/Ravel-Lite/` is also ready for the path-edit commit. PR-1's own commit on Atlas main and the Ravel-Lite path-edit commit must land as a coordinated pair *in this order* (Atlas first, Ravel-Lite immediately after) — never invert. See risk R1 in the design spec §5.

- [ ] **Step 1.1: Snapshot copy `component-ontology` into Atlas**

```bash
cp -r ~/Development/atlas-contracts/crates/component-ontology /Users/antony/Development/Atlas/crates/component-ontology
```

Then verify:

```bash
ls /Users/antony/Development/Atlas/crates/component-ontology/
```

Expected: `Cargo.toml`, `src/`, optionally `tests/`. No `target/`, no `Cargo.lock`. If `target/` was copied, `rm -rf` it.

- [ ] **Step 1.2: Snapshot copy `atlas-index` into Atlas**

```bash
cp -r ~/Development/atlas-contracts/crates/atlas-index /Users/antony/Development/Atlas/crates/atlas-index
ls /Users/antony/Development/Atlas/crates/atlas-index/
```

Expected: `Cargo.toml`, `src/`, `tests/`. No `target/`, no `Cargo.lock`.

- [ ] **Step 1.3: Add per-crate `release.toml` to `component-ontology`**

Write `crates/component-ontology/release.toml`:

```toml
# Independent versioning: this crate publishes to crates.io from
# inside the Atlas workspace, but is excluded from workspace-wide
# `cargo release` invocations driven by Atlas's top-level
# release.toml. Bump the version here and run `cargo release` from
# this crate's directory to publish.
publish = true
```

- [ ] **Step 1.4: Add per-crate `release.toml` to `atlas-index`**

Write `crates/atlas-index/release.toml`:

```toml
# Independent versioning: this crate publishes to crates.io from
# inside the Atlas workspace, but is excluded from workspace-wide
# `cargo release` invocations driven by Atlas's top-level
# release.toml. Bump the version here and run `cargo release` from
# this crate's directory to publish.
publish = true
```

- [ ] **Step 1.5: Edit Atlas workspace `Cargo.toml` — add members**

In `Cargo.toml:3-16`, the `members` array currently lists 7 entries. Add the two schema crates as new entries (preserving the trailing comma convention):

```toml
[workspace]
resolver = "2"
members = [
  "crates/atlas-analyzers",
  "crates/atlas-engine",
  "crates/atlas-llm",
  "crates/atlas-reports",
  "crates/atlas-cli",
  "crates/component-ontology",
  "crates/atlas-index",
  "crates/analyzers/python",
  "crates/analyzers/csharp",
  "crates/analyzers/dart",
  "crates/analyzers/elixir",
  "crates/analyzers/racket",
  "crates/analyzers/lispkit",
  "evaluation/harness",
]
```

- [ ] **Step 1.6: Edit Atlas workspace `Cargo.toml` — rewrite path-deps and drop comment block**

Replace `Cargo.toml:45-50`:

```toml
# Public data-format / vocabulary crates, hosted at linkuistics/atlas-contracts.
# Local path deps during development so changes in ../atlas-contracts propagate to consumers
# without a commit/push round-trip. Before publishing, flip these to a git+rev pin (the upstream
# repo must be committed and pushed first) and revert after release.
component-ontology = { path = "../atlas-contracts/crates/component-ontology" }
atlas-index = { path = "../atlas-contracts/crates/atlas-index" }
```

with:

```toml
# Public data-format / vocabulary crates, in-tree as workspace members
# (folded from atlas-contracts in Phase 5). Independent versioning at
# 0.1.0; per-crate release.toml overrides keep them out of workspace-
# wide `cargo release` invocations. Both publish to crates.io.
component-ontology = { path = "crates/component-ontology" }
atlas-index = { path = "crates/atlas-index" }
```

- [ ] **Step 1.7: Verify the schema crates inherit Atlas's `repository` metadata**

The schema crates' own `Cargo.toml`s use `repository.workspace = true`. After the fold they inherit Atlas's `workspace.package.repository = "https://github.com/linkuistics/Atlas"`. Confirm by reading both `crates/component-ontology/Cargo.toml` and `crates/atlas-index/Cargo.toml` and checking each declares `repository.workspace = true` (or, if they hardcode `repository = "https://github.com/linkuistics/atlas-contracts"`, edit the value to either `repository.workspace = true` or `"https://github.com/linkuistics/Atlas"`).

```bash
grep -n 'repository' crates/component-ontology/Cargo.toml crates/atlas-index/Cargo.toml
```

Expected: each line is either `repository.workspace = true` or `repository = "https://github.com/linkuistics/Atlas"`. Fix any line that still points at `atlas-contracts`.

- [ ] **Step 1.8: Update workspace-root `release.toml`**

If `release.toml` exists at the Atlas workspace root, ensure it carries `publish = false` (the workspace default; the two schema crates' per-crate overrides flip this back to `true`). If it does not exist, create it:

```toml
# Default for Atlas workspace members: do not publish.
# Per-crate `release.toml` files in `crates/component-ontology/` and
# `crates/atlas-index/` override this to `publish = true` for the two
# schema crates that publish to crates.io.
publish = false
```

- [ ] **Step 1.9: First build sanity check**

```bash
cargo build --workspace
```

Expected: clean build. The two new workspace members compile; consumer crates (`atlas-cli`, `atlas-engine`, etc.) still pick the schema crates up via `component-ontology = { workspace = true }` / `atlas-index = { workspace = true }` because the workspace-dependency rewrites at lines 49–50 redirect the path resolution.

If build fails on a metadata gap (description, license, repository, readme path), fix the schema-crate `Cargo.toml` and re-run before proceeding.

- [ ] **Step 1.10: Run the workspace test suite**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean. Multi-root tests (`multi_root.rs`, `multi_root_path_deps.rs`, `atlas_contracts_in_ravel_lite.rs`) still pass — they are not deleted until PR-2 + PR-4. Output is allowed to take 10+ minutes; do not pipe through `tail` (memory `feedback_no_tail_pipe_for_long_tests`).

- [ ] **Step 1.11: `cargo publish --dry-run` for `component-ontology`**

```bash
cargo publish --dry-run -p component-ontology
```

Expected: clean. Capture the full output and paste into the PR description. If a metadata gap surfaces (typically `description` missing or `license` missing), fix in `crates/component-ontology/Cargo.toml` and re-run.

- [ ] **Step 1.12: `cargo publish --dry-run` for `atlas-index`**

```bash
cargo publish --dry-run -p atlas-index
```

Expected: clean. Capture output for the PR description; fix metadata gaps if any.

- [ ] **Step 1.13: Relocate atlas-contracts website content into Atlas's docs tree**

Currently `~/Development/atlas-contracts/website/` contains `index.md` + `meta.yml`. Atlas's own `website/` already has `index.md` + `meta.yml` plus subdirectories. Two viable layouts; pick one in PR-1 and document the choice in the PR description:

  - **Option A (recommended):** Create `website/docs/schema/` and copy atlas-contracts's `index.md` + `meta.yml` there as schema-crate docs. Add a navigation entry in Atlas's existing `meta.yml` that links to the new section.
  - **Option B:** If atlas-contracts's `index.md` content is short enough, inline it as a section into an existing Atlas docs page; delete the standalone files.

```bash
mkdir -p /Users/antony/Development/Atlas/website/docs/schema/
cp ~/Development/atlas-contracts/website/index.md /Users/antony/Development/Atlas/website/docs/schema/index.md
cp ~/Development/atlas-contracts/website/meta.yml /Users/antony/Development/Atlas/website/docs/schema/meta.yml
```

Then update Atlas's top-level `website/meta.yml` to include a nav link to `docs/schema/`.

- [ ] **Step 1.14: Diff `defaults/` and fold unique content**

```bash
diff -r ~/Development/atlas-contracts/defaults/ /Users/antony/Development/Atlas/defaults/
```

`atlas-contracts/defaults/` contains only `ontology.yaml`. Atlas's `defaults/` contains `component-kinds.md`, `component-kinds.yaml`, `prompts/`. If `ontology.yaml` is genuinely absent on the Atlas side, copy it across:

```bash
cp ~/Development/atlas-contracts/defaults/ontology.yaml /Users/antony/Development/Atlas/defaults/ontology.yaml
```

Document in the PR description what was unique vs. what was already present in identical form.

- [ ] **Step 1.15: Run the polyglot smoke test**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. `cargo build --release --workspace` precedes the test invocation per memory `feedback_release_workspace_build_for_polyglot` — `cargo test --workspace --release` does not build standalone analyser `[[bin]]` targets that the polyglot test discovers via runtime path lookup. Cold-call count = Phase 2 PR-14 baseline (~26); warm-call count = 0.

- [ ] **Step 1.16: Commit Atlas PR-1**

```bash
git add crates/component-ontology crates/atlas-index \
        Cargo.toml release.toml \
        website/docs/schema/ website/meta.yml \
        defaults/
git commit -m "$(cat <<'EOF'
phase5: PR-1 fold atlas-contracts in-tree as workspace members

Snapshot-copy crates/component-ontology and crates/atlas-index from
~/Development/atlas-contracts/ into Atlas/crates/. Add both to
[workspace] members; rewrite [workspace.dependencies] paths from
"../atlas-contracts/crates/..." to "crates/...". Per-crate
release.toml overrides keep them out of workspace-wide cargo release
invocations; both publish independently to crates.io at 0.1.0.
Website content relocated to website/docs/schema/. defaults/
ontology.yaml folded.

Phase 5 design spec: docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md §3.2.
Pairs with a coordinated commit in ~/Development/Ravel-Lite/ that
rewrites its path-deps from ../atlas-contracts/crates/ to
../Atlas/crates/.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 1.17: Coordinated cross-repo commit — Ravel-Lite path-edit**

Edit `~/Development/Ravel-Lite/Cargo.toml`:

```toml
# Lines 51 + 56, before:
component-ontology = { path = "../atlas-contracts/crates/component-ontology" }
# ... (line 56)
atlas-index = { path = "../atlas-contracts/crates/atlas-index" }
```

Replace both with:

```toml
component-ontology = { path = "../Atlas/crates/component-ontology" }
# ... (line 56)
atlas-index = { path = "../Atlas/crates/atlas-index" }
```

Also update the surrounding comment blocks (currently around lines 47–55 documenting the atlas-contracts path-dep convention) to describe the in-tree fold.

- [ ] **Step 1.18: Verify Ravel-Lite local build is clean**

```bash
cd ~/Development/Ravel-Lite/
cargo build
cd /Users/antony/Development/Atlas
```

Expected: clean. If the build fails because Ravel-Lite's checkout is on a feature branch with diverged dependencies, capture the error in the PR description and confirm with the user before proceeding.

- [ ] **Step 1.19: Commit the Ravel-Lite path-edit**

```bash
cd ~/Development/Ravel-Lite/
git add Cargo.toml
git commit -m "$(cat <<'EOF'
deps: redirect atlas-contracts path-deps at Atlas in-tree fold

component-ontology and atlas-index were folded into the Atlas
workspace as part of Atlas Phase 5 (monorepo consolidation, part 1).
Path-deps in this Cargo.toml now point at ../Atlas/crates/...
instead of the now-archived ../atlas-contracts/crates/... .

Coordinated with Atlas commit <ATLAS-PR-1-SHA>.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
cd /Users/antony/Development/Atlas
```

- [ ] **Step 1.20: Update Phase 5 status file**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`:

- Flip PR-1 checkbox from `[ ]` to `[x]`.
- Append under "### PR-1": commit sha + Ravel-Lite commit sha + website-merge resolution chosen + dry-run outputs reference + any defaults/ delta.

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "$(cat <<'EOF'
phase5: PR-1 status checkbox + per-PR notes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: PR-2 — Drop discovery *(deletion + CLI surface change)*

**Files:**
- Delete: `crates/atlas-engine/src/root_expansion.rs` (469 LOC)
- Modify: `crates/atlas-engine/src/lib.rs:42,107` (remove `mod root_expansion;` + the `pub use` re-exports of `expand_roots` + `expand_roots_with_warnings`)
- Modify: `crates/atlas-cli/src/main.rs:67-79,231-243` (remove `additional_roots` field, `--additional-root` arg, canonicalisation block, `index_config.additional_roots = ...` assignment)
- Modify: `crates/atlas-cli/src/pipeline.rs:36,117-128,129,162,175-184,283-319,748,805-820,1145,1243` (remove `expand_roots` import, `additional_roots` field + default, `all_roots()` accessor, the two `manual_iter` chains, doc comment retext)

**Note:** PR-2 leaves `Workspace.roots: Vec<PathBuf>` intact at this PR boundary — it now always holds a length-1 vec. PR-3 collapses the type.

- [ ] **Step 2.1: Delete `root_expansion.rs`**

```bash
git rm crates/atlas-engine/src/root_expansion.rs
```

- [ ] **Step 2.2: Remove `mod root_expansion;` and the `pub use` re-export from `lib.rs`**

In `crates/atlas-engine/src/lib.rs`, delete two lines:

- Line 42: `pub mod root_expansion;`
- Line 107: `pub use root_expansion::{expand_roots, expand_roots_with_warnings};`

```bash
grep -n "root_expansion" crates/atlas-engine/src/lib.rs
```

Expected: no hits.

- [ ] **Step 2.3: Delete the `additional_roots` field on `IndexArgs` in `main.rs`**

In `crates/atlas-cli/src/main.rs:66-79`, the `IndexArgs` struct currently reads:

```rust
#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Root of the codebase to index. The first analysed root; peer
    /// roots may be added with one or more `--additional-root` flags
    /// (Phase 1 plumbing — PR-4 will populate them automatically from
    /// path-dep walking).
    root: PathBuf,

    /// Additional analysed root. May be repeated. Each path becomes a
    /// peer root in the multi-root `Workspace`; components under it
    /// land in `components.yaml` alongside the primary root's. Output
    /// is still written under the primary root only.
    #[arg(long = "additional-root")]
    additional_roots: Vec<PathBuf>,
```

Replace with:

```rust
#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Path to the workspace root. Defaults to the current directory.
    root: PathBuf,
```

- [ ] **Step 2.4: Delete the canonicalisation block + assignment in `main.rs`**

In `crates/atlas-cli/src/main.rs:231-243`, delete:

```rust
    // Canonicalise additional roots eagerly so id allocation and
    // path-relativisation see absolute paths only.
    let additional_roots: Vec<PathBuf> = args
        .additional_roots
        .into_iter()
        .map(|p| {
            p.canonicalize()
                .with_context(|| format!("failed to resolve additional-root {}", p.display()))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut index_config = atlas_cli::IndexConfig::new(root);
    index_config.additional_roots = additional_roots;
```

Replace with the single line:

```rust
    let mut index_config = atlas_cli::IndexConfig::new(root);
```

- [ ] **Step 2.5: Delete the `expand_roots` import in `pipeline.rs`**

In `crates/atlas-cli/src/pipeline.rs:34-38`, the import currently reads (split across lines):

```rust
use atlas_engine::{
    ensure_atlas_gitignore, expand_roots, external_components_yaml_snapshot,
    /* ... */
};
```

Remove `expand_roots` from the import list.

- [ ] **Step 2.6: Delete `IndexConfig.additional_roots` field**

In `crates/atlas-cli/src/pipeline.rs:115-152`, the `IndexConfig` struct currently has the multi-root doc comment block (lines 117–122) and the `additional_roots: Vec<PathBuf>` field (lines 126–129). After edit:

```rust
/// Configuration for `atlas index`. Populated from CLI flags or
/// constructed by tests.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub root: PathBuf,
    pub output_dir: PathBuf,
    pub max_depth: u32,
    /// Bound on parallel `is_component` calls inside L8's map step.
    /// Plumbed through to [`atlas_engine::FixedpointConfig::map_concurrency`].
    pub map_concurrency: usize,
    pub recarve: bool,
    pub dry_run: bool,
    pub respect_gitignore: bool,
    /// Skip loading `components.overrides.yaml` and
    /// `subsystems.overrides.yaml` from the output dir. Files on disk
    /// are untouched. The fingerprint's `backend_version` is suffixed
    /// with `+overrides=disabled` so cache entries do not bleed
    /// between with/without runs.
    pub no_overrides: bool,
    /// Per-prompt SHA map embedded into `components.yaml`'s
    /// `cache_fingerprints.prompt_shas`. Left as `None` by tests that
    /// do not care; the CLI binary fills it from the embedded prompt
    /// corpus.
    pub prompt_shas: Option<std::collections::BTreeMap<String, String>>,
    /// Fingerprint to stamp onto the workspace input. When `None`,
    /// the backend's `fingerprint()` is installed verbatim.
    pub fingerprint_override: Option<LlmFingerprint>,
}
```

(I.e., delete the multi-root doc-comment paragraph at lines 117–122 and the `additional_roots: Vec<PathBuf>` field + its preceding doc comment at lines 126–129.)

- [ ] **Step 2.7: Delete the `additional_roots: Vec::new()` field-default**

In `crates/atlas-cli/src/pipeline.rs:158-173`, `IndexConfig::new` currently reads:

```rust
    pub fn new(root: PathBuf) -> Self {
        let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
        IndexConfig {
            root,
            additional_roots: Vec::new(),
            output_dir,
            max_depth: atlas_engine::DEFAULT_MAX_DEPTH,
            /* ... */
```

After edit (drop the `additional_roots` line and adjust the leading doc comment to stop mentioning "no additional roots"):

```rust
    /// Reasonable defaults for a command-line invocation: output
    /// directory is `<root>/.atlas/`, max depth per §8.2.
    pub fn new(root: PathBuf) -> Self {
        let output_dir = root.join(DEFAULT_OUTPUT_SUBDIR);
        IndexConfig {
            root,
            output_dir,
            max_depth: atlas_engine::DEFAULT_MAX_DEPTH,
            /* ... */
```

- [ ] **Step 2.8: Delete `IndexConfig::all_roots()`**

In `crates/atlas-cli/src/pipeline.rs:174-185`, the helper currently reads:

```rust
    /// Full analysed root set, primary first. Equivalent to
    /// `[self.root.clone()] + self.additional_roots`; provided as a
    /// helper because every pipeline call site needs the same
    /// concatenation.
    pub fn all_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::with_capacity(1 + self.additional_roots.len());
        roots.push(self.root.clone());
        roots.extend(self.additional_roots.iter().cloned());
        roots
    }
```

Delete the entire method (the closing `}` at line 185 of the `impl IndexConfig` block stays).

In-crate callers of `config.all_roots()` (find them via `grep -n 'all_roots' crates/atlas-cli/src/pipeline.rs`) collapse to `vec![config.root.clone()]` for now; PR-3 collapses the type further.

- [ ] **Step 2.9: Delete the two `manual_iter` chains around `expand_roots` calls**

In `crates/atlas-cli/src/pipeline.rs`, two analogous blocks exist around lines 283–325 and 805–820. Each currently runs `expand_roots(&config.root)`, then merges with `config.additional_roots`, then iterates the union for further work. The shape (cleaned):

```rust
    let auto_expanded = expand_roots(&config.root).context("failed to expand path-dep roots")?;
    let manual_iter = config.additional_roots.iter().map(|r| (true, r));
    let auto_iter = auto_expanded.iter().map(|r| (false, r));

    for (is_manual, r) in manual_iter.chain(auto_iter) {
        /* per-root work */
    }
```

Replace each block with the single-root form:

```rust
    /* per-root work, against config.root only */
```

i.e., lift the loop body out and rebind `r` to `&config.root`. The `is_manual` flag was always unused in the second of the two blocks (`for (_is_manual, r)` at line 818); in the first block (`for (is_manual, r)` at line 306) the flag distinguishes user-specified from auto-discovered roots — after the fold there's only one root so the user-specified branch is dead.

For each of the two blocks, walk the body and:
1. Remove the `is_manual` case-split (keep the `is_manual = true` branch's behaviour, since the surviving root is by definition the user-supplied one).
2. Replace iteration with a direct reference to `&config.root`.

This is the largest mechanical edit in PR-2 — read the surrounding ~50 lines on each side carefully so the block structure stays intact.

- [ ] **Step 2.10: Update doc comments at lines 117–128, 748, 1145, 1243**

In `crates/atlas-cli/src/pipeline.rs`:

- Lines 117–122: doc comment on `IndexConfig` describing multi-root state — already retexted in step 2.6.
- Line 748: doc comment likely reading `/// 2. Discover roots via [`expand_roots`].` — change to describe the single-root form.
- Lines 1145, 1243: doc comments referencing peer-root behaviour — retext to describe single-root behaviour.

```bash
grep -n "additional_root\|additional-root\|expand_roots\|peer.root\|primary root" crates/atlas-cli/src/pipeline.rs
```

Expected: zero hits after edit (or only intentional historical references in commented-out code, which should not exist).

- [ ] **Step 2.11: `cargo build --workspace`**

```bash
cargo build --workspace
```

Expected: clean. If a call site in `crates/atlas-engine/src/*.rs` or in tests references `expand_roots`, those will fail compilation here — that is expected for PR-3/PR-4 surfaces (`crates/atlas-engine/tests/multi_root_path_deps.rs:28` uses `expand_roots, expand_roots_with_warnings`). PR-2 must temporarily mark those tests with `#[ignore]` or PR-2 must be sequenced *after* PR-4. The simplest sequencing: stop the workspace build from gating on the soon-to-be-deleted multi-root tests by *not* deleting `expand_roots` re-exports until they are unreferenced.

**Decision (per design §3.3):** PR-2 keeps the workspace clean by leaving `Workspace.roots: Vec<PathBuf>` populated as a length-1 vec; the multi-root tests in `crates/atlas-engine/tests/` still compile against `Workspace.roots` (and against `expand_roots` if the tests use it). After PR-2, run `cargo build --workspace` to confirm no test references the now-deleted `expand_roots` import. If any does, the path is:

  1. The test will be deleted in PR-4 anyway, so the cleanest fix is to **delete it now** within the scope of PR-2 — but that bleeds PR-4's scope into PR-2.
  2. Alternative: defer the `mod root_expansion;` deletion to PR-4 and keep the module file deleted (i.e., comment the `mod` line out, with a TODO). Rejected — placeholder.
  3. **Preferred:** sequence PR-4 *before* PR-2 (rewrite the dep graph). Rejected — PR-4's salvaged test depends on the singular-root `Workspace.root` which only exists post-PR-3.

The way out: **PR-2 deletes `root_expansion.rs` and the `mod root_expansion;` line, but the `expand_roots` re-export at `lib.rs:107` and the test imports of `expand_roots` survive PR-2 only if no other deletion gates compilation on them.**

In practice: confirm with `grep -n 'expand_roots' crates/atlas-engine/tests/ crates/atlas-cli/tests/`. If hits exist that block compilation, **defer the deletion of those test files into PR-2** (specifically, `multi_root_path_deps.rs` which `use`s `expand_roots`). Do not split this into a sub-PR — bundle the targeted test deletion with PR-2 and document in PR-2's status note. PR-4 then deletes the remaining two multi-root tests.

Concrete instruction: if `cargo build --workspace` after step 2.11 fails *only* because `crates/atlas-engine/tests/multi_root_path_deps.rs` references the now-gone `expand_roots`, `git rm crates/atlas-engine/tests/multi_root_path_deps.rs` as part of PR-2, and note this scope-bleed in PR-2's status entry.

- [ ] **Step 2.12: `cargo test --workspace --release --no-fail-fast`**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean. Multi-root tests that survived PR-2 (i.e., `multi_root.rs` and `atlas_contracts_in_ravel_lite.rs`) still pass — they exercise `Workspace.roots` against length-1 input, which is the trivial single-root case.

- [ ] **Step 2.13: `--additional-root` produces a clap error**

```bash
cargo run --quiet -- index . --additional-root /tmp 2>&1 | head -5
```

Expected: clap emits something like `error: unexpected argument '--additional-root' found`. Capture the exact text into PR-2's status note.

- [ ] **Step 2.14: Polyglot smoke test**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold-call count = Phase 2 PR-14 baseline (~26); warm-call count = 0.

- [ ] **Step 2.15: Commit PR-2**

```bash
git add -A
git commit -m "$(cat <<'EOF'
phase5: PR-2 drop discovery — delete root_expansion + --additional-root

- crates/atlas-engine/src/root_expansion.rs: deleted (469 LOC).
- crates/atlas-engine/src/lib.rs: removed mod root_expansion + pub use.
- crates/atlas-cli/src/main.rs: removed --additional-root flag,
  IndexArgs.additional_roots field, canonicalisation block.
- crates/atlas-cli/src/pipeline.rs: removed expand_roots import,
  IndexConfig.additional_roots field, IndexConfig::all_roots() helper,
  the two manual_iter chains around expand_roots, and updated doc
  comments. Workspace.roots Vec<PathBuf> still holds a length-1 vec
  at this boundary — PR-3 collapses the type.

Phase 5 design spec §3.3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 2.16: Flip PR-2 status checkbox**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`. Mark PR-2 `[x]`. Append under "### PR-2": commit sha + the captured `--additional-root` clap error text + note whether `multi_root_path_deps.rs` was deleted in PR-2 scope (per step 2.11 disposition).

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "phase5: PR-2 status checkbox

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
"
```

---

### Task 3: PR-3 — Singularise `Workspace` *(type + call-site refactor)*

**Files:**
- Modify: `crates/atlas-engine/src/db.rs:9-15,61-114,178-244` (Workspace input shape, `primary_root` deletion, `AtlasDatabase::new` + `from_workspace_input` signatures)
- Delete: `crates/atlas-engine/src/roots.rs` (61 LOC)
- Modify: `crates/atlas-engine/src/lib.rs` (remove `mod roots;` + the `pub use roots::best_root_for` re-export)
- Modify: `crates/atlas-engine/src/l2_candidates.rs:60-66`
- Modify: `crates/atlas-engine/src/l3_classify.rs:54,117,131,941`
- Modify: `crates/atlas-engine/src/l4_tree.rs:149,195,299,1037`
- Modify: `crates/atlas-engine/src/l5_surface.rs:41,274,282,285,432,535,609,624,692,792,1138,1224,1230,1315,1321,1480,1552,1558` (~17 sites; remove `use crate::roots::best_root_for`)
- Modify: `crates/atlas-engine/src/l6_compose_edges.rs:72`
- Modify: `crates/atlas-engine/src/l6_composition.rs:83`
- Modify: `crates/atlas-engine/src/l6_paths.rs:11,53`
- Modify: `crates/atlas-engine/src/l8_recurse.rs:51,252,293`
- Modify: `crates/atlas-engine/src/l9_projections.rs:39,51,200,325,415-417,436`
- Modify: `crates/atlas-engine/tests/l7_l8_fixedpoint.rs:86` (comment update)
- Modify: `crates/atlas-cli/tests/l6_participant_surface_sha.rs:287` (stale `expand_roots` comment)

**Type-shape change locked in by PR-3:**

```rust
// crates/atlas-engine/src/db.rs (post-PR-3)
#[salsa::input]
pub struct Workspace {
    pub root: PathBuf,
    // ...other fields unchanged...
}
```

No `primary_root()` method, no `Vec<PathBuf>` anywhere in the engine.

- [ ] **Step 3.1: Edit `db.rs` — Workspace input shape**

In `crates/atlas-engine/src/db.rs:69-79`, the Salsa input currently reads:

```rust
#[salsa::input]
pub struct Workspace {
    #[id]
    pub roots: Vec<PathBuf>,
    /* ... */
}
```

Replace with:

```rust
#[salsa::input]
pub struct Workspace {
    #[id]
    pub root: PathBuf,
    /* ... */
}
```

- [ ] **Step 3.2: Delete `Workspace::primary_root` and update doc comments**

In `crates/atlas-engine/src/db.rs:102-114`, delete the `primary_root` method:

```rust
impl Workspace {
    /* delete this entire method body — lines 102 to ~114 */
    pub fn primary_root(&self, db: &dyn salsa::Database) -> PathBuf {
        let roots = self.roots(db);
        roots
            .first()
            .cloned()
            .expect("Workspace.roots must be non-empty (enforced by AtlasDatabase::new)")
    }
}
```

If `impl Workspace` block becomes empty after removal, delete the empty `impl` block too.

Also update the module-level doc comments at `db.rs:9-15`:

```rust
//! - The list of filesystem roots being analysed. Atlas vNext is
//!   multi-root: `roots[0]` is the primary root (where `atlas index`
//!   was invoked) and any additional roots are peer manifest-roots
//!   (...)
//!   `roots.len() == 1` natural common case.
```

→

```rust
//! - The single filesystem root being analysed (where `atlas index`
//!   was invoked).
```

And update the doc comment at `db.rs:61-68`:

```rust
/// Run-wide L0 inputs. A single `Workspace` handle is created in
/// `AtlasDatabase::new`; queries key off it.
///
/// `root` is the analysed filesystem root.
```

(Replace the multi-root paragraph that mentions "primary" + "peer manifest-roots reached via path-dep walking".)

- [ ] **Step 3.3: Update `AtlasDatabase::new` and `from_workspace_input` signatures**

In `crates/atlas-engine/src/db.rs:178-244`, both constructors take `roots: Vec<PathBuf>`. Change both to `root: PathBuf`. The body block at lines 209–212 currently asserts:

```rust
        debug_assert!(
            !roots.is_empty(),
            "AtlasDatabase::new: `roots` must be non-empty (the primary root is `roots[0]`)"
        );
```

Delete the assertion entirely (a `PathBuf` is always present by the type system).

The Salsa input construction at `db.rs:242` currently reads:

```rust
        let workspace = Workspace::new(
            ...,
            roots,
            ...
        );
```

Change `roots,` → `root,`.

Also update the doc comment at lines 174–183 to describe the single-root signature.

- [ ] **Step 3.4: `cargo build --workspace` — capture all call-site failures**

```bash
cargo build --workspace 2>&1 | head -200
```

Expected: many compile errors. rustc enforces correctness here. Note each error site (file + line) — the next steps walk the layer files in order to fix them.

- [ ] **Step 3.5: Delete `roots.rs` and remove `mod roots;` + the re-export**

```bash
git rm crates/atlas-engine/src/roots.rs
```

In `crates/atlas-engine/src/lib.rs`, find and delete (use exact line numbers from `grep -n 'mod roots\|use roots' crates/atlas-engine/src/lib.rs`):

- The `mod roots;` declaration.
- The `pub use roots::best_root_for;` re-export at line 108.

Confirm:

```bash
grep -n "mod roots\|roots::" crates/atlas-engine/src/lib.rs
```

Expected: zero hits.

- [ ] **Step 3.6: Rewrite `l2_candidates.rs`**

In `crates/atlas-engine/src/l2_candidates.rs:60-67`, the current code reads:

```rust
    // of the workspace roots, so reconstruct the absolute form by
    // resolving against the matching root. (...)
    let roots = workspace.roots(db);
    /* ... uses roots somehow ... */
```

Replace with:

```rust
    // resolve against the workspace root.
    let root = workspace.root(db);
    /* ... uses root directly ... */
```

Read the surrounding 30 lines (50–90) before editing to understand how `roots` is consumed; the rewrite is mechanical (replace iteration with direct field access on the singular path).

- [ ] **Step 3.7: Rewrite `l3_classify.rs` — three sites + the `use` import**

- Line 54: `use crate::roots::best_root_for;` → delete the line.
- Line 117: `let roots = workspace.roots(dyn_db).clone();` → `let root = workspace.root(dyn_db);`
- Line 131: `let owning_root = best_root_for(&roots, &candidate_dir)` → `let owning_root = candidate_dir.starts_with(&root).then(|| root.as_path());`
- Line 941: same shape as line 131; rewrite analogously, keeping the surrounding `Option<&Path>` semantics intact.

The semantic substitution: `best_root_for(roots, p)` returned the longest-prefix root in `roots`. With one root, this becomes `if p.starts_with(root) { Some(root) } else { None }`. Express that with `.then(|| ...)` for callers that already use `Option<&Path>`.

- [ ] **Step 3.8: Rewrite `l4_tree.rs` — four sites**

- Line 149: `let roots = workspace.roots(db as &dyn salsa::Database).clone();` → `let root = workspace.root(db as &dyn salsa::Database);`
- Line 195: same.
- Line 299: `let owning_root = match crate::roots::best_root_for(roots, dir) { ... }` → `let owning_root = if dir.starts_with(&root) { Some(root.as_path()) } else { None };` (preserving the surrounding `match` arms — the non-`Some` arm becomes the `else` body).
- Line 1037: `let owning_root = crate::roots::best_root_for(roots, dir);` → `let owning_root = dir.starts_with(&root).then(|| root.as_path());`

Walk each site, read ±30 lines, and apply the transform.

- [ ] **Step 3.9: Rewrite `l5_surface.rs` — ~17 sites + the `use` import**

Delete `use crate::roots::best_root_for;` at line 41.

For each `best_root_for(roots, &segment.path)` site (lines 432, 535, 624, 792, 1138, 1230, 1321, 1480, 1558), apply the singular-root transform:

```rust
        } else if let Some(owning_root) = best_root_for(&roots, &segment.path) {
```

→

```rust
        } else if segment.path.starts_with(&root) {
            let owning_root = &root;
            // ...
```

(Or, if the surrounding `else if let Some(...)` form is more legible, use `.then(|| ...)` to keep the `Some` shape.)

For the helper-function call sites at lines 624, 792, 1230, 1321, 1480, 1558 that currently take `roots: &[PathBuf]` as a parameter, change those helper signatures to take `root: &Path` directly and update bodies. The signatures appear at the top of each `resolve_*_component_dir` helper (search for `fn resolve_` to find them).

For the `let roots = workspace.roots(db).clone();` site at line 285, replace with `let root = workspace.root(db);`.

For the doc comments at lines 274 + 282 + 609 + 692 + 1224 + 1315 + 1552 (which reference "the workspace roots" in prose), retext to "the workspace root".

This is the largest single file in PR-3. Run `cargo build -p atlas-engine` after editing this file alone to surface any leftover error.

- [ ] **Step 3.10: Rewrite `l6_compose_edges.rs:72`**

Line 72: `let roots = workspace.roots(db as &dyn salsa::Database).clone();` → `let root = workspace.root(db as &dyn salsa::Database);`

Update the surrounding code that consumed `roots` (read ±30 lines) to use `root` directly.

- [ ] **Step 3.11: Rewrite `l6_composition.rs:83`**

Line 83: same shape — `let roots = workspace.roots(db as &dyn salsa::Database).clone();` → `let root = workspace.root(db as &dyn salsa::Database);`

- [ ] **Step 3.12: Rewrite `l6_paths.rs` — one site + the `use` import**

- Line 11: `use crate::roots::best_root_for;` → delete.
- Line 53: `if let Some(root) = best_root_for(roots, rel) { ... }` → `if rel.starts_with(root) { ... }` (binding is already named `root`; the helper just becomes a prefix check).

Since `l6_paths.rs` likely takes `roots: &[PathBuf]` as a function argument (read the function signature ~line 40), change the parameter to `root: &Path` and update internal call sites that pass `&[PathBuf]` to pass `&Path`.

- [ ] **Step 3.13: Rewrite `l8_recurse.rs` — two sites + the `use` import**

- Line 51: `use crate::roots::best_root_for;` → delete.
- Line 252: `let roots = workspace.roots(db as &dyn salsa::Database).clone();` → `let root = workspace.root(db as &dyn salsa::Database);`
- Line 293: `let owning_root = best_root_for(&roots, abs_dir);` → `let owning_root = abs_dir.starts_with(&root).then(|| root.as_path());`

- [ ] **Step 3.14: Rewrite `l9_projections.rs` — six sites + the `use` import**

- Line 39: `use crate::roots::best_root_for;` → delete.
- Line 51, 200, 325: `let roots = workspace.roots(db as &dyn salsa::Database).clone();` → `let root = workspace.root(db as &dyn salsa::Database);`
- Line 415: doc comment — retext from "the longest matching prefix among `workspace.roots()`" to "the workspace root".
- Line 417: `let roots = workspace.roots(db);` → `let root = workspace.root(db);`
- Line 436: `let owning_root = best_root_for(roots, path).unwrap_or(dir.as_path());` → `let owning_root = if path.starts_with(&root) { root.as_path() } else { dir.as_path() };`

- [ ] **Step 3.15: Update `tests/l7_l8_fixedpoint.rs:86` comment**

Read line 86 (one-line comment likely referencing multi-root behaviour). Retext to describe single-root. Pure prose edit; no logic change.

- [ ] **Step 3.16: Update `crates/atlas-cli/tests/l6_participant_surface_sha.rs:287` comment**

Line 287 currently reads (or similar):

```rust
    // PR-4's expand_roots discovers crate-a's directory as a peer root.
```

Retext to describe single-root behaviour or delete the comment if it is now stale and adds nothing.

- [ ] **Step 3.17: Update consumer-facing call sites — `Workspace::new` invocations and `AtlasDatabase::new` calls**

Run:

```bash
grep -rn "Workspace::new\|AtlasDatabase::new\|from_workspace_input" crates/ 2>&1
```

Each call site that previously passed `roots: Vec<PathBuf>` (or `vec![root]`) now passes `root: PathBuf` directly. Walk each hit and update.

- [ ] **Step 3.18: `cargo build --workspace` — clean**

```bash
cargo build --workspace
```

Expected: clean. If errors remain, return to the earlier per-file steps and fix the missed sites.

- [ ] **Step 3.19: `cargo test --workspace --release --no-fail-fast`**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean. The two surviving multi-root tests (`multi_root.rs`, `atlas_contracts_in_ravel_lite.rs`) likely *fail* compilation now because they reference `Workspace.roots` (plural). Apply the same triage as PR-2 step 2.11: if compilation gating depends on a soon-deleted test, **delete the test file inside PR-3** and document the scope-bleed in PR-3's status note. PR-4 will pick up the remaining deletion.

The cleanest pre-merge state for PR-3: only the salvaged single-root tests + non-multi-root tests survive. PR-4 then becomes purely *adding* `contract_edge_in_workspace.rs`.

- [ ] **Step 3.20: Polyglot smoke test**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold-call count = Phase 2 PR-14 baseline (~26); warm-call count = 0.

- [ ] **Step 3.21: Run the multi-root vocabulary audit grep**

```bash
git grep -E 'multi.root|multi-root|workspace\.roots' crates/
```

Expected: zero hits in `crates/atlas-engine/src/`, `crates/atlas-cli/src/`. Surviving hits should be only in test fixtures or doc comments that are intentionally retained as historical context — each of those must be explicitly justified in PR-3's description.

- [ ] **Step 3.22: Commit PR-3**

```bash
git add -A
git commit -m "$(cat <<'EOF'
phase5: PR-3 collapse Workspace.roots to singular root

- crates/atlas-engine/src/db.rs: Workspace.roots: Vec<PathBuf>
  collapses to root: PathBuf; Workspace::primary_root method
  deleted; AtlasDatabase::new + from_workspace_input signatures
  updated; module doc comments retexted.
- crates/atlas-engine/src/roots.rs: deleted (61 LOC).
- crates/atlas-engine/src/lib.rs: removed mod roots + the
  pub use best_root_for re-export.
- L2/L3/L4/L5/L6/L8/L9 source files: every workspace.roots() /
  best_root_for site rewritten to use the singular root via
  path.starts_with(&root). ~30 sites touched across 9 files.
- crates/atlas-engine/tests/l7_l8_fixedpoint.rs:86 + crates/
  atlas-cli/tests/l6_participant_surface_sha.rs:287: stale
  multi-root comments retexted.

Phase 5 design spec §3.4. Multi-root vocabulary audit attached
as PR description. No semantic change vs. the Phase 5 PR-2
boundary — prior plural-root state always held a length-1 vec.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3.23: Flip PR-3 status checkbox**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`. Mark PR-3 `[x]`. Append under "### PR-3": commit sha + audit grep output (zero hits expected) + scope-bleed note (which multi-root tests if any were deleted in PR-3 vs deferred to PR-4).

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "phase5: PR-3 status checkbox

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
"
```

---

### Task 4: PR-4 — Salvage tests *(test suite surgery)*

**Files:**
- Create: `crates/atlas-cli/tests/contract_edge_in_workspace.rs` (~400 LOC)
- Delete: `crates/atlas-engine/tests/multi_root.rs` (154 LOC; if not already deleted in PR-3 scope-bleed)
- Delete: `crates/atlas-engine/tests/multi_root_path_deps.rs` (742 LOC; if not already deleted in PR-2 scope-bleed)
- Delete: `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` (593 LOC)

**Salvage scope:** the new test reproduces AC#1–5 from `atlas_contracts_in_ravel_lite.rs:21-36` (the doc comment block listing the original five acceptance criteria), against a single-root two-crate fixture (one consumer crate plus a stand-in for the schema crate it depends on).

- [ ] **Step 4.1: Read the original AC#1–5 carefully**

```bash
sed -n '21,36p' /Users/antony/Development/Atlas/crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs
```

Expected output (paste into the new test's doc comment header verbatim, retexted for single-root):

```
1. components.yaml lists components from both roots.
2. related-components.yaml carries a `consumes-contract` edge from
   the consumer to a contract id under the `atlas-contracts/`
   namespace.
3. The atlas-contracts component's surfaces.yaml lists that
   contract id under contracts_defined.
4. A no-op re-run makes zero LLM calls (persistent cache hit).
5. Editing the defining binding's source invalidates only the L6
   batch and atlas-contracts's L5 — the consumer's L5 entry still
   hits the persistent cache, and the deterministic Cargo-registry
   classifier short-circuits L3 so no Classify call fires.
```

The single-root retext: drop "from both roots" in AC#1 (the new fixture has both crates inside the same root), and drop the `atlas-contracts/` namespace assumption in AC#2/AC#3 — pick a synthetic component id like `schema-crate/foo` for the new fixture.

- [ ] **Step 4.2: Write the failing test (skeleton)**

Create `crates/atlas-cli/tests/contract_edge_in_workspace.rs`. Begin with a header that maps each original AC to a `#[test]` (the existing test file uses two `#[test]` functions: one for AC#1+2+3 and one for AC#4+5 — preserve that split):

```rust
//! Contract-edge family in a single-root workspace.
//!
//! End-to-end smoke test that exercises the Phase 1 PR-12 acceptance
//! flow against the post-Phase-5 single-root model: a consumer crate
//! has a Cargo path-dep on a sibling crate (both workspace members of
//! the same root). The rust-surface analyser detects the sibling's
//! `pub struct Foo` → emits a `data-format` contract under
//! `<sibling-id>/foo`. L6's contract-edge batch emits a
//! `consumes-contract` edge from the consumer to that contract id.
//!
//! ## Acceptance criteria (salvaged from the deleted Phase 1 PR-12
//! test atlas_contracts_in_ravel_lite.rs lines 21-36)
//!
//! 1. components.yaml lists both crates' components.
//! 2. related-components.yaml carries a `consumes-contract` edge
//!    from the consumer to the schema crate's contract id.
//! 3. The schema crate's surfaces.yaml lists that contract id under
//!    contracts_defined.
//! 4. A no-op re-run makes zero LLM calls (persistent cache hit).
//! 5. Editing the defining binding's source invalidates only the L6
//!    batch and the schema crate's L5 — the consumer's L5 entry
//!    still hits the persistent cache, and the deterministic Cargo-
//!    registry classifier short-circuits L3 so no Classify call
//!    fires.

use std::path::Path;
use std::sync::{Arc, Mutex};

// ... (imports lifted from atlas_contracts_in_ravel_lite.rs verbatim)

fn build_fixture() -> tempfile::TempDir {
    // Single-root workspace with two sibling crates: `consumer` and
    // `schema-crate`. consumer/Cargo.toml has `path = "../schema-crate"`
    // for `schema-crate`. schema-crate/src/lib.rs defines:
    //
    //     #[derive(Serialize, Deserialize)]
    //     pub struct Foo { /* ... */ }
    //
    // consumer/src/main.rs uses `schema_crate::Foo`.
    todo!("build the single-root fixture; mirror the multi-root fixture's structure but place both crates under one [workspace] members array")
}

#[test]
fn ac_1_2_3_components_edge_and_surfaces() {
    let tmp = build_fixture();
    // ... cold run, assert AC#1, AC#2, AC#3
    todo!()
}

#[test]
fn ac_4_5_cache_hit_and_targeted_invalidation() {
    let tmp = build_fixture();
    // ... cold → no-op rerun → edit → re-run, assert AC#4 + AC#5
    todo!()
}
```

- [ ] **Step 4.3: Run the new test → confirm it fails**

```bash
cargo test -p atlas-cli --test contract_edge_in_workspace --release
```

Expected: FAIL (with `not yet implemented` panic from the `todo!()` calls).

- [ ] **Step 4.4: Port the fixture builder**

Read the original `build_fixture` (or whatever the equivalent helper is called — likely a function around line 40–120 of `atlas_contracts_in_ravel_lite.rs`) and adapt:

- The original creates *two* tempdirs (one for the primary root, one for the peer root) connected by a `path = "../<peer>"` declaration in the consumer's `Cargo.toml`.
- The salvaged version creates *one* tempdir holding a single workspace `Cargo.toml` with `[workspace] members = ["consumer", "schema-crate"]`, plus the two crate directories inside.
- `schema-crate/src/lib.rs` defines a `pub struct Foo` with `#[derive(Serialize, Deserialize)]`.
- `consumer/src/main.rs` imports and uses `schema_crate::Foo`.

Lift the LLM canned-response setup verbatim (the `Arc<Mutex<...>>` shared backend) — that pattern is unchanged by the single-root collapse.

- [ ] **Step 4.5: Port AC#1+2+3 assertions**

Adapt the assertions from the original test's first `#[test]` function. The single-root differences:

- `components.yaml` now lists components keyed by their workspace-relative paths (`consumer`, `schema-crate`) rather than by per-root namespaces. Assertion: both component ids appear in the YAML.
- `related-components.yaml` lists a `consumes-contract` edge from `consumer` to the contract id under `schema-crate/foo`.
- `schema-crate/surfaces.yaml` lists the contract id under `contracts_defined`.

- [ ] **Step 4.6: Port AC#4+5 assertions**

Adapt the second `#[test]` function. The flow (unchanged from the multi-root original):

1. Cold run. Capture LLM call count.
2. No-op re-run. Assert call count delta = 0 (AC#4).
3. Edit `schema-crate/src/lib.rs` (e.g., add a doc comment to `Foo`'s field, or add a new field). Re-run.
4. Assert: only the schema crate's L5 + the L6 batch were invalidated. Specifically, no `Classify` call fired for the consumer or for the schema crate (the Cargo-registry classifier short-circuits L3); the consumer's L5 cache key is unchanged.

The mechanics: each layer-cache hit/miss is captured by the canned `Arc<Mutex<...>>` LLM backend. Walk the original's invalidation assertions and adapt them to the new component ids.

- [ ] **Step 4.7: Run the new test → confirm it passes**

```bash
cargo test -p atlas-cli --test contract_edge_in_workspace --release --no-fail-fast
```

Expected: both `#[test]` functions PASS.

- [ ] **Step 4.8: Delete the original multi-root tests (any that survive)**

```bash
git rm crates/atlas-engine/tests/multi_root.rs 2>/dev/null || true
git rm crates/atlas-engine/tests/multi_root_path_deps.rs 2>/dev/null || true
git rm crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs
```

(`atlas_contracts_in_ravel_lite.rs` definitely survived to this point — its salvage was the whole point of the PR. The two `multi_root*.rs` files may have been deleted earlier as PR-2/PR-3 scope-bleed; the `2>/dev/null || true` suppresses the "file not in index" error in that case.)

- [ ] **Step 4.9: `cargo build --workspace` clean**

```bash
cargo build --workspace
```

Expected: clean.

- [ ] **Step 4.10: `cargo test --workspace --release --no-fail-fast`**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean. The new `contract_edge_in_workspace` test passes; no other test references the deleted files.

- [ ] **Step 4.11: Polyglot smoke test**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold-call count = Phase 2 PR-14 baseline (~26); warm-call count = 0.

- [ ] **Step 4.12: Commit PR-4**

```bash
git add -A
git commit -m "$(cat <<'EOF'
phase5: PR-4 salvage contract-edge test for single-root model

- crates/atlas-cli/tests/contract_edge_in_workspace.rs: new, ~400 LOC.
  Single-root rewrite of the deleted atlas_contracts_in_ravel_lite.rs.
  Preserves AC#1-5 of the original (Phase 1 PR-12 acceptance) against
  a synthetic two-crate fixture (consumer + schema-crate) where both
  crates are workspace members of the same root.
- crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs: deleted (593 LOC).
- crates/atlas-engine/tests/multi_root.rs: deleted (154 LOC).
- crates/atlas-engine/tests/multi_root_path_deps.rs: deleted (742 LOC).

Net diff: ~-1090 LOC after the salvage. AC#1-5 mapping table
attached to PR description: each assertion in the new test cites
its original line in the deleted test, and any assertion deemed
redundant under single-root is explicitly justified.

Phase 5 design spec §3.5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4.13: Flip PR-4 status checkbox**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`. Mark PR-4 `[x]`. Append under "### PR-4": commit sha + the AC#1–5 mapping table (each old assertion line → new assertion line).

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "phase5: PR-4 status checkbox

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
"
```

---

### Task 5: PR-5 — Retext canonical system-model design *(docs only)*

**Files:**
- Modify: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` (delete §5.3, renumber §5.4–§5.6 → §5.3–§5.5; mark Phase 5 SHIPPED in §10.5; delete §10.1 multi-root architectural seam callout; delete glossary "Multi-root workspace" entry at line ~1450)
- Modify: `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` (lines 19, 177, 253: retext §5.3 references)

**Note:** PR-5 is parallel-safe with PR-2/3/4. It edits no code and the surfaces touched (the two design docs) are disjoint from any PR-2/3/4 file. It can be dispatched in a separate worktree concurrent with the deletion sequence; it merges any time before PR-6.

- [ ] **Step 5.1: Read the canonical design §5.3 in full to understand what's being deleted**

```bash
sed -n '434,452p' /Users/antony/Development/Atlas/docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
```

Expected: the full text of §5.3 "Multi-root workspace". This entire section goes away in PR-5.

- [ ] **Step 5.2: Delete §5.3 entirely**

In `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`, locate `### 5.3 Multi-root workspace` (line 434) and delete from that header through to the line immediately before `### 5.4 File layout` (line 453). Result: §5.4's content immediately follows §5.2's content.

- [ ] **Step 5.3: Renumber §5.4, §5.5, §5.6**

Find and rename:

- `### 5.4 File layout` → `### 5.3 File layout`
- `### 5.5 Cache architecture` → `### 5.4 Cache architecture`
- `### 5.6 Server mode (eventual)` → `### 5.5 Server mode (eventual)`

```bash
grep -n "^### 5\." docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
```

Expected: §5.1, §5.2, §5.3 (was §5.4), §5.4 (was §5.5), §5.5 (was §5.6). No §5.6.

The convention from `feedback`: inbound references in older Phase 1/2/3/4 spec/plan files are *not* retroactively updated. Do not chase those down.

- [ ] **Step 5.4: Mark Phase 5 SHIPPED in §10**

Find `### 10.5 Phase 5 — Monorepo consolidation` (line 1179). Currently reads (approximately):

```markdown
### 10.5 Phase 5 — Monorepo consolidation

Fold atlas-contracts (component-ontology + atlas-index) into Atlas;
delete multi-root machinery.
```

Replace the body with the §7 retext from the design spec (verbatim — that's the canonical retext; substitute PR-6's eventual commit sha for `<sha>`):

```markdown
### 10.5 Phase 5 — Monorepo consolidation, part 1

**SHIPPED 2026-05-10.** Folded `atlas-contracts` (schema crates
`component-ontology` and `atlas-index`) into the Atlas repo as
workspace members; retired the multi-root architectural seam (deleted
`expand_roots`, the `--additional-root` CLI flag,
`IndexConfig.additional_roots`, and the `roots.rs::best_root_for`
helper; collapsed `Workspace.roots: Vec<PathBuf>` to
`Workspace.root: PathBuf`). Folding Ravel + Ravel-Lite into Atlas is
deferred to a later phase (post-Phase-5, slot TBD), possibly tied to
a Bazel build-system migration for the polyglot tree. Final commit:
`<PR-6-COMMIT-SHA>`.
```

The `<PR-6-COMMIT-SHA>` placeholder gets replaced post-PR-6 by an amend or follow-up commit; for PR-5 itself, leave it as a literal `<PR-6-COMMIT-SHA>` and document in PR-5's status note that PR-6 fills it in.

- [ ] **Step 5.5: Delete §10.1 multi-root architectural seam callout**

Find `### 10.1 Phase 1 — Architectural seam` (line 1092). The bullet list under "**Goal:** Establish the model. Multi-root, contract-first, scattered ..." includes a line `- Multi-root \`Workspace\` (Salsa input becomes plural roots).` (line ~1099).

The design spec instruction: "delete the multi-root callout. If §10.1 has other content, it stays; if multi-root was the only seam listed, the section is removed."

Read §10.1's full body. If multi-root is one of multiple bullet items, delete only that bullet. If §10.1's entire content is the multi-root description, delete the section header and body, and renumber §10.2 etc. up by one.

Per the surrounding context, §10.1 is "Phase 1 — Architectural seam" with several bullet items (multi-root, contract-first, scattered `.atlas/`, etc.). Delete only the multi-root bullet at line ~1099.

- [ ] **Step 5.6: Delete the glossary entry "Multi-root workspace"**

Find the glossary entry at line ~1450:

```markdown
- **Multi-root workspace**: a `Workspace` Salsa input with plural roots,
  one analysed per ...
```

Delete the entry (header + body — typically 2–4 lines).

- [ ] **Step 5.7: Retext the override-scoping spec lines 19, 177, 253**

In `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`:

- **Line 19:** currently mentions multi-root context for §5.3. Retext the reference to point at the new §5.3 (file layout) and drop multi-root vocabulary. Read ±5 lines for surrounding context.
- **Line 177:** currently mentions "the multi-root experience". Retext to drop multi-root language; the surrounding paragraph describes the warning-vs-error behaviour for `--strict-overrides` and is otherwise unaffected. Suggested rewrite:

  > **Phase 2 addition:** a `--strict-overrides` CLI flag escalates the warning to a hard error. Phase 1 ships warning-only; we want this to be forgiving while users learn the discovery rules.

- **Line 253:** currently lists "§5.3 (multi-root)" among design spec sections. Drop the parenthetical "(multi-root)" and update the §5.x numbers in the list to match the post-PR-5 canonical-design numbering. Read ±5 lines.

- [ ] **Step 5.8: Sanity-check the canonical design renders correctly**

```bash
grep -n "^### 5\.\|^### 10\.\|Multi-root\|multi-root\|multi.root" docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
```

Expected: §5.1–§5.5 (no §5.6); §10.1–§10.10; zero hits for "Multi-root" or "multi-root" except in *historical* phase-status callouts (e.g., §10.1's bullet list of *what Phase 1 originally introduced*, if you chose to retain that historical record). The deleted glossary entry produces zero hits.

- [ ] **Step 5.9: Commit PR-5**

```bash
git add docs/superpowers/specs/2026-05-06-atlas-system-model-design.md \
        docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md
git commit -m "$(cat <<'EOF'
phase5: PR-5 retext canonical system-model design

- 2026-05-06-atlas-system-model-design.md:
  - delete §5.3 "Multi-root workspace" in entirety.
  - renumber §5.4 → §5.3, §5.5 → §5.4, §5.6 → §5.5.
  - §10.5: mark Phase 5 SHIPPED with full §7 retext.
  - §10.1: delete the multi-root architectural-seam bullet.
  - glossary: delete "Multi-root workspace" entry.
- 2026-05-06-override-scoping-scattered-atlas.md:
  - lines 19, 177, 253: retext §5.3 references for the
    post-Phase-5 single-root layout.

Inbound references in older Phase 1/2/3/4 spec/plan files
intentionally not retroactively updated (specs are time-snapshots).

Phase 5 design spec §3.6 + §7. <PR-6-COMMIT-SHA> placeholder in
§10.5 gets replaced post-PR-6.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5.10: Flip PR-5 status checkbox**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`. Mark PR-5 `[x]`. Append under "### PR-5": commit sha + note "<PR-6-COMMIT-SHA> placeholder in §10.5 to be filled in by a follow-up commit after PR-6 lands."

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "phase5: PR-5 status checkbox

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
"
```

---

### Task 6: PR-6 — Acceptance + closeout *(verification only)*

**Files:**
- Modify: `docs/superpowers/plans/2026-05-10-phase5-status.md` (closeout entry + Upgrade notes subsection)
- Modify: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` (replace `<PR-6-COMMIT-SHA>` placeholder in §10.5 with actual sha)
- Modify: memory files
  - `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase4_plus_roadmap.md`
  - `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_monorepo_consolidation.md`
  - `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase5_split_and_ravel_bazel.md`

**Manual post-merge steps (tracked in status file; not gated by CI):**

1. atlas-contracts GitHub repo: README updated to point at Atlas, then archive-flag set in repo settings.
2. (User-side, not Atlas-side) `rm -rf ~/Development/atlas-contracts/`.
3. (User-side) any local `.atlas/` outputs from prior multi-root runs deleted.

- [ ] **Step 6.1: Run the polyglot smoke test (cumulative regression guard)**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast 2>&1 | tee /tmp/phase5-pr6-polyglot.log
```

Expected: clean. Cold-call count = Phase 2 PR-14 baseline (~26); warm-call count + reports = 0. Capture stdout to `/tmp/phase5-pr6-polyglot.log` for the closeout note.

- [ ] **Step 6.2: Run the structural-invariant audit greps**

```bash
git grep -E 'multi.root|multi-root|additional_root|expand_roots|best_root_for' crates/ 2>&1 | tee /tmp/phase5-pr6-audit.log
```

Expected: zero hits — except possibly in the `tests/` subtree where a re-introduced fixture might mention multi-root in a comment. Each surviving hit must be explicitly justified in the closeout note.

```bash
test ! -f crates/atlas-engine/src/root_expansion.rs && echo "OK: root_expansion.rs gone"
test ! -f crates/atlas-engine/src/roots.rs && echo "OK: roots.rs gone"
cargo run --quiet -- index --help 2>&1 | grep -q "additional-root" && echo "FAIL: --additional-root still in help" || echo "OK: --additional-root not in help"
```

Expected: `OK: root_expansion.rs gone`; `OK: roots.rs gone`; `OK: --additional-root not in help`.

- [ ] **Step 6.3: Verify `Cargo.toml` workspace shape**

```bash
grep -n 'crates/component-ontology\|crates/atlas-index\|atlas-contracts' Cargo.toml
```

Expected: `crates/component-ontology` and `crates/atlas-index` listed in `members` and as `[workspace.dependencies]` paths; zero hits for `atlas-contracts` (the cross-repo string).

- [ ] **Step 6.4: Replace the `<PR-6-COMMIT-SHA>` placeholder in §10.5**

This step lands as a separate commit (the placeholder cannot reference its own commit sha pre-self-reference). The two-step pattern:

1. Compute the SHA that PR-6's *first* commit will land at — but we cannot, because the placeholder edit IS that commit.
2. **Workaround:** the placeholder gets replaced with the SHA of PR-6's first commit (the closeout commit) by a *follow-up* commit landed second within PR-6's scope. Keep both commits part of PR-6; the second is administrative.

Alternative (cleaner, matches the Phase 4 PR-8 pattern): land the closeout commit, then immediately:

```bash
PR6_SHA=$(git rev-parse HEAD)
sed -i.bak "s/<PR-6-COMMIT-SHA>/${PR6_SHA}/" docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
rm docs/superpowers/specs/2026-05-06-atlas-system-model-design.md.bak
git add docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
git commit -m "phase5: PR-6 backfill PR-6 commit sha into canonical §10.5

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
"
```

(The two PR-6 commits remain on the Phase 5 worktree branch; they merge together.)

- [ ] **Step 6.5: Write the closeout note in the status file**

Edit `docs/superpowers/plans/2026-05-10-phase5-status.md`. Append to the bottom:

```markdown
## Phase 5 — complete

2026-05-10. All seven PRs merged to main. Cumulative LOC delta:
PR-0 (docs only) + PR-1 (~+1500 from snapshot copy of two crates;
~−10 from Cargo.toml comment + path edits) + PR-2 (−500) +
PR-3 (−150) + PR-4 (−1090) + PR-5 (−40 in design prose) +
PR-6 (administrative) = roughly **+1500 −1790 = −290 LOC inside
crates/, plus +the two folded schema crates as net new code in-tree
that previously lived in atlas-contracts**. Net production-code
deletion (excluding the snapshot-copied schema crates): **~−600 LOC**.
Net test-code deletion: **~−1090 LOC**. Net documentation deletion:
**~−40 lines** in canonical design.

Final commits:
- PR-0: `<sha>`
- PR-1 (Atlas): `<sha>`
- PR-1 (Ravel-Lite, separate repo): `<sha>`
- PR-2: `<sha>`
- PR-3: `<sha>`
- PR-4: `<sha>`
- PR-5: `<sha>`
- PR-6: `<sha>` (closeout) + `<sha>` (sha-backfill follow-up)

### Upgrade notes

Atlas Phase 5 collapses the multi-root `Workspace` Salsa input to a
singular root, which is a hard upgrade discipline boundary:

> **Before upgrading past Phase 5, delete `.atlas/` in every workspace
> previously indexed by Atlas.** Persisted multi-root output state
> (componentstate keyed by per-root namespacing, peer-root-aware path
> resolution baked into cached fingerprints) is not forward-compatible.
> No migration command exists; no version-aware decoder is shipped.
> Re-run `atlas index <root>` from a clean state.

This applies to every persisted `.atlas/` directory anywhere on disk.
Atlas's `--additional-root` flag is also gone; any user-side
automation that passed it produces a clap error.

### Manual post-merge checklist

- [ ] atlas-contracts GitHub repo: README updated to point at Atlas.
- [ ] atlas-contracts GitHub repo: archive flag set in repo settings.
- [ ] User-side: `rm -rf ~/Development/atlas-contracts/`.
- [ ] User-side: any local `.atlas/` outputs from prior multi-root
      runs deleted.
- [ ] (If a schema-crate version bump is wanted concurrent with
      Phase 5:) `cargo release` from inside `crates/component-ontology/`
      and `crates/atlas-index/` to publish to crates.io from Atlas's
      home. Otherwise defer to next schema change.
```

Flip PR-6 checkbox to `[x]`.

- [ ] **Step 6.6: Update memory files**

Edit `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase4_plus_roadmap.md`:

- Mark Phase 5 as SHIPPED with date 2026-05-10 and the PR-6 commit sha.
- Advance Phase 6 (user-facing schema cleanups) to "next up."

Edit `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_monorepo_consolidation.md`:

- Mark "atlas-contracts in-tree" complete (shipped Phase 5).
- Note that remaining Ravel + Ravel-Lite fold is deferred to a post-Phase-5 polyglot fold (potentially Bazel-based).

Edit `~/.claude/projects/-Users-antony-Development-Atlas/memory/project_phase5_split_and_ravel_bazel.md`:

- Update with shipped state for Phase 5's A + C scope.
- Reaffirm Ravel/Ravel-Lite fold is the remaining deferred work.

(Memory edits do not commit to the Atlas repo — they live in `~/.claude/`. They're listed here for completeness; the agent updates them in-place after PR-6 lands.)

- [ ] **Step 6.7: Commit the closeout entry**

```bash
git add docs/superpowers/plans/2026-05-10-phase5-status.md
git commit -m "$(cat <<'EOF'
phase5: PR-6 acceptance + closeout

- Polyglot smoke test: clean. Cold ~26 calls (Phase 2 PR-14 baseline);
  warm + reports = 0.
- Audit greps: zero hits for multi.root|multi-root|additional_root|
  expand_roots|best_root_for in crates/ (excluding tests with
  intentional historical comments, justified inline).
- root_expansion.rs and roots.rs do not exist.
- --additional-root produces a clap error.
- Cargo.toml: schema crates listed as members; no atlas-contracts
  string anywhere.
- Status file: closeout note + Upgrade notes subsection (.atlas/
  deletion required across upgrade boundary) + manual post-merge
  checklist.

Phase 5 — Monorepo consolidation, part 1: COMPLETE.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6.8: (Manual; tracked) Archive atlas-contracts on GitHub + delete local checkout**

These steps live outside the Atlas repo and are tracked in the status file's manual post-merge checklist (step 6.5). Do not attempt them from inside the Atlas worktree:

1. atlas-contracts GitHub repo: edit README to point at Atlas (`https://github.com/linkuistics/Atlas`), commit, push.
2. atlas-contracts GitHub repo: open repo Settings → Archive this repository → confirm.
3. User-side, only after the Ravel-Lite path-edit commit (step 1.19) is confirmed merged: `rm -rf ~/Development/atlas-contracts/`.
4. User-side: `find ~ -name '.atlas' -type d 2>/dev/null` and confirm with the user before removing any local Atlas output directories that pre-date Phase 5.

These are surfaced to the user as a final closeout question, not auto-executed.

---

## 5. Acceptance summary

Phase 5 ships when every box below is `[x]`.

### 5.1 Behavioural invariants

- [ ] **LLM-call budget unchanged.** Cold polyglot smoke test ≈ Phase 2 PR-14 baseline. Warm/reports = 0. Verified at every PR boundary.
- [ ] **Six-file editorial tier preserved.** No new files added; none removed.
- [ ] **`atlas-reports` stays pure-function.** No engine state mutation from the reports pathway.
- [ ] **Polyglot smoke test green** at PR-1, PR-2, PR-3, PR-4, PR-5 (no-op for PR-5 — docs only), PR-6.

### 5.2 Structural invariants

- [ ] `git grep -E 'multi.root|multi-root|additional_root|expand_roots|best_root_for' crates/` returns zero non-test, non-deleted-file hits.
- [ ] `Workspace` Salsa input has a singular `root: PathBuf` field; no `roots` field, no plural accessor, no `primary_root` method.
- [ ] `crates/atlas-engine/src/{root_expansion.rs,roots.rs}` do not exist.
- [ ] `--additional-root` does not appear in `atlas index --help` (clap error on use).
- [ ] `Cargo.toml` lists `crates/component-ontology` and `crates/atlas-index` as workspace members; no `path = "../atlas-contracts/..."` strings remain.

### 5.3 Quantitative invariants (descriptive, not gating)

- [ ] Net production-code deletion: ≥ 600 LOC.
- [ ] Net test-code deletion: ≥ 1,000 LOC after the salvaged test.
- [ ] Net documentation deletion: ~30 lines in canonical design.md.
- [ ] Status file closeout includes total-LOC summary and per-PR commit list.

### 5.4 Per-PR gates

| PR | Must hold |
|----|-----------|
| PR-0 | Plan + status + continuation prompt present; no code touched. |
| PR-1 | Workspace builds + tests clean; polyglot smoke test green; both `cargo publish --dry-run` clean; Ravel-Lite local build clean post-coordination commit; website-merge resolution documented. |
| PR-2 | Workspace builds + tests clean; polyglot smoke test green; `--additional-root` produces clap error. |
| PR-3 | Workspace builds + tests clean; polyglot smoke test green; multi-root vocabulary audit attached. |
| PR-4 | Workspace tests clean with zero net coverage loss; AC#1–5 mapped in PR description. |
| PR-5 | Docs-only; canonical design.md §5.3 deleted; §10.5 marks Phase 5 SHIPPED with PR-6's eventual SHA. |
| PR-6 | All structural + behavioural invariants hold; status file closeout includes Upgrade notes (`.atlas/` deletion required); §10.5 SHA backfilled. |

---

## 6. Cross-repo coordination steps (in order)

| # | Step | When | By whom |
|---|------|------|---------|
| 1 | Atlas PR-1 lands | Phase 5 PR-1 (step 1.16) | engineer |
| 2 | Ravel-Lite Cargo.toml path-edit commit | Immediately after step 1 (step 1.17–1.19) | engineer (separate repo) |
| 3 | Atlas PR-2 → PR-6 land in order | Phase 5 sequence | engineer |
| 4 | crates.io publish from Atlas (only if a schema-crate version bump is wanted concurrent with Phase 5; otherwise next time the schema changes) | After PR-6 green | engineer, manual `cargo release` |
| 5 | Archive `atlas-contracts` repo on GitHub | After PR-6 green AND step 2 confirmed | repo admin (GitHub UI) |
| 6 | Remove `~/Development/atlas-contracts/` working copy locally | Any time after steps 1+2 | user |

---

## 7. Notes for the executing engineer

- **Do not pipe `cargo test --release ...` invocations through `tail`.** The polyglot smoke test pegs at 99% CPU for ~10 minutes; buffered tail makes a working process look stuck. Let stdout pass through. (Memory `feedback_no_tail_pipe_for_long_tests`.)
- **Run `cargo build --release --workspace` before the polyglot test.** `cargo test --workspace --release` does not build standalone analyser `[[bin]]` targets that the polyglot test discovers via runtime path lookup. (Memory `feedback_release_workspace_build_for_polyglot`.)
- **Snapshot copy, not `git subtree`.** The atlas-contracts fold is a plain directory copy with a single import commit. No history preservation. (Memory `feedback_user_low_git_history_value`.)
- **Singular type, end-to-end.** `Workspace.root: PathBuf` has no iterator, no `as_slice()`, no `primary_root()`. Every consumer reads the field directly. Do not introduce a wrapper helper. (Memory `feedback_no_iterator_stubs_for_singletons`.)
- **Worktree base verification.** When the user dispatches a parallel-safe subagent (e.g., PR-5 alongside PR-2/3/4), confirm the subagent's worktree base commit matches current main before proceeding. (Memory `feedback_worktree_base_verification`.)
- **Scope-bleed disposition for PR-2/PR-3.** Compilation gating may force a multi-root test deletion to bleed into PR-2 or PR-3 ahead of its planned PR-4 home. Document the bleed in the affected PR's status note rather than splitting into a sub-PR. The plan's per-PR LOC accounting still works because the deletion lands eventually.
- **`<PR-6-COMMIT-SHA>` placeholder in PR-5.** PR-5 lands the canonical-design retext with the literal placeholder. PR-6 backfills the actual SHA in a follow-up commit (step 6.4). Both commits stay on the Phase 5 worktree branch.
