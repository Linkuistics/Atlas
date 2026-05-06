# Contract content-sha canonicalisation

**Status:** Spec (resolves design open question §11.2.2). Companion to
`2026-05-06-atlas-vnext-phase1-plan.md` (Phase 1 implementation).
**Date:** 2026-05-06.
**Supersedes:** open question §11.2.2 in
`2026-05-06-atlas-system-model-design.md`. That open question is
considered closed once this spec lands.

---

## 1. Problem

A contract carries a `fingerprint` field — its `content_sha` — that other
components' caches key against (design §3.2, §3.4, §8.2). When a
contract's content sha changes, every component that consumes it
recomputes its L6 edge entries on the next access; when it does not
change, those entries cache-hit.

For caching to be correct, the content sha must change exactly when the
contract's *meaning* changes. It must not change for cosmetic edits to
the source representation:

- A YAML schema reordered (`schema.yaml` keys swapped, comments edited,
  trailing whitespace trimmed) describes the same contract.
- A Rust struct reformatted by `cargo fmt`, with the doc-comment moved or
  rephrased, describes the same binding.

Without a canonicalisation rule, every cosmetic edit thrashes the L6
cache across every consumer. The point of this spec is to fix the
algorithm — *one* algorithm per contract source kind — so the engine
treats two cosmetically-different sources as equivalent iff their
canonical forms hash the same.

This spec is normative. PR-7 (surfaces.yaml emission) implements the
code-derived branch; the schema-derived branch is specified now so
Phase 2 has nothing to invent and a single test fixture exercises it
in Phase 1.

---

## 2. Algorithm by contract source

A contract's content sha is computed from its **definition source**, not
from any of its bindings. (Bindings have their own content shas, see
§4.) Two algorithms apply, selected by the source kind:

### 2.1 Code-derived contracts

A code-derived contract is one whose authoritative definition is a
language-specific binding — typically a Rust struct with
`#[derive(Serialize, Deserialize)]`, a TypeScript `interface`, or a
Haskell type. The binding's source bytes are the canonical form.

**Algorithm:**

```
content_sha = sha256(canonical_serialisation_of_binding_AST)
```

For Phase 1, "canonical serialisation of the binding AST" is the
binding's content sha as L5 already computes it: SHA-256 over the
file-byte-range covered by the binding `span` in the binding's source
file. Phase 1 emits Rust bindings only and the binding span is
unambiguous (`pub struct Foo { … }` from the start of `pub` to the
closing brace), so the byte-range hash is sufficient.

Phase 2 generalises this to per-language AST canonicalisation via
language-specific analysers (e.g. rust-analyzer, ts-morph). The Phase 1
algorithm and the Phase 2 algorithm produce different shas; Phase 2's
schema bump (`SurfacesFile.schema_version: 2`) signals the
recomputation. This is acceptable because Phase 2 is internal-only and
ships its own reader fork.

**Phase 1 contract kinds covered by this branch:**

- `library-api` (Rust only in Phase 1).
- `data-format` whose source is a Rust binding annotated with
  `#[derive(Serialize, Deserialize)]`. The owning struct is the
  authoritative form; the on-disk YAML/JSON is a projection.

**Worked example.** A contract `atlas-contracts/components-yaml-schema`
is defined by `pub struct ComponentEntry` in
`atlas-contracts/crates/atlas-index/src/schema.rs`, span `[69, 95]`. The
content sha is

```
content_sha = sha256(bytes[start_offset..end_offset])
```

