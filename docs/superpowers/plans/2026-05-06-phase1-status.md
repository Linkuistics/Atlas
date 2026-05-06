# Atlas vNext Phase 1 — Status

Companion to `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
This file tracks per-PR completion state across sessions. The session
prompt at `docs/superpowers/plans/2026-05-06-phase1-session-prompt.md`
reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-06 (PR-0 landed).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Companion specs (docs only) — *blocks PR-1+*
- [x] PR-1  — Schema definitions for new types
- [ ] PR-2  — Persistent content-addressed cache (no wiring)
- [x] PR-3  — Multi-root `Workspace` (Salsa input)
- [x] PR-4  — Path-dep root expansion to fixed point
- [ ] PR-5  — Plugin protocol + three reference analysers
- [ ] PR-6  — Scattered per-component `.atlas/` writers
- [ ] PR-7  — `surfaces.yaml` emission (Rust binding shape)
- [ ] PR-8  — Contract participants in `related-components.yaml`
- [ ] PR-9  — Composition edges from Dockerfiles
- [ ] PR-10 — Wire persistent cache into L3 / L5 / L6
- [ ] PR-11 — L6 cache key includes participant surface shas
- [ ] PR-12 — Acceptance: atlas-contracts visible in Ravel-Lite

When every box is `[x]`, Phase 1 is complete and the session prompt
should report success and stop.

## Dependency graph (informational; canonical in plan §4)

```
PR-0 ─┬─> PR-1 ─┬─> PR-5 ─┬─> PR-7 ─> PR-8
      │         │         ├─> PR-9
      │         │         └─> PR-10 ──> PR-11
      │         └─> PR-2 ─/                │
      │                                    │
      └─> PR-3 ──> PR-4                    │
                  └─> PR-6 ──> PR-7        │
                                           ▼
                            PR-12 (depends on everything)
```

Parallel-safe waves:
- Wave 1 (after PR-0): PR-1, PR-2, PR-3 concurrently.
- Wave 2 (after PR-1, PR-3): PR-4, PR-5, PR-6 concurrently.
- Wave 3 (after PR-2 + PR-5 + PR-6): PR-7 (then PR-8), PR-9, PR-10.
- Wave 4: PR-11 (after PR-7 + PR-10).
- Wave 5: PR-12 (after all).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples
of what's worth recording: deviations from the plan that the next
session needs to know, surprising fixture quirks, manual verification
steps that succeeded, follow-up cleanup deferred.

### PR-0
2026-05-06 — Landed in same commit as the plan/design/status/session-prompt
docs (all were untracked from the prior planning session). Two new specs:
`2026-05-06-contract-content-sha-canonicalisation.md` (resolves §11.2.2)
and `2026-05-06-override-scoping-scattered-atlas.md` (resolves §11.2.3).
Memory entry `feedback_phase1_open_questions` records the closure.

Load-bearing for downstream review: PR-7 must implement both branches of
the canonicalisation algorithm (§2.1 byte-range for Phase 1 code-derived,
§2.2 canonical YAML for the test-only schema-derived fixture). PR-6 must
emit the per-component-scoping warning in the format spelled out in §6 of
the override-scoping spec.

### PR-1
2026-05-06 — Landed in `/Users/antony/Development/atlas-contracts` as
three commits: `34fc2f9` (initial), `bef736d` (spec fixup —
Contract.fingerprint field rename, nested subprocess config), `2d8c54c`
(quality fixes — LibraryApi validate, fixture pinning, AtlasConfigFile
default hygiene).

Atlas main repo is now red until PR-3 lands. Expected.

Notes for downstream PRs:
- `Stage` (kebab-case lowercase: `l1`..`l9`), `CostClass`, `Confidence`,
  `Transport`, `SubprocessConfig`, `AnalyzerSpec` available from
  `atlas_index::analyzers`. PR-2 imports `Stage` from here.
- `ComponentsFile.roots: Vec<PathBuf>` (singular `root` deleted). PR-3
  must adopt this; every Atlas-side consumer of `ComponentsFile.root`
  is currently broken.
- `ComponentEntry.languages: BTreeSet<String>` (singular `language`
  deleted). PR-3 must adopt.
- `EdgeKind` extended with `DefinesContract`, `ImplementsContract`,
  `ConsumesContract`, `BundledInto`, `PublishedAs`, `DeployedWith`,
  `ReleasedWith`, `BundledFromExternal`. `Orchestrates` already existed.
  PR-8 / PR-9 consume these.
- `LifecycleScope::Release` does NOT exist; `published-as` and
  `released-with` are tagged `deploy` lifecycle in the ontology YAML.
  Design §3.5 table text says `release`, but the ontology is canonical.
  Future work can add `Release` if needed.
- `CacheFingerprints.analyzer_registry_sha` is NOT yet added (deferred
  to PR-5). PR-5 must add it before any writer lands.
- `LibraryApi::validate()` enforces `kind == LibraryApi`. PR-5/PR-7
  callers should `validate()` before serialising.
- `AnalyzerSpec::validate()` rejects (Subprocess, None) and (InProcess,
  Some) pairs. PR-5 callers should validate.

### PR-2
(none yet)

### PR-3
2026-05-06 — Landed on Atlas main as a single commit (see git log).
Atlas main repo went from red (post-PR-1 contract break) back to green:
the Atlas-side schema adoption, multi-root Salsa input rename, and per-root
L1/L2/L3/L4/L9 walks all flowed through this PR.

Key shape changes downstream PRs depend on:
- `Workspace.roots: Vec<PathBuf>` is the canonical shape; `set_roots`
  is the setter; `Workspace::primary_root(db)` returns `roots[0]` for
  single-root call paths.
- `AtlasDatabase::new(backend, roots: Vec<PathBuf>, fp)` — `roots` must
  be non-empty (asserted at construction).
- `seed_filesystem(db, &[PathBuf], respect_gitignore)` and
  `seed_filesystem_excluding(db, &[PathBuf], &[PathBuf], respect_gitignore)`
  take slices. Single-root convenience wrappers
  (`seed_filesystem_one`, `seed_filesystem_excluding_one`) are exported
  for tests / out-of-tree callers.
- `Classification.languages: BTreeSet<String>` everywhere; the v1
  `language: Option<String>` field is gone. Pin form is unchanged
  (`pins[id]["language"]: Value`); the engine widens it to a
  one-element set on read.
- `IndexConfig.additional_roots: Vec<PathBuf>` plumbs the multi-root
  set into the CLI; the new `--additional-root` repeated flag exposes
  it. PR-4 will populate it automatically; today it's manual.

PR-4 owns the path-dep walk that auto-populates `additional_roots`.
PR-4 also owns `<output>/.atlas/config.yaml#roots` persistence (mentioned
under PR-4 in the plan).

