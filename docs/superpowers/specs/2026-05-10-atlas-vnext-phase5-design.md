# Atlas vNext Phase 5 — Monorepo consolidation, part 1 (design spec)

Status: brainstormed and approved 2026-05-10. Companion plan + status
file land in PR-0 of Phase 5 itself.

Phase 4 shipped on 2026-05-09 (7 code/docs PRs + PR-0 plan; final
commit `f80e179` on Atlas main). This document captures the canonical
scope of Phase 5 — a focused **monorepo consolidation, part 1** — and
the post-Phase-5 architecture that subsequent phases inherit.

---

## 0. Reading order

§1 (one-paragraph summary) → §2 (scope) → §3 (PR enumeration) →
§4 (post-Phase-5 architecture) → §6 (acceptance summary). Skim §5
(risks) and §7 (canonical design.md retext) on demand. §8 lists
references.

---

## 1. Summary

Phase 5 retires the multi-root architectural seam introduced in Phase 1
(canonical design.md §5.3). The seam was originally invented to let
Atlas analyse a primary repo plus its `path = "../sibling"` Cargo
dependencies as one knowledge graph; folding the canonical sibling
target (`atlas-contracts`) in-tree removes the structural reason for
the seam, after which the engine collapses to a single-root model.

**No new user-facing capability, no schema change, no new LLM call
sites.** Cold polyglot LLM-call count must remain at the Phase 2 PR-14
baseline (the same regression guard as Phase 4 PR-13). The phase is
deletion-shaped: ~600 LOC of production code, ~1,000 LOC of test code,
and ~30 lines of canonical design prose go away.

Two structural moves: **A — fold `atlas-contracts` in-tree** (snapshot
copy of `component-ontology` + `atlas-index` into `Atlas/crates/`,
website-content relocation, Ravel-Lite cross-repo coordination) and
**C — delete the multi-root machinery** (drop `expand_roots`, the
`--additional-root` CLI flag, `IndexConfig.additional_roots`, the
`roots.rs::best_root_for` helper, and collapse
`Workspace.roots: Vec<PathBuf>` to `Workspace.root: PathBuf`).

Total: 7 PRs (PR-0 plan + PR-1 fold + PR-2 drop discovery + PR-3
singularise Workspace + PR-4 salvage tests + PR-5 retext design + PR-6
acceptance). Sized between Phase 4's 9 PRs and Phase 3's 14.

The original Phase 5 scope as drafted in
`project_monorepo_consolidation` also included folding **Ravel**
(Elixir) and **Ravel-Lite** (Rust + vendored Lua) into Atlas. That
work is *deferred* to a later phase (post-Phase-5, slot TBD), possibly
tied to a build-system migration to Bazel for the polyglot tree. See
`project_phase5_split_and_ravel_bazel`.

---

## 2. Scope

### 2.1 In scope (Phase 5)

**A — atlas-contracts fold:**

- Snapshot copy `crates/component-ontology` and `crates/atlas-index`
  from `~/Development/atlas-contracts/` into `Atlas/crates/`. No git
  subtree-merge; plain directory copy with a single import commit.
- Add the two crates to Atlas's `[workspace] members`; delete the
  `[workspace.dependencies] component-ontology = { path = "../atlas-contracts/..." }`
  lines (Atlas `Cargo.toml` lines 45–50 today).
- Per-crate `release.toml` overrides keep them out of workspace-wide
  `cargo release` invocations. Both crates retain independent
  versioning at `0.1.0`; they continue publishing to crates.io from
  inside Atlas.
- Merge `atlas-contracts/website/` (which documents the schema
  crates) into Atlas's `website/` documentation tree at an appropriate
  location (PR-1 proposes the destination path).
- Diff `atlas-contracts/defaults/` against Atlas's `defaults/`; fold
  any unique content.
