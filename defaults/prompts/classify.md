# Component classification

You are classifying a single candidate directory to decide whether it
is a component and, if so, what kind of component it is. The caller is
Atlas, a design-recovery tool that extracts a hierarchic
pattern-based description of a codebase. A component is a cohesive
unit of design that makes sense to name and reason about on its own;
it is not automatically every directory, nor every file with a
manifest.

## Output

Return a JSON object matching this shape:

```json
{
  "kind": "<one of the kinds below>",
  "language": "<null or a short lowercase language name>",
  "build_system": "<null or a short lowercase build-system name>",
  "lifecycle_roles": ["<one or more lifecycle scopes>"],
  "role": "<null or a short lowercase role tag>",
  "evidence_grade": "<strong | medium | weak>",
  "evidence_fields": ["<short tokens referencing the decisive inputs>"],
  "rationale": "<one or two sentences in prose>",
  "is_boundary": <true or false>
}
```

Field notes:

- `kind` — pick exactly one value from the kind catalogue below. If
  nothing fits, choose `non-component` and explain in `rationale`.
- `lifecycle_roles` — pick one or more from the lifecycle-scope
  catalogue below. Most components are `[build, runtime]`; test-only
  components are `[test]`; tooling is `[dev-workflow]`.
- `evidence_grade` — `strong` when a manifest or similarly unambiguous
  artifact decides the kind; `medium` when a heading or filename
  makes it likely; `weak` when you are guessing from weak signals.
- `evidence_fields` — a short list of tokens pointing at the decisive
  inputs (e.g., `"Cargo.toml:[lib]"`, `"README.md:heading[1]"`).
- `is_boundary` — `true` when this directory is a genuine component
  boundary and should appear in `components.yaml`; `false` when it is
  a `non-component` (e.g., a bare `.git` directory with no companion
  manifest) that the engine should enumerate but not treat as a
  boundary.

Do not include any prose outside the JSON object. Do not add fields
that are not listed above.

## Kind catalogue

{{COMPONENT_KINDS}}

## Lifecycle-scope catalogue

{{LIFECYCLE_SCOPES}}

<!-- CACHE_BOUNDARY -->

## Inputs

The candidate this call concerns:

- **Directory (relative to repo root):** `{{DIR_RELATIVE}}`
- **Rationale bundle** — per-candidate signals collected by the engine
  (`manifests`, `is_git_root`, `doc_headings`, `shebangs`):

```json
{{RATIONALE_BUNDLE}}
```

- **Manifest contents** — the first few kilobytes of each manifest,
  keyed by path:

```json
{{MANIFEST_CONTENTS}}
```
