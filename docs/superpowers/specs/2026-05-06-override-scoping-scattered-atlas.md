# Override scoping under scattered `.atlas/`

**Status:** Spec (resolves design open question §11.2.3). Companion to
`2026-05-06-atlas-vnext-phase1-plan.md` (Phase 1 implementation).
**Date:** 2026-05-06.
**Supersedes:** open question §11.2.3 in
`2026-05-06-atlas-system-model-design.md`. That open question is
considered closed once this spec lands.

---

## 1. Problem

Phase 1 introduces scattered per-component `.atlas/` directories
(design §4.6, §5.4): each component carries its own
`<component-path>/.atlas/overrides.yaml` alongside the source it
describes. The top-level `<primary-root>/.atlas/components.overrides.yaml`
remains the place for cross-component pins and additions. Multi-root
workspaces (design §5.3) add another wrinkle: peer roots discovered via
path-dep walking each carry their own top-level overrides too.

Three things need to be defined before PR-6's writers and L4's override
merger can land:

1. **Discovery order.** In what sequence does L4 read the various
   override files?
2. **Merge semantics.** When two override files set conflicting values
   for the same `(component_id, override_key)` tuple, who wins, and is
   the conflict reported?
3. **Per-component scoping.** May a per-component `overrides.yaml`
   carry pins for unrelated components, or only for itself and its
   sub-components?

This spec is normative for Phase 1. PR-6 implements it; the override
merge in `crates/atlas-engine/src/l4_tree.rs` is extended accordingly.

---

## 2. Override file types and locations

Three classes of override file exist in Phase 1:

