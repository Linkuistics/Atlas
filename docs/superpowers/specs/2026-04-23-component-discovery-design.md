# Atlas Component Discovery — Design

**Status:** Design spec (not yet implemented).
**Date:** 2026-04-23.
**Scope:** Atlas v1 — the component-discovery and edge-discovery pipeline
produced by migrating and extending Ravel-Lite's ontology + discover
stages into Atlas.

This document is the canonical v1 design. An implementation plan will
follow. Divergence between this document and the implementation is a
bug — in one or the other — to be resolved by conscious decision, not
drift.

## 0. Summary

Atlas v1 is a Rust CLI, `atlas index <root>`, that analyses a
heterogeneous codebase and produces a hierarchical component index plus
a relationship graph over those components. The analysis is a
demand-driven fixedpoint computation implemented on top of Salsa,
combining filesystem/manifest evidence, LLM classification and surface
extraction, and graph-structural signals (SCCs, cliques, seam density).

The tool's output is a set of four YAML files intended as versioned,
human-editable source artefacts, maintained as the analysed codebase
evolves:

- `components.yaml` — tool-generated internal component tree
- `components.overrides.yaml` — human-authored pins, suppressions,
  hand-added components
- `external-components.yaml` — external leaves (crates, RFCs, services)
- `related-components.yaml` — edge graph, schema inherited unchanged
  from `component-ontology.md`

The v1 release also migrates the existing `component-ontology` Rust
library, its YAML schema, and the Stage 1/Stage 2 discovery prompts
from Ravel-Lite into Atlas, making Atlas the canonical owner of the
ontology and Ravel-Lite a downstream consumer.

## 1. Motivation and background

Atlas is Linkuistics' design-recovery tool. Its vision (see README)
covers multi-level architectural extraction from large codebases. This
design addresses the *first* load-bearing subsystem: identifying *what
the components are*, at *multiple granularities simultaneously*, for
codebases that mix CLIs, apps, websites, libraries, services, and
specs in several languages.

Ravel-Lite already has a mature ontology of component-to-component
relationships (17 edge kinds × 7 lifecycle scopes × direction,
documented in `component-ontology.md`) and a Stage 1 / Stage 2
discovery pipeline that operates on a hand-maintained list of
projects (`projects.yaml`). The gap: the *projects list itself* is
authored by hand. Atlas closes that gap, at richer granularity, and
takes ownership of the ontology so discovery and relationship-modelling
sit in the same tool.

Two initial industrial use cases drive v1:

1. A multi-repo development workspace (`~/Development/`, ~15 git repos
   of mixed relatedness, ~50–100 plausible components).
2. A large monorepo merged from many formerly-independent repos
   (~500 plausible components, no single top-level manifest).

Atlas must handle both with the same invocation shape.

## 2. Non-negotiable requirements

The following drove architectural choices and must survive
implementation without erosion:

1. **Industrial quality.** Outputs are trustworthy enough to ship to
   customers.
2. **Incremental re-runs.** LLM spend is the dominant cost; a re-run
   after a small source change must touch only what the change affects.
3. **Ongoing maintenance.** The four YAMLs are living artefacts that
   evolve with the codebase. Human edits must survive re-runs.
4. **Fail loudly on misconfiguration.** Budget exhaustion, cycle
   detection, schema mismatch, and backend errors are hard stops, not
   silent fallbacks.
5. **Ontology alignment.** The relationship graph uses the existing
   `component-ontology.md` schema unchanged. No schema fork.

## 3. Architecture overview

### 3.1 Top-level shape

Atlas is a Cargo workspace with five crates:

