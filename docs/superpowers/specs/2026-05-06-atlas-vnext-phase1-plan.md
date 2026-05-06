# Atlas vNext Phase 1 — Implementation Plan

**Status:** Plan (forward-looking; Phase 1 of the Atlas vNext system-model
redesign). Companion to `2026-05-06-atlas-system-model-design.md`.
**Date:** 2026-05-06.
**Treatment:** Greenfield. No migration command, no on-disk format
compatibility with v1, no transition window. The v1 codebase is the
starting point for refactoring; the v1 *outputs* are not a constraint.
A user upgrading is expected to delete `.atlas/` and re-run.

**Scope:** Decomposes Phase 1 (§10.1 of the design spec) into an ordered
sequence of independently-mergeable PRs that establish the architectural
seam — multi-root `Workspace`, scattered per-component `.atlas/`,
contract-first `surfaces.yaml`, plugin protocol with three reference
analysers, and persistent content-addressed cache. Phase 1 still ships
as a one-shot CLI.

The "atlas-contracts visible in Ravel-Lite" outcome is a natural
consequence of correct Phase 1 implementation, not a special case
bolted on at the end.

---

## 0. Reading order

Before this plan, read:

1. The design spec §0, §3, §4, §5, §6, §8, §10.1 (load-bearing for
   Phase 1) and §11.2 (open questions; this plan resolves the two that
   block Phase 1 schemas).
2. Memory entry `project_atlas_vnext_system_model_design` (one-paragraph
   recap; not load-bearing but cheap).

§10.5 of the design spec (v1 migration) is dropped under greenfield
treatment. The v1 design spec (`2026-04-23-component-discovery-design.md`)
is reference-only — useful for understanding why the v1 mechanisms this
plan reuses look the way they do, but not a compatibility constraint.

This plan does *not* re-derive the architecture; it operationalises it.
Where this plan and the design spec disagree, the design spec wins;
where the spec is silent on sequencing, this plan is canonical.

---

## 1. Phase 1 deliverable, restated

End of Phase 1, an Atlas user running `atlas index .` from Ravel-Lite
where `Cargo.toml` declares a path-dep on `../atlas-contracts/...` shall
see:

- `<ravel-lite>/.atlas/components.yaml` containing both
  `ravel-lite/*` components **and** `atlas-contracts/*` components.
- `<ravel-lite>/.atlas/related-components.yaml` containing
  `defines-contract`, `implements-contract`, `consumes-contract` edges
  whose participants include contract ids defined in atlas-contracts'
  per-component `surfaces.yaml`.
- A `<ravel-lite>/.atlas/cache/<stage>/<sha>.blob` hierarchy
  (filesystem-native persistent cache) whose hit rate on a no-op re-run
  is 100%.
- Per-component `.atlas/` directories scattered under each component's
  source path, with `component.yaml`, `surfaces.yaml`, optional
  `overrides.yaml`, and a component-scoped cache shard.

Out of scope, deferred to later phases: subprocess analyser transport,
non-Rust language analysers, server mode, drift / impact / modularity
reports.

---

## 2. Open-question pre-conditions

Two of the eight open questions in design §11.2 block Phase 1 schemas.
A third can be deferred to Phase 2. The remaining five are not on the
Phase 1 critical path.

### 2.1 §11.2.2 — Contract content-sha canonicalisation (BLOCKING)

**Problem:** A contract's `content_sha` must be stable across cosmetic
edits (key reordering in YAML, comment changes, whitespace). Without a
canonicalisation rule, every contract sha is brittle and the L6 cache
keyed on participant shas thrashes.

**Phase 1 resolution (companion spec PR-0b):** Two algorithms,
selected by contract kind:

- **Code-derived contracts** (`library-api`, and Rust-binding-derived
  `data-format` contracts where the binding is the source of truth):
  The content sha is `sha256(canonical_serialisation_of_binding_AST)`.
  In Phase 1 the canonical serialisation is the binding's content sha
  (SHA-256 of the file-byte-range covered by the binding span), since
  the only language is Rust and the binding span is unambiguous.
  Phase 2 generalises to per-language AST canonicalisers.
- **Schema-derived contracts** (`data-format` whose source is a YAML
  schema or JSON Schema, `wire-protocol` whose source is a `.proto` or
  OpenAPI document): The content sha is computed by parsing the source,
  emitting a canonical YAML/JSON form (sorted keys, no comments,
  whitespace-normalised), and SHA-256ing the bytes. Use the existing
  `serde_yaml::Value` ordering (BTreeMap) and explicit `to_string` to
  guarantee determinism.

For Phase 1, only Rust-binding-derived `data-format` and Rust
`library-api` contracts are emitted. The schema-derived branch is
specified now (so Phase 2 has nothing to invent) and a single
test-only `wire-protocol` contract exercises it before the spec lands.

**Owner:** PR-0b (specs/2026-05-06-contract-content-sha-canonicalisation.md).

### 2.2 §11.2.3 — Override scoping under scattered `.atlas/` (BLOCKING)

