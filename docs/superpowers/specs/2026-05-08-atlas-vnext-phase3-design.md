# Atlas vNext — Phase 3 design: drift, impact, modularity, composition divergence

**Status:** Design spec. Implementation plan to follow via the
`superpowers:writing-plans` skill (separate file, dated alongside this one).

**Greenfield non-negotiable:** as with Phase 1 and Phase 2, Phase 3 makes no
on-disk format-compatibility promises. A user upgrading deletes `.atlas/` and
re-runs. There is no migration command.

---

## 0. Reading order

1. §1 Summary — what Phase 3 ships, in one screen.
2. §2 Scope — what's in, what's deferred.
3. §3 Architecture — the new `atlas-reports` crate and how it fits.
4. §4 The four reports — per-analysis specs (formulas, schemas, semantics).
5. §5 Editorial-vs-derived rule — the file-classification regime introduced
   by Phase 3, including the Phase 1 retrofit work that lands in this phase.
6. §6 Cache discipline — pure-derived vs stateful-derived files, atomic
   write requirements.
7. §7 Testing strategy — per-PR units, stateful-file tests, Phase 1
   retrofit regression sweep, integration smoke-test fixture.
8. §8 Risks — Phase 3 specific.
9. §9 Out of scope — deferred to Phase 4 / Phase 5.
10. §10 Design-doc touch-ups — what changes in the project-wide
    `2026-05-06-atlas-system-model-design.md`.
11. §11 References.

---

## 1. Summary

Phase 3 delivers the four LLM-tooling-facing analyses listed in design §10.3
(minus pattern detection, deferred to Phase 4):

- **Drift report** — contracts whose content sha changed since the previous
  `atlas drift` run, with the list of bindings still pinned to the prior sha.
- **Impact query** — given a contract or component id, the transitive
  downstream consumer set, partitioned by language, deploy graph, and
  lifecycle.
- **Modularity report** — per-component coupling (Ca/Ce/Instability),
  cohesion, surface stability, surface complexity. Per-subsystem aggregates
  with >2σ outlier flags.
- **Composition divergence report** — components that are deploy-coupled but
  not build-coupled (or vice versa), severity = count of contract content-sha
  changes since drift baseline on the divergent edge.

Phase 3 also lands a project-wide cleanup that the report machinery makes
unavoidable to confront: the **editorial-vs-derived classification** of
on-disk files. Phase 1's `surfaces.yaml`, per-component `component.yaml`,
top-level `components.yaml`, and `related-components.yaml` are reclassified
as derived (gitignored, under `cache/`). User assertions previously implicit
in those files migrate to extended `overrides.yaml` schemas. The editorial
tier collapses to **six file types**: top-level `overrides`,
`external-components`, `subsystems`, `analyzers`, `config`, and per-component
`overrides`. Everything else is cache.

All Phase 3 work is greenfield (no migration); CLI-shaped (no server-mode
machinery — server is Phase 5); and introduces zero new LLM call sites (every
report is a deterministic projection of L4–L8 engine outputs).

Estimated PR count: ~16 PRs (4 cache-path-migration + 1 overrides-schema +
6 report-implementation + 4 plumbing + 1 smoke-test integration). Comparable
to Phase 2's 15 PRs.

---

## 2. Scope and non-goals

### 2.1 In scope

- New `crates/atlas-reports/` workspace member: pure-function library
  exposing `drift()`, `impact()`, `modularity()`, `divergence()`.
- Four CLI subcommands: `atlas drift`, `atlas impact <id>`, `atlas
  modularity`, `atlas divergence`. Pure subcommands; `atlas index` does not
  auto-trigger any of them.
- File outputs (all gitignored, under `cache/`):
  - Drift baseline snapshot (stateful).
  - Drift, modularity rollup, composition divergence reports (top-level).
  - Per-component modularity report with 5-deep history (stateful).
- Stdout-only output for impact (`--json`/`--yaml`/`--human` formats).
- **Phase 1 retrofit**: relocate `surfaces.yaml`, per-component
  `component.yaml`, top-level `components.yaml`, and `related-components.yaml`
  from git-tracked locations to gitignored `<scope>/.atlas/cache/`.
- **Overrides schema extension**: add `edges_add` and `edges_suppress` to
  top-level `overrides.yaml`; add field-level overrides (`language`,
  `kind`, `lifecycle`, `subsystem`) to per-component `overrides.yaml`.
- **Gitignore mechanism**: `atlas-cli` writes `.atlas/.gitignore` with one
  line `cache/` at each `.atlas/` scope on first cache-write. Idempotent;
  honoured if user has customised.
- Modularity formulas (§4.3 below): Ca/Ce/Instability (Robert Martin),
  LCOM4-adapted cohesion, surface stability over last-5-indexes,
  surface complexity = `provided_contracts × avg_bindings_per_contract`.
- Subsystem aggregates with >2σ outlier flags. Reuses existing
  `subsystems.yaml`.
- Integration smoke-test fixture exercising all four reports end-to-end.

### 2.2 Out of scope (deferred)

- **Pattern detection** (originally in design §10.3) — deferred to Phase 4
  along with the LLM-callback-channel and confidence-calibration machinery
  it would need.
- **Server mode** (file watcher, reactive recomputation, query API,
  subscriptions) — Phase 5.
