---
name: Atlas Phase 3 PR-6 — edges_add / edges_suppress + per-component field overrides are canonical mechanisms
description: Atlas overrides.yaml gained edges_add/edges_suppress (top-level) and per-component field_overrides (language/kind/lifecycle/subsystem) per design §5.5 — these are the canonical user-authoring seams the engine reads, not optional shortcuts.
type: project
originSessionId: phase3-pr6
---

Phase 3 PR-6 (commits a0a9a8c on atlas-contracts and the matching engine
commit on Atlas) extended `OverridesFile` with three new authoring
mechanisms:

1. **`edges_add: Vec<EdgeAdd>`** — hand-authored edges unioned with the
   analyser-discovered set at L6. Every entry requires a non-empty
   `reason` (deserialise-time enforced).
2. **`edges_suppress: Vec<EdgeSuppress>`** — hand-authored edges
   subtracted from the analyser-discovered set, matched by exact
   `(kind, from, to)` triple. Same required `reason`. A suppress that
   matches no edge logs a warning to stderr and is otherwise a no-op.
3. **`overrides:` block** (`field_overrides`) on per-component
   `<component>/.atlas/overrides.yaml` — supersedes analyser-emitted
   `language` / `kind` / `lifecycle` / `subsystem` for the owning
   component. Carried under serde rename `overrides` to mirror design
   §5.5's example.

`subsystem` is captured in the schema for forward compatibility but has
no destination on `ComponentEntry` yet — Phase 6 (user-facing schema
cleanups) adds the destination field. The other three flow through L4
onto the component descriptor.

**Why:** these are the canonical mechanisms the user authors when
the analyser is wrong. Phase 9's LLM analysers are expected to emit
candidate edges as `edges_add` suggestions for a human to accept;
suppression similarly gives the user a way to silence false-positive
analyser edges without rewriting the analyser. Treating them as the
*canonical* (not the *expedient*) channel means future edge providers
should plumb through this mechanism rather than inventing new
side-channels.

**How to apply:**

- Phase 9 LLM analysers that propose edges should emit them as
  `edges_add` candidates, not as direct `Edge` rows. The engine reads
  the merged `edges_add`/`edges_suppress` internally via
  `pub(crate) merged_overrides(db)`; external consumers (analysers,
  reports) read the post-merge edge set via the existing public
  projections (`all_proposed_edges`, `RelatedComponentsFile`), not
  via the internal helper.
- The `reason` field is deserialise-required. Any new pre-population
  flow that writes `edges_add` entries MUST populate `reason` —
  Atlas refuses to load entries without it.
- Per-component override files apply the existing scoping rule (Phase
  1 PR-0c): a per-component overrides.yaml at `D/.atlas/overrides.yaml`
  may only declare `pins` / `additions` / `field_overrides` for `D`'s
  own component or its sub-components. Out-of-scope pins are a hard
  error, not a warning.
- Conflict resolution: per-component values win over top-level pins
  for the same field on the same component (closest-source-wins,
  consistent with the existing pin precedence rule).
- Edges_add lifecycle defaults to `Design`; evidence_grade is `Strong`;
  rationale carries the user-supplied reason verbatim. Authors don't
  set those fields directly — the engine fills them in to keep the
  overrides.yaml shape minimal.