**Problem:** Per-component `<component>/.atlas/overrides.yaml` and the
top-level `<root>/.atlas/components.overrides.yaml` co-exist. Resolution
order, conflict handling, and merge semantics need a spec before the
scattered-`.atlas/` writers (PR-6) land.

**Phase 1 resolution (companion spec PR-0c):**

- **Discovery order.** L4 (override merge) reads override files in this
  order: (1) top-level `<primary-root>/.atlas/components.overrides.yaml`;
  (2) peer-root `<root>/.atlas/components.overrides.yaml` files in
  lexicographic root order; (3) per-component
  `<component-path>/.atlas/overrides.yaml` files, ordered by component
  id ascending.
- **Merge semantics.** Override entries (`additions`, `pins`,
  `suppressions`) are merged with last-writer-wins on the same
  `(component_id, override_key)` tuple. The merge order is the
  discovery order above, so per-component overrides win over top-level.
- **Conflict reporting.** When two override files set conflicting
  pin values for the same component, `atlas index` emits a `warning`
  on stderr naming both files and the resolved (winner) value. A
  `--strict-overrides` flag (Phase 2) escalates to a hard error;
  Phase 1 ships warning-only.
- **Per-component override scoping.** A per-component
  `<component>/.atlas/overrides.yaml` may only carry pins/additions
  for that component or its sub-components (i.e., ids whose namespace
  prefix matches the component's id). Cross-component pins in a
  per-component file are rejected with a hard error. This rule keeps
  the scattered-`.atlas/` invariant — data co-locates with source —
  enforceable.

**Owner:** PR-0c (specs/2026-05-06-override-scoping-scattered-atlas.md).

### 2.3 §11.2.1 — Surface schema for non-Rust languages (DEFERRED)

Phase 1 emits binding records with `language: rust` only.
`SurfacesFile.schema_version` ships as integer `1`. Phase 2 bumps to
`2` when the first non-Rust analyser validates the binding shape, and
ships its own (non-greenfield, internal-only) reader fork at that point.

### 2.4 Other open questions

- §11.2.4 (contract rename-match): Phase 2. v1 component rename-match
  covers component moves; contract id stability is good enough for
  Phase 1 because Phase 1 contracts are derived from Rust bindings
  whose location moves trigger v1 rename-match anyway.
- §11.2.5 (server auth): Phase 4.
- §11.2.6 (LLM confidence thresholds): Phase 1 ships v1 thresholds
  unchanged; Phase 2 calibrates.
- §11.2.7 (cache compression): Phase 1 ships uncompressed; the cache
  key does not include the compression algorithm (PR-2 design note).
- §11.2.8 (worktree consistency): Phase 1 records per-root commit shas
  in `config.yaml` for forensic value but does not validate consistency.

---

## 3. v1 mechanisms reused as starting points

These v1 mechanisms are extended rather than rewritten. They are
*starting points*, not compatibility constraints — under greenfield
treatment, the team is free to refactor any of them when the new code
demands it. The point of listing them is to avoid duplicate work and to
make the "where to look in the existing code" question cheap.

| Mechanism | Location in v1 codebase | How Phase 1 uses it |
|---|---|---|
| Salsa engine + tracked queries | `crates/atlas-engine/src/db.rs`, `l1_queries.rs`, `l2_candidates.rs`, `l3_classify.rs`, `l4_tree.rs` | Kept as the engine. PR-3 mutates the `Workspace` input shape but keeps the tracked-query pattern. |
| Evidence-driven classification | `crates/atlas-engine/src/types.rs` (`Classification`, `RationaleBundle`), `l3_classify.rs` | Kept. Analyser results adapt to the existing `Classification` shape via PR-5's adapter. |
| Rename-match | `atlas-contracts/crates/atlas-index/src/rename_match.rs` | Kept verbatim. Used unchanged in PR-3 (multi-root). Contract rename-match is Phase 2. |
| Override merge | `crates/atlas-engine/src/l4_tree.rs` | Extended in PR-6 per the override scoping spec; v1 single-file merge order is the inner case. |
| Prompt-sha → LLM fingerprint chain | `crates/atlas-llm` (`LlmFingerprint`, `compute_prompt_shas`) | Kept. The persistent cache (PR-2) keys off the same fingerprint. |
| BudgetSentinel + TokenCounter | `crates/atlas-cli/src/backend.rs` | Kept. Phase 1 wraps every analyser call (PR-5) inside the same sentinel. |
| In-memory `LlmResponseCache` | `crates/atlas-engine/src/llm_cache.rs` | Kept as the in-process front cache. PR-10 makes it a write-through layer over the persistent store; the separate `cache_io.rs` JSON file is **deleted**. |
| Tombstone emit-once via prior filter | L4 prior-filter in `l4_tree.rs` | Kept. Documented in memory `tombstone_emit_once_design`. |
| `all_components` not Salsa-tracked | L4 (memory: `all_components_not_salsa_tracked`) | Kept. Per-component-tree memoisation stays in the CLI/L9 layer. |

The v1 file `crates/atlas-cli/src/cache_io.rs` (the JSON LLM cache
on-disk format) is **deleted** in PR-10. The v1 schema files
(`components.yaml` v1 shape) are likewise replaced by the new shape;
no reader path for the old format exists in any Phase 1 code.

---

## 4. PR sequence

PRs are numbered in dependency order. PR sizes are estimates excluding
tests and excluding generated code. Each PR ends with passing
`cargo test --workspace` and a working `atlas index` against the new
test fixtures.

### PR-0 — Companion specs (no code)

**Intent:** Land this plan and the two blocking open-question
resolution specs before any schema PR.

**Files:**
- Create: `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md` (this file).
- Create: `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md` (per §2.1).
- Create: `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` (per §2.2).

**Acceptance criteria:**
- All three documents land in `docs/superpowers/specs/`.
- The two open-question resolutions reference `2026-05-06-atlas-system-model-design.md` §11.2.2 and §11.2.3 by section number and supersede those open questions.
- A new memory entry `feedback_phase1_open_questions` notes that §11.2.2 and §11.2.3 are resolved.

**LOC:** 0 code, ~250 lines docs each.

---

### PR-1 — Schema definitions for the new types (types-only, no I/O)

**Intent:** Add the Rust types that Phase 1 schemas project to/from. No
file writers, no readers, no pipeline wiring. Allows downstream PRs to
work in parallel without merge conflicts on type definitions.

**Files (in `atlas-contracts/crates/atlas-index/src/`):**
- Create: `surfaces.rs` — `SurfacesFile`, `Contract`, `ContractKind`, `Binding`, `BindingRole`, `LibraryApi`, `PubItem`, `SURFACES_SCHEMA_VERSION = 1`.
- Create: `analyzers.rs` — `AnalyzersFile`, `AnalyzerSpec`, `Stage`, `CostClass`, `Confidence`, `Transport`, `ApplicabilityPredicate`.
- Create: `config.rs` — `AtlasConfigFile` with `schema_version: u32 = 1`, `roots: Vec<PathBuf>`, `operations: BTreeMap<String, ModelRouting>`, `override_search: Vec<PathBuf>`.
- Create: `per_component.rs` — `PerComponentFile` envelope (single-component projection plus surfaces/overrides pointers).
- Modify: `schema.rs` — replace `language: String` with `languages: BTreeSet<String>` on `ComponentEntry`; extend the `kind` enum with deliverable variants (`docker-image`, `published-crate`, `helm-release`, `k8s-deployment`, `homebrew-bottle`, `orchestration-script`, `ci-pipeline`); add `roots: Vec<PathBuf>` to `ComponentsFile`. Drop the `root: PathBuf` field from `ComponentsFile`.
- Modify: `lib.rs` — re-export new types.
- Modify (in `component-ontology` crate): `lib.rs` — extend `EdgeKind` enum with `DefinesContract`, `ImplementsContract`, `ConsumesContract`, `BundledInto`, `PublishedAs`, `DeployedWith`, `ReleasedWith`, `Orchestrates`, `BundledFromExternal`. Existing kebab-case serialisation handles the new variants without further work.

**Acceptance criteria:**
- `cargo test -p atlas-index` and `cargo test -p component-ontology` pass.
- Round-trip serde tests for every new type (one test per type, asserting `serde_yaml::to_string` then `from_str` is identity).
- The Rust binding span used in `Binding.span` matches the span form already used by L5 surface analysis.

**LOC:** ~700-1000.

---

### PR-2 — Persistent content-addressed cache (no wiring)

**Intent:** Implement the filesystem-native cache (design §5.5, §8.3)
as a standalone module. Ship the read/write API. Do not yet wire it
into any L-stage; PR-10 swaps in the persistent store as the in-memory
cache's backing layer.

**Files (in `crates/atlas-engine/src/`):**
- Create: `cache/mod.rs` — `PersistentCache` struct, public API (`get`, `put`, `gc`).
- Create: `cache/fingerprint.rs` — `FingerprintBuilder`, contributors enumeration matching design §8.1 table (one builder method per stage).
- Create: `cache/layout.rs` — on-disk path layout (`<output>/cache/<stage>/<sha>.blob`), atomic write (tempfile-then-rename), read.
- Create: `cache/test_fixtures.rs` — `TempCache` helper for tests.
- Modify: `lib.rs` — re-export `PersistentCache`, `FingerprintBuilder`.

**Public API surface (load-bearing):**
```rust
pub struct PersistentCache { /* root dir */ }

impl PersistentCache {
    pub fn open(root: &Path) -> Result<Self>;
    pub fn get(&self, stage: Stage, fingerprint: &Sha256Hex) -> Result<Option<Vec<u8>>>;
    pub fn put(&self, stage: Stage, fingerprint: &Sha256Hex, blob: &[u8]) -> Result<()>;
    pub fn gc(&self, mark_set: &BTreeSet<(Stage, Sha256Hex)>) -> Result<GcReport>;
}

pub struct FingerprintBuilder { /* sha256 hasher */ }

impl FingerprintBuilder {
    pub fn new(stage: Stage, analyzer_id: &str, analyzer_version: &str) -> Self;
    pub fn add_file_content_sha(&mut self, sha: &Sha256Hex);
    pub fn add_prompt_sha(&mut self, sha: &Sha256Hex);
    pub fn add_llm_fingerprint(&mut self, fp: &LlmFingerprint);
    pub fn add_participant_surface_sha(&mut self, sha: &Sha256Hex); // L6 only
    pub fn finalise(self) -> Sha256Hex;
}
```

**Acceptance criteria:**
- `cargo test -p atlas-engine cache::` passes.
- Property-based test: any input change to the `FingerprintBuilder` produces a different fingerprint (`proptest`).
- Atomic-write test: a kill -9 mid-write leaves no `.tmp` files reachable from `get()`.
- GC test: a round-trip `put → mark → gc` removes unmarked entries.

**LOC:** ~700-1000.

---

### PR-3 — Multi-root `Workspace` (Salsa input)

**Intent:** Replace `Workspace.root: PathBuf` with `Workspace.roots: Vec<PathBuf>` and propagate to every consumer. Single-root behaviour (`roots.len() == 1`) is the natural common case; multi-root is dormant until PR-4 enables path-dep discovery.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `db.rs` — `Workspace` input field rename + setter rename (`set_root` → `set_roots`); existing `set_workspace_files` unchanged (the file vec covers all roots transparently).
- Modify: `ingest.rs` — `seed_filesystem` accepts a slice of roots; the implementation is a per-root walk concatenated into one `Vec<File>` and one `Vec<PathBuf>` for git boundaries.
- Modify: `l1_queries.rs` — change callers that compute `dir` from `workspace.root()` to enumerate over `workspace.roots()`.
- Modify: `l2_candidates.rs` — the L2 driver walks every root.
- Modify: `l4_tree.rs` — `all_components` walks per-root candidates and unions the resulting trees. ID allocation in `identifiers.rs` already uses the manifest-root as the namespace prefix; multi-root naturally gives `atlas-contracts/atlas-index` and `ravel-lite/billing-core` the right ids.
- Modify: `l9_projections.rs` — `components_yaml_snapshot` writes `roots: Vec<PathBuf>` (no legacy single-string `root` field).
- Modify: `crates/atlas-cli/src/pipeline.rs` — accept primary root + optional additional roots from CLI; defaults to `vec![primary]`.

**Acceptance criteria:**
- `cargo test --workspace` passes.
- New unit test: a two-root workspace where the second root contains one Cargo crate produces the union component set with stable ids.
- New cache-hit test: a no-op re-run on a single-root workspace makes zero LLM calls.
- New cache-hit test: a no-op re-run on a two-root workspace makes zero LLM calls.

**LOC:** ~1000-1300 across many files (mostly mechanical renames).

---

### PR-4 — Path-dep root expansion to fixed point

**Intent:** Walk path-deps in every reachable manifest under the primary root; for each path-dep target outside the primary root, walk up to the enclosing manifest-root (Cargo `[workspace]`, npm workspace, filesystem root) and add it as an additional root. Iterate to fixed point.

**Files (in `crates/atlas-engine/src/`):**
- Create: `root_expansion.rs` — `expand_roots(primary: &Path) -> Result<Vec<PathBuf>>` performing the fixed-point walk. Cycle detection via a visited set.
- Modify: `manifest_parse.rs` — extract path-dep targets from `[dependencies]` and `[dev-dependencies]` in Cargo.toml; the existing parser already handles the structural form (toml crate is the dep-of-record per memory `feedback_toml_parsing`).
- Modify: `crates/atlas-cli/src/pipeline.rs` — call `expand_roots(primary)` before seeding the database; persist the discovered roots to `<output>/.atlas/config.yaml#roots` for auditability.

**Acceptance criteria:**
- New integration test: `tests/multi_root_path_deps.rs` constructs a fixture with `crate-a` at the primary root path-dep'ing `crate-b` outside the root; the test asserts `expand_roots` returns both manifest-roots.
- Cycle test: a `crate-a → crate-b → crate-a` cycle terminates in finitely many iterations and emits a warning, not an error (per design risk row "Cross-tree path-dep cycles").
- Test fixture without escaping path-deps: `expand_roots` returns `vec![primary]`.

**LOC:** ~700-900.

---

### PR-5 — Plugin protocol + three reference analysers

**Intent:** Establish the analyser registry (design §5.2, §7.1) and ship three reference analysers: a Cargo classifier, a Dockerfile classifier, and an LLM-classify analyser. All three are in-process. Subprocess transport is Phase 2.

**Files (new crate):**
- Create: `crates/atlas-analyzers/Cargo.toml`, `src/lib.rs`, `src/registry.rs`, `src/dispatcher.rs`.
- Create: `crates/atlas-analyzers/src/cargo_classifier.rs` — implements Cargo classification under the new trait. Lift the working logic from v1's `crates/atlas-engine/src/l3_classify.rs` Cargo branch as a starting point; refactor for the trait shape without preserving v1 behaviour as a constraint.
- Create: `crates/atlas-analyzers/src/dockerfile_classifier.rs` — parses `FROM`, `COPY --from=…`, `LABEL`, `ENV`, `EXPOSE`, `CMD`, `ENTRYPOINT`. Used at L1 (file enumeration) to seed deliverable candidates and at L3 for `kind: docker-image` classification.
- Create: `crates/atlas-analyzers/src/llm_classify.rs` — wraps the LLM classify call under the `Analyzer` trait. Returns `Confidence::Declines` for inputs the deterministic classifiers handled and `Confidence::Graded` otherwise.

**Files (in existing crates):**
- Modify: `crates/atlas-engine/src/l3_classify.rs` — replace direct Cargo branching with a registry dispatch. The L3 entry-point asks the registry for the cheapest applicable analyser.
- Modify: `crates/atlas-engine/src/lib.rs` — accept an `Arc<AnalyzerRegistry>` on `AtlasDatabase::new`; defaults to `AnalyzerRegistry::builtin()`.
- Modify: `crates/atlas-cli/src/main.rs` — load `<output>/.atlas/analyzers.yaml` if present and merge with built-in defaults; pass the resulting registry to `AtlasDatabase::new`.
- Modify: `Cargo.toml` (workspace) — add `atlas-analyzers` to `members`.

**Public API:**
```rust
pub trait Analyzer: Send + Sync {
    fn id(&self) -> &str;
    fn stage(&self) -> Stage;
    fn cost_class(&self) -> CostClass;
    fn version(&self) -> &str;

    fn applies(&self, target: &Target) -> bool;
    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput>;
    fn analyse(&self, ctx: &AnalysisContext, target: &Target) -> AnalyzerResult;
}

pub enum AnalyzerResult {
    Confident(Box<dyn StageOutput>),
    Graded { output: Box<dyn StageOutput>, confidence: f32 },
    Declines,
    Error(AnalyzerError),
}

pub struct AnalyzerRegistry { /* ordered analyser list */ }

impl AnalyzerRegistry {
    pub fn builtin() -> Self;                          // 3 reference analysers
    pub fn merge_yaml(&mut self, yaml: &AnalyzersFile);
    pub fn dispatch(&self, ctx: &AnalysisContext, target: &Target) -> AnalyzerResult;
}
```

**Acceptance criteria:**
- `cargo test --workspace` passes.
- New unit test: `dispatch_picks_cheapest_applicable` — given Cargo + LLM both apply, Cargo wins.
- New unit test: `dispatch_falls_through_on_declines` — given an analyser that returns `Declines`, the dispatcher consults the next.
- New integration test: a fixture with a `Dockerfile` produces a `kind: docker-image` component (no LLM call needed).
- New integration test: a fixture with `Cargo.toml` containing a `[lib]` section is classified `rust-library` by the Cargo analyser without an LLM call.

**LOC:** ~1200-1500.

---

### PR-6 — Scattered per-component `.atlas/` writers

**Intent:** L9 emits a per-component `<component>/.atlas/component.yaml`
in addition to the top-level `<output>/.atlas/components.yaml`. The
top-level file remains the synthesis. Wires per-component
`overrides.yaml` discovery per PR-0c.

**Files:**
- Modify: `crates/atlas-engine/src/l9_projections.rs` — add
  `per_component_yaml_snapshot(db: &AtlasDatabase, component_id: &ComponentId) -> Arc<PerComponentFile>`. The output is a single-component projection plus an
  `analyser_id`, `analyser_version`, `fingerprint`, `surfaces_path: surfaces.yaml`, `overrides_path: overrides.yaml` envelope.
- Modify: `crates/atlas-cli/src/pipeline.rs` — after writing the top-level YAMLs, walk every component and write its per-component file. Atomic write (tempfile-then-rename), one mkdir-p per component path.
- Modify: `crates/atlas-engine/src/l4_tree.rs` — extend override merge to discover per-component `overrides.yaml` files per the §2.2 spec.

**Acceptance criteria:**
- `cargo test --workspace` passes.
- New integration test: `tests/scattered_atlas_layout.rs` runs `atlas index` on a multi-component fixture and asserts every component path has a `.atlas/component.yaml` whose content matches the projection of that component from the top-level `components.yaml`.
- New unit test: per-component override file scoping (per §2.2) — a per-component override file containing a pin for an unrelated component fails the run with a clear error.
- New unit test: discovery order — a per-component pin overrides a top-level pin for the same `(component_id, key)` tuple.

**LOC:** ~700-1000.

---

### PR-7 — `surfaces.yaml` emission (Rust binding shape)

**Intent:** L5 produces a `surfaces.yaml` per component with
`contracts_defined`, `contracts_implemented`, `contracts_consumed`, and
`library_apis`. Phase 1 emits Rust bindings only; `schema_version: 1`.

**Files:**
- Modify: `crates/atlas-engine/src/l5_surface.rs` — extend `SurfaceRecord` (kept as the inner record) with new top-level fields `contracts: Vec<Contract>`, `bindings: Vec<Binding>`, `library_apis: Vec<LibraryApi>`. Wire to the analyser registry (PR-5).
- Create: `crates/atlas-analyzers/src/rust_surface_analyzer.rs` — Phase 1 surface analyser for Rust components. Uses the existing LLM Stage 1 path to extract `purpose`, `consumes_files`, etc., and adds binding-extraction by reading `pub` items from `src/lib.rs` and `src/main.rs` (regex-based; rust-analyzer wire-up is Phase 2). For each `pub struct` with `#[derive(Serialize, Deserialize)]`, emit a `data-format` contract whose binding is the struct definition.
- Modify: `crates/atlas-engine/src/l9_projections.rs` — add `surfaces_yaml_snapshot(db, component_id) -> Arc<SurfacesFile>`.
- Modify: `crates/atlas-cli/src/pipeline.rs` — write `<component>/.atlas/surfaces.yaml` per component during the post-index walk (alongside PR-6's `component.yaml` writer).

**Acceptance criteria:**
- New integration test: `tests/surfaces_emission_rust.rs` runs a multi-crate fixture and asserts that a crate with a `pub struct Foo` (serde-derived) emits a `data-format` contract whose binding span matches the `Foo` definition.
- Cache test: contract content sha is stable across whitespace changes inside the binding span (verifies PR-0b canonicalisation rule for code-derived contracts).
- Schema-version check: `surfaces.yaml` reads as `schema_version: 1`.

**LOC:** ~900-1200.

---

### PR-8 — Contract participants in `related-components.yaml` + edge kinds

**Intent:** L6 emits `defines-contract`, `implements-contract`, `consumes-contract` edges between components and contracts. Contracts appear as edge participants by their fully-qualified id (e.g., `atlas-contracts/components-yaml-schema`), not as components.

**Files:**
- Modify: `component-ontology/src/lib.rs` — `EdgeKind` already extended in PR-1; this PR adds the participant-resolution validator (`validate_contract_participants_resolve`) that walks every contract participant in `related-components.yaml` and asserts a corresponding contract definition exists in some per-component `surfaces.yaml`.
- Modify: `crates/atlas-engine/src/l6_edges.rs` — extend the candidate-edge proposer to read every component's surface (from PR-7's emission) and emit the three contract edge kinds. The edge participants list is `[component_id, contract_id]` (component first, contract second; the order is canonical per design §6.4).
- Modify: `crates/atlas-engine/src/l9_projections.rs` — `related_components_yaml_snapshot` includes the new edges in lexicographic sort order.
- Modify: `crates/atlas-cli/src/pipeline.rs` — call the participant validator after writing `related-components.yaml`; emit a hard error if a contract participant does not resolve.

**Acceptance criteria:**
- New integration test: a fixture with two crates (one defining a `data-format` contract, one consuming it) produces a `related-components.yaml` containing a `defines-contract` edge and a `consumes-contract` edge.
- Validator test: a hand-crafted `related-components.yaml` with a `consumes-contract` edge whose contract id resolves to no defining component fails the run.
- New unit test: a workspace where every component has an empty `surfaces.yaml` produces no contract edges.

**LOC:** ~700-1000.

---

### PR-9 — Composition edges from Dockerfiles

**Intent:** Dockerfile-driven composition edges (`bundled-into`, `deployed-with`) populate `related-components.yaml`. Builds on the Dockerfile analyser from PR-5.

**Files:**
- Modify: `crates/atlas-analyzers/src/dockerfile_classifier.rs` — extend to extract `COPY` source paths and resolve them to source components (by enclosing-manifest-root lookup). Each resolved (source-component, deliverable) pair becomes a `bundled-into` edge candidate.
- Modify: `crates/atlas-engine/src/l6_edges.rs` — the Dockerfile analyser is consulted at L6 to emit composition edges. The deliverable-component is the Dockerfile's enclosing component (a `kind: docker-image` component already created by PR-5's L1/L3 work).
- Modify: `crates/atlas-engine/src/l9_projections.rs` — composition edges emitted in lexicographic order alongside contract edges and v1 edges.

