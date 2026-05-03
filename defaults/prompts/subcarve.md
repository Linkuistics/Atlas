# Sub-carve decision

You are deciding whether a single component should be *sub-carved* —
that is, whether its interior hides one or more distinct sub-components
that Atlas should treat as children in the component tree, and if so,
which sub-directories to open up as new candidate roots.

The caller is Atlas, a design-recovery tool that builds a hierarchic
map of a codebase. A component is sub-carved when its internal
structure is better modelled as a small cluster of cohesive
sub-components than as a single atomic unit.

## When to sub-carve

Sub-carve when the component is a library or package whose source tree
has a clear split along responsibility lines — e.g. a Rust library
with `src/auth/` and `src/billing/` directories that touch disjoint
parts of the code. Do *not* sub-carve when the component is:

- A CLI binary, service, website, spec, docs repo, or config repo —
  these are leaf-like by convention.
- A library whose interior is genuinely uniform (one module, one
  thing done well).
- Already at the depth cap declared in `STRUCTURAL_SIGNALS.max_depth`.

A high `seam_density` (many edges stay *inside* the component) is a
mild signal in favour of sub-carving. A present `modularity_hint` is
a strong signal — the engine's bisection analysis has already found
a clean internal partition and is asking you to confirm it and name
the directories. An absent `modularity_hint` means the engine has no
deterministic opinion; your judgement decides.

## Output

Return **only** a JSON object matching this schema — no prose outside
the object, no extra fields:

```json
{
  "should_subcarve": true,
  "sub_dirs": ["src/auth", "src/billing"],
  "rationale": "<one or two sentences in prose>"
}
```

Field notes:

- `should_subcarve` — `true` when the engine should open up one or
  more sub-directories as new candidate roots; `false` when this
  component is the right unit as-is. If `false`, `sub_dirs` is
  ignored.
- `sub_dirs` — relative-to-repository-root paths of sub-directories
  to add as L2 candidate roots. Each path must lie inside one of
  `COMPONENT_PATHS`. Typical shape is `src/<name>` or
  `packages/<name>`. Do not propose the component's own root path.
- `rationale` — plain-English justification tied to the evidence
  (which directories, why they look independent).

<!-- CACHE_BOUNDARY -->

## Inputs

The component this call concerns:

- **Component id:** `{{COMPONENT_ID}}`
- **Declared kind** (one of Atlas's kinds; treated opaquely here):
  `{{COMPONENT_KIND}}`
- **Paths** the component covers, relative to the repository root:

```json
{{COMPONENT_PATHS}}
```

- **Structural signals** — `seam_density` (internal-vs-external edge
  ratio; higher = more isolated), `modularity_hint` (`null` when the
  engine has no hint; otherwise an object with `partition_a`,
  `partition_b`, `cross_edges`, `total_internal_edges`),
  `cliques_touching` (maximal cliques of size ≥ 3 involving this
  component), and `current_depth` / `max_depth`:

```json
{{STRUCTURAL_SIGNALS}}
```

- **Edge neighbourhood** — edges touching this component (may be empty
  on the first iteration of the fixedpoint):

```json
{{EDGE_NEIGHBOURHOOD}}
```

- **Pin-suppressed children** — ids the user has explicitly pinned as
  *not* children of this component. Do not re-propose directories that
  would re-create these ids:

```json
{{PIN_SUPPRESSED_CHILDREN}}
```