- **All Phase 2 §7 cleanup deferrals**: subprocess convergence, `rust-analyzer`
  integration, TS-as-subprocess, bidirectional LLM callback, `--strict-overrides`
  flag, contract rename-match (§11.2.4), confidence calibration (§11.2.6),
  cache compression (§11.2.7), worktree commit-sha annotations (§11.2.8) — all
  Phase 4.
- **Phase 2 closeout cleanups**: `LenientBackend` test-helper extraction,
  decoder consolidation onto `decode_subprocess_surface_payload`,
  `is_manifest_file` extension for Makefile/shell auto-discovery, L8
  phantom-subcomponent fix, per-language Phase 3 refinements (full
  tree-sitter-dart, raco-driven Racket dep resolution, Phoenix sub-kinds,
  Mix umbrella, LispKit symbolic resolution) — all Phase 4.
- **Salsa-tracking the report queries**. `atlas-reports` ships as pure
  functions with a documented Phase 5 conversion path. Salsa pays its way
  only when something upstream is changing during compute (server mode).
- **Upstream / subsystem-input variants of impact query.** Design-minimum
  per §10.3.
- **`--gate` / `--strict` exit-code flags** for CI integration. Reports
  ship with `--json` for machine consumption; CI scripting on top is
  trivial; an opinionated gate flag is a future-PR call.
- **Severity / threshold interpretation of modularity scores.** Numbers
  only; user interprets. Pass/fail thresholds are a downstream tooling
  concern.
- **Cohesion formula calibration.** Ships with the v1 LCOM4-adapted
  formula explicitly flagged as subject-to-revisit after first real-world
  numbers from dull and Linkuistics.

### 2.3 Non-negotiables

- **Greenfield.** No on-disk format compat with Phase 1/2. Upgrading users
  delete `.atlas/` and re-run.
- **Tests are the gate.** Acceptance criteria in the eventual Phase 3 plan
  are non-negotiable. Subagents run `cargo test/clippy/fmt` clean before
  reporting DONE; orchestrator independently re-verifies.
- **Lints and fmt clean everywhere** (memory `feedback_fix_all_lints`).
- **Use the `toml` crate** (memory `feedback_toml_parsing`).
- **Use the existing `serde_yaml` crate** for all YAML reads/writes.
- **No new LLM call sites.** Every Phase 3 report is deterministic.

---

## 3. Architecture

### 3.1 New crate: `crates/atlas-reports/`

A new workspace member. Pure-function library: no I/O for input data, all
I/O for output through caller-supplied paths. No engine queries fired from
inside the crate; reports observe an already-computed engine output.

### 3.2 Public API surface (initial sketch)

```rust
pub struct ReportInputs<'a> {
    pub db: &'a EngineDb,
    pub workspace: &'a Workspace,
}

pub fn drift(
    inputs: ReportInputs,
    prev_snapshot: Option<ContractShaSnapshot>,
) -> (DriftReport, ContractShaSnapshot);

pub fn impact(
    inputs: ReportInputs,
    target: ImpactTarget,
) -> Result<ImpactReport, ImpactError>;

pub fn modularity(
    inputs: ReportInputs,
    prior_per_component: HashMap<ComponentId, ModularityHistory>,
) -> ModularityReport;

pub fn divergence(
    inputs: ReportInputs,
    drift_baseline: Option<&ContractShaSnapshot>,
) -> DivergenceReport;
```

Notes:

- Drift returns *both* the report and the new snapshot, separating "what
  changed" from "advance the baseline forward". The CLI handler writes both
  files atomically.
- Impact returns a `Result` because target-id-not-found is a real failure
  mode (CLI exits with code 2 + stderr suggestion list).
- Modularity takes per-component prior history; the CLI handler reads each
  component's prior `modularity.yaml` before invocation and assembles the
  map. This keeps `atlas-reports` I/O-free.
- Divergence reads but does not modify the drift snapshot — drift owns that
  state.

### 3.3 Data flow

```
atlas index   ──▶  EngineDb  ──┬──▶  atlas-reports::drift     ──▶  CLI writes
                                │      ┌─▶  .atlas/cache/reports/drift.yaml
                                │      └─▶  .atlas/cache/contract-shas-snapshot.yaml
                                ├──▶  atlas-reports::impact    ──▶  CLI prints stdout (JSON or human)
                                ├──▶  atlas-reports::modularity ─▶  CLI writes
                                │      ┌─▶  per-component .atlas/cache/modularity.yaml (each)
                                │      └─▶  .atlas/cache/reports/modularity-rollup.yaml
                                └──▶  atlas-reports::divergence ─▶  CLI writes
                                       └─▶  .atlas/cache/reports/composition-divergence.yaml
```

The reports never trigger engine recomputation. They observe whatever the
engine has already produced.

### 3.4 CLI dispatch

Subcommands:

- `atlas drift [--json|--yaml|--human] [--no-write]`
- `atlas impact <id> [--json|--yaml|--human]`
- `atlas modularity [--json|--yaml|--human] [--no-write]`
- `atlas divergence [--json|--yaml|--human] [--no-write]`

Each subcommand:

1. Loads the engine database (cached or recomputed; the existing engine
   layer makes this transparent).
2. Calls the corresponding `atlas-reports` function.
3. Renders output to stdout in the requested format (default: `human`).
4. Writes the persistent file(s) unless `--no-write` is set
   (`--no-write` is rejected for `impact`, which never persists).

`atlas index` is unchanged: it computes the graph and writes engine outputs.
It does **not** auto-trigger any report.

### 3.5 Phase 5 conversion path (documented, not implemented)

When server mode (Phase 5) is built, each pure function in `atlas-reports`
becomes a Salsa query:

