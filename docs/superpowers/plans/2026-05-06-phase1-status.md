# Atlas vNext Phase 1 — Status

Companion to `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
This file tracks per-PR completion state across sessions. The session
prompt at `docs/superpowers/plans/2026-05-06-phase1-session-prompt.md`
reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-07 (PR-5 + PR-6 + PR-7 + PR-8 + PR-9 + PR-10 landed).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Companion specs (docs only) — *blocks PR-1+*
- [x] PR-1  — Schema definitions for new types
- [x] PR-2  — Persistent content-addressed cache (no wiring)
- [x] PR-3  — Multi-root `Workspace` (Salsa input)
- [x] PR-4  — Path-dep root expansion to fixed point
- [x] PR-5  — Plugin protocol + three reference analysers
- [x] PR-6  — Scattered per-component `.atlas/` writers
- [x] PR-7  — `surfaces.yaml` emission (Rust binding shape)
- [x] PR-8  — Contract participants in `related-components.yaml`
- [x] PR-9  — Composition edges from Dockerfiles
- [x] PR-10 — Wire persistent cache into L3 / L5 / L6
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
2026-05-06 — Landed on Atlas main as `4d52431` (initial) + `597ade3`
(quality fixes — GC race counting, FingerprintBuilder Clone removal,
Sha256Hex invariant doc).

Public API available from `atlas_engine`:
- `PersistentCache::open(&Path) -> Result<Self>`
- `PersistentCache::{get, put, gc, root}` — content-addressed store at
  `<root>/cache/<stage>/<sha>.blob` (caller passes the full cache root,
  e.g. `<output>/.atlas/cache`).
- `FingerprintBuilder::new(stage, analyzer_id, analyzer_version)` plus
  five `add_*` methods (file_content_sha, prompt_sha, llm_fingerprint,
  participant_surface_sha) and `finalise(self) -> Sha256Hex`. Tag-byte
  framing + BTreeSet accumulator give order-independent fingerprints.
  No `Clone` derive — single-shot by design.
- `GcReport { kept, removed, bytes_freed }`, `Sha256Hex = String`.

**Spec deviation noted, deferred to a future PR:** the plan signature
is `gc(&BTreeSet<(Stage, Sha256Hex)>)` but `Stage` from atlas-contracts
only derives `Hash + Eq`, not `Ord`. PR-2 ships `gc(&HashSet<...>)` and
documents the divergence. The first PR that touches atlas-contracts
again should add `#[derive(PartialOrd, Ord)]` to `Stage` and restore
the `BTreeSet` signature in atlas-engine.

PR-10 wires the cache into L3/L5/L6; PR-2 itself does no wiring.

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
(both add their own `pub mod` line and `pub use` re-export). PR-2
landed first as `4d52431`; PR-4 cleanly stacked at `8e60d32`.