- **Cross-repo coordination:** Ravel-Lite's `Cargo.toml` lines 51 + 56
  (`path = "../atlas-contracts/crates/{component-ontology,atlas-index}"`)
  rewrite to `path = "../Atlas/crates/..."`. Separate-repo commit,
  landed alongside Atlas PR-1.

**C — multi-root retirement:**

- Delete `crates/atlas-engine/src/root_expansion.rs` (469 LOC).
- Delete `crates/atlas-engine/src/roots.rs` (61 LOC); inline
  `best_root_for(roots, path)` callers as
  `path.strip_prefix(workspace.root)`. No iterator stubs, no
  shim helpers.
- Delete the `--additional-root` CLI flag and its plumbing in
  `atlas-cli/src/main.rs` and `atlas-cli/src/pipeline.rs`.
- Delete `IndexConfig.additional_roots: Vec<PathBuf>` and the
  `roots()` accessor that built `[root] + additional_roots`.
- Collapse the Salsa input `Workspace.roots: Vec<PathBuf>` to
  `Workspace.root: PathBuf`. Update L3 / L4 / L8 / L9 call sites.
- Salvage `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs`
  (593 LOC) as `crates/atlas-cli/tests/contract_edge_in_workspace.rs`
  (~400 LOC), preserving AC#1–5 against a single-root sibling fixture.
- Delete `crates/atlas-engine/tests/multi_root.rs` (154 LOC) and
  `crates/atlas-engine/tests/multi_root_path_deps.rs` (742 LOC).
- Retext canonical system-model design `§5.3 Multi-root workspace`
  (delete + renumber), `§10` (mark Phase 5 shipped), `§10.1`
  Architectural seam callout (delete), and the glossary entry.

### 2.2 Out of scope (Phase 5)

- **Folding Ravel and Ravel-Lite into Atlas.** Deferred to a later
  phase (post-Phase-5, slot TBD); may include a build-system migration
  to Bazel.
- **New analyser, schema, or LLM-call work.** Phase 5 is structural
  cleanup only.
- **Backwards-compat with persisted multi-root `.atlas/` outputs.**
  Hard upgrade discipline: users delete `.atlas/` before upgrading.
  Documented in PR-6's status-file Upgrade notes.
- **Major-version bump on `atlas-engine`.** No external library
  consumers exist; the type change `Workspace.roots: Vec<PathBuf>`
  → `root: PathBuf` is an internal refactor.

---

## 3. PR enumeration

### 3.1 PR-0 — Plan + status scaffolding *(docs only)*

Files added:

- `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md` —
  implementation plan downstream of this spec, populated by
  `writing-plans`.
- `docs/superpowers/plans/2026-05-10-phase5-status.md` —
  checkbox-tracked PR status file, mirroring the Phase 3 / Phase 4
  format.
- `docs/superpowers/prompts/2026-05-10-vnext-continue.md` —
  continuation prompt for cross-machine resumption.

No code changes.

### 3.2 PR-1 — Fold A: atlas-contracts in-tree *(structural)*

Atlas-side changes:

- `crates/component-ontology/` ← snapshot copy from
  `~/Development/atlas-contracts/crates/component-ontology/`. Files
  preserved: `Cargo.toml`, `src/`, any `tests/`. New file:
  `crates/component-ontology/release.toml` (per-crate override so
  workspace-level `cargo release` excludes this member).
- `crates/atlas-index/` ← snapshot copy. Same shape. Per-crate
  `release.toml` override.