| Crate | Role |
|---|---|
| `component-ontology` | Migrated from Ravel-Lite; owns `EdgeKind`, `LifecycleScope`, `EvidenceGrade`, `Edge`, `RelatedComponentsFile`. Zero dependency on the rest of Atlas. Consumable by any tool. |
| `atlas-index` | Schema + reader for the four Atlas YAMLs. Handles rename-matching logic and merge-with-overrides. Thin; no LLM, no Salsa. |
| `atlas-engine` | Salsa-backed query graph. The fixedpoint computation. Depends on `component-ontology` and `atlas-index`. |
| `atlas-llm` | LLM adapter. v1 ships the `ClaudeCode` backend only. Narrow interface designed for later additions (DirectApi, Codex). |
| `atlas-cli` | `atlas` binary. Command parsing, configuration, atomic file writes, driver for `atlas-engine`. |

Ravel-Lite depends on `component-ontology` and `atlas-index` only.
Atlas has no dependency on Ravel-Lite, ever.

### 3.2 Invocation contract

```
atlas index <root-dir>
    [--output-dir <dir>]
    [--budget <N-tokens>]
    [--max-depth <N>]
    [--recarve]           # force reconsideration of boundaries
    [--dry-run]           # compute without writing
```

`<root-dir>` may be:

- A single git repository
- A directory containing many git repositories (the "workspace" case)
- A merged monorepo with no `.git` at its root

The tool treats all three identically. `.git` boundaries are evidence,
not structural.

### 3.3 Output files

All four output files live in `<output-dir>` (default `.atlas/` under
`<root-dir>` unless the root is read-only). All writes are atomic
(tmp + rename). All four have an integer `schema_version` and follow
the same versioning discipline as `related-components.yaml` — a hard
error on read for any other version.

See §5 for schema details.

### 3.4 Top-level data flow

```
┌──────────────────────────────────────────────────────┐
│ Inputs (hashed, L0)                                  │
│   file_content, file_tree_sha,                       │
│   prior_components_yaml, prior_externals_yaml,       │
│   prior_related_components_yaml,                     │
│   components_overrides_yaml,                         │
│   prompt_versions, model_fingerprint                 │
└──────────────────────────────────────────────────────┘
                        │
                        ▼
┌──────────────────────────────────────────────────────┐
│ Salsa query graph (§4)                               │
│   L1 Enumeration                                     │
│   L2 Candidate generation                            │
│   L3 Classification (LLM + deterministic mix)        │
│   L4 Tree assembly                                   │
│   L5 Surface extraction                              │
│   L6 Edge proposal                                   │
│   L7 Graph-structural analysis                       │
│   L8 Recursion decision (back-edge to L2)            │
│   L9 Projections                                     │
└──────────────────────────────────────────────────────┘
                        │
                        ▼
┌──────────────────────────────────────────────────────┐
│ Outputs (atomic writes)                              │
│   components.yaml                                    │
│   external-components.yaml                           │
│   related-components.yaml                            │
│   (components.overrides.yaml is an input only)       │
└──────────────────────────────────────────────────────┘
```

No hand-written pass loop. The CLI demands the two projection queries
(`components_yaml_snapshot()`, `related_components_yaml_snapshot()`)
and the rest falls out of Salsa's demand-driven evaluation.

## 4. The query graph

The engine is organised as nine logical layers. Each query is a pure
function of its inputs; all caching, invalidation, and memoisation is
Salsa's responsibility.

### 4.1 Layer definitions

**L0 — Inputs (external facts, hashed).**

- `file_content(path)` — file bytes, hashed.
- `file_tree_sha(dir)` — deterministic tree hash of a directory.
- `prior_components_yaml()`, `prior_externals_yaml()`,
  `prior_related_components_yaml()` — last run's outputs.
- `components_overrides_yaml()` — human pins/suppressions/additions.
- `prompt_version(prompt_id)` — SHA of the prompt template file after
  token substitution.
- `model_fingerprint()` — `(model_id, backend_version)`.

**L1 — Enumeration.**

- `manifests_in(dir)` — all package-manifest files in a subtree.
- `git_boundaries(dir)` — `.git` directories and submodule markers.
- `doc_headings(dir)` — top-level README headings, suitable for human
  intent signals.
- `shebangs(dir)` — first-line interpreter markers on executable files.

**L2 — Candidate generation.**