Quality fixes follow-up at commit `b96977f`:
- `enclosing_manifest_root` now binds to the **innermost** enclosing
  `[workspace]` Cargo.toml (matching Cargo's own resolution rule), not
  the outermost. Two independently-versioned workspaces under a common
  parent are now correctly kept separate.
- `expand_roots` API split: now also exposes
  `expand_roots_with_warnings(primary, &mut dyn Write)` so tests can
  capture the cycle warning without process plumbing. The plain
  `expand_roots` is a thin wrapper writing to stderr.
- New test `cycle_between_two_peers_emits_warning` exercises the
  visited-set cycle branch with two true peer roots and asserts the
  warning text appears in the captured stream.
- Manual `--additional-root` paths that fail canonicalisation now emit
  a stderr warning and are skipped (was: silently inserted in
  non-canonical form, breaking dedup against auto-discovered roots).
- Cycle-warning emission moved off `eprintln!` and onto the writer
  argument; pipeline.rs's `persist_discovered_roots` warning still uses
  `eprintln!` (its own concern, not in PR-4 scope).
- Dropped the unreachable `is_inside_any(target)` early-exit; the
  defence-in-depth `is_inside_any(candidate, &result)` guard remains
  for benign workspace-member aliasing.

Carry-over for downstream PRs:
- PR-5 onwards do NOT need to know about `expand_roots_with_warnings`
  unless they want to plumb their own warning sinks. The default
  `expand_roots` is the call site.
- The `<output>/.atlas/config.yaml` writer in pipeline.rs uses
  `AtlasConfigFile` from atlas-contracts (PR-1's type) and is the
  canonical config-write site. PR-5+ adding new config keys should
  extend `AtlasConfigFile`, not introduce parallel YAML writers.

### PR-5
2026-05-07 — Landed as two stacked commits:
1. atlas-contracts: `Stage` gains `PartialOrd + Ord`;
   `CacheFingerprints.analyzer_registry_sha: String` (with
   `#[serde(default)]`) + docstring pinning the
   `sha256(serde_yaml::to_string(&AnalyzersFile))` computation.
   Tests + golden snapshot updated.
2. Atlas: new crate `atlas-analyzers`, plus engine/CLI wiring,
   plus FingerprintBuilder.add_analyzer_registry_sha + GC mark-set
   restored to `BTreeSet<(Stage, Sha256Hex)>`.

**New public API (from `atlas_analyzers::*`):**
- `Analyzer` trait — `id() / stage() / cost_class() / version() /
  applies() / fingerprint_inputs() / analyse()`.
- `AnalyzerRegistry::builtin / empty / register / len / is_empty /
  iter_dispatch_order / analyzers_for_stage / merge_yaml /
  registry_sha / declared / dispatch / dispatch_with_filter`.
- `DispatchOutcome { Confident, Graded, AllDeclined { errors } }`.
- `AnalyzerResult { Confident, Graded, Declines, Error }`.
- `AnalyzerError { MalformedInput, CallFailed, Internal }`.
- `Target { dir, languages, manifests, top_level_files }` +
  `TargetFile { name, relpath, bytes, content_sha }`.
- `AnalysisContext::deterministic_only / with_llm`.
- `FingerprintInput { FileContentSha, Custom { tag, bytes } }`.
- `StageOutput` marker trait + `impl_stage_output!` macro (must be
  invoked by every external crate that registers its own analyser
  output type — a blanket impl was deliberately rejected because
  `Box<dyn StageOutput>` would itself satisfy a blanket impl,
  breaking downcasts).
- Three reference analysers: `CargoClassifier`,
  `DockerfileClassifier`, `LlmClassifyAnalyzer` plus their output
  structs (`CargoClassificationOutput`,
  `DockerfileClassificationOutput`, `LlmClassifyOutput`).
- `LlmHook` trait (`Send` only — *not* `Sync` — see below) with
  `LlmHookError { Setup, Call }`.
- `REGISTRY_HASH_NAMESPACE` const.

**New atlas-engine wiring:**
- `AtlasDatabase::new_with_registry(backend, roots, fp,
  Arc<AnalyzerRegistry>)` — explicit-registry constructor.
  `AtlasDatabase::new` now defers to it with
  `AnalyzerRegistry::builtin()`.
- `AtlasDatabase::analyzer_registry() -> &Arc<AnalyzerRegistry>`.
- `FingerprintBuilder::add_analyzer_registry_sha(&Sha256Hex)` (tag
  byte `0x05`). Tag table in `cache/fingerprint.rs` updated.
- `PersistentCache::gc(&BTreeSet<(Stage, Sha256Hex)>)` — `Stage` now
  derives `Ord`, so the plan's literal API is honoured. (PR-2
  shipped `&HashSet<...>` and noted the temporary divergence.)

**Carry-over obligations paid off:**
- PR-1 status note: `CacheFingerprints.analyzer_registry_sha` —
  added in atlas-contracts with `#[serde(default)]` for backward-
  compat parsing; populated by `l9_projections::components_yaml_snapshot`.
- PR-2 status note: `BTreeSet` GC signature restored.

**Deviations from the brief (PR-6+ reviewers please note):**
- `AnalyzerRegistry::dispatch` takes an extra `stage: Stage` filter
  not shown in the brief's stub. This is necessary because PR-7
  will add L5 analysers to the same registry — without a stage
  filter the L3 driver would dispatch them and produce nonsense
  outputs. Same reason for the `dispatch_with_filter` helper
  (cost-class filter), which the L3 adapter uses to interleave the
  legacy non-Cargo deterministic rules between the registry's
  deterministic and LLM passes.
- `dispatch` returns `DispatchOutcome` rather than the brief's
  `AnalyzerResult`. The dispatcher accumulates errors that a single
  analyser would lose; a single `AnalyzerResult` value cannot
  express "all declined, here are the errors I saw along the way".
  Functionally equivalent at the L3 adapter — the adapter
  translates `AllDeclined` into "fall through".
- `LlmHook: Send` (not `Send + Sync`) because the engine-side
  hook impl closes over a `!Sync` `AtlasDatabase` (Salsa's
  `ZalsaLocal` is `!Sync`). The hook is constructed and used
  on the same thread inside `is_component`. Clippy flagged the
  resulting `Arc<dyn LlmHook>` wrap with `arc_with_non_send_sync`;
  the call site has a documented `#[allow]`.
- `StageOutput` is *not* a blanket-impl trait. Adding a blanket
  impl over `T: Any + Send + Sync` made `Box<dyn StageOutput>`
  itself satisfy the trait, and method resolution silently routed
  `boxed.as_any()` through the box rather than the inner value —
  downcasts silently failed. The `impl_stage_output!` macro is the
  one stamping path; every concrete output type must opt in.
  External crates that register their own analyser outputs must
  call `atlas_analyzers::impl_stage_output!(MyOutput);` at the
  type's definition site. This is documented on the trait.

**L3 driver flow (now):**
1. Pin short-circuit (unchanged).
2. Registry dispatch — deterministic cost classes only (Cargo,
   Dockerfile). Adapter downcasts to `*ClassificationOutput` and
   builds a `Classification`.
3. Legacy non-Cargo deterministic rules in `heuristics::classify_deterministic`
   (npm, pyproject, bare-git). The Cargo rules previously here have
   moved into `CargoClassifier`.
4. Registry dispatch — LLM cost classes only. The
   `LlmClassifyAnalyzer` (the only LLM analyser shipped in PR-5)
   wraps an engine-side `EngineLlmHook` that routes through
   `db.call_llm_cached(...)`, preserving v1 LLM behaviour.
5. Weak-unknown fallback.

**Cache-wiring deferral for PR-10:**
- `is_component` computes an L3 fingerprint via
  `FingerprintBuilder::new(L3, "l3-driver", "1.0.0")` +
  `add_analyzer_registry_sha` + `add_llm_fingerprint` +
  `add_file_content_sha` per loaded manifest, and stores the
  result in a `_l3_fingerprint` binding. **The fingerprint is not
  yet consulted to skip the call** — that's PR-10's job. The
  binding is named with a leading underscore so the unused-variable
  lint stays quiet without reaching for `#[allow]`. PR-10 should
  swap the binding into a `cache.get(L3, &fp)?` short-circuit.
- Analyser `fingerprint_inputs` is callable and deterministic on
  every PR-5 analyser. PR-10 will pull these into the dispatcher's
  fingerprint loop; PR-5 calls `analyse` directly.

**Per-PR notes for PR-6 reviewers:**
- The new `<output>/.atlas/analyzers.yaml` reader lives in
  `atlas-cli/src/pipeline.rs`; PR-6's per-component scattered
  layout will need a similar `<component>/.atlas/analyzers.yaml`
  read path *if* per-component analyser overrides land in Phase 1
  (not currently planned — `analyzers.yaml` is workspace-wide for
  PR-5).
- The `AnalyzersFile.merge_yaml` path emits a stderr warning for
  invalid specs; PR-6's `--strict-overrides` flag (Phase 2 per the
  spec) is the place to escalate to a hard error.

**Files created:**
- `crates/atlas-analyzers/Cargo.toml`
- `crates/atlas-analyzers/src/lib.rs`
- `crates/atlas-analyzers/src/registry.rs`
- `crates/atlas-analyzers/src/dispatcher.rs`
- `crates/atlas-analyzers/src/cargo_classifier.rs`
- `crates/atlas-analyzers/src/dockerfile_classifier.rs`
- `crates/atlas-analyzers/src/llm_classify.rs`

**Files modified (Atlas):**
- `Cargo.toml` — `atlas-analyzers` added to members.
- `crates/atlas-engine/Cargo.toml` — `atlas-analyzers` dep.
- `crates/atlas-engine/src/db.rs` — registry field + accessor +
  `new_with_registry`.