- `Cargo.toml` (workspace root):
  - Add `crates/component-ontology` and `crates/atlas-index` to
    `[workspace] members`.
  - **Rewrite** (not delete) the path-dep wiring at lines 45–50:
    `path = "../atlas-contracts/crates/component-ontology"` →
    `path = "crates/component-ontology"`, similarly for `atlas-index`.
    Drop the `# Public data-format / vocabulary crates...` comment
    block (it described the cross-repo arrangement that no longer
    exists) and replace with a one-line note that the two crates are
    workspace members published to crates.io.
  - Consumers (`atlas-cli`, `atlas-engine`, etc.) continue to pick the
    schema crates up via `component-ontology = { workspace = true }` /
    `atlas-index = { workspace = true }` — the per-crate consumer
    `Cargo.toml` files are unchanged.
  - **Inherited metadata change:** the schema crates' own
    `Cargo.toml`s use `repository.workspace = true`. After the fold,
    they inherit Atlas's `workspace.package.repository =
    "https://github.com/linkuistics/Atlas"` (replacing the prior
    `https://github.com/linkuistics/atlas-contracts` value from their
    old workspace). Reflected on the next crates.io publish.
- `release.toml` (workspace root): documented to defer to per-crate
  overrides for the two schema crates; remainder of workspace stays
  `publish = false`.
- `website/`: atlas-contracts website content (schema-crate docs)
  relocates into Atlas's docs tree. PR-1 description proposes the
  destination path (likely `website/docs/schema/` or similar) and
  enumerates the file list moved.
- `defaults/`: diff atlas-contracts's `defaults/` against Atlas's;
  fold any unique content; document the diff result in PR-1.

Cross-repo coordination (separate Ravel-Lite commit):

- `~/Development/Ravel-Lite/Cargo.toml` lines 51 + 56:
  `path = "../atlas-contracts/crates/{component-ontology,atlas-index}"`
  → `path = "../Atlas/crates/..."`. Lands in Ravel-Lite's main branch
  immediately after Atlas PR-1.

Acceptance for PR-1:

- `cargo build --workspace` clean.
- `cargo test --workspace --release` clean.
- Phase 3 polyglot smoke test green.
- `cargo publish --dry-run -p component-ontology` and
  `cargo publish --dry-run -p atlas-index` clean (output attached to
  PR description).
- Ravel-Lite local build clean (separately verified post-Ravel-Lite
  commit in `~/Development/Ravel-Lite/`).
- Website-merge resolution documented in PR description.

### 3.3 PR-2 — Drop discovery *(deletion + CLI surface change)*

Files deleted:

- `crates/atlas-engine/src/root_expansion.rs` (469 LOC).
- The corresponding `mod root_expansion;` line in
  `crates/atlas-engine/src/lib.rs`.

Files edited:

- `crates/atlas-cli/src/main.rs`: delete the `additional_roots:
  Vec<PathBuf>` field on `Args` (lines 67–79: doc comment +
  `#[arg(long = "additional-root")]` binding); delete the
  canonicalisation + plumbing block (lines 233–243).
- `crates/atlas-cli/src/pipeline.rs`:
  - Delete `IndexConfig.additional_roots: Vec<PathBuf>` (line 129)
    and the field-default in `Default for IndexConfig` (line 162).
  - Delete the `roots()` accessor (lines 176–183) which built
    `[self.root.clone()] + self.additional_roots`. Replace with
    direct use of `self.root`.
  - Delete the manual-`additional_roots` merge logic at lines
    284–319 and 816 (the `manual_iter` chains).
  - Update doc comments at lines 117–128, 1145, 1243 to describe the
    now-singular root.

Type changes: none yet — `Workspace.roots: Vec<PathBuf>` still holds
a length-1 vec at this PR boundary. PR-3 collapses the type.

Net diff: ~−500 LOC.

Acceptance: `cargo build --workspace` clean; `cargo test --workspace
--release` clean (multi-root tests are now redundant but still pass —
they get deleted in PR-4); polyglot smoke test green;
`--additional-root` produces a clap error.

### 3.4 PR-3 — Singularise `Workspace` *(type + call-site refactor)*

Files edited:

- `crates/atlas-engine/src/db.rs`:
  - `Workspace.roots: Vec<PathBuf>` → `Workspace.root: PathBuf`
    (line 72). The Salsa input macro signatures change accordingly
    (lines 189, 205).
  - Update doc comments at lines 10 + 105 to describe the singular
    root.