- `candidate_components_at(dir)` — one entry per plausible boundary.
  Inputs: L1 facts plus `subcarve_plan(parent)` (back-edge from L8).

**L3 — Classification.**

- `is_component(candidate) : Classification` — LLM or deterministic.
  Produces kind, language, build_system, lifecycle_roles, role,
  evidence_grade, evidence_fields, rationale.
- Short-circuits to `components_overrides_yaml()` when pinned.

**L4 — Tree assembly.**

- `component_parent(id)`, `component_children(id)`,
  `component_path_segments(id)` — the hierarchical index.
- Enforces the acyclicity invariant (no component is its own
  ancestor).
- Hosts the rename-match step (§5.5).

**L5 — Surface extraction.** (Ported from Ravel-Lite Stage 1.)

- `surface_of(id)` — purpose, produces_files, consumes_files,
  network_endpoints, external_tools_spawned, data_formats, etc.

**L6 — Edge proposal.** (Ported from Ravel-Lite Stage 2.)

- `candidate_edges(a, b)` — proposed edges with evidence, grade,
  rationale. Iterates all ordered pairs where surfaces exist.

**L7 — Graph-structural analysis.**

- `sccs()`, `cliques(min_k)`, `seam_density(id)`,
  `modularity_hint(id)` — structural signals on the current edge
  graph.

**L8 — Recursion decision.**

- `should_subcarve(id) : bool` — per-kind policy (CLI stops; library
  recurses), modulated by L7 signals.
- `subcarve_plan(id) : list<dir>` — the sub-directories to treat as
  new candidate roots. This output feeds L2's
  `candidate_components_at` for those sub-dirs, closing the
  query-graph cycle.

**L9 — Projections.**

- `components_yaml_snapshot()`,
  `external_components_yaml_snapshot()`,
  `related_components_yaml_snapshot()`.

### 4.2 The back-edge and fixedpoint

The back-edge from L8 to L2 is the mechanism for graph-structural
corrections. It is a *query*-graph cycle, not a *value* cycle: L4's
acyclicity invariant ensures data is a DAG. Salsa handles this
directly as long as each query terminates — which the combination of
per-kind stopping rules, depth budget, and structural quiescence
guarantees.

### 4.3 Stopping conditions

Recursion terminates when one of the following holds for every live
candidate:

1. `--max-depth` is reached.
2. Per-kind policy says stop (e.g., CLI at depth 1).
3. `should_subcarve(id)` returns false — structural quiescence.

The fixedpoint is the set of queries whose values have stabilised.
Salsa's cache is the evidence of that fixedpoint — no new queries are
demanded once quiescence is reached.

### 4.4 Human edits as inputs

`components_overrides_yaml()` is an L0 input. Every query that could
be overridden reads it as part of its input fingerprint:

- L3 classification checks for a pin on `kind`, `role`, `language`,
  `build_system`; returns the pinned value verbatim when present.
- L4 checks for `suppress: true` on the component; if present, the
  component is excluded from the tree.
- L2 reads `additional_components` from the overrides file and
  includes them as candidates that bypass L3 classification.

The consequence: changing an override invalidates exactly the queries
that depended on that field. Overrides are first-class participants in
the fixedpoint.

## 5. Data model

### 5.1 `components.yaml` (tool-generated, internal components only)

```yaml
schema_version: 1
root: /Users/antony/Development                # absolute, recorded at first run
generated_at: 2026-04-23T10:00:00Z
cache_fingerprints:
  model: claude-opus-4-7[1m]
  prompts:
    classify: sha256:...
    stage1-surface: sha256:...
    stage2-edges: sha256:...
    subcarve: sha256:...

components:
  - id: ravel-lite
    parent: null
    kind: rust-library
    lifecycle_roles: [build, runtime, dev-workflow]
    language: rust
    build_system: cargo
    role: llm-orchestrator
    path_segments:
      - path: Ravel-Lite
        content_sha: "sha256:abc123..."
    manifests:
      - Ravel-Lite/Cargo.toml
    doc_anchors:
      - Ravel-Lite/README.md
    evidence_grade: strong
    evidence_fields:
      - manifest:Ravel-Lite/Cargo.toml
      - git-boundary:Ravel-Lite/.git
    rationale: |
      Cargo workspace root with .git boundary; README declares single
      project.
```

