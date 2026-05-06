# Atlas vNext Phase 1 — Status

Companion to `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
This file tracks per-PR completion state across sessions. The session
prompt at `docs/superpowers/plans/2026-05-06-phase1-session-prompt.md`
reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-07 (PR-5 + PR-6 landed).

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