- `crates/atlas-engine/src/roots.rs`: **file deleted** (61 LOC).
  - `mod roots;` line in `lib.rs` deleted.
  - `best_root_for(roots, path)` inlined at every call site as
    `path.strip_prefix(workspace.root).ok()`.
- `crates/atlas-engine/src/l4_tree.rs`: rewrite the three multi-root
  id-derivation sites at lines ~397, ~582, ~810.
- `crates/atlas-engine/src/l8_recurse.rs`: rewrite the three
  manifest-path-resolution sites at lines ~65, ~96, ~278.
- `crates/atlas-engine/tests/l7_l8_fixedpoint.rs` line 86: comment
  update.
- Any other call site that iterates `workspace.roots()` — pattern is
  mechanical (replace iteration with direct field access).

Net diff: ~−150 LOC; ~20 sites touched.

Acceptance: `cargo build --workspace` clean; `cargo test --workspace
--release` clean; polyglot smoke test green; `git grep -E
'multi.root|multi-root|workspace\.roots' crates/` audit attached to PR
description with all surviving hits explicitly justified.

### 3.5 PR-4 — Salvage tests *(test suite surgery)*

Files added:

- `crates/atlas-cli/tests/contract_edge_in_workspace.rs` (~400 LOC).
  Rebuilt from `atlas_contracts_in_ravel_lite.rs`; preserves AC#1–5
  (component listing, contract-edge round-trip, surfaces emission,
  cache-hit on no-op rerun, edit-invalidates-only-affected-L5).
  Fixture is a single-root workspace with two sibling crates:
  `consumer` and a stand-in for the schema crate it depends on.

Files deleted:

- `crates/atlas-engine/tests/multi_root.rs` (154 LOC).
- `crates/atlas-engine/tests/multi_root_path_deps.rs` (742 LOC).
- `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` (593 LOC)
  — replaced by the salvaged version.

Net diff: −1,489 + 400 = ~−1,090 LOC.

Acceptance: `cargo test --workspace --release` clean with zero net
coverage loss versus the pre-PR-4 test suite. PR description maps each
of original AC#1–5 (lines 21–36 of the deleted file) to the
corresponding assertion in the new test; any assertion deemed
redundant must be explicitly justified, not silently omitted.

### 3.6 PR-5 — Retext canonical system-model design *(docs only)*