### 5.2 `components.overrides.yaml` (human-authored)

```yaml
schema_version: 1

pins:
  ravel-lite:
    kind:  {value: rust-library, reason: "LLM kept classifying as cli"}
    role:  {value: llm-orchestrator}
  ravel-lite/scratch:
    suppress: true

additions:
  - id: unknown-wizard
    kind: cli
    language: rust
    path_segments: [{path: legacy/wizard}]
    rationale: "no manifest; declared manually"
```

This file is the sole human-override surface. All human intent lives
here; `components.yaml` can be deleted and regenerated without losing
judgment.

### 5.3 `external-components.yaml` (tool + human, separated from internal)

```yaml
schema_version: 1

externals:
  - id: ext:serde
    kind: rust-crate
    language: rust
    purl: pkg:cargo/serde
    homepage: https://serde.rs
    discovered_from:
      - manifest:Ravel-Lite/Cargo.toml
    evidence_grade: strong

  - id: ext:rfc-2119
    kind: spec
    url: https://www.rfc-editor.org/rfc/rfc2119
    discovered_from:
      - prose:Ravel-Lite/docs/component-ontology.md
    evidence_grade: medium
```

Internal and external separation matters for downstream consumers: a
plan can target internal components, never external ones.

### 5.4 `related-components.yaml` (inherited)

Schema unchanged from `component-ontology.md` §7. Edges reference
identifiers from any of the three component files; the ontology's
opaque-string stance accommodates this. Atlas validates that every
participant exists in the union of the three component files.

### 5.5 Identifier stability and rename-matching

- **Allocation.** A confirmed component receives an identifier derived
  from its primary path, kebab-cased and with slash separators for
  hierarchy (e.g., `ravel-lite`, `ravel-lite/config`). Collisions get
  a short content-hash suffix.
- **Persistence.** Identifiers live in `components.yaml` and are read
  on every re-run as L0 input via `prior_components_yaml()`.
- **Rename-match on re-run.** L4 compares new candidates against prior
  entries by `path_segments` content_sha overlap. Threshold for a
  match: ≥ 70% of files by content hash. A match inherits the prior
  identifier; `path_segments` is updated.
- **Deletion.** A prior entry with no match is retained once with
  `deleted: true`; removed on the next clean run. Gives one cycle for
  a human reviewer to catch false deletions.

### 5.6 Four files, three version numbers

`related-components.yaml` retains its existing `schema_version: 2`.
`components.yaml`, `components.overrides.yaml`, and
`external-components.yaml` start at `schema_version: 1`. Each bumps
independently on incompatible change. Atlas refuses to read a version
it does not understand.

## 6. Pass structure and fixedpoint semantics

See §4 for the query graph. This section describes runtime behaviour.

### 6.1 Cold start

On a first run against a codebase with no prior `components.yaml`:

1. CLI demands `components_yaml_snapshot()`.
2. Salsa transitively demands all upstream queries; L0 inputs are
   computed by reading the filesystem.
3. L2 starts at the root; L3 classifies candidates; confirmed
   components become L4 tree nodes; L5/L6 produce surfaces and edges;
   L7 analyses; L8 decides whether to sub-carve; L2 re-enters for
   sub-directories.
4. Recursion terminates at stopping conditions (§4.3).
5. Projections are written atomically.

### 6.2 Steady-state re-run

On a re-run after a small source change:

1. File hashes for touched files change; `file_tree_sha(containing_dir)`
   changes.
2. L1–L2 queries for the touched subtree re-run; usually produce
   identical results → Salsa halts propagation.
3. L3 classification re-runs only if the candidate's inputs actually
   changed; typically cached.
4. L5 `surface_of(id)` re-runs for the affected component — one LLM
   call.