- The function body migrates into a `#[salsa::tracked]` query method.
- `prev_snapshot` (drift) becomes a Salsa input updated by the file-watcher
  on snapshot writes.
- `prior_per_component` (modularity) becomes a Salsa input keyed by
  component id.
- File-write side-effects move out of `atlas-reports` and into the server's
  settled-state writer.
- Public API shape stays identical; CLI callers don't need to change.

This conversion is mechanical because the Phase 3 functions are already
side-effect-free over their inputs.

---

## 4. The four reports

### 4.1 Drift report

#### Inputs

- Current engine output: every contract with its current `content_sha`,
  every binding with its `binding_content_sha` and `derived_from_contract_sha`
  (the contract sha the binding was last computed against — Phase 1's
  surface schema records this).
- Optional prior snapshot from `.atlas/cache/contract-shas-snapshot.yaml`.
  Absent on first run.

#### Snapshot format (`.atlas/cache/contract-shas-snapshot.yaml`)

```yaml
schema_version: 1
captured_at: 2026-05-08T14:23:01Z
contract_shas:
  - id: "atlas-contracts/index-schema/v1"
    content_sha: "sha256:abc123..."
  - id: "atlas-contracts/eval-schema/v1"
    content_sha: "sha256:def456..."
```

Sorted by id for deterministic file content.

#### Drift report schema (`.atlas/cache/reports/drift.yaml`)

```yaml
schema_version: 1
generated_at: 2026-05-08T14:25:01Z
baseline_captured_at: 2026-05-07T09:11:42Z          # null on first run
contracts_changed:
  - id: "atlas-contracts/index-schema/v1"
    prior_content_sha: "sha256:abc123..."
    current_content_sha: "sha256:abc999..."
    pinned_bindings:
      - component: "ravel-lite/api"
        binding_content_sha: "sha256:bind7..."
        pinned_to: "sha256:abc123..."             # the prior sha
        language: typescript
      - component: "ravel-lite/worker"
        binding_content_sha: "sha256:bind8..."
        pinned_to: "sha256:abc123..."
        language: rust
contracts_added:
  - id: "atlas-contracts/new-schema/v1"
    current_content_sha: "sha256:new111..."
contracts_removed:
  - id: "atlas-contracts/dead-schema/v1"
    prior_content_sha: "sha256:dead222..."
summary:
  total_contracts: 47
  changed: 1
  added: 1
  removed: 1
  pinned_bindings_count: 2
```

#### Semantics

- A binding is "pinned to the prior sha" iff
  `binding.derived_from_contract_sha == prior_content_sha != current_content_sha`.
- Bindings whose `derived_from_contract_sha == current_content_sha` are
  up-to-date; not reported.
- Contracts in `contracts_removed` may actually be renames; rename-match is
  Phase 4 (§11.2.4). v1 reports them as removed.

#### First-run UX

- No snapshot → `drift.yaml` is written with `baseline_captured_at: null`
  and all change arrays empty; only `summary.total_contracts` is populated.
- CLI prints: *"No prior baseline found. Captured baseline of N contracts.
  Run `atlas drift` again after changes to see drift."*
- Exit code: 0.

#### Run order

1. Load engine outputs.
2. Read prior snapshot (or `None`).
3. Compute report and new snapshot.
4. Write `.atlas/cache/reports/drift.yaml` (atomic temp+rename).
5. Write `.atlas/cache/contract-shas-snapshot.yaml` (atomic temp+rename).
6. Print summary.

### 4.2 Impact query

#### Invocation

```
atlas impact <id> [--json | --yaml | --human]
```

`<id>` is either a contract id or a component id. The two namespaces are
disjoint by Phase 1 construction.

#### Traversal

- **Contract input**: walk `consumed-by` edges from the contract; recurse
  through every consumer's contracts' `consumed-by` edges. Use a seen-set
  for cycle safety.
- **Component input**: union of impact sets across every contract the
  component provides.
- Edge type used: `consumes` (contract-consumer edge from
  `related-components.yaml`). `depends-on` build edges are *not* walked.
- Direction: downstream consumers only.

#### Output schema (stdout)

```yaml
schema_version: 1
generated_at: 2026-05-08T14:30:11Z
target:
  kind: contract        # or component
  id: "atlas-contracts/index-schema/v1"
direct_consumers:
  - "ravel-lite/api"
  - "ravel-lite/worker"
transitive_consumers:
  - "ravel-lite/api"
  - "ravel-lite/worker"
  - "ravel-lite/dashboard"
  - "ops/observability-shipper"
partitions:
  by_language:
    typescript: ["ravel-lite/api", "ravel-lite/dashboard"]
    rust: ["ravel-lite/worker"]
    elixir: ["ops/observability-shipper"]
  by_deploy_graph:
    "compose:dev": ["ravel-lite/api", "ravel-lite/worker", "ravel-lite/dashboard"]
    "compose:ops": ["ops/observability-shipper"]
  by_lifecycle:
    runtime: ["ravel-lite/api", "ravel-lite/worker", "ravel-lite/dashboard", "ops/observability-shipper"]
    build-time: []
    test-only: []
summary:
  direct_count: 2
  transitive_count: 4
```

**Three independent partitions, not a 3D grid.** Each `partitions.<axis>`
maps every component in `transitive_consumers` to its value on that axis;
the same component appears in all three axes.

`human` format prints an indented tree of consumers with annotations.

#### Edge cases

