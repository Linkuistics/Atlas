# component-ontology

Host-agnostic ontology of component-relationship edges and the schema
for `related-components.yaml`. Migrated from Ravel-Lite; consumed by
Atlas, Ravel-Lite, and any future tool that needs a shared edge
vocabulary.

## What's in the box

- **`EdgeKind`** — 17 edge kinds in 7 families (dependency, linkage,
  generation, communication, orchestration, testing, specification).
- **`LifecycleScope`** — 7 lifecycles (`design`, `codegen`, `build`,
  `test`, `deploy`, `runtime`, `dev-workflow`).
- **`EvidenceGrade`** — `strong` / `medium` / `weak` with criteria
  documented in `defaults/ontology.yaml`.
- **`Edge`** — kind + lifecycle + 2 participants + evidence + rationale.
  Validates direction, distinct participants, evidence presence.
- **`RelatedComponentsFile`** — `schema_version: 2`; idempotent
  `add_edge` keyed on a canonical (kind, lifecycle, participants) tuple.
- **`yaml_io::{load, load_or_default, save_atomic}`** — strict /
  tolerant load + tmp-then-rename atomic save.
- **`cli::{parse_edge_kind, parse_lifecycle_scope, parse_evidence_grade}`**
  — kebab-case parsers for host CLIs, with vocabulary-listing errors.
- **`defaults::{parse_embedded, render_kinds_for_prompt, ...}`** —
  parsed form of the shipped `defaults/ontology.yaml`, plus a markdown
  renderer used by Stage 2 prompt substitution.

## Canonical files

- `defaults/ontology.yaml` (at the workspace root) — the single source
  of truth for the kind / lifecycle / evidence-grade vocabulary. The
  crate embeds it via `include_str!`; a drift test asserts bijection
  with the Rust enum surface.
- `docs/component-ontology.md` (in Atlas) — the spec the Rust types
  realise. (Migrated to Atlas in stage M4.)

## Usage

Add to your `Cargo.toml` as a path or git dependency
(registry publication is not in scope per design §9.5):

```toml
[dependencies]
component-ontology = { path = "../Atlas/crates/component-ontology" }
```

```rust
use component_ontology::{Edge, EdgeKind, LifecycleScope, EvidenceGrade,
                          RelatedComponentsFile, save_atomic};

let mut file = RelatedComponentsFile::default();
file.add_edge(Edge {
    kind: EdgeKind::DependsOn,
    lifecycle: LifecycleScope::Build,
    participants: vec!["my-cli".into(), "my-lib".into()],
    evidence_grade: EvidenceGrade::Strong,
    evidence_fields: vec!["my-cli.Cargo.toml".into()],
    rationale: "Cargo manifest declares my-lib as a path dep.".into(),
})?;
save_atomic(std::path::Path::new("related-components.yaml"), &file)?;
```

## Versioning

- `RelatedComponentsFile.schema_version` — currently `2`. Independent of
  the ontology-definition version.
- `OntologyYaml.schema_version` (in `defaults/ontology.yaml`) —
  currently `1`. Versions the vocabulary itself.
- `load` and `load_or_default` hard-error on a mismatch; the upgrade
  path is delete-and-regenerate at the producer (no in-memory
  upgrades).

## Stability

Pre-1.0; breaking changes are possible at any patch bump until the
crate is stamped 1.0.0. The `schema_version` constants are the
contract that matters for on-disk compatibility.

## Dependencies

`anyhow`, `serde`, `serde_yaml`. Test-only: `regex`, `tempfile`.