5. L6 `candidate_edges` re-runs for that component against each
   neighbour whose surface may have changed — a handful of LLM calls.
6. L7/L8/L9 re-project.

Total cost scales with the blast radius of the change, not the size of
the codebase.

### 6.3 Override change

On a run where only `components.overrides.yaml` has changed:

1. `components_overrides_yaml()` input hash changes.
2. Salsa invalidates every query that read the changed pin.
3. Those queries re-run; typically no LLM calls are needed because
   pins are short-circuits.
4. Downstream effects propagate as usual.

### 6.4 Model or prompt change

On a run where `model_fingerprint` or any `prompt_version` has
changed:

1. The affected input hash changes.
2. Salsa invalidates every LLM-backed query that depended on it.
3. All such queries re-run (potentially many LLM calls — this is the
   expensive case).
4. This is how intentional re-analysis is driven.

## 7. LLM interface

### 7.1 Backend abstraction

```rust
pub struct LlmRequest {
    pub prompt_template: PromptId,
    pub inputs: Json,               // structured, deterministically ordered
    pub schema: ResponseSchema,
}

pub struct LlmFingerprint {
    pub template_sha: Sha256,
    pub ontology_sha: Sha256,
    pub model_id: String,
    pub backend_version: String,
}

pub trait LlmBackend {
    fn call(&self, req: &LlmRequest) -> Result<Json>;
    fn fingerprint(&self) -> LlmFingerprint;
}
```

v1 ships one implementation: `ClaudeCodeBackend`, which spawns
`claude -p ... --output-format json` as a subprocess. DirectApi and
Codex backends are deferred.

### 7.2 Memoisation key

```
memo_key(call) = hash(
    call.prompt_template_id,
    call.template_sha,
    call.ontology_sha,
    call.model_id,
    call.backend_version,
    canonicalise(call.inputs)
)
```

The template SHA covers the post-substitution prompt, so changes to
`ontology.yaml` correctly invalidate any prompt that depends on it.

### 7.3 Prompt organisation

```
defaults/prompts/
  classify.md          # L3 classification (new, Atlas)
  subcarve.md          # L8 sub-carve decision (new, Atlas)
  stage1-surface.md    # L5 (migrated from Ravel-Lite with token edits)
  stage2-edges.md      # L6 (migrated from Ravel-Lite with token edits)
```

Substitution tokens: `{{ONTOLOGY_KINDS}}`, `{{COMPONENT_KINDS}}`,
`{{LIFECYCLE_SCOPES}}`, `{{COMPONENT_PATH_SEGMENTS}}`,
`{{COMPONENT_MANIFESTS}}`. Substitution is pre-hash.

### 7.4 Budget and failure mode

- `--budget N` sets a hard token ceiling.
- The engine tracks cumulative token usage across the run.
- On budget exhaustion, the current query fails with a specific error;
  the run aborts; outputs are not written.
- A subsequent run with a larger budget picks up from cache.

No graceful degradation. No fallback. No partial writes.

## 8. Evaluation

### 8.1 Ground truth

- Two hand-authored golden indexes for v1: `~/Development/` and the
  merged monorepo.
- Goldens live at `evaluation/goldens/<name>/components.golden.yaml`
  plus `related-components.golden.yaml` plus `notes.md` explaining
  boundary decisions.

### 8.2 Structural invariants

Enforced as Rust tests, run against every evaluation target regardless
of golden:

- Every internal component has `path_segments` non-empty.
- No two components' `path_segments` overlap except along
  parent-child lines.
- Every discovered manifest is covered by exactly one component.
- No `.git` boundary falls inside a single component's path without
  explicit evidence in `rationale`.
- Every edge's participants exist in the union of the three component
  files.
- Rename-match is transitive: a round-trip move preserves identity.
- Fixedpoint terminates within 8 iterations on each golden target
  (initial hard cap; tunable down as empirical data accumulates).

### 8.3 Metrics against goldens

Per (target, run) pair:

- **Component coverage** — % of golden components matched (path
  overlap ≥ threshold).