- Cycles (legal): cycle members appear once each; seen-set traversal.
- Target not found: exit code 2; stderr lists Levenshtein-1 candidates;
  no file written.
- Empty result: exit 0; arrays empty; `summary.direct_count: 0`.

### 4.3 Modularity report

#### Concrete metric definitions

| Metric | Formula | Notes |
|---|---|---|
| Afferent coupling (Ca) | count of distinct components that consume any contract this component provides | `consumes` edges only. Self-loops excluded. |
| Efferent coupling (Ce) | count of distinct components whose contracts this component consumes | `consumes` edges only. Self-loops excluded. |
| Instability (I) | `Ce / (Ca + Ce)` | Range 0.0–1.0. When `Ca + Ce == 0`, defined as `0.0`. |
| Cohesion | `1 - ((distinct_consumer_sets - 1) / (num_provided_contracts - 1))` | LCOM4-adapted. With 0 or 1 provided contracts, defined as `1.0` (vacuous — no fragmentation possible). With all-contracts-share-one-consumer-set: `1.0` (perfect cohesion). With every-contract-has-a-unique-consumer-set: `0.0`. |
| Surface stability | `matching_adjacent_pairs / total_adjacent_pairs` over the history of last 5 indexes | An adjacent pair `(entry[i], entry[i+1])` matches iff their `surface_fingerprint` fields are equal. With N history entries there are N-1 adjacent pairs; `total_adjacent_pairs = max(N-1, 0)`. With <2 history entries (no pairs possible), defined as `1.0`. |
| Surface complexity | `provided_contracts × avg_bindings_per_contract` | Integer. Raw count. |

**Cohesion v1 calibration caveat:** The LCOM4-adapted formula is the
highest-uncertainty piece. Real-world numbers may show it pegged near 1.0 or
0.0 in unhelpful ways. The spec ships the v1 formula as-is and explicitly
flags it as **subject to revisit after Phase 3 ships against dull and
Linkuistics**. Any revision is a docs+formula change; the schema does not
change.

#### Per-component file schema (`<component>/.atlas/cache/modularity.yaml`)

```yaml
schema_version: 1
component_id: "ravel-lite/api"
generated_at: 2026-05-08T14:35:11Z
metrics:
  afferent_coupling: 3
  efferent_coupling: 2
  instability: 0.4
  cohesion: 0.83
  surface_stability: 1.0
  surface_complexity: 8
history:
  - generated_at: 2026-05-07T09:11:42Z
    surface_fingerprint: "sha256:hist1..."
    metrics: { afferent_coupling: 3, efferent_coupling: 2, instability: 0.4, cohesion: 0.83, surface_complexity: 8 }
  - generated_at: 2026-05-06T15:02:11Z
    surface_fingerprint: "sha256:hist2..."
    metrics: { ... }
  # up to 5 entries, oldest dropped on rotation
```

History is embedded in the per-component file (not split into a sibling
history file). Reasoning: ergonomic (one file per component), naturally
bounded at 5 entries (~1KB/entry), per-developer per-checkout state.

**History rotation semantics:**

- On a run where input fingerprint matches the most-recent history entry's
  `surface_fingerprint`: no append (no duplicate); file rewritten only if
  `generated_at` field changes.
- On a run where input fingerprint differs: prepend current entry; if total
  entries >5, drop oldest.
- History entries are immutable once written.

#### Subsystem aggregate

For each subsystem in `subsystems.yaml`, compute `mean` and `stddev` of each
metric across member components. Flag any member whose value is `>2σ` from
the subsystem mean as an outlier *for that metric*. Components not in any
subsystem are excluded from rollup aggregates and listed in
`unattached_components`.

#### Top-level rollup schema (`.atlas/cache/reports/modularity-rollup.yaml`)

```yaml
schema_version: 1
generated_at: 2026-05-08T14:35:11Z
subsystems:
  - id: "ravel-lite/runtime"
    members: ["ravel-lite/api", "ravel-lite/worker"]
    aggregates:
      afferent_coupling: { mean: 2.5, stddev: 0.7 }
      efferent_coupling: { mean: 1.5, stddev: 0.7 }
      instability:       { mean: 0.45, stddev: 0.07 }
      cohesion:          { mean: 0.81, stddev: 0.05 }
      surface_stability: { mean: 1.0, stddev: 0.0 }
      surface_complexity:{ mean: 7.5, stddev: 0.7 }
    outliers: []
  - id: "ravel-lite/observability"
    members: ["ops/observability-shipper", "ops/log-aggregator", "ops/metrics-collector"]
    aggregates: { ... }
    outliers:
      - component_id: "ops/log-aggregator"
        metric: "instability"
        value: 0.95
        subsystem_mean: 0.50
        deviation_sigmas: 2.4
unattached_components:
  count: 3
  ids: ["misc/scratch-tool", "misc/seed-script", "misc/loadgen"]
```

#### Run order

1. Load engine outputs.
2. Read prior `<component>/.atlas/cache/modularity.yaml` for each component
   (history); `None` if absent.
3. Compute current metrics for each component; rotate history.
4. Write each per-component file (atomic).
5. Read `subsystems.yaml`; compute aggregates and outliers.
6. Write rollup (atomic).
7. Print summary.

### 4.4 Composition divergence report

#### Inputs

- Build graph: `depends-on` edges from L4–L8.
- Deploy graph: composition edges from `related-components.yaml`
  (`bundled-into`, `co-deployed-with`, `orchestrated-by`, etc.).