**Acceptance criteria:**
- New integration test: a fixture with `deploy/billing/Dockerfile` containing `COPY target/release/billing-core /usr/local/bin/` produces a `bundled-into` edge from `<crate-namespace>/billing-core` to `<crate-namespace>/billing-image`.
- `deployed-with` edge test: a Dockerfile bundling two binaries from the same source tree produces a `deployed-with` edge between them (symmetric).
- Test fixture without Dockerfiles: no composition edges emitted.

**LOC:** ~800-1100.

---

### PR-10 — Wire persistent cache into L3 / L5 / L6

**Intent:** Make the persistent content-addressed cache (PR-2) the
durability layer for LLM call results. The in-memory `LlmResponseCache`
becomes a write-through wrapper. The v1 `cache_io.rs` (JSON file) is
**deleted** under greenfield treatment.

**Files:**
- Modify: `crates/atlas-engine/src/llm_cache.rs` — `LlmResponseCache::new_with_persistent(PersistentCache)` constructor; `call_cached` now writes both layers (in-memory hit short-circuits the persistent read).
- **Delete:** `crates/atlas-cli/src/cache_io.rs` and its module declaration in `lib.rs`.
- Modify: `crates/atlas-cli/src/pipeline.rs` — open the persistent cache at startup; pass it to `AtlasDatabase::new`. Remove all `cache_io::load_into` and `cache_io::save_from` calls.
- Modify: `crates/atlas-engine/src/l3_classify.rs`, `l5_surface.rs`, `l6_edges.rs` — every LLM call contributes a fingerprint via `FingerprintBuilder` (PR-2).