File edited: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`.

- **Delete §5.3 "Multi-root workspace"** in its entirety. Renumber
  subsequent §5.x sections (§5.4 → §5.3, etc.) for readability;
  inbound references in older Phase 1/2/3/4 spec/plan files are *not*
  retroactively updated (per the convention "specs are time-snapshots,
  not living documents").
- §10 "Roadmap": mark Phase 5 as **shipped** with date and PR-6's
  commit reference. Following Phase 4 PR-8's pattern.
- §10.1 "Architectural seam": delete the multi-root callout. If §10.1
  has other content, it stays; if multi-root was the only seam listed,
  the section is removed.
- Glossary: delete the "Multi-root workspace" entry (line ~1450).

File edited: `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`.

- Lines 19, 177, 253 reference §5.3 in the canonical design. Update
  prose to describe the post-Phase-5 single-root layout (overrides
  still operate against the engine's per-component layer, just no
  peer-root resolution).

Net diff: ~−40 lines net (delete §5.3 + glossary + §10.1 callout;
renumber §5.4+ → §5.3+).

PR-5 description includes the full text of the new §10 (the roadmap
entry for "Phase 5 — SHIPPED YYYY-MM-DD") so reviewers see the exact
prose before it lands.

### 3.7 PR-6 — Acceptance + closeout *(verification only)*

- Phase 3 polyglot smoke test (cold ≈ Phase 2 PR-14 baseline; warm =
  0). Same regression guard as Phase 4 PR-13 / Phase 3 PR-13.
- `git grep -E 'multi.root|multi-root|additional_root|expand_roots|best_root_for' crates/`
  returns zero non-test, non-deleted-file hits. Surviving prose may
  exist in *historical* spec/status files (Phase 1/2/3/4 status); those
  are intentionally left as-is.
- Six-file editorial tier preserved.
- `atlas-reports` stays pure-function.
- Status file (`docs/superpowers/plans/2026-05-10-phase5-status.md`)
  closeout entry: total LOC delta, PR enumeration, link to design.
  Includes an explicit **Upgrade notes** subsection documenting the
  hard upgrade discipline: users delete `.atlas/` before upgrading.
  No migration path. (Atlas has no `CHANGELOG.md` by convention; the
  per-phase status file is the canonical release-notes location.)

Manual post-merge steps (not gated by CI; tracked in status file):

1. Atlas Phase 5 closeout commit on `main`.
2. Ravel-Lite path-edit commit confirmed in its repo.
3. atlas-contracts GitHub repo: README updated to point at Atlas, then
   archive flag set in repo settings.
4. (User-side, not Atlas-side:) `rm -rf ~/Development/atlas-contracts/`.
5. (User-side:) any local `.atlas/` outputs from prior multi-root
   runs deleted.

Memory + roadmap updates (concurrent with PR-6):

- `project_phase4_plus_roadmap.md`: mark Phase 5 SHIPPED, add commit
  list, advance Phase 6 to "next up."
- `project_monorepo_consolidation.md`: mark "atlas-contracts in-tree"
  complete; remaining work (Ravel + Ravel-Lite) deferred to the
  post-Phase-5 polyglot fold.
- `project_phase5_split_and_ravel_bazel.md`: update with shipped
  state.

---

## 4. Post-Phase-5 architecture

What the engine looks like after the dust settles. This is the surface
future phases (6+) inherit.

### 4.1 Workspace API (singular)

```rust
// crates/atlas-engine/src/db.rs (post-Phase-5 shape)
#[salsa::input]
pub struct Workspace {
    pub root: PathBuf,
    // ...other fields unchanged...
}
```

No `roots()` accessor. No `Vec<PathBuf>`. No "primary vs peer"
distinction anywhere in the codebase. Salsa input setters take a
single `PathBuf`.

### 4.2 Pipeline flow (`atlas index`)

1. CLI canonicalises the path argument (defaults to cwd) → single
   `PathBuf`.
2. `IndexConfig { root: PathBuf, ... }` is constructed (no
   `additional_roots`, no `roots()` builder).
3. Salsa input `Workspace.root` is set.
4. Layers L1..L9 execute over that single root.
5. `.atlas/` outputs land under `<root>/.atlas/` (scattered `.atlas/`
   mode is unchanged — it's per-component, not per-root).

`expand_roots()` and the entire path-dep walking machinery are gone.
If a consumer crate path-deps a sibling crate inside the same
workspace, that sibling is discovered through the existing
Cargo-workspace member walk — no special case.

### 4.3 Path relativisation

Every layer that needs "where does this path live relative to the
workspace" calls `path.strip_prefix(workspace.root)`. Direct, no
helper. Sites that previously used `best_root_for(workspace.roots(),
path)` collapse to a one-liner per call.

### 4.4 Contract-edge family (single-root)

The Phase 1 PR-12 acceptance pattern still works in single-root mode:

- A consumer crate has a Cargo path-dep on a sibling crate (both
  workspace members of the same root).
- The rust-surface analyser detects the sibling's exported types
  (e.g., `pub struct Foo`) → emits a `data-format` contract under
  `<sibling-id>/foo`.
- L6's contract-edge batch emits a `consumes-contract` edge from the
  consumer to that contract id.
- The persistent cache keys off the participant-surface-sha
  (unchanged).

`contract_edge_in_workspace.rs` exercises exactly this flow with a
synthetic two-crate fixture.

### 4.5 CLI surface

```
atlas index [PATH]
  PATH    Path to the workspace root (defaults to current directory)