where `start_offset` and `end_offset` are computed from the line/column
span. Whitespace inside the span affects the sha; whitespace outside
the span does not. Reordering fields inside the struct *does* change
the sha — that is intended, because field order is meaningful for
stable serialisation. Reformatting via `cargo fmt` *does* change the
sha if it touches bytes in the span; the Phase 2 AST canonicaliser will
absorb that. Phase 1 accepts this limitation; downstream tooling that
runs `cargo fmt` between commits will see one extra surface
recomputation per affected component, which is correct (the surface
file's binding spans changed) and bounded (one re-run, not thrashing).

### 2.2 Schema-derived contracts

A schema-derived contract is one whose authoritative definition is a
schema document — a YAML schema, a JSON Schema, a `.proto` file, an
OpenAPI document. The schema's parsed structural form is the canonical
form; comments, whitespace, and key ordering are stripped.

**Algorithm:**

```
1. Parse the source into a structural AST.
2. Emit a canonical serialisation:
   - Object keys sorted ascending (lex order).
   - Arrays preserved in declaration order.
   - No comments.
   - No trailing whitespace; one space after each `:` and `,`.
   - UTF-8, no BOM.
   - Newline at end of file.
3. content_sha = sha256(canonical_bytes)
```

For Phase 1 the implementation is concrete:

- **YAML / JSON Schema sources:** parse with `serde_yaml` (which
  handles both YAML and JSON), into `serde_yaml::Value`. Sort
  `Mapping` entries by key (since `serde_yaml::Mapping` preserves
  insertion order, the canonicaliser walks the tree and rebuilds each
  Mapping as a `BTreeMap` of `(String, Value)`, then converts back to
  `Mapping`). Emit via `serde_yaml::to_string` (which does *not*
  preserve comments).
- **`.proto` sources:** Phase 1 does not emit any `.proto`-derived
  contracts. The algorithm above is the specified target; a future
  analyser supplies the parser.
- **OpenAPI sources:** treated as YAML/JSON.

**Phase 1 contract kinds covered by this branch:**

- `wire-protocol` whose source is a YAML/JSON document. (Phase 1 emits
  exactly one test-only `wire-protocol` contract from a hand-crafted
  YAML schema fixture, exercising the canonicaliser before any
  consumer relies on it.)
- Any future `data-format` whose source is a YAML/JSON Schema rather
  than a Rust binding.

**Worked example.** Two YAML schemas equivalent up to cosmetic edits:

```yaml
# v1
type: object
properties:
  name: {type: string}
  age: {type: integer}
required: [name, age]
```

```yaml
# v2 — reordered, commented, reflowed
required:
  - name
  - age
properties:
  age:
    type: integer
  name:
    type: string  # the component name
type: object
```

Both canonicalise to:

```yaml
properties:
  age:
    type: integer
  name:
    type: string
required:
- age
- name
type: object
```

and produce the same content sha. (Note `required` array preserves
declaration order *of the original document*; the `[name, age]` vs
`- name / - age` form is purely a flow-vs-block YAML detail. The
canonicaliser does not reorder array elements — array order is
semantic in JSON Schema.)

---

## 3. Phase 1 scope and Phase 2 deferral

Phase 1 emits exactly two contract kinds:

1. **Rust-binding-derived `data-format`** contracts (code-derived branch,
   §2.1). The most common case. PR-7 covers this.
2. **Rust `library-api`** contracts (code-derived branch, §2.1). The
   in-process API surface of each Rust component. PR-7 covers this.

Plus one test-only contract:

3. **A YAML-schema-derived `wire-protocol`** fixture (schema-derived
   branch, §2.2). Exercises the canonicaliser without coupling Phase 1
   to any real wire-protocol surface area. PR-7's acceptance test
   cohort includes a stability test for this fixture.

Phase 2 expands scope as follows:

- Per-language AST canonicalisation for code-derived contracts (replaces
  the byte-range hash). Bumps `SurfacesFile.schema_version` to 2.
- Real `.proto` and OpenAPI parsers for schema-derived contracts.
- TypeScript / Python / Haskell binding-derived contracts.

The schema-derived algorithm specified above does not change between
Phase 1 and Phase 2; Phase 2 only adds new source-format parsers.

---

## 4. Relationship to binding content shas

A binding (the language-specific projection of a contract — see
design §3.3) has its own `content_sha`, independent of the contract's
`content_sha`:

- A binding's content sha hashes the binding's source bytes (file +
  span). Algorithm: identical to §2.1's byte-range form, regardless of
  whether the *contract* is code-derived or schema-derived. The
  binding sha is per-language by construction.
- A contract's content sha hashes the contract's definition. For a
  code-derived contract the contract sha and the defining-binding sha
  are computed by the same algorithm but the *contract* sha is
  authoritative for cross-component cache keying. (For Phase 1 they
  are byte-equal, since the contract algorithm reduces to the binding
  algorithm. Phase 2 separates them.)
- For a schema-derived contract, the contract sha and any binding sha
  are computed by *different* algorithms and will not be byte-equal.
  Drift detection between binding and contract is a Phase 3
  responsibility (drift report); Phase 1 records both shas without
  acting on their relationship.

Implementation lives in `atlas-contracts/crates/atlas-index/src/surfaces.rs`
(types, PR-1) and `crates/atlas-engine/src/l5_surface.rs` plus
`crates/atlas-analyzers/src/rust_surface_analyzer.rs` (computation,
PR-7).

---

## 5. Test obligations

PR-7's acceptance criteria include a content-sha stability test for
code-derived contracts (whitespace inside the binding span changes the
sha; whitespace *outside* the binding span does not). PR-7 also adds a
canonicaliser unit test for the schema-derived branch using the YAML
example from §2.2.

A property-based test (proptest) belongs in PR-2's cache module, asserting
that two `serde_yaml::Value` trees structurally equal under the
canonicalisation rule produce equal canonical bytes. The property is
"canonical form is a function of structural value, not of source
syntax."

---

## 6. References

- Design spec: `2026-05-06-atlas-system-model-design.md` §3.2, §3.3,
  §3.4 (contracts/bindings/surfaces); §8.2 (cross-component
  invalidation); §11.2.2 (the open question this spec resolves).
- Phase 1 plan: `2026-05-06-atlas-vnext-phase1-plan.md` §2.1
  (resolution summary), §4 PR-7 (implementation), §5 (acceptance gate).