- Drift snapshot (read-only) from `.atlas/cache/contract-shas-snapshot.yaml`.

#### Semantics

For each unordered pair of components `{A, B}`:

- Build-coupled: a direct edge in `depends-on` (transitive coupling
  intentionally not flagged in v1; clarity over completeness).
- Deploy-coupled: a direct edge in any composition edge type.

A pair is **divergent** iff exactly one of the two is true.

**Severity = count of contracts the pair shares whose `content_sha`
changed since the drift baseline.** Concretely:

- Shared contracts = the intersection of {contracts A consumes ∪ A provides}
  and {contracts B consumes ∪ B provides}.
- Severity = count of those shared contracts where
  `baseline.contract_shas[id]` is missing (contract added since baseline) OR
  `current_content_sha != baseline.contract_shas[id]` (contract changed since
  baseline). Both cases count as drift relative to baseline.
- If no drift baseline exists, severity is `null` for all pairs and the
  report header notes "drift baseline absent".

#### Output schema (`.atlas/cache/reports/composition-divergence.yaml`)

```yaml
schema_version: 1
generated_at: 2026-05-08T14:40:11Z
drift_baseline_at: 2026-05-07T09:11:42Z          # null if no drift run yet
divergent_pairs:
  - components: ["ravel-lite/api", "ops/observability-shipper"]
    coupling: deploy_only                          # or build_only
    deploy_edges: ["co-deployed-with"]
    severity: 2
    drifting_contracts:
      - "atlas-contracts/log-schema/v1"
      - "atlas-contracts/metric-schema/v1"
  - components: ["ravel-lite/worker", "ravel-lite/dashboard"]
    coupling: build_only
    build_edges: ["depends-on"]
    severity: 0
    drifting_contracts: []
summary:
  total_pairs_examined: 187
  divergent_count: 2
  by_severity: { 0: 1, 1: 0, 2: 1 }
```

#### Run order

1. Load engine outputs (build edges, deploy edges).
2. Read drift snapshot (read-only; `None` if absent).
3. Iterate component pairs; classify; compute severity per divergent pair.
4. Write `.atlas/cache/reports/composition-divergence.yaml` (atomic).
5. Print summary.

---

## 5. Editorial-vs-derived rule

### 5.1 The rule

**A file is editorial iff its content is what the user asserted (or
accepted as correct after analyser emission). A file is derived iff its
content is a function of editorial files plus engine version plus source
content.**

Editorial files are merge-meaningful: the developer is the merge oracle.
3-way text merge works because semantically there is a human who can
choose between conflicting hand-edited values.

Derived files are not merge-meaningful: there is no merge oracle except
"recompute". Standard text merge produces values that are neither branch's
truth.

The rule is enforced by gitignoring derived files. A merge conflict on a
derived file is structurally impossible because git never sees it.

### 5.2 Editorial tier (six file types, in git, mergeable)

| File | Concern |
|---|---|
| `.atlas/overrides.yaml` | All overrides on the synthesised graph (component-field overrides at top level, **`edges_add` / `edges_suppress` for user-asserted or false-positive edges**) |
| `.atlas/external-components.yaml` | User-asserted external nodes |
| `.atlas/subsystems.yaml` | User-tagged subsystem groupings |
| `.atlas/analyzers.yaml` | Analyser registry config |
| `.atlas/config.yaml` | Runtime config |
| `<component>/.atlas/overrides.yaml` | Per-component user assertions (language correction, kind correction, lifecycle correction, subsystem tag). Created on demand only — empty components don't have one. |

### 5.3 Derived tier (gitignored, regenerable, under `<scope>/.atlas/cache/`)

| File | Type |
|---|---|
| `<component>/.atlas/cache/surfaces.yaml` | Phase 1 retrofit (analyser-emitted surface) |
| `<component>/.atlas/cache/component.yaml` | Phase 1 retrofit (analyser-emitted descriptor) |
| `.atlas/cache/components.yaml` | Phase 1 retrofit (synthesised registry projection) |
| `.atlas/cache/related-components.yaml` | Phase 1 retrofit (analyser-discovered edges union edges_add minus edges_suppress) |
| `.atlas/cache/contract-shas-snapshot.yaml` | Phase 3 net-new (drift baseline, stateful) |
| `.atlas/cache/reports/drift.yaml` | Phase 3 net-new (drift report) |
| `.atlas/cache/reports/modularity-rollup.yaml` | Phase 3 net-new (modularity rollup) |
| `.atlas/cache/reports/composition-divergence.yaml` | Phase 3 net-new (divergence) |
| `<component>/.atlas/cache/modularity.yaml` | Phase 3 net-new (per-component modularity, stateful with history) |
| Existing LLM cache, fingerprint cache, Salsa state | Phase 1 — already under `cache/` |

### 5.4 Phase 1 retrofit (in scope for Phase 3)

Four files move from git-tracked to gitignored:

- `<component>/.atlas/surfaces.yaml` → `<component>/.atlas/cache/surfaces.yaml`
- `<component>/.atlas/component.yaml` → `<component>/.atlas/cache/component.yaml`
- `.atlas/components.yaml` → `.atlas/cache/components.yaml`
- `.atlas/related-components.yaml` → `.atlas/cache/related-components.yaml`

The Phase 3 plan will sequence these as standalone migration PRs (one per
file, or grouped where convenient). Each migration PR:

1. Updates the writer to emit at the new path.
2. Updates every reader to read from the new path (grep audit).
3. Removes the old path's writer.
4. Adds gitignore entries via the gitignore mechanism (§5.5).
5. Updates affected tests; verifies engine end-to-end output unchanged
   modulo path differences.

**Greenfield rule applies**: no migration command. A user upgrading deletes
`.atlas/` and re-runs `atlas index`; the new layout populates clean.

### 5.5 Schema additions to overrides

#### Top-level `overrides.yaml`

New fields:

```yaml
edges_add:
  - kind: bundled-into
    from: component-a
    to: component-b
    reason: "manual annotation - we ship them together but no Dockerfile reflects it"
edges_suppress:
  - kind: depends-on
    from: component-a
    to: component-b
    reason: "false positive — analyser sees the import but it's a dev-only path"
```

The engine reads the analyser-discovered edges (cached under
`.atlas/cache/related-components.yaml`), unions `edges_add`, and subtracts
`edges_suppress`. The result is the canonical edge set used by the rest of
the pipeline. **`reason` is required** on both add and suppress entries.

#### Per-component `overrides.yaml`

New fields (all optional):

```yaml
overrides:
  language: rust              # if analyser misclassified
  kind: rust-library          # field override
  lifecycle: build-time       # if analyser got it wrong
  subsystem: ravel-lite/runtime  # tag override
```

Field overrides supersede analyser-emitted values for the corresponding
fields in the per-component `component.yaml` cache blob.

### 5.6 Gitignore mechanism

`atlas-cli` (or any cache-writing path in the engine) writes a one-line
`.gitignore` at each `.atlas/` scope on first cache-write, containing
`cache/`. Idempotent: file written iff absent. If the file exists with
different content (user customised), respect it but log a warning that
`cache/` was not present.

The `.gitignore` files at each scope are themselves committed to git. The
result is that running `atlas` on a fresh checkout populates `cache/`
locally; the user never thinks about it.

---

## 6. Cache discipline

### 6.1 Pure-derived files

Files whose content is a deterministic function of source + analyser
version + schema version, with no cross-run state:

- `<component>/.atlas/cache/surfaces.yaml`
- `<component>/.atlas/cache/component.yaml`
- `.atlas/cache/components.yaml`
- `.atlas/cache/related-components.yaml`
- `.atlas/cache/reports/drift.yaml` (pure given snapshot)
- `.atlas/cache/reports/composition-divergence.yaml`
- `.atlas/cache/reports/modularity-rollup.yaml`

Cache discipline (carry forward from Phase 1 §8.1):

- Compute fingerprint = SHA256 over inputs.
- Cache hit (fingerprint matches stored): serve from cache; no recompute.
- Cache miss: recompute; write blob; update fingerprint record.

The Phase 3 reports' fingerprints include: relevant L4–L8 engine output
shas, analyser version, report-machinery version. They do **not** include
the prior snapshot (drift) or prior history (modularity) — those are
stateful and managed inside `atlas-reports`, not by the fingerprint cache.

### 6.2 Stateful-derived files

Files whose content depends on cross-run state in addition to source:

- `.atlas/cache/contract-shas-snapshot.yaml` — last-drift-run snapshot.
- `<component>/.atlas/cache/modularity.yaml` history field — last 5 runs.

These are owned by `atlas-reports`; the engine's fingerprint cache layer
does not manage them. Discipline:

- On run with current input fingerprint matching most-recent state's
  fingerprint: no append (no duplicate history entry); file rewritten only
  if `generated_at` changes.
- On run with current input fingerprint differing: advance state (new
  snapshot or new history entry, oldest dropped if >5).

### 6.3 Atomic write requirements

All cache writes use temp-file + rename to prevent corruption from
interrupted writes:

```
write to <path>.tmp.<pid>
fsync <path>.tmp.<pid>
rename <path>.tmp.<pid> -> <path>
```

Critical for the stateful files (drift snapshot, modularity history) where
a half-written state would corrupt the baseline. Documented explicitly in
the spec so reviewers catch any non-atomic implementation.

---

## 7. Testing strategy

### 7.1 Per-PR unit tests

Each report's pure function has unit tests in
`crates/atlas-reports/src/<analysis>.rs` and `tests/<analysis>.rs`:

- Construct a minimal in-memory `EngineDb` (or a test-fixture
  `ReportInputs`) with hand-written components, contracts, edges.
- Call `atlas_reports::<analysis>(inputs, ...)`.
- Assert on the returned struct (not on serialised YAML).

Per-analysis test inventory:

- **Drift**: first-run-no-baseline, baseline-unchanged, baseline-changed,
  contract-added, contract-removed, contract-pinned-binding-detected,
  pinned-binding-up-to-date.
- **Impact**: direct-only, transitive, cycle-safe, partition correctness
  on each axis, target-not-found, empty-result, contract-input,
  component-input.
- **Modularity**: per-formula tests (Ca/Ce/I/cohesion/surface-stability/
  surface-complexity), each with a fixture that pins the expected number
  to a hand-computed value. Subsystem-aggregate tests with and without
  outliers. Empty-subsystems test.
- **Divergence**: pair-classification (build-only, deploy-only, both,
  neither). Severity computation with and without drift baseline. Empty
  result.

### 7.2 Stateful-file tests

Two-run tests where the first run captures, the second reads + updates:

- Drift snapshot: run 1 captures baseline, run 2 with one contract changed
  reports drift correctly and rewrites snapshot.
- Modularity history: run 1 writes history with one entry; runs 2–6 each
  add an entry; run 7 verifies oldest is dropped (FIFO at 5).