```

Removed: `--additional-root <PATH>` (passing it produces a clap
error). No other CLI surface changes.

### 4.6 External consumers

- **crates.io:** `component-ontology` and `atlas-index` continue
  publishing, now from inside Atlas. Independent versioning preserved.
  Per-crate `release.toml` overrides keep them out of workspace-wide
  `cargo release` invocations.
- **Ravel-Lite:** path-deps `../Atlas/crates/{component-ontology,atlas-index}`.
  Stays this way until the deferred polyglot-fold phase moves Ravel-
  Lite into the Atlas workspace, at which point the path-dep collapses
  to a sibling-member reference.
- **atlas-contracts on GitHub:** archived; README points at Atlas;
  serves as a historical reference for any inbound link.

### 4.7 What this unlocks

- **Phase 6** (user-facing schema cleanups): doesn't need to thread
  `Vec<PathBuf>` through any new code — every new touch operates on
  the singular root.
- **Phase 7** (per-language refinements): per-analyser code now sees a
  single root, simplifying classification logic that previously had to
  be peer-root-aware.
- **Phase 9** (LLM-driven analyses): impact-graph queries no longer
  need the "which root does this path belong to" disambiguation step.

---

## 5. Risks and coordination

### R1 — Ravel-Lite cross-repo race condition *(high impact, easy to mitigate)*

If Atlas PR-1 lands and the user later runs
`rm -rf ~/Development/atlas-contracts/`, Ravel-Lite's local build
breaks because lines 51 + 56 of its `Cargo.toml` still point there.
Symmetric risk if the Ravel-Lite path-edit lands *before* Atlas PR-1.

**Mitigation:** PR-1 lands as a coordinated commit pair, in this
order:

1. Atlas PR-1 commit (creates `Atlas/crates/{component-ontology,atlas-index}`).
2. *Immediately after*, Ravel-Lite-side commit updating its
   `Cargo.toml`.
3. *Only then* is `~/Development/atlas-contracts/` safe to remove.

PR-1's description enumerates these three steps as an explicit
checklist.

### R2 — crates.io publish-flow regression *(medium impact, medium effort)*

Schema crates change canonical home; `cargo release` semantics change
(workspace-wide `publish = false` plus per-crate overrides for two
members). First publish from the new home should be smoke-tested with
`cargo publish --dry-run` before PR-1 lands, to catch any `Cargo.toml`
metadata gap (description, license, repository, readme path) that the
move might surface.

**Mitigation:** PR-1 task list includes
`cargo publish --dry-run -p component-ontology` and
`cargo publish --dry-run -p atlas-index` from a clean checkout, with
output in the PR description. Per-crate `release.toml` overrides are
written in PR-1, not deferred.

### R3 — `website/` documentation relocation *(medium impact, surface only at PR-1)*

atlas-contracts's `website/` documents the schema crates themselves
(`component-ontology` and `atlas-index` API/schema docs). That content
becomes part of Atlas's documentation site under an appropriate
section. The work is content-relocation (where in the Atlas docs nav
does the schema-crate doc subtree belong), not navigation
deconfliction.

**Mitigation:** PR-1 proposes the destination path in its description
(likely `website/docs/schema/` or similar) so review focuses on
placement, not mechanics. If the relocation is substantial, it may
graduate to a separate PR-1.5 so the schema-fold portion isn't held up
by editorial review of website content.

### R4 — Salvaged-test coverage gap *(low impact, easy to verify)*

`contract_edge_in_workspace.rs` is a rewrite of
`atlas_contracts_in_ravel_lite.rs`. AC#1–5 should carry over verbatim,
but the rewrite might silently drop an assertion tied to multi-root
path semantics specifically (e.g., a path-canonicalisation check that
becomes vacuous in single-root and gets dropped without flagging the
lost coverage).

**Mitigation:** PR-4's description maps each of original AC#1–5 (lines
21–36 of the deleted file) to the corresponding assertion in the new
test; any assertion deemed redundant must be explicitly justified.

### R5 — Missed call site in PR-3 *(low impact, rustc enforces)*

`Workspace.roots: Vec<PathBuf>` → `root: PathBuf` is a type change.
Any call site that compiled against `Vec<PathBuf>` will fail
compilation; rustc enforces correctness. The risk is *semantic* sites
that compile but mean the wrong thing (e.g., a doc comment referencing
peer-root behaviour that's no longer accurate).

**Mitigation:** PR-3's description includes a
`git grep -E 'multi.root|multi-root|workspace\.roots' crates/` audit
pre- and post-edit, with all surviving hits explicitly justified.

### R6 — Persisted multi-root `.atlas/` outputs *(handled by upgrade discipline)*

Phase 5 makes a hard upgrade-discipline statement: users delete
`.atlas/` before upgrading. No backwards-compat with persisted
multi-root state, no version-aware decoder, no migration code. PR-6's
status file closeout includes an Upgrade notes subsection stating the
requirement.

### Coordination steps (in order)

| # | Step | When | By whom |
|---|---|---|---|
| 1 | Atlas PR-1 lands | Phase 5 PR-1 | engineer |
| 2 | Ravel-Lite Cargo.toml path-edit commit | Immediately after step 1 | engineer (separate repo) |
| 3 | Atlas PR-2 → PR-6 land in order | Phase 5 sequence | engineer |
| 4 | crates.io publish from Atlas (if a schema-crate version bump is wanted concurrent with Phase 5; otherwise next time the schema changes) | After PR-6 green | engineer, manual `cargo release` |
| 5 | Archive `atlas-contracts` repo on GitHub | After PR-6 green AND step 2 confirmed | repo admin (GitHub UI) |
| 6 | Remove `~/Development/atlas-contracts/` working copy locally | Any time after steps 1+2 | user |

---

## 6. Acceptance criteria

### 6.1 Behavioural invariants (must hold)

- **LLM-call budget unchanged.** Cold polyglot smoke test ≈ Phase 2
  PR-14 baseline. Warm/reports = 0. Phase 5 is deletion-only.
  Regression here is a hard failure.
- **Six-file editorial tier preserved.** No new files introduced;
  none removed.
- **`atlas-reports` stays pure-function.** No engine state mutation
  from the reports pathway.
- **Polyglot smoke test green** across every PR boundary, not just at
  PR-6.

### 6.2 Structural invariants (must hold)

- `git grep -E 'multi.root|multi-root|additional_root|expand_roots|best_root_for' crates/`
  returns zero non-test, non-deleted-file hits. Surviving prose may
  exist in historical spec/status files; those are intentionally left
  as-is.
- `Workspace` Salsa input has a singular `root: PathBuf` field; no
  `roots` field, no plural accessors.
- `crates/atlas-engine/src/{root_expansion.rs,roots.rs}` do not
  exist.
- `--additional-root` does not appear in `atlas index --help`.
- `Cargo.toml` lists `crates/component-ontology` and
  `crates/atlas-index` as workspace members; no
  `path = "../atlas-contracts/..."` strings remain.

### 6.3 Quantitative invariants (descriptive, not gating)

- Net production-code deletion: ≥ 600 LOC.
- Net test-code deletion: ≥ 1,000 LOC (after the salvaged test).
- Net documentation deletion: ~30 lines in canonical design.md.
- Status file closeout includes total-LOC summary and per-PR commit
  list.

### 6.4 Per-PR acceptance gates

| PR | Must hold |
|----|-----------|
| PR-0 | Plan + status file present; no code touched. |
| PR-1 | Workspace builds + tests clean; polyglot smoke test green; both `cargo publish --dry-run` clean; Ravel-Lite local build clean post-coordination commit; website-merge resolution documented. |
| PR-2 | Workspace builds + tests clean; polyglot smoke test green; `--additional-root` produces clap error. |
| PR-3 | Workspace builds + tests clean; polyglot smoke test green; multi-root vocabulary audit attached. |
| PR-4 | Workspace tests clean with zero net coverage loss; AC#1–5 mapped in PR description. |
| PR-5 | Docs-only; canonical design.md §5.3 deleted; §10 marks Phase 5 shipped with PR-6's eventual SHA. |
| PR-6 | All structural + behavioural invariants hold; status file closeout includes Upgrade notes (`.atlas/` deletion required). |

---

## 7. Canonical design.md retext (PR-5)

The canonical system-model design's §10 entry for Phase 5 reads, after
PR-5:

> **Phase 5 — Monorepo consolidation, part 1.** *SHIPPED YYYY-MM-DD.*
> Folded `atlas-contracts` (schema crates `component-ontology` and
> `atlas-index`) into the Atlas repo as workspace members; retired the
> multi-root architectural seam (deleted `expand_roots`, the
> `--additional-root` CLI flag, `IndexConfig.additional_roots`, and
> the `roots.rs::best_root_for` helper; collapsed `Workspace.roots:
> Vec<PathBuf>` to `Workspace.root: PathBuf`). Folding Ravel + Ravel-
> Lite into Atlas is deferred to a later phase (post-Phase-5, slot
> TBD), possibly tied to a Bazel build-system migration for the
> polyglot tree. Final commit: `<sha>`.

§5.3 "Multi-root workspace" is deleted; §5.4 "File layout" becomes the
new §5.3, etc. The §10.1 "Architectural seam" callout is deleted.
Glossary entry "Multi-root workspace" is deleted.

---

## 8. References

### Memories

- `project_phase4_plus_roadmap` — phase ordering after Phase 3 ships;
  Phase 4 SHIPPED 2026-05-09; Phase 5 next-up.
- `project_monorepo_consolidation` — long-term direction (now
  partially superseded by `project_phase5_split_and_ravel_bazel`).
- `project_phase5_split_and_ravel_bazel` — Phase 5 scoped to A + C
  only; Ravel/Ravel-Lite folding deferred.
- `feedback_user_low_git_history_value` — snapshot copy preferred
  over subtree-merge for the atlas-contracts fold.
- `feedback_no_iterator_stubs_for_singletons` — drives the aggressive
  shape of `Workspace.root: PathBuf` (no iterator accessor).
- `feedback_no_tail_pipe_for_long_tests` — applies to PR-3 / PR-4 /
  PR-6 polyglot test runs.
- `feedback_release_workspace_build_for_polyglot` — applies to PR-6
  polyglot smoke test setup.

### Prior phase specs

- `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` —
  canonical system-model design; §5.3 + §10 + §10.1 + glossary
  retexted in PR-5.
- `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`
  — three §5.3 references retexted in PR-5.
- `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md` —
  prior phase, established the deletion-shaped phase pattern Phase 5
  follows.

### Related code

- `crates/atlas-engine/src/db.rs` — `Workspace` Salsa input, modified
  in PR-3.
- `crates/atlas-engine/src/root_expansion.rs` — deleted in PR-2.
- `crates/atlas-engine/src/roots.rs` — deleted in PR-3.
- `crates/atlas-engine/src/{l4_tree.rs,l8_recurse.rs}` — call-site
  rewrites in PR-3.
- `crates/atlas-cli/src/{main.rs,pipeline.rs}` — CLI flag + plumbing
  removal in PR-2.
- `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` —
  salvaged in PR-4.
- `crates/atlas-engine/tests/{multi_root.rs,multi_root_path_deps.rs}`
  — deleted in PR-4.