| Class | Path | Scope |
|---|---|---|
| **Top-level (primary)** | `<primary-root>/.atlas/components.overrides.yaml` | All components from any root. |
| **Top-level (peer)** | `<peer-root>/.atlas/components.overrides.yaml` | Components rooted under that peer. (Cross-root pins in a peer's top-level file are *legal*; the peer is conceptually the primary root of its own checkout, just consumed via path-dep here.) |
| **Per-component** | `<component-path>/.atlas/overrides.yaml` | The owning component and its sub-components only — see §5. |

All three classes share the same on-disk schema (a list of `additions`,
`pins`, and `suppressions` per design §6.1's override conventions). They
differ only in discovery order and scoping rules.

---

## 3. Discovery order

L4 reads override files in the following deterministic order. Every
file's contents are merged into a single override set with the merge
semantics in §4.

1. **Primary-root top-level**:
   `<primary-root>/.atlas/components.overrides.yaml`.
2. **Peer-root top-level**, in lexicographic order of the peer-root
   path:
   `<peer-root[0]>/.atlas/components.overrides.yaml`,
   `<peer-root[1]>/.atlas/components.overrides.yaml`,
   …
   The lex order is over absolute paths after canonicalisation
   (`std::fs::canonicalize`). This makes the order independent of the
   order in which path-dep expansion discovered the peers.
3. **Per-component**, in ascending order of `component_id`:
   for each component in `all_components` whose path contains a
   `.atlas/overrides.yaml`, read that file. Components are sorted by
   their full id string (`atlas-contracts/atlas-index` <
   `atlas-contracts/component-ontology` < …) under standard
   `Ord for String` semantics.

The rationale for "per-component last" is §1's design intent: data
co-located with the component is authoritative for that component, so
its override has the final say.

---

## 4. Merge semantics

Override entries are keyed by `(component_id, override_key)`. The
override key is one of:

- A specific field name (e.g. `kind`, `lifecycle_roles`, `parent`).
- The sentinel `*addition` for a component declared by an `additions`
  block (a synthetic component the user is asserting into existence).
- The sentinel `*suppression` for a component the user is suppressing.

The merge rule is **last-writer-wins** under the discovery order from
§3. The implementation walks the discovery order, and for each entry
inserts into a `BTreeMap<(ComponentId, OverrideKey), OverrideValue>`,
overwriting any existing value. After the walk, the BTreeMap is the
materialised override set L4 applies.

Key consequences:

- A primary-root pin for `ravel-lite/billing-core#kind: rust-library`
  is overwritten by a per-component pin in
  `ravel-lite/billing-core/.atlas/overrides.yaml` setting
  `#kind: docker-image` — the per-component pin wins.
- A peer-root pin and a primary-root pin conflict: the peer-root pin
  wins iff its peer sorts later than the primary root in lex order.
  In practice the primary root is the user's `cwd`, the peers are
  discovered via path-dep, and their absolute paths typically sort
  *after* the primary root, so peers shadow primary. **This is
  intentional**: a path-dep'd repo is acting as the source of truth
  for its own components, including their classification.
- An `additions` block creating a component id that another override
  file then *suppresses* yields the suppression iff the suppression
  comes later in discovery order. Phase 1 ships this raw. Stricter
  semantics (e.g. "addition + suppression on the same id is an error")
  are a Phase 2 polish.

---

## 5. Per-component scoping rule

A per-component `<component-path>/.atlas/overrides.yaml` may only
carry overrides for component ids that lie within its own namespace
prefix. Concretely:

> If the owning component's id is `P`, a per-component overrides
> file at `<P-path>/.atlas/overrides.yaml` may carry entries for
> exactly: id `P`, or any id of the form `P/<suffix>`.

Cross-component pins in a per-component file (e.g. an
`atlas-contracts/atlas-index/.atlas/overrides.yaml` containing a pin
for `atlas-contracts/component-ontology`) are rejected with a hard
error at L4 merge time. The error names both the offending file and
the offending component id.

This rule is what makes the scattered-`.atlas/` invariant
operationally enforceable: data co-locates with source, and data about
*another* component does not belong in this component's directory.
Without this rule, a hostile or careless per-component override could
silently rewrite the classification of a sibling component, undoing
the value of co-location.

The top-level `components.overrides.yaml` (primary or peer) has no
such restriction — it is by design the place for cross-component pins.

**Note on workspace components.** A workspace component (e.g.
`atlas-contracts`) whose path is the manifest-root path *does* have
sub-component children (`atlas-contracts/atlas-index`, etc.). A
per-component overrides file at `atlas-contracts/.atlas/overrides.yaml`
may pin its own children — that is well-formed, since their ids share
the namespace prefix. This is the well-known monorepo case where
workspace-level overrides describe many crates from one place.

---

## 6. Conflict reporting

When two override files set conflicting values for the same
`(component_id, override_key)`, the merge algorithm in §4 resolves
the conflict deterministically (last writer wins). The user is
informed:

- **Phase 1 behaviour:** a `warning` is emitted on stderr naming both
  files involved, the override key, and the resolved (winner) value.
  Format:

  ```
  warning: override conflict on (ravel-lite/billing-core, kind):
    primary       <primary-root>/.atlas/components.overrides.yaml: rust-library
    per-component <primary-root>/billing-core/.atlas/overrides.yaml: docker-image
    resolved value: docker-image  (per-component wins by discovery order)
  ```

- **Phase 2 addition:** a `--strict-overrides` CLI flag escalates the
  warning to a hard error. Phase 1 ships warning-only; we want the
  multi-root experience to be forgiving while users learn the
  discovery rules.

The validator that enforces the per-component scoping rule (§5) is
*not* configurable: cross-component pins in a per-component file are
*always* an error, never a warning. The scoping rule is an invariant,
not a preference.

---

## 7. Worked example

Workspace shape:

```
~/Development/Ravel-Lite/                  (primary)
  .atlas/components.overrides.yaml
  billing-core/
    .atlas/overrides.yaml
~/Development/atlas-contracts/             (peer, via path-dep)
  .atlas/components.overrides.yaml
  crates/atlas-index/
    .atlas/overrides.yaml
```

Discovery order (per §3):

1. `~/Development/Ravel-Lite/.atlas/components.overrides.yaml`
2. `~/Development/atlas-contracts/.atlas/components.overrides.yaml`
   (peer; lex-sorted before next peer if any)
3. `~/Development/Ravel-Lite/billing-core/.atlas/overrides.yaml`
   (per-component, id `ravel-lite/billing-core`)
4. `~/Development/atlas-contracts/crates/atlas-index/.atlas/overrides.yaml`
   (per-component, id `atlas-contracts/atlas-index`)

If file 1 pins `ravel-lite/billing-core#kind: rust-library`, file 2
pins it to `something-else`, and file 3 pins it to `docker-image`:

- File 2's pin is rejected by §5 (file 2 is a peer-root top-level —
  legal — but if the pin appears in a *per-component* file at
  `atlas-contracts/.../...overrides.yaml` for a `ravel-lite/...` id,
  §5 hard-errors). In this example, file 2 is top-level, so the pin
  is legal, just superseded.
- The merged value is `docker-image` (file 3 wins by discovery order).
- A warning is emitted naming files 1, 2, and 3.

If instead file 4 contains a pin for `ravel-lite/billing-core` (a
cross-component pin in a per-component file): §5 hard-errors.

---

## 8. Test obligations

PR-6's acceptance tests cover:

- **Discovery order test.** Two pins on the same key from a top-level
  file and a per-component file resolve to the per-component value.
- **Per-component scoping rejection.** A per-component override file
  carrying a pin for an unrelated component causes the run to fail
  with a clear error naming the file and id.
- **Lex-order test for peer roots.** A workspace with two peer roots
  resolves consistently across runs (the peer with the lex-larger
  canonical path wins on conflict).
- **Warning-only behaviour.** A primary/per-component conflict
  emits a warning on stderr but does not fail the run.

The override-merge implementation in
`crates/atlas-engine/src/l4_tree.rs` is the canonical home for the
discovery walk; the per-component scoping validator lives next to the
merge as a precondition check.

---

## 9. References

- Design spec: `2026-05-06-atlas-system-model-design.md` §4.6 (data
  co-locates with source), §5.3 (multi-root), §5.4 (file layout),
  §6.1 (override schema), §11.2.3 (the open question this spec
  resolves).
- Phase 1 plan: `2026-05-06-atlas-vnext-phase1-plan.md` §2.2
  (resolution summary), §4 PR-6 (implementation), §5 (acceptance
  gate).
- v1 single-file override merge:
  `crates/atlas-engine/src/l4_tree.rs` is the inner case this spec
  generalises.