- Atomic write: simulate kill-during-write fixture (write to temp, kill
  before rename). Verify the snapshot is either fully-old or fully-new,
  never half-written. Run the suite under both happy-path and
  injected-failure modes.

### 7.3 Phase 1 retrofit regression tests

Each cache-path migration PR includes:

- **Reader sweep audit**: a script (committed to the repo) that greps for
  hardcoded references to the old paths. PR fails if any are present.
- **End-to-end sweep**: run engine on a Phase 2 fixture and assert the
  output matches Phase 2's recorded output bit-for-bit, modulo the cache
  path differences.

### 7.4 Overrides schema extension tests

For each new schema field (`edges_add`, `edges_suppress`, per-component
field overrides):

- Fixture exercising the field has the expected effect on engine output.
- Fixture exercising malformed entries (missing `reason`, unknown edge
  kind, etc.) produces the expected error.
- `edges_suppress` matching nothing logs a warning, leaves output
  unchanged.

### 7.5 Integration smoke-test fixture (final PR of Phase 3)

Modelled on Phase 2's PR-14:

- **Fixture**: extend Phase 2's `polyglot-dull-shape` workspace with
  Phase 3 triggers:
  - One contract whose `content_sha` is rewritten between two indexes
    (drives drift).
  - Two components with deploy-coupling but no build-coupling (drives one
    divergent pair).
  - Two components with build-coupling but no deploy-coupling (drives a
    second divergent pair).
  - One component with deliberate outlier modularity (10× the other
    components' efferent coupling — drives the >2σ outlier flag).
  - One subsystem with three members for rollup math.
  - One user-asserted edge in `overrides.yaml::edges_add` and one
    user-suppressed edge in `edges_suppress`.

- **Run order**:
  1. `atlas index` (cold).
  2. `atlas drift` (first run, captures baseline).
  3. Mutate one contract.
  4. `atlas index` (warm + delta).
  5. `atlas drift` (second run, reports delta).
  6. `atlas modularity`, `atlas divergence`, `atlas impact <known-id>`.
  7. Assert on the structured output of each.

- **LLM call budget assertions**: cold = same as Phase 2's ~26 (Phase 3
  introduces zero LLM call sites); warm rerun = 0; post-edit = same as
  Phase 2 baseline (no Phase 3 amplification).

- **Hermetic**: runs end-to-end in a temp workspace using
  `OverridesFile.additions` and a synthetic engine state, no network.

---

## 8. Risks (Phase 3 specific)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Phase 1 retrofit (4 file moves + overrides schema extension) leaves dangling readers I don't find. | High | High | Sweep tests + grep audit during each retrofit PR. Greenfield rule means no migration path needed; only "does anything still read the old path". |
| Modularity cohesion formula produces useless numbers in real workspaces (LCOM4 variants notoriously calibration-sensitive). | Medium | Medium | Spec ships v1 formula with explicit "subject to revisit after dull/Linkuistics calibration" note; revision is formula+docs, schema unchanged. |
| Stateful-file (drift snapshot, modularity history) corruption from non-atomic writes. | Medium | Medium | All stateful writes use temp-file + rename. Test with kill-during-write fixture. Documented explicitly in §6.3. |
| Impact-query cycle on contract graph with mutual consumption. | Low | Medium | Cycle-safe traversal with seen-set. Specific test fixture. |
| Severity rating in divergence report requires a drift baseline; first-run divergence has `severity: null` everywhere. | Medium | Low | Documented in §4.4; CLI prints clear "drift baseline absent — run drift first for severity ratings". |
| Overrides schema extension fights with existing override semantics from Phase 1. | Medium | Medium | Each schema-extension PR includes a backwards-compatibility test against Phase 2's `OverridesFile.additions` use sites (those need to keep working). |
| Modularity per-component file growth across long-running workspaces. | Low | Low | Hard cap at 5 history entries with FIFO rotation. Cap is in v1 spec; no growth concern in practice. |
| Phase 3 PR count blowup from the Phase 1 retrofit; estimate is ~16 PRs but could grow. | Medium | Low | Monitor early Wave PRs; if retrofit work surprises with >2× the LOC estimate, surface and split (Phase 1 PR-12 deviation precedent applies). |
| Subsystem aggregates when `subsystems.yaml` is empty / absent. | Medium | Low | Modularity rollup gracefully reports `subsystems: []` and `unattached_components: { count, ids }`. No error. |
| Gitignore mechanism interferes with user-customised gitignores. | Low | Low | Idempotent write iff file absent. If present with different content, respect and warn that `cache/` is not in their gitignore. User can fix manually. |

---

## 9. Out of scope for Phase 3

These items are deferred to later phases. A reviewer flagging them as
missing should redirect to the relevant phase.

### 9.1 Deferred to Phase 4 (convergence + cleanups + LLM analyses)

- Pattern detection (originally design §10.3) — needs LLM machinery that
  Phase 4 introduces. (now Phase 9)
- Subprocess convergence: migrate Cargo / Dockerfile / RustSurface /
  LlmClassify / TS-as-subprocess from in-process to subprocess. (now Phase 8)