- `crates/atlas-engine/src/cache/mod.rs` — GC `BTreeSet` signature.
- `crates/atlas-engine/src/cache/fingerprint.rs` —
  `add_analyzer_registry_sha` + tag table + tests + proptest.
- `crates/atlas-engine/src/heuristics.rs` — Cargo rules removed
  (moved to `CargoClassifier`).
- `crates/atlas-engine/src/l3_classify.rs` — registry dispatch
  driver.
- `crates/atlas-engine/src/l9_projections.rs` —
  `analyzer_registry_sha` field populated.
- `crates/atlas-cli/Cargo.toml` — `atlas-analyzers` dep.
- `crates/atlas-cli/src/pipeline.rs` — `analyzers.yaml` load and
  `AtlasDatabase::new_with_registry` call.
- `crates/atlas-engine/tests/l2_l3_queries.rs` — two new
  acceptance tests (`pr5_cargo_lib_classified_via_registry_without_llm`,
  `pr5_dockerfile_classified_as_docker_image_without_llm`).

**Files modified (atlas-contracts):**
- `crates/atlas-index/src/analyzers.rs` — `Stage` derives
  `PartialOrd + Ord` + total-order test.
- `crates/atlas-index/src/schema.rs` —
  `CacheFingerprints.analyzer_registry_sha` field + tests.
- `crates/atlas-index/src/yaml_io.rs` — fixture updated.
- `crates/atlas-index/tests/golden_snapshots.rs` — fixture updated.
- `crates/atlas-index/tests/snapshots/components.yaml` — golden
  output now includes `analyzer_registry_sha:` line.

### PR-6
2026-05-07 — Landed on Atlas main as a single commit `3c1b518`.

**New public API (atlas-engine):**
- `per_component_yaml_snapshot(db, component_id) -> anyhow::Result<Arc<PerComponentFile>>`
  — single-component projection plus envelope (`schema_version`,
  `surfaces_path`, `overrides_path`, `analyser_id`, `analyser_version`,
  `fingerprint`). Phase 1 placeholders: `analyser_id="l3-driver"`,
  `analyser_version=L3_DRIVER_VERSION` (`"1.0.0"`),
  `fingerprint=sha256(serde_yaml::to_string(&entry))`. PR-7 swaps
  these for per-analyser identity (plumbed through L3 dispatch) and
  the surfaces.yaml fingerprint per design §6.2.
- `L3_DRIVER_VERSION` constant — exported so PR-7 can read the prior
  value when computing the fingerprint lineage.
- `try_assemble_with_warnings(db, &mut dyn Write)` — sibling to
  `try_assemble` that takes a warning sink, mirroring PR-4's
  `expand_roots_with_warnings`. The plain `try_assemble` is now a
  thin wrapper writing to stderr.
- `TreeAssemblyError::PerComponentScopeViolation { file, offending_id, owner_prefix }`
  and `TreeAssemblyError::PerComponentParseError { file, message }`
  — new variants for spec §5 (cross-component pin rejected) and a
  malformed per-component file (read or parse error).