- **Spurious rate** — % of tool-output components not in golden.
- **Kind classification accuracy** on matched components.
- **Edge precision/recall** — kind, lifecycle, direction axes.
- **Identifier stability** — % of identifiers preserved across a
  no-op re-run.

v1 targets: coverage ≥ 0.9, spurious ≤ 0.1, kind ≥ 0.85, edge
precision ≥ 0.8, identifier stability ≥ 0.95 on no-op re-run.

### 8.4 Differential evaluation

A run is compared to the previous run's output. Large diffs with small
input changes are flagged. This catches non-determinism, bad
memoisation, and spurious re-carving.

### 8.5 Research-track experiments

The harness is the experiment platform. Each of the following is a
one-query swap:

- L3 classification: `HeuristicOnly`, `LlmOnly`, `Hybrid`.
- Prompt A/B: `stage2-edges-v1.md` vs `stage2-edges-v2.md`.
- L8 sub-carve threshold sweeps.
- L7 on/off (does graph-structural correction improve tree quality?).

### 8.6 Reporting

Each run writes `evaluation/results/<date>-<target>.yaml` with all
metrics. A small report generator produces HTML trend views. **No CI
gating in v1.**

## 9. Migration plan: Ravel-Lite → Atlas

### 9.1 What moves

| Ravel-Lite source | Atlas destination |
|---|---|
| `src/ontology/` (crate code) | `crates/component-ontology/` |
| `defaults/ontology.yaml` | `defaults/ontology.yaml` |
| `docs/component-ontology.md` | `docs/component-ontology.md` |
| `defaults/discover-stage1.md` | `defaults/prompts/stage1-surface.md` |
| `defaults/discover-stage2.md` | `defaults/prompts/stage2-edges.md` |
| Schema fields in `src/related_components.rs` | `crates/component-ontology/` |

### 9.2 What stays in Ravel-Lite

- Phase loop, subagent dispatch, agent prompts.
- Plan/backlog machinery.
- `src/projects.rs` initially; rewritten in M5 to read Atlas's
  `components.yaml`.
- Ravel-Lite's CLI surface.

### 9.3 Dependency direction

Post-migration:

- Atlas depends on nothing Ravel-Lite–specific.
- Ravel-Lite depends on `component-ontology` and `atlas-index` as
  library crates.
- Discovery prompts live only in Atlas.

### 9.4 Stages

Each stage is one or two PRs; after any stage, both projects build
and test green.

- **M0 — Atlas bootstrap.** Cargo workspace, empty crate skeletons,
  CI hooks. No Ravel-Lite change.
- **M1 — Crate copy.** `component-ontology` code copied into Atlas.
  Ravel-Lite continues using its own copy.
- **M2 — Ravel-Lite switches.** Ravel-Lite's `Cargo.toml` switches
  to Atlas's crate via path or git dependency. In-tree copy deleted.
- **M3 — Prompt migration.** Stage 1/2 prompts copied into Atlas with
  minor token edits (project → component). Ravel-Lite keeps its copies
  functional until M4.
- **M4 — Schema document migration.** `component-ontology.md` and
  `ontology.yaml` move canonically to Atlas. Ravel-Lite's in-tree
  copies are deleted; its ontology-consistency test switches to
  validate against Atlas's copy.
- **M5 — Ravel-Lite becomes a consumer.** `src/projects.rs` is
  refactored to load Atlas's `components.yaml` via the `atlas-index`
  reader crate. A single `related-components.yaml` exists in the
  ecosystem.

### 9.5 Dependency form for M2

Path or git dependency, per workspace-layout convenience. Registry
publication is deferred.

### 9.6 Compatibility tests

- After M2, existing `related-components.yaml` files parse bit-identical
  to pre-M2.
- After M5, Ravel-Lite's `ravel-lite state related-components` command
  operates unchanged against Atlas-produced YAMLs.

## 10. v1 scope and deferred work

### 10.1 In scope