- Bidirectional LLM callback channel for subprocess analysers. (now Phase 8)
- `rust-analyzer` integration replacing `syn` (stretch). (now Phase 8 stretch)
- LLM confidence threshold calibration (§11.2.6). (now Phase 9)
- Contract rename-match (§11.2.4). (now Phase 6)
- `--strict-overrides` flag. (now Phase 6)
- Cache compression (§11.2.7). (now Phase 6)
- Worktree commit-sha consistency annotations (§11.2.8). (now Phase 6)
- Phase 2 closeout cleanups: `LenientBackend` extraction, decoder
  consolidation, `is_manifest_file` extension for Makefile/shell, L8
  phantom-subcomponent fix. (now Phase 4)
- Per-language Phase 3 refinements: full tree-sitter-dart, raco-driven
  Racket dep resolution, Phoenix sub-kinds for Elixir, Mix umbrella
  decomposition, LispKit `(import …)` symbolic resolution. (now Phase 7)

### 9.2 Deferred to Phase 5 (server mode)

- File watcher and Salsa input updates.
- gRPC + HTTP+GraphQL query API.
- Subscription primitives (contract sha, surface sha).
- Server lifecycle (start, restart, GC).
- CLI as thin client to co-located server.
- Optional Grafeo derived index for ad-hoc Cypher/GQL/SPARQL queries.
- Reactive recomputation of reports.
- Phase 5 query API authentication and authorisation (§11.2.5).

### 9.3 Deferred indefinitely

- `--gate` / `--strict` exit-code flags for CI integration on reports
  (low priority; users can script on top of `--json`).
- Pass/fail thresholds for modularity scores (downstream tooling concern).
- Upstream / subsystem-input variants of impact query.
- Modularity history depth >5 entries.
- Per-language coupling normalisation.
- Multi-tenant / SaaS hosting (design §11.3 explicit non-goal).

---

## 10. Design-doc touch-ups required

These edits land in
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` as a single
PR alongside (or as the first PR of) Phase 3:

1. **§10 renumbering.** Insert a new §10.4 "Convergence and cleanups"
   (Phase 4); renumber the old §10.4 "Server mode" → §10.5; renumber §10.5
   "Migration from v1" → §10.6 (and mark it OBSOLETE — superseded by the
   greenfield non-negotiable adopted in Phase 1).

2. **§10.3 Phase 3 scope clarification.** Replace the current bullet list
   with the four-analysis canonical scope (drift, impact, modularity,
   composition divergence). Move pattern detection out of §10.3; add it to
   the new §10.4. Add an explicit note: *"§10.3 introduces no new LLM call
   sites; all analyses are pure aggregations over L4–L8 outputs."*

3. **§10.4 (new — "Convergence and cleanups"):** Phase 4 scope as
   enumerated in §9.1 above.

4. **§4.5 ("Cache architecture") clarification.** Append: *"Cache files are
   local-only and gitignored by convention. `atlas-cli` writes a one-line
   `.gitignore` at each `.atlas/` scope on first cache-write, containing
   `cache/`. Cache portability across hosts is via explicit `atlas cache
   export/import` commands (deferred); cache is not shared via git."*

5. **§4.6 ("Data co-locates with source") clarification.** Append:
   *"Co-located means same directory tree as source, not git-tracked
   alongside source. Editorial files are git-tracked; derived files (cache,
   reports) are gitignored."*

6. **§6 file-layout sections** get a "Git status" column. Editorial tier
   files marked `tracked`; derived tier marked `gitignored (under cache/)`.
   Update §6.1 (`components.yaml`), §6.2 (`component.yaml`), §6.3
   (`surfaces.yaml`), §6.4 (`related-components.yaml`) to reflect the new
   cache locations.

7. **§11.2 open questions.** Add a new closed-question entry: *"§11.2.9
   Editorial-vs-derived classification of on-disk files. **RESOLVED in
   Phase 3:** editorial = user-asserted only (overrides / external-components
   / subsystems / analyzers / config + per-component overrides); everything
   else is derived and gitignored under `cache/`. Phase 1 files surfaces.yaml,
   component.yaml, components.yaml, related-components.yaml are retrofit to
   derived tier in Phase 3."*

8. **§11.2.5 ("Phase 4 query API auth") renumber.** Update text to "defer
   to Phase 5 design" (since server mode is now Phase 5).

9. **§12 risks table.** Add a new row: *"Phase 3 retrofit (4 file-path
   moves + overrides schema extensions) leaves dangling readers. | Medium |
   High | Sweep tests + grep audit during each retrofit PR; greenfield
   rule means no migration path."*

The design-doc touch-up PR can be the first PR of Phase 3 (PR-0 alongside
plan + status file), or bundled with the first retrofit PR. The Phase 3
plan will sequence this.

---

## 11. References

- Project design spec:
  `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
  (especially §10.3, §11.2, §12).
- Phase 1 plan:
  `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
- Phase 2 plan:
  `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`.
- Phase 1 status (per-PR notes):
  `docs/superpowers/plans/2026-05-06-phase1-status.md`.
- Phase 2 status (per-PR notes):
  `docs/superpowers/plans/2026-05-07-phase2-status.md`.
- Open-question resolutions:
  - `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md`
  - `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`
- Continuation prompt (currently Phase-2-shaped; Phase-3-shaped successor
  to be written): `docs/superpowers/prompts/2026-05-07-vnext-continue.md`.
- Memory entries that constrain Phase 3:
  - `feedback_phase1_open_questions` — Phase 1 §11.2.2 / §11.2.3 closed.
  - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
  - `feedback_fix_all_lints` — every PR runs cargo clippy/fmt clean.
  - `project_monorepo_consolidation` — long-term direction; informs that
    Phase 3 should not over-invest in multi-root-specific report flavours.
