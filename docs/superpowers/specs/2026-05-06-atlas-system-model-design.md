# Atlas System Model — Design / PRD

**Status:** Design spec (forward-looking; supersedes v1's internal-only architecture).
**Date:** 2026-05-06.
**Scope:** The architectural target for Atlas as it evolves from a one-shot CLI
that emits four YAML files into a long-running system-model service consumed by
LLM tooling for cross-repo, polyglot, contract-aware refactoring and modularity
analysis. Builds on the v1 component-discovery design dated 2026-04-23.

This document is the canonical forward-looking design. An implementation plan
will follow per phase. Divergence between this document and the implementation
is a bug — in one or the other — to be resolved by conscious decision, not
drift. Where this document conflicts with the v1 design spec, this document
wins; v1 mechanisms that survive are explicitly named.


## 0. Summary

Atlas evolves from a one-shot codebase-discovery CLI into a **system model
service**: a long-running, queryable model of the structure and semantics of a
software system, maintained incrementally and consumed primarily by LLM-driven
refactoring and modularity tooling.

Three conceptual changes drive the redesign:

1. **Multi-repo as tree expansion, not federation.** A set of repos linked by
   path dependencies is analysed as one logical workspace. The git repo
   boundary plays the same role as a `components.overrides.yaml` entry in a
   monorepo: a boundary-discovery signal, not an architectural concept. Atlas
   produces the same system-analysis result whether the input is one repo with
   override-seeded boundaries or many repos discovered by following path deps.

2. **Contracts, not types, are the unit of cross-component coupling.** Where v1
   tracks components and edges, the new model adds a first-class **contract**
   node. A contract is the language-agnostic shape of an interface — a YAML
   schema, a wire protocol, a shared filesystem layout, a CLI argv shape, an
   environment-variable namespace. Components **define**, **implement**, or
   **consume** contracts. Language-specific bindings (Rust structs, TS
   interfaces, Haskell types, Prolog clauses) are projections of contracts into
   specific languages, not the source of truth for coupling. Polyglot
   refactoring impact analysis traverses contracts.

3. **Build-time and deploy-time composition are different graphs.** Source
   manifests (Cargo.toml, package.json, .cabal) describe what compiles
   together. Deployment artefacts (Dockerfiles, k8s manifests, compose, helm,
   release configs, orchestration scripts) describe what ships and runs
   together. The two often disagree, and that disagreement is itself
   meaningful signal. Atlas tracks both and surfaces their divergence.

Around those changes, the system gains a **pluggable analyzer protocol**
(deterministic-first, LLM-fallback, in-process or subprocess), a
**content-addressed persistent cache** keyed on input fingerprints across
process boundaries, **scattered per-component `.atlas/` directories** that
travel with the source, and an eventual **server mode** with reactive
recomputation and a query API.

Atlas remains plain-text-canonical. YAML is the source of truth; any
graph-database projection is derived. The redesign reshapes what's stored, not
the principle that humans and tools can review, edit, and version-control the
artefacts.


## 1. Vision and consumers

### 1.1 What Atlas is becoming

Atlas v1's consumer is a human reading `components.yaml`. Atlas's evolving
consumer is an **LLM agent** doing one of:

- **Cross-repo refactoring**: "I'm changing the `ComponentEntry` contract; show
  me every implementor and consumer across all reachable repos so the change
  can be propagated atomically."
- **Modularity improvement**: "Identify components whose surfaces are unstable
  or whose internal cohesion is poor, and propose extractions or merges."
- **Pattern detection**: "Find components that match this architectural
  pattern; find anti-patterns that should be remediated."
- **Polyglot contract custody**: "The TypeScript SDK is taking over ownership
  of this protocol from the Rust core; show me every consequence and every
  current implementor."
- **Composition-aware extraction**: "Can I extract this component as an
  independent service, or is it deploy-time-coupled to others through shared
  Docker image, env vars, or orchestration?"

These consumers need a **structured, navigable, semantically-precise** model.
The v1 four-YAML output is a foundation; the redesign adds the abstractions
and the runtime shape that LLM tooling can actually drive.

### 1.2 Use case classes

| Use case | Primary signal | Headline output |
|---|---|---|
| Onboarding | Components, subsystems, doc anchors | Navigable map |
| Cross-repo refactoring | Contracts, bindings, cross-tree edges | Impact set |
| Modularity work | Coupling/cohesion metrics, deploy graph | Extraction/merge proposals |
| Pattern detection | Recurring component shapes, recurring edge shapes | Pattern report |
| Drift surveillance | Surface content shas, contract content shas | Drift report |
| Composition reasoning | Build graph vs deploy graph | Composition report |

### 1.3 Non-consumers (for clarity)

- **Live cluster state.** Atlas reasons about *intended* deployment composition
  from checked-in artefacts. Observed cluster state (which pods are running
  right now, which images are deployed in production) is operations data and
  out of scope.
- **Code generation.** Atlas describes the system; it does not generate code.
  Code-generation tooling is a downstream consumer of Atlas's contracts.
- **Real-time IDE integration.** The eventual server mode supports
  near-real-time updates (seconds, not milliseconds); it is not designed to
  back IDE features that demand sub-100ms responses.


## 2. Non-negotiable requirements

The following drive architectural choices and must survive implementation
without erosion:

1. **Industrial quality.** Outputs are trustworthy enough to ship to customers.
   Inherited from v1.
2. **Plain text canonical.** YAML on disk is the source of truth. Every
   in-memory model and every projected database is derivable from the YAMLs;
   no hidden state lives only in a binary store. Hand-edits to YAML survive
   re-runs.
3. **Multi-repo == monorepo equivalence.** A multi-repo source layout linked
   via path dependencies and a single monorepo with equivalent
   `components.overrides.yaml` boundaries produce the same component graph.
4. **Determinism over fuzziness.** Deterministic analysers (manifest parsing,
   AST analysis, Dockerfile parsing) run before LLM analysers. LLMs are used
   only where deterministic methods are not feasible or are insufficiently
   confident.
5. **Pluggability.** New languages, new manifest types, new deployment formats
   are added by registering analysers, not by modifying the engine core.
6. **Polyglot first-class.** A component can contain multiple languages.
   Cross-component coupling is tracked via language-agnostic contracts. No
   language is a privileged citizen.
7. **Incremental re-runs.** LLM and analyser cost is the dominant runtime
   expense. Re-running after a small source change must touch only what the
   change affects. The cache is correctness-bearing, not optimisation.
8. **Live edits.** Human edits to override files (`components.overrides.yaml`,
   `subsystems.overrides.yaml`) take effect on next run (one-shot mode) or
   immediately (server mode).
9. **Fail loudly.** Budget exhaustion, schema mismatch, and analyser errors
   are hard stops, not silent fallbacks. Inherited from v1.
10. **Server-readiness in schemas.** Every schema decision is taken with the
    server target in mind: stable global ids, content-addressable identities,
    no implicit relationships between fields, edges with typed kinds and
    explicit participants.


## 3. Conceptual model

### 3.1 Components

A **component** is a discrete unit of code with an identifiable boundary,
classified by kind, lifecycle role, and (one or more) language. v1's component
concept survives intact, with two extensions:

- `language: String` becomes `languages: BTreeSet<String>`. A `billing-service`
  containing Rust + C bindings + Python automation is one component with three
  languages.
- The `kind` vocabulary expands beyond source-code kinds (`rust-library`,
  `npm-package`, `workspace`) to include **deliverable kinds**: `docker-image`,
  `published-crate`, `helm-release`, `k8s-deployment`, `homebrew-bottle`,
  `orchestration-script`, `ci-pipeline`. A deliverable is a component whose
  source is a deployment or publish artefact rather than (or in addition to)
  source code.

Component identity is a stable namespaced id (`atlas-contracts/atlas-index`,
`ravel-lite/knowledge-graph`). Ids derive from the enclosing manifest-root
(Cargo `[workspace]`, npm workspace, etc.) and are stable across re-indexings
modulo explicit rename-match (v1 mechanism survives).

### 3.2 Contracts

A **contract** is the language-agnostic shape of an interface that crosses
component boundaries. Contract kinds:

| Kind | Examples |
|---|---|
| `data-format` | YAML schema for `.atlas/components.yaml`; protobuf for an RPC payload; JSON Schema for a config file. |
| `wire-protocol` | An HTTP API surface; a gRPC service definition; a message-queue payload schema. |
| `filesystem-layout` | The structure of a `.atlas/` directory; the layout of a checkpointed state directory; well-known paths in `/var/run`. |
| `process-interface` | A CLI argv/env/exit-code shape; a shell hook contract; a systemd unit interface. |
| `environment-namespace` | A set of env vars consumed together (e.g. `DATABASE_URL`, `DATABASE_POOL_SIZE`). |
| `library-api` | An in-process API surface, language-bound (the only contract kind that does not cross language boundaries). |

A contract has a stable id (`atlas-contracts/components-yaml-schema`,
`ravel-lite/intent-state-dir-layout`), a content sha (a normalised hash of the
contract definition), and an owner: the component that **defines** it.

Cross-component coupling traverses contracts. An impact query — "what is
affected if contract X changes shape?" — yields the set of components that
**implement** X (provide a binding in their language) and the set that
**consume** X (call into a binding).

### 3.3 Bindings

A **binding** is a language-specific projection of a contract. The
`ComponentEntry` Rust struct in `atlas-contracts/atlas-index` is a binding to
the `components-yaml-schema` contract; a TypeScript `ComponentEntry` interface
in a Ravel-Lite TS consumer is a different binding to the same contract.

Bindings are not first-class graph nodes; they are **attributes of components**
recorded in `surfaces.yaml`. Each binding records:

- The contract id it binds to.
- The language of the binding.
- The location of the binding (file path, line range, or symbol name within
  the component's source).
- A content sha of the binding (so drift between binding and contract is
  detectable).
- The role: `defining-binding` (the binding the contract is derived from),
  `implementing-binding` (an implementation of the contract for this
  language), or `consuming-binding` (a usage site).

A component without an in-process API has zero `library-api` contracts.
Components without bindings to any data-format/wire-protocol/etc. contracts
are loosely coupled at the system level (which is itself useful signal for
modularity work).

### 3.4 Surfaces

A **surface** is a component's complete interface to the system: the set of
contracts it defines, implements, and consumes, plus the bindings to those
contracts in the component's language(s). Surfaces are the load-bearing input
to L6 edge proposal: the set of cross-component edges is a function of who
defines, implements, and consumes which contracts.

Surfaces are a **content-addressed projection** (per-component
`surfaces.yaml`), with a content sha that L6 edge caching depends on.
Cross-component cache invalidation is mediated by surface content shas:
when component A's surface sha changes, every L6 cache entry whose
participants include A misses and recomputes.

### 3.5 Deliverables and composition

A **deliverable** is a component whose source is a deployment or publish
artefact (per §3.1). Deliverables compose other components: a Docker image
*bundles* one or more source components; a published-crate *releases* the
source crate it points at; an orchestration-script *invokes* services.

**Composition edges** (new in `related-components.yaml`):

| Edge kind | Direction | Lifecycle | Meaning |
|---|---|---|---|
| `bundled-into` | source-component → deliverable | deploy | Source component's artefact is included in the deliverable. |
| `published-as` | source-component → deliverable | release | Source component is published to a registry as the deliverable. |
| `deployed-with` | component ↔ component | runtime | Two components co-deploy via shared deliverable, network, volume, or env. |
| `released-with` | component ↔ component | release | Two components are version-locked in a coordinated release. |
| `orchestrates` | orchestration-script → component | runtime | Script invokes the component as part of a managed lifecycle. |
| `bundled-from-external` | external-package → deliverable | deploy | A non-source-tree package (e.g. a base Docker image) contributes to the deliverable. |

Deploy-time coupling is often invisible from build-time manifests. A
Dockerfile that COPYs two binaries into one image creates `deployed-with`
between their source components even if no `depends-on` edge exists.

### 3.6 Edges

The full edge taxonomy is the union of:

- **v1 edges** (depends-on, links-statically, etc., from `component-ontology`).
- **Contract edges**: `defines-contract`, `implements-contract`,
  `consumes-contract`, with the participants being a component and a
  contract.
- **Composition edges** (per §3.5).

Each edge carries `evidence_grade` and `evidence_fields[]`, inherited from v1.

### 3.7 Subsystems

A **subsystem** is a hand-drawn or analyser-proposed grouping of components
with a shared purpose. v1's subsystem mechanism survives intact, with one
extension: subsystems can include contracts and deliverables, not only source
components.

### 3.8 Evidence

**Evidence-driven classification** is preserved from v1. Every classification,
edge, and contract carries its evidence grade and the specific evidence fields
that produced it. Hand-overrides claim `strong` evidence with a `notes:` field
explaining the rationale. The evidence model is the basis for trust in
LLM-driven outputs.


## 4. Architectural principles

The following invariants shape every implementation choice. Violations should
prompt a redesign of the relevant subsystem, not a workaround.

### 4.1 Plain text is canonical

YAML files on disk are the source of truth. Every in-memory model, every
projected database, every cache is derivable from the YAMLs. The reasons:

- **Reviewable.** Diffs are PR-readable; changes are traceable in git history.
- **Editable.** Overrides are hand-authored; humans must be able to write the
  YAML directly when the analyser is wrong.
- **Tool-agnostic.** Downstream consumers (Ravel-Lite, drift dashboards, agent
  frameworks) read YAML with serde/PyYAML/js-yaml without coupling to Atlas's
  Rust types.
- **Durable.** A binary database file with a proprietary schema is a hostage
  to its software; YAML survives ten years of tool churn.

### 4.2 Multi-repo equals monorepo

Atlas analyses **a tree of source code**, not a git repo. The tree may span
several git repos linked by path dependencies; it may be a single monorepo
with override-declared boundaries. The same analysis runs in both cases; the
outputs are structurally identical.

The git repo boundary is a boundary-discovery signal, equivalent in role to a
`components.overrides.yaml` entry that says "treat this directory as a
top-level component." The signal is consumed by L2 candidate generation and
discarded by the rest of the pipeline.

### 4.3 LLM is the spine; deterministic code is the scaffolding

Atlas's analytical work is performed by an LLM agent runtime over a tree of per-stage tasks. Deterministic Rust code is reserved for tasks that are *genuinely* deterministic — parsing structured manifests, walking filesystem trees, computing content shas, validating schemas, replaying cached transcripts, and supporting the agent runtime itself. Each deterministic component must justify *why it is deterministic*; "easier to code than to prompt" is not sufficient justification.

The Phase 6 → Phase 7 boundary is the inversion moment in the codebase. Phase 6 ships as the final deterministic-spine release; Phase 7 ships the LLM-spine runtime; subsequent phases retire language-specific deterministic analysers in waves. See `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` for the architectural detail behind this inversion.

### 4.4 Pluggable analysers

Adding a language or format means registering an analyser, not modifying the
engine. The plugin protocol (§7) is load-bearing: the surface area of formats
Atlas will eventually support — Cargo, npm, Cabal, .csproj, pyproject,
Dockerfile, k8s, compose, helm, release.toml, GitHub Actions, GitLab CI,
shell scripts, Justfile, Makefile, terraform — cannot be hard-coded.

### 4.5 Content-addressed cache

Every cacheable computation is keyed by a fingerprint of its complete inputs:
file content shas, ontology sha, prompt template sha, model id, backend
version, and (for cross-component computations) participant surface shas. The
cache is persistent across processes and across restarts; an Atlas server
warm-starts from an existing cache; cross-process re-use of analyser results
is a property of the cache, not a coordination protocol.

Cache files are local-only and gitignored by convention. `atlas-cli` writes a
one-line `.gitignore` at each `.atlas/` scope on first cache-write, containing
`cache/`. Cache portability across hosts is via explicit `atlas cache
export/import` commands (deferred); cache is not shared via git.

### 4.6 Data co-locates with source

A component's intrinsic facts (its classification, its surface, its
component-scoped LLM cache, its overrides) live in a per-component
`<component-path>/.atlas/` directory. Top-level synthesised projections
(unified `components.yaml`, cross-component edges, subsystems) live in the
top-level `.atlas/` from which `atlas index` was run.

The principle: data travels with the source it describes. A vendored copy of
`atlas-contracts/atlas-index` brings its `.atlas/` along; a host repo can read
that data immediately without re-running L5 surface analysis.

Co-located means same directory tree as source, not git-tracked alongside
source. Editorial files are git-tracked; derived files (cache, reports) are
gitignored.

### 4.7 Salsa as the engine

The v1 Salsa-backed query graph is preserved and extended. Salsa's
demand-driven incremental computation model is exactly what server-shaped
Atlas needs: change a file, mark inputs dirty, queries that depend on those
inputs become stale and recompute on next access. The persistent cache is a
back-store for Salsa's tracked queries; the in-memory Salsa graph is the
front.


## 5. System architecture

### 5.1 Layer model

The v1 L0–L9 layering survives, with extensions:

| Layer | Inputs | Outputs | Determinism | Cross-component? |
|---|---|---|---|---|
| L0 | Filesystem | Seeded files | Deterministic | No |
| L1 | Files | Manifests, doc headings, shebangs, git boundaries, deploy artefacts | Deterministic | No |
| L2 | L1 + override boundaries | Candidate components | Deterministic | No |
| L3 | L2 + manifest shapes + LLM (fallback) | Classified components | Mixed | No |
| L4 | L3 + overrides | Component tree, stable ids | Deterministic | No (assembles locals) |
| **L5** | L4 + source content + analyser dispatch | **Surfaces (contracts + bindings)** | Mixed (deterministic preferred, LLM fallback) | No (per-component) |
| **L6** | L5 (all participants' surfaces) + LLM (when needed) | Edges (incl. contract edges, composition edges) | Mixed | **Yes** |
| L7 | L4 + L6 | Structural metrics (SCCs, cliques, **modularity scores**) | Deterministic | Yes |
| L8 | L2 + L7 (recursive) | Sub-carved component candidates | Mixed (LLM for fuzzy boundaries) | No |
| L9 | L4 + L5 + L6 + L7 + L8 | YAML projections, cache fingerprints | Deterministic | Yes |

The load-bearing changes:

- **L5 is now a written projection.** v1's L5 surface analysis was internal
  Salsa state; the new L5 emits per-component `surfaces.yaml` files with
  contracts and bindings. This makes cross-tree caching coherent.
- **L6 cache keys include participant surface shas.** This is the only stage
  whose cache crosses component boundaries; making participant shas part of
  the key is what makes cross-tree invalidation correct.
- **L7 produces modularity scores as primary output**, not just internal
  signals consumed by L8.

### 5.2 Plugin protocol

The L1, L2, L3, L5, and L6 dispatchers consult an **analyser registry** keyed
by file pattern + language + cost class. The registry is loaded at start-up
from `.atlas/analyzers.yaml` plus built-in defaults. Each analyser declares:

- `id`: stable name (`cargo-toml-classifier`, `dockerfile-l1`, `rust-analyzer-surface`, `llm-classify-fallback`).
- `stage`: which L-stage it serves (`L3`, `L5`, `L6`, …).
- `applicability`: a deterministic predicate (file globs, language tags, manifest types).
- `fingerprint_inputs`: which input bytes contribute to the analyser's cache key.
- `cost_class`: `deterministic-cheap` | `deterministic-expensive` | `llm-cheap` | `llm-expensive`.
- `confidence`: `binary` | `graded` | `declines`.
- `transport`: `in-process` (Rust trait object) | `subprocess` (stdio JSON).
- `version`: contributes to the cache key.

Dispatch picks the cheapest applicable analyser whose confidence reaches a
threshold. LLM analysers run when no deterministic analyser produces a
confident answer.

Subprocess analysers communicate via a small stdio JSON protocol: requests
include `(stage, component_id, input_blob)`, responses include
`(output_blob, confidence, fingerprint_contribution, error?)`. Crash isolation
matters in server mode — a buggy Haskell-surface analyser must not take down
the server.

### 5.3 File layout

```
<primary-root>/
  .atlas/                                # top-level synthesis from this vantage
    components.yaml                      # full unified component list
    related-components.yaml              # cross-component edges
    subsystems.yaml
    external-components.yaml             # genuinely third-party only
    config.yaml                          # roots:, analyzer config, model routing
    analyzers.yaml                       # analyser registry overrides
    cache/                               # content-addressed persistent cache
      <stage>/<fingerprint>.blob
    llm-cache.json                       # backwards-compatible v1 LLM cache (deprecated, see §10)

  <component-path>/
    .atlas/                              # per-component intrinsic data
      component.yaml                     # this component's entry only
      surfaces.yaml                      # this component's contracts and bindings
      overrides.yaml                     # component-scoped overrides (optional)
      cache/                             # component-scoped cache shards
        <stage>/<fingerprint>.blob
```

Per-component `.atlas/` directories travel with the source. When a sibling
repo is pulled in via path-dep, its per-component `.atlas/` directories are
read as authoritative for those components (subject to fingerprint validation).

### 5.4 Cache architecture

The persistent cache is a content-addressed object store keyed by
`(stage, component_id_or_pair, input_fingerprint_sha)`, with the value being
the serialised stage output. Storage:

- **Filesystem-native**, one blob per cache entry. Reasons: git-friendly,
  human-inspectable, no external dependency, debuggable. Tradeoff: more
  inodes; a future SQLite-backed cache is admissible if filesystem inode
  pressure becomes real.

The cache key contributors per stage are listed in §5.1. Invalidation is
implicit: wrong fingerprint, miss. There is no explicit "invalidate" call;
correctness is a property of fingerprint completeness.

For L6 (the only cross-component stage), the cache key includes the surface
shas of all edge participants. When a participant's surface changes, every
L6 cache entry that named it as a participant misses on the next access.

### 5.5 Server mode (eventual)

Server mode (Phase 10) makes Atlas long-running:

- A **file watcher** (notify-rs) feeds change events. Affected Salsa inputs
  update; downstream queries become stale; on next access they recompute.
- LLM-bearing stages **debounce** per-component changes within a configurable
  window (default: 5s) so a developer's mid-edit saves don't trigger
  immediate re-classification.
- A **query API** (gRPC for typed clients, HTTP+GraphQL for ad-hoc; both
  speak the same backing model) serves graph queries, contract lookups,
  drift reports, and modularity metrics.
- **Subscriptions** support "notify on contract sha change", "notify on
  component surface change", and (lower priority) "notify on query result
  change."
- The persistent cache is the durability boundary across restarts; warm-start
  is a function of cache hit rate, not of a separate snapshot.
- **Process boundary defaults** flip: subprocess analysers are the default
  for non-core stages (crash isolation matters); in-process analysers remain
  for the deterministic core.

The CLI `atlas index .` is preserved as a thin client that talks to a
co-located server (auto-spawned if absent) and prints the YAML projections.
One-shot mode remains the supported degenerate case.


## 6. Data model

This section specifies the YAML schemas. Every schema is versioned; schema
changes go through a migration spec.

### 6.1 Top-level `components.yaml`

**Path:** `<primary-root>/.atlas/cache/components.yaml`. **Git status:**
`gitignored (under cache/)` — derived tier (Phase 3 retrofit; see §11.2.9).

```yaml
schema_version: 2
root: /Users/antony/Development/Ravel-Lite          # primary root
roots:                                              # all analysed roots
  - /Users/antony/Development/Ravel-Lite
  - /Users/antony/Development/atlas-contracts
generated_at: 2026-05-06T07:12:12Z
cache_fingerprints:
  ontology_sha: …
  prompt_shas:
    classify: …
    surface: …
    edges: …
  model_id: …
  backend_version: …
  analyzer_registry_sha: …                          # NEW
components:
  - id: ravel-lite
    kind: workspace
    languages: [rust]                               # was language: rust
    lifecycle_roles: [build]
    build_system: cargo
    path_segments:
      - path: ''
        content_sha: …
    manifests: [Cargo.toml]
    doc_anchors: …
    evidence_grade: strong
    evidence_fields: [Cargo.toml:[workspace]]
    rationale: …
    deleted: false

  - id: atlas-contracts/atlas-index
    parent: atlas-contracts
    kind: rust-library
    languages: [rust]
    lifecycle_roles: [build, runtime]
    build_system: cargo
    path_segments:
      - path: crates/atlas-index
        content_sha: …
    manifests: [crates/atlas-index/Cargo.toml]
    evidence_grade: strong
    rationale: …
    deleted: false

  - id: ravel-lite/billing-image
    kind: docker-image                              # NEW: deliverable kind
    languages: []                                   # deliverables have no source language
    lifecycle_roles: [deploy]
    path_segments:
      - path: deploy/billing
        content_sha: …
    manifests: [deploy/billing/Dockerfile]
    evidence_grade: strong
    evidence_fields:
      - Dockerfile:FROM
      - Dockerfile:COPY
    rationale: Dockerfile defines a deployable image bundling crate-a and crate-b.
```

### 6.2 Per-component `<component-path>/.atlas/component.yaml`

**Path:** `<component-path>/.atlas/cache/component.yaml`. **Git status:**
`gitignored (under cache/)` — derived tier (Phase 3 retrofit; see §11.2.9).

A single-component projection of the same data, plus pointers to surfaces
and overrides:

```yaml
schema_version: 2
component:
  id: atlas-contracts/atlas-index
  parent: atlas-contracts
  kind: rust-library
  languages: [rust]
  …
surfaces_path: surfaces.yaml
overrides_path: overrides.yaml                       # optional
analyser_id: cargo-toml-classifier                   # which analyser produced this
analyser_version: 1.0.3
fingerprint: …                                       # cache key for re-derivation
```

The top-level `components.yaml` is a synthesis of all per-component
`component.yaml` files; both files exist (the per-component file is the
intrinsic record, the top-level file is the system view from this vantage).

### 6.3 Per-component `<component-path>/.atlas/surfaces.yaml`

**Path:** `<component-path>/.atlas/cache/surfaces.yaml`. **Git status:**
`gitignored (under cache/)` — derived tier (Phase 3 retrofit; see §11.2.9).

```yaml
schema_version: 1
component_id: atlas-contracts/atlas-index
fingerprint: <surface-content-sha>                   # the value other components cache against

contracts_defined:
  - id: atlas-contracts/components-yaml-schema
    kind: data-format
    fingerprint: <contract-content-sha>
    definition_binding:
      language: rust
      symbol: ComponentEntry
      file: src/schema.rs
      span: [69, 95]
      content_sha: …
    description: |
      The on-disk YAML schema for .atlas/components.yaml. Defined in Rust
      via serde-attribute introspection; canonicalised to a JSON Schema
      representation for cross-language consumers.

contracts_implemented:
  - contract_id: atlas-contracts/components-yaml-schema
    role: defining-binding                            # this component is the contract owner
    binding:
      language: rust
      symbol: ComponentEntry
      file: src/schema.rs
      span: [69, 95]
      content_sha: …

contracts_consumed:
  - contract_id: <some-other-contract>
    binding:
      language: rust
      symbol: <usage-site>
      file: …
      span: […]
      content_sha: …

# Surfaces also include language-bound API contracts where in-process
# consumers exist. These do not cross language boundaries.
library_apis:
  - id: atlas-contracts/atlas-index/public-api
    kind: library-api
    language: rust
    fingerprint: <api-content-sha>
    pub_items:
      - name: ComponentsFile
        file: src/schema.rs
        kind: struct
      - name: load_components_yaml
        file: src/yaml_io.rs
        kind: fn
```

A consumer of the contract in another language records its binding in its own
`surfaces.yaml`:

```yaml
# ravel-lite/<consumer>/.atlas/surfaces.yaml
contracts_consumed:
  - contract_id: atlas-contracts/components-yaml-schema
    binding:
      language: typescript
      symbol: ComponentEntry
      file: src/atlas/components.ts
      span: [4, 28]
      content_sha: …
```

### 6.4 `related-components.yaml` extended edge vocabulary

**Path:** `<primary-root>/.atlas/cache/related-components.yaml`. **Git
status:** `gitignored (under cache/)` — derived tier (Phase 3 retrofit; see
§11.2.9).

```yaml
schema_version: 2
edges:
  # v1 edges (depends-on, links-statically, …) survive verbatim.

  - kind: depends-on
    lifecycle: build
    participants:
      - ravel-lite
      - atlas-contracts/atlas-index
    evidence_grade: strong
    evidence_fields: [ravel-lite:Cargo.toml:dependencies]

  # NEW: contract edges
  - kind: defines-contract
    participants:
      - atlas-contracts/atlas-index
      - atlas-contracts/components-yaml-schema
    evidence_grade: strong
    evidence_fields: [surfaces.yaml:contracts_defined]

  - kind: implements-contract
    participants:
      - atlas-contracts/atlas-index
      - atlas-contracts/components-yaml-schema
    evidence_grade: strong

  - kind: consumes-contract
    participants:
      - ravel-lite/atlas-reader
      - atlas-contracts/components-yaml-schema
    evidence_grade: strong

  # NEW: composition edges
  - kind: bundled-into
    lifecycle: deploy
    participants:
      - ravel-lite/billing-core
      - ravel-lite/billing-image
    evidence_grade: strong
    evidence_fields: [Dockerfile:COPY:target/release/billing-core]

  - kind: deployed-with
    lifecycle: runtime
    participants:
      - ravel-lite/billing-core
      - ravel-lite/billing-admin
    evidence_grade: strong
    evidence_fields: [Dockerfile:bundles-both, compose.yaml:shared-network]
```

Contracts appear as participants in `related-components.yaml` edges but are
**not** internal components; they are first-class graph nodes referenced by
fully-qualified id. A future schema-validator step ensures every contract
participant resolves to a contract definition in a per-component
`surfaces.yaml`.

### 6.5 `external-components.yaml`

Schema unchanged from v1, but **semantically narrower**: contains only
genuinely third-party packages (crates.io, npm, PyPI, NuGet). Cross-tree path
dependencies pointing at sibling Atlas-indexed roots no longer appear here;
they're resolved into internal components.

### 6.6 `analyzers.yaml`

```yaml
schema_version: 1
analyzers:
  - id: cargo-toml-classifier
    stage: L3
    applicability:
      file_globs: ['**/Cargo.toml']
    cost_class: deterministic-cheap
    transport: in-process
    version: 1.0.3

  - id: rust-analyzer-surface
    stage: L5
    applicability:
      languages: [rust]
    cost_class: deterministic-expensive
    transport: subprocess
    subprocess:
      command: [rust-analyzer-surface, --stage=L5]
      timeout_seconds: 60
    version: 0.4.1

  - id: llm-classify-fallback
    stage: L3
    applicability:
      always: true
    confidence: declines           # only runs if no other analyser is confident
    cost_class: llm-cheap
    transport: in-process
    version: 1.0.0

  - id: dockerfile-l1
    stage: L1
    applicability:
      file_globs: ['**/Dockerfile', '**/*.Dockerfile']
    cost_class: deterministic-cheap
    transport: in-process
    version: 0.2.0

  - id: shell-script-llm
    stage: L6
    applicability:
      file_globs: ['scripts/**/*.sh', 'deploy.sh', 'build.sh']
    cost_class: llm-cheap
    transport: in-process
    version: 0.1.0
```

The registry has built-in defaults; `analyzers.yaml` overrides or extends
them per-workspace.

### 6.7 `config.yaml`

```yaml
schema_version: 2

roots:
  - /Users/antony/Development/Ravel-Lite
  - /Users/antony/Development/atlas-contracts

# Per-stage model routing (v1 mechanism survives).
operations:
  classify:
    model: claude-code/claude-sonnet-4-6
  surface:
    model: claude-code/claude-sonnet-4-6
  edges:
    model: claude-code/claude-sonnet-4-6
  subcarve:
    model: claude-code/claude-sonnet-4-6

# Optional per-component override paths if the user wants them outside the
# scattered .atlas/ convention (e.g. a monorepo with all overrides in one
# place):
override_search:
  - .atlas/overrides.yaml
  - <component-path>/.atlas/overrides.yaml
```

### 6.8 Schema versioning

Every schema file carries `schema_version`. A version bump is required when:

- Required fields are added or removed.
- The semantic meaning of an existing field changes.
- The relationship between files changes (e.g., a field moves from
  `components.yaml` to `surfaces.yaml`).

Atlas reads the previous schema version, migrates in-memory, and writes the
new version. A migration spec accompanies each version bump.


## 7. Plugin protocol detail

### 7.1 Analyser interface (Rust trait, in-process)

> **RETIRED Phase 7.** The `Analyzer` trait is superseded by the `Tool` trait defined in `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §5.1. The text below is retained as historical context for v1 / Phase-1-through-Phase-6 deterministic-spine behaviour; new analytical work uses the agent runtime described in the recast spec.

```rust
pub trait Analyzer {
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
```

`Target` carries the component id, file paths, source content handles, and
metadata from earlier stages. `AnalysisContext` carries the cache, the LLM
backend handle, and progress reporting.

### 7.2 Subprocess analyser protocol

Subprocess analysers communicate over stdio with line-delimited JSON:

```
→ {"req":1,"stage":"L5","component_id":"…","target":{…},"fingerprint":"…"}
← {"req":1,"result":"confident","output":{…},"fingerprint_contribution":"…"}
```

A handshake message at start-up exchanges versions, capabilities, and timeout
settings. Process lifecycle: spawn at registry init; reuse across requests;
respawn on crash.

### 7.3 Cost classes and dispatch

> **RETIRED Phase 7.** Cost-class dispatch (`deterministic-cheap < deterministic-expensive < llm-cheap < llm-expensive`) is replaced by LLM-agent dispatch per recast spec §4.2. The text below is retained as historical context for the deterministic-spine era.

For a given target, the dispatcher orders applicable analysers by cost class
ascending (`deterministic-cheap` < `deterministic-expensive` < `llm-cheap` <
`llm-expensive`). The first analyser that returns `Confident` or `Graded` with
confidence above the threshold wins. `Declines` falls through to the next.
LLM-fallback analysers carry confidence threshold semantics: their
`Confident` is `Graded { confidence: post-threshold }` after evidence
review.

### 7.4 Confidence and evidence

Every analyser's output carries an `evidence_grade` (`weak`/`moderate`/`strong`)
and `evidence_fields` per the v1 ontology. Deterministic analysers emit
`strong` evidence by default; LLM analysers emit `moderate` unless they
self-report high confidence with citation-rich rationale (the v1 short-circuit
+ confidence pattern survives).


## 8. Caching and invalidation

### 8.1 The fingerprint discipline

Every cacheable computation declares its fingerprint inputs explicitly. The
cache key is `sha256(stage_id || analyzer_id || analyzer_version ||
sorted(fingerprint_inputs))`. There is no implicit input; every byte that
contributes to the output contributes to the key.

| Stage | Fingerprint inputs |
|---|---|
| L1 | File content sha (per file). |
| L2 | L1 fingerprints + boundary signal shas (override entries, repo boundaries). |
| L3 | L2 fingerprint + manifest content shas + analyser config + (LLM: prompt sha + ontology sha + model id + backend version). |
| L4 | L3 fingerprints of all components in scope + override fingerprint. |
| L5 | L3 fingerprint + source content shas + analyser version + (LLM: prompt sha + ontology sha + model id + backend version). |
| L6 | L5 fingerprints **of all participants** + analyser version + (LLM: prompt sha + ontology sha + model id + backend version). |
| L7 | L4 fingerprint + L6 fingerprint. |
| L8 | L2 fingerprint + L7 fingerprint + (LLM: same as L3). |
| L9 | L4 + L5 + L6 + L7 + L8 fingerprints. |

**Phase 7 extension.** When the LLM-spine agent runtime lands (see `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §6.1), the fingerprint discriminator for L3 / L5 / L6 / L8 stages extends with `iteration_number` (for fixed-point iteration) and `prior_model_sha` (so each iteration of the agent tree caches separately). The existing inputs in the table above remain canonical; the iteration extension is additive.

### 8.2 Cross-component invalidation

L6 is the only stage whose cache key crosses component boundaries. When a
participant component's surface sha changes, every L6 cache entry that lists
that participant misses on the next access. This is the mechanism by which
"atlas-contracts/atlas-index changed its `ComponentEntry` shape" propagates to
"every Ravel-Lite consumer's edge cache misses and recomputes."

The same mechanism handles cross-tree invalidation: a peer root's surfaces are
fingerprinted no differently than the primary root's, and L6 in the primary
root cites the peer root's surface shas in its cache keys.

### 8.3 Cache durability

The cache is filesystem-native (per §5.4). Reasons:

- Git-diffable: cache contents can (optionally) be committed to share warm
  starts across team members.
- Inspectable: `cat .atlas/cache/L5/<sha>.blob` works for debugging.
- No external dependency: filesystem is universally available.
- Atomic write: write-tempfile-then-rename gives crash-consistent updates.

A future SQLite-backed cache is admissible if inode pressure or filesystem-level
tooling becomes prohibitive at scale.

### 8.4 Cache GC

A cache entry whose key is no longer referenced by any current fingerprint
chain is eligible for GC. GC is opportunistic, not synchronous: a `--gc` flag
on the CLI walks the current state, marks reachable entries, sweeps the rest.
In server mode, GC runs periodically (default: daily) in a background task.


## 9. Server mode

Server mode is the Phase 10 target. The CLI continues to work as a degenerate
client.

### 9.1 Architecture

```
┌─────────────────┐
│ atlas index .   │ ── thin client; auto-spawns server if absent
│  (CLI)          │ ── prints YAML projections
└────────┬────────┘
         │ gRPC / HTTP+GraphQL
         ▼
┌─────────────────────────────────────────────┐
│ atlas server                                 │
│  ┌────────────────┐  ┌─────────────────┐    │
│  │ Salsa engine   │  │ Analyser pool   │    │
│  │ (in-memory)    │◄─┤ (in-proc + sub) │    │
│  └────────┬───────┘  └─────────────────┘    │
│           │                                  │
│           ▼                                  │
│  ┌────────────────┐  ┌─────────────────┐    │
│  │ Persistent     │  │ Query API        │    │
│  │ cache + YAMLs  │  │ (gRPC / GraphQL) │    │
│  └────────────────┘  └─────────────────┘    │
│           ▲                                  │
│           │                                  │
│  ┌────────┴───────┐                         │
│  │ File watcher   │                         │
│  └────────────────┘                         │
└─────────────────────────────────────────────┘
```

### 9.2 Reactive recomputation

- File watcher (notify-rs or platform-native) emits change events.
- Each event is mapped to one or more Salsa input updates (`File::set_bytes`,
  manifest reparse, etc.).
- Salsa's incremental engine marks dependent queries stale; they recompute
  on next access.
- LLM-bearing recomputations are debounced: edits within a per-component
  window (default: 5s) coalesce into a single recomputation.
- The persistent cache absorbs recomputation: if a component's source
  content sha lands back at a previously-seen value, the L5 surface cache hits.

### 9.3 Query API

Two surfaces, one backing model:

- **gRPC** for typed clients. Methods: `GetComponent`, `GetSurface`,
  `ListEdges`, `ListContracts`, `RunImpactQuery`, `RunModularityReport`,
  `Subscribe`. Schema is generated from the same Rust types used by the
  YAML projections.
- **HTTP+GraphQL** for ad-hoc clients. The schema mirrors gRPC; resolver
  implementations call the same Salsa queries.

Both speak structured types over the wire; **neither uses Cypher/GQL/SPARQL
directly**. A Grafeo-backed query API (Cypher/GQL) is a deferred role-B
addition (§11.4) when concrete consumers want it.

### 9.4 Subscriptions

Subscription targets, in priority order:

1. **Contract content sha changes**: `Subscribe(contract_id)` yields events
   when a contract's content sha transitions.
2. **Component surface sha changes**: `Subscribe(component_id, "surface")`.
3. **Edge set changes for a component**: `Subscribe(component_id, "edges")`.
4. **Query result changes** (low priority, v2): `Subscribe(query)` yields
   events when the query's result diverges from its last yielded value.

Subscriptions are server-push (gRPC streaming, GraphQL subscriptions). A
client may also poll.

### 9.5 Consistency model

Default: **return-stale-with-annotation**. A query during recomputation
returns the most recent settled state, with a `staleness` annotation listing
which input shas are pending recomputation. Clients can opt into
`wait-for-fresh` mode per-query. Blocking by default would punish interactive
LLM consumers.

### 9.6 Restart semantics

- On start-up, the server reloads `.atlas/` (top-level + per-component) into
  the Salsa graph.
- The persistent cache is consulted before any analyser runs.
- The file watcher is initialised after the initial reload completes.
- In-flight LLM requests are not durable across restart; they are simply
  retried (idempotency is a property of the cache).

### 9.7 Concurrency

- Multiple read queries proceed in parallel; Salsa handles in-memory
  concurrency.
- Writes (cache updates, projection regenerations) are serialised behind a
  single mutex per shard. The shard granularity is per-component for
  per-component caches and global for the top-level synthesis.
- The query API is concurrency-safe regardless; clients see consistent
  snapshots.

### 9.8 Process boundary

Subprocess analysers are the default for non-core stages. Crash isolation
matters when running long-lived alongside potentially-buggy third-party
analysers (Haskell, Prolog, future contributions). The deterministic core
(Cargo-toml parser, override loader, fingerprint hasher) stays in-process
for performance.


## 10. Phasing and migration

### 10.1 Phase 1 — Architectural seam

**Goal:** Establish the model. Multi-root [retired Phase 5], contract-first, scattered
`.atlas/`, plugin protocol with reference plugins. Still ships as a one-shot
CLI.

**Scope:**
- Path-dep walking with fixed-point root expansion.
- Scattered per-component `.atlas/` directories.
- `surfaces.yaml` projection with contracts as first-class.
- Contract node type in `related-components.yaml` participant namespace.
- Composition edge kinds (Dockerfile-driven, release.toml-driven).
- Plugin protocol skeleton with three reference analysers:
  - Cargo (deterministic, in-process; replaces hardcoded v1 Cargo path).
  - Dockerfile (deterministic, in-process).
  - LLM-classify-fallback (preserves v1 behaviour).
- Persistent content-addressed cache (filesystem-native).
- Migration step from v1 layout (one-time `atlas migrate-v1` command).

**Out of scope for Phase 1:**
- Non-Cargo language analysers (Cabal, .csproj, pyproject).
- Subprocess analysers.
- Server mode.
- Drift / impact / modularity reports.

**Deliverable:** The original "atlas-contracts component visible in
Ravel-Lite" outcome falls out of correct Phase 1 implementation, not via
special-casing.

### 10.2 Phase 2 — Pluggability and polyglot

**Goal:** Validate the plugin abstraction. Add non-Rust language coverage and
deploy-format coverage.

**Scope:**
- Subprocess analyser transport (stdio JSON).
- npm + TypeScript surface analyser.
- Python + pyproject analyser.
- Kubernetes manifest analyser (Deployment, Service, ConfigMap).
- Compose analyser.
- Helm chart analyser.
- LLM-fallback analyser for shell scripts (the "fuzzy orchestration" case).
- One non-mainstream language as a stretch goal: Haskell or C# (validates the
  protocol against an ecosystem that's not Rust/JS-shaped).

**Out of scope for Phase 2:**
- Server mode.
- Modularity reports.

### 10.3 Phase 3 — Drift, impact, modularity

**Goal:** Deliver the LLM-tooling-facing analyses that the redesign was
built to support.

**Scope (four canonical analyses):**
- **Drift report**: a contract whose content sha changed, with the list of
  bindings (across components, across roots) whose binding-content-sha is
  pinned to the previous contract sha.
- **Impact query**: given a contract or component, the transitive set of
  consumers, partitioned by language, deploy graph, and lifecycle.
- **Modularity report**: per-component coupling, cohesion, surface stability,
  surface complexity. Per-subsystem aggregates.
- **Composition divergence report**: components that are deploy-coupled but
  not build-coupled (or vice versa), with severity rated by surface drift.

§10.3 introduces no new LLM call sites; all analyses are pure aggregations
over L4–L8 outputs.

### 10.4 Phase 4 — Cleanup release

**Goal:** Pay down internal-quality debt accumulated across
Phases 1–3 and align canonical documentation with the validated
post-Phase-3 phase ordering. No new user-facing capability, no
schema change, no LLM call sites.

**Scope (~9 PRs):**
- LenientBackend extraction; decoder consolidation; L8 phantom-
  subcomponent fix (Phase 2 closeouts).
- `atomic_write` helper convergence; `build_engine_database` /
  `build_database_for_reports` convergence; sweep-test boilerplate
  consolidation; orphan `save_related_components_atomic` removal.
- This §10 retext + Phase 3 design §9.1 forward-pointer update.

See `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`
for the canonical Phase 4 scope.

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
`a302ce525bebd2df546472542f798f3c129426ba`.

### 10.6 Phase 6 — User-facing schema cleanups

**SHIPPED 2026-05-11.** Final deterministic-spine release before the LLM-spine recast begins in Phase 7. Five PRs landed (PR-0 plan + PR-2 contract rename-match owner-follows + PR-3 subsystem field overlay + PR-4 --strict-overrides + closed enum + dual-mode contract test + PR-5 acceptance + closeout + this retext). Original PR-1 (Makefile/shell manifest recognition) deferred to Phase 9c per `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §11.3 after pre-flight found the polyglot fixture already surfaces Makefile/`*.sh` via `additions:`. Closes the §11.2.4 contract-rename-match canonical-design open question (α id-embeds-owner implementation; β content-sha-stable deferred to Phase 10). Companion design + plan: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. Final commit: `cab2727`.

### 10.7 Phase 7 — LLM-spine runtime

LLM-spine runtime: agent runtime, toolbox, transcript cache, event bus, TUI, fixed-point iteration loop, audit lane. No language retirements; existing deterministic classifiers wrap as `Tool` implementations the agent invokes. Calibrates the cache primitive against known-good reference behaviour and ships the live TUI progress UX. See `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §11.1.

### 10.8 Phase 8 — Cargo retirement

First language LLM-driven: retires `cargo_classifier.rs` and Cargo-specific surface analysis in favour of LLM agents driving the toolbox. Calibrates the cold-token budget for the polyglot smoke test (locks in empirical per-language numbers; warm = 0 invariant unchanged). See recast spec §11.2.

### 10.9 Phase 9 — Remaining language retirements (waves)

Retires the remaining 9 hand-coded language classifiers in three waves: 9a (TS/JS + Python), 9b (C# + Dart), 9c (Elixir + Racket + LispKit + Compose + Dockerfile + the deferred Make/shell classifier from Phase 6's pre-pivot brainstorm). Each wave is its own phase with its own PRs, budget assertions, and reference-output comparisons. Mature-language surface analyser code (Rust + TS/JS) collapses to text-scoping `Tool` implementations; weak-tooling languages get no text-scoping helpers — agents read whole files. See recast spec §11.3.

### 10.10 Phase 10 — LLM-driven analyses

Pattern detection (recurring component / edge shapes; anti-patterns) as a new L8 agent stage; fuzzy contract matching (deferred from Phase 6 pre-pivot brainstorm) extends contract rename-match with semantic similarity beyond owner-follows / content-sha-stability; qualitative LLM-driven augmentation to existing Phase 3 modularity reports; LLM confidence threshold calibration. **Moved earlier** than today's §10.9 placement, since the agent runtime makes these analyses natural once it exists. See recast spec §11.4.

### 10.11 Phase 11 — Server mode + web-app subscriber

Long-running service with reactive recomputation, query API, file watcher, Salsa input updates, gRPC / HTTP+GraphQL, subscriptions, lifecycle, GC. Also ships the **web-app subscriber** to the agent runtime's event bus (the server already runs the bus across process boundaries; the web app subscribes via WebSocket / SSE). See recast spec §11.5.

### 10.12 Migration from v1

> **OBSOLETE.** Superseded by the greenfield non-negotiable adopted in
> Phase 1. There is no migration path from v1 layouts; a user upgrading
> deletes `.atlas/` and re-runs `atlas index`. The historical text below is
> retained for context only.

A `atlas migrate-v1` command runs once per workspace:

1. Reads the existing top-level `.atlas/components.yaml`,
   `external-components.yaml`, `related-components.yaml`,
   `subsystems.yaml`, `llm-cache.json`.
2. Splits per-component data into per-component `.atlas/` directories.
3. Synthesises empty `surfaces.yaml` per component (populated on next index).
4. Bumps `schema_version` to 2 in all top-level files.
5. Writes `analyzers.yaml` with the v1 default analyser set (Cargo, npm,
   LLM-classify-fallback) so the existing v1 behaviour is reproducible
   bit-for-bit.
6. Preserves the v1 LLM cache by translating its entries to the new
   fingerprint scheme.

The migration is one-way: the new layout supersedes the old. v1 binaries
read the v1 layout; new binaries read both for one release cycle, then drop
v1 reading support.


## 11. Decisions and open questions

### 11.1 Decisions

| Decision | Rationale |
|---|---|
| YAML stays canonical. | Reviewable, editable, durable, tool-agnostic. |
| Multi-root over federation. | Eliminates a special concept; produces same result as monorepo with overrides. |
| Contracts as first-class nodes. | Polyglot coupling traverses contracts, not language types. |
| Build graph and deploy graph both first-class. | The disagreement between them is meaningful signal. |
| Pluggable analysers, deterministic-first. | Surface area is too large to hard-code; LLM cost is too high to use indiscriminately. |
| Filesystem-native persistent cache. | Git-friendly, debuggable, no dependency. |
| Scattered per-component `.atlas/`. | Data travels with source; cache portability across hosts. |
| Salsa survives. | The incremental engine is exactly what server mode needs. |
| gRPC + HTTP+GraphQL for query API; no graph DB in v1 server. | Native types are sufficient; Grafeo is a deferred role-B option. |

### 11.2 Open questions

The following are flagged for resolution before the relevant phase ships.

1. **Surface schema details for non-Rust languages.** Phase 1 ships only the
   Rust binding shape. Phase 2 must define the binding schema for
   TypeScript/Python/Haskell/etc. before the corresponding analysers land.
   Risk: schema churn between Phase 1 and Phase 2 if the Rust shape is too
   Rust-specific.

2. **Contract content-sha canonicalisation.** A contract's content sha is the
   hash of its definition. For a YAML-schema contract, the hash must be
   stable across cosmetic differences (key ordering, comment changes). The
   canonicalisation algorithm is undefined; needs a spec before Phase 1
   writes contracts.

3. **Override scoping.** Per-component `overrides.yaml` and a top-level
   `components.overrides.yaml` co-exist. Resolution order, conflict handling,
   and merge semantics need a spec. v1's override merge logic is the starting
   point.

4. **Contract migration when a component moves.** When a contract's owner
   component is renamed or moved (e.g., a refactor that splits a crate), the
   contract's id should follow the new owner. The rename-match mechanism
   from v1 covers components; an analogous mechanism for contracts is
   needed.

5. **Phase 10 query API authentication and authorisation.** Server mode running
   in a multi-tenant environment (CI, shared dev server) needs an auth model.
   Defer to Phase 10 design (server mode is now §10.10 under the validated
   post-Phase-3 ordering; see §10.10).

6. **LLM analyser confidence thresholds.** The threshold for "confident" vs
   "declines" is per-analyser. Defaults need calibration against real
   workspaces. Phase 1 ships defaults; Phase 2 calibrates.

7. **Cache compression.** Filesystem-native cache blobs may be large
   (LLM-classify outputs with full evidence trails). Compression (zstd)
   per-blob is admissible; needs a decision on whether the cache key includes
   the compression algorithm.

8. **Worktree refactoring consistency annotations.** When `roots:` points at
   a set of worktrees at different commits, edges across them are valid only
   if mutually consistent. The schema includes a hook for per-root commit
   shas in `config.yaml`; the analysis logic that flags inconsistency is not
   yet specified.

9. **Editorial-vs-derived classification of on-disk files. RESOLVED in
   Phase 3:** editorial = user-asserted only (overrides / external-components
   / subsystems / analyzers / config + per-component overrides); everything
   else is derived and gitignored under `cache/`. Phase 1 files
   `surfaces.yaml`, `component.yaml`, `components.yaml`,
   `related-components.yaml` are retrofit to derived tier in Phase 3.

### 11.3 Explicit non-goals

- **Replacing language-specific analysis tools.** Atlas does not reimplement
  rust-analyzer, ts-morph, or jedi. It dispatches to them.
- **Running tests, builds, or deployments.** Atlas analyses; it does not
  execute. Build artefacts are inputs (e.g., a `target/` directory's content
  shas) only when the user opts in.
- **Embedding an LLM.** Atlas dispatches to LLMs via the existing provider
  layer. No model is bundled.
- **Generating code.** Atlas describes the system. Code generation is
  downstream tooling.
- **Real-time IDE integration.** Server mode supports near-real-time updates
  (seconds). IDE-grade latency (<100ms) is out of scope.
- **Multi-tenant SaaS.** Atlas is designed for single-tenant deployments
  (one user's machine, one team's CI). Multi-tenant hosting is a future
  business question, not an architectural requirement.

### 11.4 Deferred: Grafeo as a derived projection

Once Phase 10 ships and the server has concrete polyglot consumers issuing
ad-hoc graph queries (Cypher, GQL, SPARQL), publishing a Grafeo-backed
derived index alongside the YAMLs becomes an attractive role-B addition:

- LLM agents writing Cypher against Atlas's data avoid coupling to Atlas's
  Rust types.
- Polyglot tooling (Python data-science scripts, TS dashboards) gets a
  unified query API.
- Vector + BM25 + graph hybrid queries support pattern detection workloads
  Atlas's native API would not naturally express.

The schemas in §6 are designed to project cleanly into a Grafeo LPG: every
node has a stable id, every edge is typed with explicit participants,
contracts are nodes not embedded blobs. The projection is a mechanical
translation when the time comes.


## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Phase 1 schema churn invalidates Phase 2 analysers. | Medium | High | Ship Phase 1's schema as version 2-rc1; bump to 2 only after Phase 2's first non-Rust analyser validates the shape. |
| Plugin protocol abstraction leaks Cargo idioms into other ecosystems. | Medium | High | The protocol is designed against Cargo + Dockerfile + LLM-fallback in Phase 1, three concrete cases with deliberately different shapes. Phase 2's first non-Rust analyser is the abstraction-confirmation milestone. |
| Cache invalidation correctness bugs propagate stale data. | Medium | High | Fingerprint completeness is auditable: a debug command prints every fingerprint contributor for a given query. Property-based tests assert that any input change produces a different fingerprint. |
| Shell-script LLM analysis has too high a false-positive rate to be useful. | Medium | Medium | Surface confidence prominently; default to "advisory only" output with a flag to elevate. Real-world calibration in Phase 2. |
| Server mode's reactive recomputation amplifies LLM spend during active development. | Medium | High | Per-component debouncing (default 5s); explicit budgets in server mode; opt-in "no LLM during edit" mode. |
| YAML-as-canonical vs server-state-as-canonical confusion. | Low | High | Server always writes through to YAML on settled state; YAML is always re-readable as ground truth. |
| Cross-tree path-dep cycles cause analysis loops. | Low | Medium | Fixed-point iteration with cycle detection; cycles are warned, not errored (cycles are legal in some build systems). |
| Phase 3 retrofit (4 file-path moves + overrides schema extensions) leaves dangling readers. | Medium | High | Sweep tests + grep audit during each retrofit PR; greenfield rule means no migration path. |


## 13. Alternatives considered

### 13.1 Federation as a separate concept

Initial proposal: a `linked-repos.yaml` registry, cross-repo edges as a
distinct edge category, sibling repos treated as a special node type.
Rejected because it introduced a special concept that the data model didn't
need: a multi-repo setup is just a tree expanded across path deps, with
identical data shape to a monorepo.

### 13.2 Grafeo as the source of truth

Initial proposal: `.grafeo` binary file replaces YAML; YAML becomes optional
export. Rejected on the grounds that YAML's reviewability, hand-editability,
and tool-agnostic readability are load-bearing for Atlas's UX. A graph
database is an admissible derived projection; not an admissible source of
truth.

### 13.3 SQLite cache instead of filesystem

Considered for Phase 1. Rejected for git-friendliness, inspectability, and
zero-dependency reasons. Admissible later if filesystem inode pressure
becomes real at scale.

### 13.4 Type-first instead of contract-first surfaces

Initial proposal: L5 surfaces are language types (Rust pub items, TS export
shapes, etc.). Rejected because polyglot coupling cannot be expressed via
language types — a Rust struct and a TS interface bound to the same YAML
schema are not coupled "via a type" but via the schema. Contract-first
captures this; type-first does not.

### 13.5 Build-graph-only

Initial proposal: track only build-time composition (manifests). Rejected
because deploy-time composition (Dockerfiles, k8s, scripts) carries
modularity-relevant signal that build manifests cannot see — co-deployment,
shared env vars, runtime orchestration. Tracking both is necessary;
disagreement between the two graphs is itself useful.


## 14. References

- v1 design: `docs/superpowers/specs/2026-04-23-component-discovery-design.md`
- L8 map-reduce design: `docs/superpowers/specs/2026-05-04-l8-map-reduce-design.md`
- Multi-provider LLM config: `docs/superpowers/specs/2026-05-02-multi-provider-llm-config-design.md`
- Subsystem seeding: `docs/superpowers/specs/2026-05-01-subsystem-seeding-design.md`
- Engine progress events: `docs/superpowers/specs/2026-05-01-engine-progress-events-design.md`
- Component ontology: `atlas-contracts/crates/component-ontology/README.md`
- Atlas-index schemas: `atlas-contracts/crates/atlas-index/src/schema.rs`
- Salsa: <https://github.com/salsa-rs/salsa>
- notify-rs: <https://github.com/notify-rs/notify>
- Grafeo (deferred role-B option): <https://github.com/GrafeoDB/grafeo>


## 15. Glossary

- **Analyser**: a registered function (in-process or subprocess) that
  produces output for one L-stage given a target. Dispatched by the registry
  based on applicability and cost class.
- **Binding**: a language-specific projection of a contract (Rust struct, TS
  interface, etc.). Recorded in a component's `surfaces.yaml`. Not a
  first-class graph node.
- **Component**: a discrete unit of code or deliverable with an identifiable
  boundary, classified by kind, lifecycle role, and language(s). v1 concept
  preserved with language-set extension.
- **Composition edge**: an edge in `related-components.yaml` describing
  bundling, co-deployment, coordinated release, or orchestration. Distinct
  from build-time `depends-on` edges.
- **Content-addressed cache**: a persistent store keyed by a sha of all
  inputs to a computation. Hit means the computation has been seen with these
  exact inputs; miss means recompute.
- **Contract**: the language-agnostic shape of an interface that crosses
  component boundaries. First-class graph node. Distinct from a binding.
- **Deliverable**: a component whose source is a deployment or publish
  artefact (Dockerfile, helm chart, release.toml entry, etc.).
- **Federation**: a rejected concept — multi-repo as a special architectural
  layer. Replaced by multi-root workspace (retired Phase 5).
- **Fingerprint**: the sha of all inputs to a cacheable computation,
  including content shas, analyser version, and (for LLM analysers) prompt
  and model identifiers.
- **L0–L9**: the layered analysis pipeline, inherited from v1 and extended.
- **Per-component `.atlas/`**: a directory at a component's source path
  holding that component's intrinsic data (component entry, surfaces,
  overrides, scoped cache). Travels with the source.
- **Plugin protocol**: the interface analysers conform to, supporting
  in-process Rust trait objects and subprocess stdio JSON.
- **Role-B Grafeo**: Grafeo as a derived query index alongside YAML,
  deferred to Phase 10 (server mode) and beyond.
- **Surface**: a component's complete interface to the system: the contracts
  it defines, implements, consumes, plus the bindings.
- **Top-level `.atlas/`**: the directory at the primary root holding
  synthesised projections (unified components.yaml, edges, subsystems).