- Everything in §3 through §9.
- Single CLI entry point with flags as in §3.2.
- ClaudeCode backend only.
- Fail-loud on budget exhaustion.
- Four YAMLs, three version numbers, atomic writes.
- Coarse-to-fine pass with demand-driven fixedpoint; back-edge from
  L7/L8 to L2 wired but confidence-gated.
- Content-hash-based identifier allocation with ≥ 70% rename-match.
- Ravel-Lite migration M0–M5.
- Two goldens; evaluation harness with invariants, metrics,
  differential diff; no CI gating.

### 10.2 Deferred

- Additional LLM backends (DirectApi, Codex).
- Advanced rename-matching (below the 70% threshold).
- Daemon / file-watch mode.
- Web UI / Datalog query surface.
- Pattern detection, class/type/function-level analysis.
- Multi-repo federation as a first-class mode.
- Streaming / agentic LLM calls.
- CI integration.
- Registry publication.
- Per-repo Stage 1/2 prompt copies in parallel after M5.

### 10.3 Success criteria

Atlas v1 is complete when all hold:

1. `atlas index /Users/antony/Development/` on a clean checkout
   produces a `components.yaml` agreeing with the golden at ≥ 0.9
   coverage and ≤ 0.1 spurious rate.
2. A second run with no file changes produces zero LLM calls and an
   identical `components.yaml`.
3. A run after a single-file change re-runs only queries transitively
   dependent on that file; LLM call count matches the affected
   component count.
4. A pin in `components.overrides.yaml` survives deletion and
   regeneration of `components.yaml`.
5. Ravel-Lite, post-migration, runs its existing
   `related-components` workflow successfully against Atlas-produced
   YAMLs with no schema-level changes.
6. The merged-monorepo golden scores within 5 percentage points of
   the `~/Development/` golden on all metrics.

## 11. Decisions log

A summary of the decisions made during brainstorming, as a reference
for readers who want to trace "why this, not something else":

- **D1.** Atlas owns the ontology; Ravel-Lite becomes a consumer.
  (Opposite of the status quo.)
- **D2.** Component identification uses a hybrid criterion:
  intent-first (package manifests, READMEs, `.git`), lifecycle-split
  (components with heterogeneous build/deploy/test footprint get
  split), graph-structural correction (SCCs, cliques, seam density)
  as validators.
- **D3.** Hierarchy is variable-depth per component kind (CLI stops
  at 1; library recurses), with a user-controllable budget knob.
- **D4.** Single-root input. Both multi-repo-directory and
  merged-monorepo use cases handled identically. Repos are signals,
  not structural gates.
- **D5.** Coarse-to-fine pass structure with demand-driven
  fixedpoint; back-edge from L7/L8 to L2 is in v1 scope.
- **D6.** Incremental model: per-subtree tree-SHA caching (design
  accommodates per-component content-hash approach for a later
  refinement).
- **D7.** Architecture: Salsa-backed query graph (not strict
  pipeline, not Datalog-as-engine). Datalog may later sit on top of
  the output YAMLs as a separate tool.
- **D8.** Four-file output contract; overrides out-of-band;
  internal/external separation.
- **D9.** ClaudeCode backend only for v1.
- **D10.** Fail-loud on budget exhaustion.
- **D11.** Two goldens for v1; no CI gating.
- **D12.** Ravel-Lite migration is v1 scope.

## 12. Open questions (deferred, not v1)

1. **Rename-match threshold tuning.** 70% is a first guess; empirical
   tuning will come from goldens.
2. **Per-kind sub-carve defaults.** The shape of "a CLI stops here; a
   library recurses there" is a policy list that will grow.
3. **Hyperedges** (see `component-ontology.md` §10). Still deferred.
4. **Datalog query surface** over the four YAMLs. A candidate follow-up
   project once v1 is stable; would reuse the user's existing Datalog
   expertise from Ravel.
5. **Multi-user / team workflows.** `components.overrides.yaml`
   conflict resolution across contributors. Out of scope for v1 but
   will be a real concern at scale.