Quality fixes follow-up at commit `09f19e4`:
- New shared helper `crates/atlas-engine/src/roots.rs::best_root_for(&[PathBuf], &Path) -> Option<&Path>`. Re-exported from `lib.rs`. Three identical longest-prefix matchers (in `l3_classify.rs`, `l8_recurse.rs`, and the misnamed `best_matching_root` in `l9_projections.rs`) are consolidated onto it. Future PRs adding root-disambiguation logic should extend this helper, not introduce new copies.
- `external_components_yaml_snapshot` `discovered_from` dedup now uses `BTreeSet` (was O(n²)).
- `pipeline.rs` per-root exclusion-vector empty-`PathBuf` sentinel documented.

### PR-4
2026-05-06 — Landed on Atlas main as a single commit. Implements the
fixed-point path-dep walk and config.yaml#roots persistence.

Public API (atlas-engine):
- `expand_roots(primary: &Path) -> anyhow::Result<Vec<PathBuf>>`,
  re-exported from `lib.rs`. Always returns the canonicalised primary
  as element 0; peer roots follow in discovery (BFS) order.
- `manifest_parse::extract_path_deps(contents: &str) -> Vec<PathBuf>`
  reads `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
  and `[workspace.dependencies]`. Malformed manifest degrades to empty
  Vec (matches `parse_cargo_toml`'s policy).

Walk semantics: per-root recursive `Cargo.toml` enumeration (skipping
`target/`, `node_modules/`, `.git/`); for each path-dep, canonicalise
the resolved target; if it's inside any known root, skip (the existing
root's L1 walk covers it); otherwise walk up to the enclosing
`[workspace]` manifest (or fall through to the crate's own directory)
and add the canonicalised manifest-root as a peer. Visited-set guard
on canonical paths terminates cycles cleanly with a `warning:` line on
stderr.

Pipeline integration (`crates/atlas-cli/src/pipeline.rs`):
- `run_index` now calls `expand_roots(&config.root)` before
  constructing the database. Discovered peer roots are merged with the
  manual `--additional-root` set; manual paths come first (they're the
  user's explicit ordering choice), auto-discovered peers follow,
  dedup is by canonicalised path via `BTreeSet`. The `--additional-root`
  CLI flag remains as the manual escape hatch (paths with no
  path-dep edge, e.g. a sibling docs repo).
- After the merge, the discovered root set is persisted to
  `<output>/.atlas/config.yaml#roots` via `serde_yaml`. The file is
  load-or-default-then-overwrite-`roots` so user-authored fields
  (`operations`, `override_search`) survive. Atomic write
  (tempfile-then-rename). Persistence failure is non-fatal (warning on
  stderr; the run continues).
- `--dry-run` skips the persistence step.

Tests:
- `crates/atlas-engine/tests/multi_root_path_deps.rs` (8 tests):
  single-root no-op, two-root path-dep, two-root with workspace-at-target,
  short cycle (back-into-primary), cross-tree cycle (visited-set guards
  the second-pass), missing path-dep skipped, in-primary path-dep not
  promoted, transitive a→b→c chain across three trees.
- New unit tests in `manifest_parse::tests` for `extract_path_deps`
  (no deps, registry-only, dependencies, dev/build, workspace, git/version
  skip, malformed-input).

Concurrency note: PR-2 (persistent cache) was dispatched in parallel.
The `lib.rs` re-exports for `expand_roots` and PR-2's
`PersistentCache`/`FingerprintBuilder` co-exist in the file naturally
(both add their own `pub mod` line and `pub use` re-export). No merge
conflict observed at this point; the controller still owns final
resolution if PR-2 lands separately.

### PR-5
(none yet)

### PR-6
(none yet)

### PR-7
(none yet)

### PR-8
(none yet)

### PR-9
(none yet)

### PR-10
(none yet)

### PR-11
(none yet)

### PR-12
(none yet)