**Acceptance criteria:**
- `cargo test --workspace` passes.
- New persistent-cache test: a no-op re-run after `rm -rf` only the in-memory state (i.e., a fresh process invocation) makes zero LLM calls — every entry hits the persistent cache.
- New persistent-cache test: a single file content change invalidates only the entries whose fingerprint cited that file.
- New persistent-cache test: deleting `<output>/.atlas/cache/` forces a full rerun (every entry misses) and the cache rebuilds.

**LOC:** ~700-900.

---

### PR-11 — L6 cache key includes participant surface shas

**Intent:** Wire `Binding::content_sha` and `SurfacesFile::fingerprint` into the L6 fingerprint per design §8.2. When a participant component's surface sha changes, every L6 cache entry naming that participant misses on next access.

**Files:**
- Modify: `crates/atlas-engine/src/l6_edges.rs` — `candidate_edges_for(component_id)` reads the surface fingerprint of every potential edge participant (the union of components in the same root + components in linked roots) and contributes each participant's sha to the L6 fingerprint via `FingerprintBuilder::add_participant_surface_sha`.
- Modify: `crates/atlas-engine/src/l9_projections.rs` — `surfaces_yaml_snapshot` emits a top-level `fingerprint` field (already specified in PR-7's schema; this PR makes it load-bearing).

**Acceptance criteria:**
- New integration test: a two-crate fixture where crate-B consumes a contract from crate-A. Run once (cold), then re-run after editing crate-A's defining binding. Crate-B's L6 cache entry misses and recomputes; a no-op re-run after that hits.
- New cross-tree test: same as above but crate-A is in a peer root reached via path-dep. Cache invalidation propagates across roots.
- New stability test: a workspace with no contract edges has the same L6 fingerprint before and after PR-11 (the participant-sha contribution is empty when the participant surface has no contracts).

**LOC:** ~600-900.

---

### PR-12 — Acceptance: atlas-contracts visible in Ravel-Lite

**Intent:** End-to-end smoke test that exercises the full Phase 1 seam — multi-root via path-dep walking, scattered `.atlas/`, contract edges, persistent cache, cross-tree invalidation.

**Files:**
- Create: `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` — checked-in fixture that mirrors the real Ravel-Lite + atlas-contracts layout (one Ravel-Lite consumer crate, one atlas-contracts defining crate, linked via path-dep). Runs `atlas index` on the Ravel-Lite primary root and asserts:
  1. The output `components.yaml` lists components from both roots.
  2. The output `related-components.yaml` contains a `consumes-contract` edge from the Ravel-Lite consumer to a contract id under the `atlas-contracts/` namespace.
  3. The atlas-contracts defining component has a per-component `surfaces.yaml` whose `contracts_defined` includes that contract id.
  4. A no-op re-run makes zero LLM calls (persistent cache hit).
  5. Editing the defining binding's source invalidates only the consumer's L6 cache entry on the next run.

**Acceptance criteria:**
- The fixture-based test passes from a clean checkout in CI.
- The same flow run manually against the real `~/Development/Ravel-Lite` and `~/Development/atlas-contracts` repos produces non-empty contract edges and a populated `surfaces.yaml` for atlas-contracts components. (Manual verification, recorded in the PR description.)

**LOC:** ~400-600 fixture + test.

---

## 5. Acceptance criteria summary (per-PR table)

The following table is the canonical acceptance gate. A PR may not land
until every row in its column is green.

| PR | Tests pass | New unit/integration tests | Smoke test contributes to |
|---|---|---|---|
| PR-0  | n/a (docs)         | n/a                                                           | n/a |
| PR-1  | atlas-index, ontology | round-trip serde for every new type                       | nothing |
| PR-2  | atlas-engine cache | atomic write, GC, fingerprint determinism (proptest)          | PR-12 step 4 |
| PR-3  | workspace          | two-root union, single-root cache hit, two-root cache hit     | PR-12 step 1 |
| PR-4  | workspace          | path-dep fixed-point, cycle warning                           | PR-12 step 1 |
| PR-5  | workspace          | dispatcher cheapest-applicable, declines fallthrough, Dockerfile, Cargo no-LLM | PR-12 step 4 |
| PR-6  | workspace          | scattered layout integration, scoping rejection, discovery order | PR-12 step 3 |
| PR-7  | workspace          | rust binding emission, contract content sha stability         | PR-12 step 3 |
| PR-8  | workspace          | contract edge emission, participant validator, empty-surfaces case | PR-12 step 2 |
| PR-9  | workspace          | Dockerfile bundled-into, deployed-with, no-Dockerfile baseline | (smoke test does not include Dockerfiles in Phase 1) |
| PR-10 | workspace          | persistent-only cache hit, single-file invalidation, cache-deleted full rerun | PR-12 step 4 |
| PR-11 | workspace          | cross-tree invalidation, edit propagation, no-contract stability | PR-12 step 5 |
| PR-12 | e2e                | atlas-contracts-in-ravel-lite five-step assertion             | this *is* the smoke test |

---

## 6. Risks (Phase 1 specific)

These are operational risks for the implementation, supplementing
design §12 (which covers architectural risks).

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| PR-3 breaks the Salsa cache hit on the in-tree fixtures due to a missed `workspace.root()` callsite. | High | Medium | The cache-hit-on-no-op-rerun test added in PR-3 is the canonical guard. CI fail-blocks the merge. |
| PR-5's analyser registry leaks dispatch overhead on the hot Cargo path. | Medium | Low | Benchmark before/after PR-5 on the new fixtures; keep dispatch under 1ms per analyser call. |
| PR-7's regex-based binding extraction misses nested `pub` items (e.g., `pub mod foo { pub struct Bar; }`). | High | Low | Phase 1 ships the cheap implementation; Phase 2 swaps in rust-analyzer. The regex limitation is documented in PR-7's description. |
| PR-10's dual-write (in-memory + persistent) doubles I/O on hot LLM-call paths. | Medium | Medium | The persistent write is async (background thread); the in-memory cache returns synchronously. A flush is performed at pipeline end before exit. |
| The `atlas-contracts visible in Ravel-Lite` smoke test fails because Ravel-Lite's `Cargo.toml` path-dep is to a workspace member not a crate root. | Medium | High | PR-4's manifest-root walk handles workspace members correctly; PR-12's fixture exercises both crate-root and workspace-member path-deps. |
| Two PRs land out of dependency order due to merge timing. | Low | High | The PR descriptions explicitly list `Depends on: PR-N`; CI should refuse to merge a PR whose dependency target has not yet landed. |
| A user upgrading runs the new binary against an old `.atlas/` and gets a confusing parse error. | Medium | Low | The new readers detect the v1 shape (e.g., a `root: PathBuf` field where `roots: Vec<PathBuf>` is expected) and emit a clear "delete `.atlas/` and re-run — the on-disk format changed in vNext" error. No migration code; just a friendly error. |

---

## 7. Out of scope for Phase 1

These items are deferred to later phases, per design §10.2-§10.4. A
reviewer who flags them as missing should redirect to the relevant
phase.

- Subprocess analyser transport (Phase 2).
- Non-Rust language analysers (Phase 2).
- npm / TypeScript / Python / Cabal / .csproj / pyproject.toml / shell-script analysers (Phase 2).
- k8s / compose / helm analysers (Phase 2).
- Drift report, impact query, modularity report (Phase 3).
- Pattern detection (Phase 3).
- Composition divergence report (Phase 3).
- Server mode: file watcher, query API, subscriptions (Phase 4).
- Grafeo derived index (Phase 4+).
- Cache compression (deferred — see design §11.2.7).
- Worktree commit-sha consistency validation (deferred — see design §11.2.8).
- Contract rename-match (Phase 2).
- `--strict-overrides` flag (Phase 2).
- **Migration / backwards compatibility with v1 on-disk formats**
  (greenfield treatment per the plan header; v1 readers, the
  `atlas migrate-v1` command, and the v1 `llm-cache.json` format are
  all deleted, not preserved).

---

## 8. References

- Design spec: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
- Open-question resolutions (companion specs landing with PR-0):
  - `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md`
  - `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`
- v1 mechanisms reused as starting points:
  - `crates/atlas-engine/src/db.rs` — Salsa workspace input.
  - `crates/atlas-engine/src/llm_cache.rs` — in-memory LLM cache.
  - `crates/atlas-engine/src/l4_tree.rs` — rename-match + override merge.
  - `crates/atlas-cli/src/backend.rs` — BudgetSentinel + TokenCounter.
  - `atlas-contracts/crates/atlas-index/src/rename_match.rs` — rename-match algorithm.
  - `atlas-contracts/crates/component-ontology/` — edge-kind vocabulary.
- Memory entries that constrain Phase 1:
  - `feedback_toml_parsing` — PR-4 must use the `toml` crate.
  - `tombstone_emit_once_design` — PR-3 must not break L4 prior-filter.
  - `all_components_not_salsa_tracked` — PR-3 must keep the tree-assembly memoisation in the CLI/L9 layer.
  - `project_distribution_brew_bottles` — `atlas-analyzers` (PR-5) is a new workspace member; release pipeline notes apply.
  - `feedback_fix_all_lints` — every PR runs `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