**Override-merge structural changes (`l4_tree.rs`):**
- `try_assemble_with_warnings` reads `workspace.components_overrides`
  (the CLI-installed primary-root file) and feeds it as the **first**
  tier into a new `merge_overrides_in_discovery_order` helper. The
  helper walks the spec §3 three-tier order:
  1. primary-root top-level (received as the workspace input).
  2. peer-root top-level — `<peer-root>/.atlas/components.overrides.yaml`
     for every `roots[i]` with `i >= 1`, lex-sorted by canonical
     absolute path.
  3. per-component — `<dir>/.atlas/overrides.yaml` files discovered
     via a direct filesystem walk (`find_per_component_overrides_under`)
     under each root, path-sorted (a deterministic stand-in for the
     spec's "id-sorted" rule, which would require post-assembly ids).
  Per-component files are validated against §5 scoping at discovery
  time. The implied owner prefix is computed in two forms (path-only
  and root-basename-prefixed) and an entry id must match either as
  `id == prefix` or `id.starts_with(format!("{prefix}/"))`. A
  violation is `TreeAssemblyError::PerComponentScopeViolation` —
  always a hard error per spec §5.
- The merge itself is last-writer-wins, keyed by `(component_id, key)`
  for pins and by `component_id` for additions. Conflicting values
  (two contributors with distinct values) emit a §6-format warning
  to the sink: `warning: override conflict on (id, key):` followed
  by one line per contributor (labelled `primary`/`peer`/
  `per-component`) and a `resolved value:` summary. `eprintln!`-
  via-stderr is the production sink; tests pass a `Vec<u8>`.

**Filesystem-walk exclusion (`ingest.rs`):**
- `seed_filesystem_inner` now universally prunes every `.atlas/`
  directory from the walk (regardless of root). PR-6's per-component
  writers create `<component>/.atlas/component.yaml` and PR-7+ adds
  more; without this exclusion the second run would treat each
  `<component>/.atlas` as an immediate sub-dir candidate (L8's
  enumerator) and feed prior outputs into L0. The override-merge
  walk reads per-component files via a direct filesystem walk, so
  it is not affected by this exclusion.
- The existing per-root `excluded_dir` mechanism (PR-3) is preserved
  alongside the new `.atlas/` prune; both run inside one
  `filter_entry` closure.

**Per-component writer (`pipeline.rs`):**
- New `write_per_component_files(db, &components_file, &roots)` is
  invoked after the top-level YAML writes and before the LLM-cache
  save. Per-component write failures are non-fatal warnings on
  stderr (same policy as PR-4's config.yaml writer); the top-level
  `components.yaml` is the canonical source.
- Atomic write via tempfile-then-rename inside the component's
  `.atlas/` directory (`mkdir -p` per component).
- `--dry-run` skips the per-component writes (the walk lives inside
  the `if !config.dry_run` block, alongside the top-level saves).

**Carry-over for PR-7 reviewers:**
- The same `write_per_component_files` walk pattern (`for entry in
  &components_file.components` → resolve `<root>/<segment[0].path>`
  → `mkdir -p <dir>/.atlas` → atomic write) is the canonical
  template for `surfaces.yaml` emission. PR-7 should add a sibling
  walk inside the same `if !config.dry_run` block (or extend
  `write_per_component_files` to emit both files in one pass).
- The placeholder fingerprint computed in `per_component_yaml_snapshot`
  (sha256 of the entry's canonical YAML) is correct per spec §6.2's
  letter (it does change when classification or path changes), but
  the surfaces fingerprint is the long-term value. PR-7's reviewer
  should swap the body of the `let entry_yaml = ...` block for
  `let surfaces = surfaces_yaml_snapshot(db, component_id); let
  fingerprint = surfaces.fingerprint.clone();`.
- `L3_DRIVER_VERSION` is a public const so PR-7 can compute the
  lineage diff.

**Deviations from the plan/spec:**
- The brief suggests deriving the per-component scoping prefix as
  "the portion before the first `/`" (manifest-root namespace). The
  implementation is stricter: it computes both the path-derived form
  and the root-basename-prefixed form and accepts either. This is
  more conservative — it actually enforces §5 against sibling-of-
  the-owner pins, where the manifest-root-namespace check would
  let them through. The change is invisible to the spec's test
  obligations (all four listed tests pass with either rule) and the
  stricter form is what the spec's worded text describes.
- Per-component override discovery walks the filesystem directly
  rather than using `Workspace.files`. The brief outlines both
  approaches; the filesystem walk is necessary because
  `seed_filesystem` excludes `.atlas/` directories (see above). The
  walk lives in `find_per_component_overrides_under` and only
  descends into the root tree, skipping `.git/`, `target/`, and
  `node_modules/` for performance.

**Debt deferred:**
- `fingerprint` field on `PerComponentFile` is the entry-yaml-sha
  placeholder; PR-7 lands the surfaces.yaml fingerprint.
- `analyser_id`/`analyser_version` are static (`"l3-driver"` /
  `"1.0.0"`); PR-7 swaps to per-analyser identity.
- The `--strict-overrides` flag (escalating warnings to errors per
  spec §6) is Phase 2.
- Per-component overrides are read directly from disk (not through
  Salsa input). A future PR that wants Salsa-tracked override
  discovery could swap the `find_per_component_overrides_under`
  walk for an L1-style tracked query — the per-component file
  contents would need to be ingested via a separate registration
  path because they live inside the universally-excluded `.atlas/`.

### PR-7
2026-05-07 — Landed on Atlas main as `6705e1c`.

**New public API (atlas-engine):**
- `surfaces_yaml_snapshot(db, &ComponentId) -> anyhow::Result<Arc<SurfacesFile>>`
  — projects onto PR-1's `SurfacesFile` schema. Aggregate
  `SurfacesFile.fingerprint` is computed via the schema-derived
  canonicaliser (spec §2.2) over the file with `fingerprint=""` zeroed.
  Errors when the component id does not resolve. Re-exported from
  `lib.rs`.
- `surface_artefacts_of(db, ComponentId) -> Arc<SurfaceArtefacts>` —
  outer wrapper around `SurfaceRecord` (kept unchanged) carrying the
  three new top-level fields `contracts`, `bindings`, `library_apis`.
  Reads `src/lib.rs` and `src/main.rs` per path-segment and runs the
  deterministic `extract_rust_surface` extractor. The original
  `surface_of` and `SurfaceRecord` shapes are untouched, so PR-10's
  Salsa-tracked L5 caching still works without re-keying.
- New module `crates/atlas-engine/src/contract_canonicalisation.rs`:
  - `code_derived_content_sha(bytes, span) -> String` (spec §2.1).
  - `canonicalise_yaml(input) -> Result<String>` (spec §2.2).
  - `schema_derived_content_sha(yaml) -> Result<String>` — `sha256(canonicalise_yaml(yaml))`.
  - `compute_surfaces_fingerprint(&SurfacesFile) -> String` —
    PR-11's single-call-site for the surfaces fingerprint algorithm.

**New public API (atlas-analyzers):**
- `RustSurfaceAnalyzer` (registered by `AnalyzerRegistry::builtin()`;
  the registry now ships four reference analysers).
- `RustSurfaceOutput { contracts, bindings, library_apis }` — opted
  into `StageOutput` via `impl_stage_output!` (no blanket impl, per
  PR-5 status note).
- `extract_rust_surface(component_id, lib_rs_bytes, main_rs_bytes,
  lib_rs_relpath, main_rs_relpath) -> RustSurfaceOutput` — the L5
  driver calls this directly per the brief's option (b); the analyser's
  `Analyzer::analyse` returns `Declines` and is reserved for a future
  Phase 2 dispatcher path.
- `ANALYZER_ID = "rust-surface-analyzer"`, `ANALYZER_VERSION = "1.0.0"`.

**Per-component fingerprint placeholder swap (paid off PR-6 hand-off):**
- `per_component_yaml_snapshot.fingerprint` now equals
  `surfaces_yaml_snapshot(...).fingerprint`. PR-6's
  sha256(serde_yaml::to_string(&entry)) placeholder is replaced.
  Other components' L6 cache keys cite this value (design §6.2);
  PR-11 makes the participant-surface contribution load-bearing.

**Pipeline integration (`pipeline.rs`):**
- New `write_yaml_atomic<T: Serialize>(target_dir, target_file, file)`
  helper used by both PR-6's `component.yaml` writer and PR-7's
  `surfaces.yaml` writer. Atomic write (tempfile-then-rename); failure
  is non-fatal warning on stderr matching PR-4 / PR-6 policy.
- The `write_per_component_files` walk now emits both files in the same
  pass. `--dry-run` skips both.

**Span convention (spec §2.1):**
- Block items (`pub struct/enum/mod/union/trait/fn { … }`): span starts
  at the first byte of `pub`, ends at the byte immediately after the
  matching `}`.
- Statement items (`pub type X = Y;`, `pub const X: T = …;`,
  `pub static X: T = …;`): span ends at the byte immediately after the
  terminating `;`.
- The scanner is a deliberate stateful byte-by-byte walker handling
  string literals, char literals, line/block comments, raw-string
  prefixes, and brace nesting so quoted braces inside strings don't
  derail it. Documented as `nested_pub_inside_pub_mod_is_phase1_known_limitation`
  — Phase 2's rust-analyzer wire-up is the canonical fix.

**Hand-off for PR-8 / PR-11 reviewers:**
- `contracts_consumed` is emitted as always-empty in PR-7. PR-8 (LLM
  Stage 2) populates it from cross-component code references. The
  schema shape is complete; PR-8 only adds entries.
- `compute_surfaces_fingerprint` is the single citation point for
  PR-11. The function is documented as such on its docstring.
- The two `TODO(PR-11)` markers in `l6_edges.rs` are untouched.
- Library-api validation (`LibraryApi::validate()` per PR-1 status
  note) runs inside `surfaces_yaml_snapshot` before the fingerprint
  is computed; an invalid library-api fails the snapshot rather than
  emitting a corrupt file.

**Deviations from the brief (reviewers please note):**
- `SurfaceRecord` was NOT extended in place; instead a new outer
  `SurfaceArtefacts` carries the three new fields alongside the
  unchanged `SurfaceRecord`. Plan §4 PR-7 said "extend `SurfaceRecord`",
  but the wrapper preserves the existing Salsa cache key and the
  separation of LLM-derived (Stage 1: `purpose`, `consumes_files`) vs
  deterministic (binding extraction) concerns. Functionally identical
  for downstream consumers — `SurfacesFile` is what they read.
- The `RustSurfaceAnalyzer::analyse` method returns `Declines`. The
  L5 driver calls `extract_rust_surface` directly (option (b) in the
  brief), so the dispatcher path is reserved for future Phase 2 use.
  Calling `analyse` through `AnalyzerRegistry::dispatch` today is a
  no-op — the analyser is registered for `analyzer_registry_sha`
  contribution, not for runtime dispatch.

**Debt deferred (DONE_WITH_CONCERNS):**
- Per-analyser `analyser_id` / `analyser_version` plumbing through L3
  dispatch — the `PerComponentFile.analyser_id="l3-driver"` /
  `analyser_version=L3_DRIVER_VERSION` placeholders from PR-6 are
  preserved. Threading per-analyser identity (`cargo-toml-classifier`,
  `dockerfile-l3`, `llm-classify-l3`) requires either tagging
  `DispatchOutcome` in the analyser crate (public-API change) or a
  per-cost-class shim in the engine. Both larger than PR-7's scope;
  unaffected downstream because PR-11 keys on
  `surfaces.yaml.fingerprint`, not on the per-component-file's analyser
  identity. Documented on `per_component_yaml_snapshot`'s docstring.

**Files created:**
- `crates/atlas-analyzers/src/rust_surface_analyzer.rs` (1266 lines incl. 23 tests).
- `crates/atlas-engine/src/contract_canonicalisation.rs` (380 lines incl. 12 tests).
- `crates/atlas-cli/tests/surfaces_emission_rust.rs` (434 lines, 6 tests).

**Files modified (Atlas):**
- `crates/atlas-analyzers/src/lib.rs` — `RustSurfaceAnalyzer`/
  `RustSurfaceOutput` re-exports + `extract_rust_surface` re-export.
- `crates/atlas-analyzers/src/registry.rs` — `builtin()` registers the
  fourth analyser; tests updated.
- `crates/atlas-engine/src/l5_surface.rs` — new `SurfaceArtefacts` +
  `surface_artefacts_of`; existing `surface_of` / `SurfaceRecord` left
  untouched.
- `crates/atlas-engine/src/l9_projections.rs` — new
  `surfaces_yaml_snapshot`; `per_component_yaml_snapshot` swapped to
  consume the surfaces fingerprint.
- `crates/atlas-engine/src/lib.rs` — re-exports for the new public API
  + `contract_canonicalisation` module.
- `crates/atlas-cli/src/pipeline.rs` — `write_yaml_atomic` helper +
  surfaces.yaml emission inside `write_per_component_files`.

### PR-8
2026-05-07 — Landed across two repos.

**atlas-contracts:** `d78d2d7` (initial validator + unit tests) +
`525ccef` (boundary unit tests after code-quality review).

**Atlas:** `c23d63d` (initial — contract edge emission, validator
wiring) + `c39dd25` (spec-review fix-up — end-to-end validator
integration test, validator placement after all top-level writes) +
`d5168e7` (code-quality fix-up — validator runs in dry-run, doc
accuracy on Salsa memoisation claim).

**Dep-graph correction for the next session:** the status-file dep
graph lists PR-11 as Wave 4 alongside PR-8 (both depending only on
PR-7+PR-10). In practice PR-11 sequentially follows PR-8 — both PRs
modify `l6_edges.rs` and `l9_projections.rs` (merge-conflict risk if
parallel) and PR-11's two main acceptance tests ("crate-B consumes
contract from crate-A" + "cross-tree consumes-contract invalidation")
require PR-8's `consumes-contract` edges to exist. The "no-contract
stability" test still works without PR-8 but two of three tests can't
be authored. Treat the dep graph as PR-7 → PR-8 → PR-11 → PR-12.

**New public API (atlas-contracts):**
- `validate_contract_participants_resolve(edges: &[Edge],
  known_contract_ids: &BTreeSet<&str>) -> Result<(),
  Vec<UnresolvedContractParticipant>>` — purely structural validator
  in `component_ontology::schema`. Re-exported from `lib.rs`.
- `UnresolvedContractParticipant { edge_kind: EdgeKind,
  component_participant: String, unresolved_contract_id: String }`.
- Result vec is sorted by `(edge_kind.as_str(), unresolved_contract_id)`
  for deterministic error messages.
- Walks only the three contract edge kinds (`DefinesContract`,
  `ImplementsContract`, `ConsumesContract`); other kinds ignored.
- Defensive `.get(1)` / `.first()` skip silently for malformed
  participants — pinned by the boundary unit test
  `validate_contract_participants_skips_malformed_edge_with_one_participant`.

**New public API (atlas-engine):**
- `contract_edges_from_surfaces(db: &AtlasDatabase) -> Vec<Edge>`
  — deterministic edge contributor in `l6_edges.rs`. For every live
  component with a code-derived contract, emits one `defines-contract`
  + one `implements-contract` edge with participants
  `[component_id, contract.id]`. Lifecycle: `design`. Evidence: `Strong`.
  Re-exported from `atlas_engine::lib`. `consumes-contract` edges are
  NOT emitted here — they flow through the existing LLM Stage 2 batch
  path.

**L6 wiring (`l6_edges.rs::all_proposed_edges`):**
- Contract edges merge alongside composition edges + LLM batch before
  canonicalisation. Order: `contract_edges` first, `composition_edges`
  second, `parsed` (LLM) last. Different EdgeKinds means no canonical-
  key collision between contract and composition edges; the
  deterministic-first ordering means an LLM restatement of a
  deterministic edge is dropped by `canonicalise_edges`'s "first wins"
  rule.
- Wired into all three return paths: multi-component (main), `live.len()
  < 2` early return, and LLM-error fallback. A single-component workspace
  that defines a contract still emits its `defines-contract` /
  `implements-contract` edges.

**Pipeline integration (`pipeline.rs`):**
- `validate_contract_participants_resolve` runs **unconditionally**
  (matching the precedent set by `check_subsystem_namespace` /
  `check_subsystem_id_members`), *before* the `if !config.dry_run { ... }`
  writes block. A dry-run with an unresolvable contract participant
  fails the same way a real run does — pinned by the dry-run integration
  test.
- On validation failure, returns `IndexError::Other` with a multi-line
  message naming every unresolved edge kind / component / contract id.
  No files are written on this error path; `outputs_written: false`.
- Validator builds `known_contract_ids` by walking every live component's
  `surfaces_yaml_snapshot` and collecting `contracts_defined[*].id`.

**Tests added:**
- atlas-contracts unit tests in `component-ontology::schema::tests` (5
  validator function tests + 2 boundary tests added by the code-quality
  fix-up): happy path, unresolved consumes-contract, non-contract edges
  ignored, sort order, sort by kind-then-id, empty edges, malformed
  edge.
- `crates/atlas-engine/tests/contract_edges.rs` (3 integration tests):
  defines + implements emission from a serde-derived struct, consumes-
  contract from canned LLM Stage 2, empty surfaces produce zero contract
  edges.
- `crates/atlas-cli/tests/contract_validator_integration.rs` (2
  end-to-end tests): real-run and dry-run both fail when an unresolvable
  contract participant is canned into the LLM Stage 2 response. Helper
  `setup_violation_run(dry_run)` factored out.

**Deviations from the brief (PR-9/PR-11 reviewers please note):**
- The brief suggested calling `surfaces_yaml_snapshot(db, &cid)` from
  `contract_edges_from_surfaces`. This would create a circular module
  dependency (`l6_edges → l9_projections → l6_edges`). Implementation
  calls `surface_artefacts_of` (from `l5_surface`) directly. The
  semantics are identical for Phase 1: the L9 projection auto-pairs
  each `contracts_defined[i]` with a 1-to-1 `contracts_implemented[i]`,
  and `contract_edges_from_surfaces` reproduces that pairing inline.
- `consumes-contract` edges flow through the LLM Stage 2 batch — no
  Stage 2 prompt template change. The {{ONTOLOGY_KINDS}} block already
  includes the three contract kinds via the bijection-tested
  `render_embedded_kinds_for_prompt` machinery, so the model can already
  emit them.
- `surfaces.yaml.contracts_consumed` remains always-empty in Phase 1
  output (matching PR-7's hand-off). PR-8 does not populate it.
- `l9_projections.rs` was NOT modified — the lex sort by
  `Edge::canonical_key()` was already in place from PR-9 and picks up
  contract edges automatically. The plan's mention of an l9 modification
  is stale.

**Carry-over for PR-11 reviewers:**
- `surface_artefacts_of` (in `l5_surface.rs`) is plain `pub fn` — NOT
  `#[salsa::tracked]`. Each call from `contract_edges_from_surfaces`
  re-walks the component's path segments and re-runs
  `extract_rust_surface`. The doc comment on
  `contract_edges_from_surfaces` flags this. PR-11 wires participant
  surface shas into the L6 fingerprint; if it pulls from
  `surface_artefacts_of`, the same caveat applies. Wrapping the
  function in `#[salsa::tracked]` is a future cleanup, deliberately
  outside PR-8's scope.
- The `TODO(PR-11)` markers in `l6_edges.rs` (in the `let l6_fingerprint
  = { ... }` block) are unchanged. PR-11 still owns adding
  `participant_surface_sha` contributions.

**Carry-over for PR-12 reviewers (test-helper extraction):**
- `crates/atlas-cli/tests/contract_validator_integration.rs` duplicates
  the fixture-copy harness (`tiny_fixture_root`, `copy_fixture_to_tmp`,
  `materialise_tiny_fixture`, `base_config`) byte-for-byte from
  `pipeline_integration.rs`. The `ContractViolationBackend` overlaps
  ~85% with `LenientBackend`. A `tests/common/mod.rs` (or
  `tests/support/`) extraction would collapse the file to ~30 lines and
  give PR-12 a shared helper to lean on. Not done in PR-8 because the
  refactor touches code outside PR-8's scope (tests other than PR-8's
  own consume `LenientBackend`); flagged here so PR-12's reviewer can
  decide whether to land it then.

**Files created (Atlas):**
- `crates/atlas-engine/tests/contract_edges.rs` (317 lines, 3 tests).
- `crates/atlas-cli/tests/contract_validator_integration.rs` (~200
  lines after the dry-run extension, 2 tests).

**Files modified (Atlas):**
- `crates/atlas-engine/src/l6_edges.rs` — `contract_edges_from_surfaces`
  + wiring into all three `all_proposed_edges` paths; doc comment
  accuracy fix.
- `crates/atlas-engine/src/lib.rs` — re-export
  `contract_edges_from_surfaces`.
- `crates/atlas-cli/src/pipeline.rs` — validator wiring (now runs
  unconditionally; placement before writes block).

**Files modified (atlas-contracts):**
- `crates/component-ontology/src/schema.rs` —
  `validate_contract_participants_resolve` +
  `UnresolvedContractParticipant` + 7 unit tests.
- `crates/component-ontology/src/lib.rs` — two new re-exports.

### PR-9
2026-05-07 — Landed on Atlas main as `0d0e581`.

**New module (atlas-engine):**
- `crates/atlas-engine/src/l6_composition.rs::composition_edges_from_dockerfiles(db) -> Vec<Edge>`
  — pure deterministic composition-edge generator. Walks every live
  `kind: docker-image` component, reads its `Dockerfile` via
  `file_content(db, ...)` (Salsa-tracked, so cache-correct), re-parses
  via `atlas_analyzers::dockerfile_classifier::parse_dockerfile` (the
  brief's option (1) — re-parse instead of threading the L3 Salsa
  output, materially simpler for the same correctness).
- Two-tier source resolver:
  - **Tier 1 (build-output basename):** `target/(release|debug)[/<sub>]/...<name>`
    is stripped to `<name>`, with the optional target-triple sub-dir
    handled, and the unique component leaf id matching `<name>` is
    returned. This is the canonical case the plan's worked example
    pins (`COPY target/release/billing-core` → `<root>/billing-core`).
  - **Tier 2 (longest-path-prefix):** falls back to longest-prefix
    match against pre-computed `(component_id, absolute_segment_dir)`
    pairs. Pre-computed once per call so per-COPY cost is O(1).
- `--from=<stage>` directives skipped (image-internal copies, no
  host-repo source).
- Self-edge guard for pathological `COPY .` patterns (would otherwise
  fail `Edge::validate`).
- Re-exports from `atlas_engine::lib`: `composition_edges_from_dockerfiles`,
  `composition_edges_arc`.

**L6 wiring (`l6_edges.rs::all_proposed_edges`):**
- Composition edges are computed *before* the `live.len() < 2` early
  return — single-component workspaces still flow composition edges
  for any `docker-image` they contain.
- LLM error-path fallback returns canonicalised composition edges
  instead of an empty `Vec` — deterministic edges no longer depend on
  LLM availability.
- Composition edges are pushed *first* into `canonicalise_edges`'s
  combined vec, exploiting its "first wins on duplicate canonical
  key" rule. The LLM batch may restate a composition edge but cannot
  override it.
- The `TODO(PR-11)` marker for `add_participant_surface_sha` is
  preserved verbatim in the unchanged fingerprint block.

**Lex-order projection (`l9_projections.rs::related_components_yaml_snapshot`):**
- Edges now sort lex-ascending on `(kind, lifecycle, participants)`
  via `Edge::canonical_key()` — single source of truth for the
  comparison. Stable sort preserves the "first wins on duplicate
  canonical key" interaction. PR-8's contract edges and v1's
  `depends-on` edges interleave with PR-9's composition edges in
  alphabetical order on `EdgeKind::as_str`.

**Tests:**
- 10 unit tests in `l6_composition::tests` covering the resolvers
  (`strip_build_output_prefix_*`, `resolve_source_to_component_*`,
  `component_id_leaf_returns_last_path_segment`,
  `normalise_path_resolves_dot_and_double_dot`).
- 5 integration tests in `crates/atlas-engine/tests/composition_edges.rs`:
  `dockerfile_copy_of_build_output_emits_bundled_into_edge`,
  `dockerfile_bundling_two_binaries_emits_deployed_with_edge`,
  `fixture_without_dockerfiles_emits_no_composition_edges`,
  `copy_from_stage_directives_are_skipped`,
  `related_components_snapshot_orders_edges_lexicographically`.

**Hand-off for PR-8 reviewers:**
- The lex-sort in `related_components_yaml_snapshot` already handles
  PR-8's contract edges via `canonical_key()`. PR-8 only needs to
  emit `defines-contract` / `implements-contract` / `consumes-contract`
  edges into `all_proposed_edges`; sorting is automatic.

**Hand-off for PR-11 reviewers:**
- The `TODO(PR-11)` marker in `l6_edges.rs` is at the unchanged line
  inside the `let l6_fingerprint = { ... }` block. PR-9 did not move
  it.
- Composition edges contribute to the L6 cache via the existing per-
  component `file_content_sha` loop (the Dockerfile lives as path-
  segment content of the docker-image component; any source or
  Dockerfile change observed by the existing fingerprint inputs).
  PR-11 still owns adding `participant_surface_sha`.

**Deviations from the brief (reviewers please note):**
- The deliverable id assertion in the integration test resolves the
  docker-image component via `kind == "docker-image"` filtering
  rather than asserting the spec's worded example id
  `<root>/billing-image` — L4's id allocator is path-derived from the
  dir basename (`deploy/billing` → leaf `billing`), not from the
  spec's hand-rolled `billing-image` namespace. The acceptance is
  semantic ("a `bundled-into` edge from the binary's source-component
  to the docker-image-component"); the slug shape is workspace-
  determined.
- atlas-contracts was NOT modified. PR-1 already provides every edge
  variant. `Edge.lifecycle: LifecycleScope` (not `Option<...>`) — the
  composition edges set it directly.

**Files created:**
- `crates/atlas-engine/src/l6_composition.rs` (504 lines incl. 10 tests).
- `crates/atlas-engine/tests/composition_edges.rs` (363 lines, 5 tests).

**Files modified (Atlas):**
- `crates/atlas-engine/src/l6_edges.rs` — composition merge + early-
  return changes; TODO(PR-11) preserved.
- `crates/atlas-engine/src/l9_projections.rs` — lex-sort by
  `canonical_key()`.
- `crates/atlas-engine/src/lib.rs` — re-exports.

### PR-10
2026-05-07 — Landed on Atlas main as `8a1da7c`. The subagent's
final report was lost to a server-side 500; this note is
reconstructed from inspecting the diff, the new tests, and the
status of the gates. All three acceptance-criteria tests pass; the
status note below was authored by the orchestrator after verifying
the implementation rather than copy-pasted from the subagent's
report.

**New public API (atlas-engine):**
- `LlmResponseCache::new_with_persistent(PersistentCache) -> Self`
  — production constructor. The plain `LlmResponseCache::new()`
  remains for tests that want in-memory-only behaviour.
- `LlmResponseCache::call_cached_with_fp(stage: Stage,
  fingerprint: &Sha256Hex, backend: &dyn LlmBackend,
  request: &LlmRequest) -> Result<Arc<Value>, LlmError>` — the new
  production call site. Lookup order: in-memory hit → persistent
  hit (deserialise blob, seed in-memory) → backend call (insert
  both layers).
- `L5_DRIVER_VERSION` and `L6_DRIVER_VERSION` constants — exposed
  alongside PR-6's `L3_DRIVER_VERSION` so future PRs can plumb the
  driver version into per-component fingerprints.

**Pipeline wiring (`pipeline.rs`):**
- `PersistentCache::open(<output>/.atlas/cache)` runs at startup.
  On error, falls back to a default `LlmResponseCache` with a
  warning on stderr (run continues). Matches PR-4 + PR-6 non-fatal
  policy.
- The persistent cache is installed onto `AtlasDatabase` via a
  setter (`set_llm_cache`) immediately after `new_with_registry`.
- The `cache_io::load_into` call at startup, the `set_persist_hook`
  callback, and the `cache_io::save_from` final flush are all
  removed. The persistent cache's `cache.put` is synchronous on
  every successful backend response, so there is no end-of-pipeline
  flush to perform.

**LLM call sites (L3 / L5 / L6):**
- L3 (`l3_classify.rs::is_component`): the `_l3_fingerprint`
  placeholder binding from PR-5 is now the real fingerprint
  consumed by `call_cached_with_fp(Stage::L3, ...)`. Contributors
  per design §8.1: `analyzer_registry_sha`, `llm_fingerprint`
  (template+ontology+model+backend), `prompt_sha` (sha256 of the
  JSON-canonicalised classify inputs), `file_content_sha` per
  consumed manifest. The deterministic registry pass short-
  circuits BEFORE the cache lookup, so Cargo / Dockerfile classifications
  never round-trip to disk.
- L5 (`l5_surface.rs::surface_of`): a `FingerprintBuilder::new(L5,
  "l5-driver", L5_DRIVER_VERSION)` is built per call. Contributors:
  `analyzer_registry_sha`, `llm_fingerprint`, `prompt_sha`,
  `file_content_sha` per file the surface analyser consumed
  (the `consumes_files` set).
- L6 (`l6_edges.rs::candidate_edges_for`): same shape. Contributors:
  `analyzer_registry_sha`, `llm_fingerprint`, `prompt_sha`,
  `file_content_sha` for the prompt's input bytes.
  **Participant surface shas are NOT contributed in PR-10.** PR-11
  is the owner. Two `TODO(PR-11)` comments mark the exact insertion
  point in `l6_edges.rs` (a loop that will call
  `FingerprintBuilder::add_participant_surface_sha` over the
  candidate edge participants, keyed by surfaces.yaml fingerprint).

**Hand-off for PR-11 reviewers:**
- The L6 fingerprint shape today produces stable cache hits across
  re-runs of the same workspace state, but does NOT invalidate when
  a *different* component's surface changes. That's the gap PR-11
  closes. The L6 driver-version (`L6_DRIVER_VERSION`) is the right
  bump point if PR-11 wants to mass-invalidate the entire L6 cache
  on first install — a single bump tears every existing entry.
- Persisting the L6 fingerprint contributors is unaffected: the
  contributors are values fed into the builder, not stored
  separately. PR-11 just adds another `add_*` call inside the same
  builder block.

**v1 paths deleted:**
- `crates/atlas-cli/src/cache_io.rs` (281 lines) — the JSON LLM
  cache file. Per plan §3 v1-mechanism table, deleted under
  greenfield treatment. Plan §7 lists this explicitly under
  "out of scope ... v1 readers, the `atlas migrate-v1` command,
  and the v1 `llm-cache.json` format are all deleted, not
  preserved."
- `mod cache_io;` line in `crates/atlas-cli/src/lib.rs`.
- Every `cache_io::load_into` and `cache_io::save_from` call site
  in `pipeline.rs`.
- `LlmResponseCache::set_persist_hook` and the per-call hook
  invocation: kept as a method (some tests still reference it),
  but the production code path no longer installs a hook.

**Tests added (`crates/atlas-cli/tests/persistent_cache_lifecycle.rs`):**
- `fresh_process_re_run_hits_persistent_cache_for_every_entry`
  — runs `atlas index` once via a `CountingBackend`, drops the
  database + LlmResponseCache, constructs a *new* cache pointing
  at the same on-disk cache root, runs `atlas index` again,
  asserts backend `call_count() == 0`.
- `single_file_content_change_invalidates_only_affected_entries`
  — a two-component fixture where editing component-A's source file
  triggers exactly N misses (one per LLM stage that cited the
  edited file's content sha) and zero misses against component-B.
- `deleting_cache_directory_forces_full_rerun` — `rm -rf` the
  cache root between runs; second run's backend `call_count` equals
  the cold-run baseline.

**Deviations from the plan:**
- **Synchronous writes inside `cache.put`** instead of the async
  background-thread shape sketched in plan §6's risk row. The
  atomic-tempfile-then-rename is cheap and the pipeline isn't
  latency-bound at the LLM-call grain; a future PR can introduce
  a write thread if profiling demands it. No flush-at-pipeline-end
  is needed because there's nothing buffered.
- **`PersistentCache::open` failure is non-fatal** — fall back to
  in-memory-only with a stderr warning. Plan §4 PR-10 said "open
  the persistent cache at startup; pass it to `AtlasDatabase::new`"
  without specifying error policy; matches the non-fatal write
  policy from PR-4 (config.yaml) and PR-6 (per-component yaml).

**Debt deferred:**
- L6 participant surface shas → PR-11.
- Cache GC: `PersistentCache::gc(&BTreeSet)` is callable but no
  caller invokes it on `atlas index`. A future PR (or an explicit
  `--gc` flag) walks every entry observed in the run and invokes
  `gc` against the complement.
- Cache compression (design §11.2.7) — Phase 1 ships uncompressed.
- Async writes — see deviations above.

**Files modified (Atlas):**
- `crates/atlas-cli/src/lib.rs` — `mod cache_io` line removed.
- `crates/atlas-cli/src/pipeline.rs` — open PersistentCache, install
  on db, drop cache_io call sites.
- `crates/atlas-cli/tests/persistent_cache_lifecycle.rs` (new) —
  three acceptance tests.
- `crates/atlas-cli/tests/agent_observer_e2e.rs`,
  `crates/atlas-cli/tests/pipeline_integration.rs` — fixtures
  updated for the new `LlmResponseCache::new_with_persistent` shape.
- `crates/atlas-engine/src/db.rs` — setter for the LLM cache.
- `crates/atlas-engine/src/ingest.rs` — minor adjacent fix.
- `crates/atlas-engine/src/l3_classify.rs` — call_cached_with_fp
  swap.
- `crates/atlas-engine/src/l5_surface.rs` — FingerprintBuilder +
  call_cached_with_fp.
- `crates/atlas-engine/src/l6_edges.rs` — same shape, with PR-11
  TODO markers.
- `crates/atlas-engine/src/lib.rs` — re-export
  `L5_DRIVER_VERSION` and `L6_DRIVER_VERSION`.
- `crates/atlas-engine/src/llm_cache.rs` — `new_with_persistent`,
  `call_cached_with_fp`, persistent layer integration.
- `crates/atlas-engine/tests/l0_l1_queries.rs` — fixture updated.

**Files deleted (Atlas):**
- `crates/atlas-cli/src/cache_io.rs` (281 lines).

### PR-11
(none yet)

### PR-12
(none yet)
