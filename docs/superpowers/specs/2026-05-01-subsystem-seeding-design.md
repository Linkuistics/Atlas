# Subsystem Seeding Design — v1

**Date:** 2026-05-01
**Status:** Approved (brainstorming → ready for plan)
**Backlog item:** `first-class-subsystem-seeding-in-overrides-yaml`

## Problem

`components.overrides.yaml` covers component-granularity seeding (`pins`, `additions`, `suppress_children`) but there is no first-class concept of a **subsystem** — a named group of components with hand-drawn boundaries. A user who knows their codebase ("auth is everything under `services/auth/*` plus `libs/identity`") cannot express that today; per-component pinning is verbose, doesn't capture grouping intent, and isn't queryable downstream as a unit.

## Goal

Let users hand-author subsystem boundaries that:

1. Name a subsystem and list its member components (by component id or path glob).
2. Carry their own metadata (role, lifecycle_roles, rationale, evidence_grade) parallel to `ComponentEntry`.
3. Are surfaced in a new `subsystems.yaml` projection so downstream consumers can query "what subsystems exist? what components belong to subsystem X?".
4. Can be named as participants in `related-components.yaml` edges, on equal footing with component ids.

## Non-goals (v1)

- Automatic subsystem discovery from graph structure (separate, larger task).
- Nested subsystems (a subsystem containing other subsystems).
- Higher levels of abstraction beyond subsystem (module, system).
- Subsystem-driven feedback into classification (lifecycle propagation, kind inheritance).

## Scope summary

| Decision | Choice |
|---|---|
| Engine integration | Projection + edge participation. No influence on L1–L8. |
| Input file | New `subsystems.overrides.yaml` (separate from `components.overrides.yaml`). |
| Output file | New `subsystems.yaml` (separate from `components.yaml`). |
| Membership form | Globs *and* ids, autodetected: contains `/` or `*` → glob, else id. |
| Validation strictness | Hard error on unknown id; warning on empty glob. |
| Edge participation | Shared id namespace across components and subsystems; uniqueness validated. `Edge.participants: Vec<String>` schema unchanged. |
| Subsystem fields | id, members, role?, lifecycle_roles, rationale, evidence_grade, evidence_fields. No `kind`, no `parent`. |
| CLI surface | `atlas validate-overrides` extended to cover subsystems. No new subcommand. |

## Schemas

### Input — `subsystems.overrides.yaml`

```yaml
schema_version: 1
subsystems:
  - id: auth
    members:
      - services/auth/*           # glob (contains '/' or '*')
      - libs/identity             # glob
      - identity-core             # id (no '/' or '*')
    role: identity-and-authorisation
    lifecycle_roles: [runtime]
    rationale: "owns all session/token/identity surfaces"
    evidence_grade: strong
    evidence_fields: []
```

Rust shape (lives in `atlas-contracts/crates/atlas-index`):

```rust
pub const SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION: u32 = 1;

pub struct SubsystemsOverridesFile {
    pub schema_version: u32,
    pub subsystems: Vec<SubsystemOverride>,
}

pub struct SubsystemOverride {
    pub id: String,
    pub members: Vec<String>,
    pub role: Option<String>,
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
}
```

### Output — `subsystems.yaml`

```yaml
schema_version: 1
generated_at: "2026-05-01T08:37:48Z"
subsystems:
  - id: auth
    role: identity-and-authorisation
    lifecycle_roles: [runtime]
    rationale: "owns all session/token/identity surfaces"
    evidence_grade: strong
    evidence_fields: []
    members:
      - auth-service
      - identity-lib
      - identity-core
    member_evidence:
      - id: auth-service
        matched_via: "services/auth/*"
      - id: identity-lib
        matched_via: "libs/identity"
      - id: identity-core
        matched_via: "id"
    notes: []
```

Rust shape:

```rust
pub const SUBSYSTEMS_SCHEMA_VERSION: u32 = 1;

pub struct SubsystemsFile {
    pub schema_version: u32,
    pub generated_at: String,
    pub subsystems: Vec<SubsystemEntry>,
}

pub struct SubsystemEntry {
    pub id: String,
    pub role: Option<String>,
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub members: Vec<String>,
    pub member_evidence: Vec<MemberEvidence>,
    pub notes: Vec<String>,
}

pub struct MemberEvidence {
    pub id: String,
    pub matched_via: String,
}
```

`generated_at` follows the Atlas convention: stamped by the CLI at write time, but the Salsa-side projection leaves it empty so byte-identity on no-op re-runs is preserved (per memory `no-op-re-run-byte-identity-needs-cache-persistence-stable-generated-at`).

## Architecture

### Layer placement

L9 only. Subsystems are an *output*, not a signal feeding back into classification. L1–L8 do not read the new Salsa input.

### Salsa input

`Workspace` gains:

```rust
pub subsystems_overrides: Arc<SubsystemsOverridesFile>,
```

with a setter that follows the `salsa-0-26-input-setters-do-not-check-equality` guard pattern (skip-if-equal).

### L9 projection

```rust
pub fn subsystems_yaml_snapshot(db: &AtlasDatabase) -> Arc<SubsystemsFile>;
```

Algorithm:

1. Read `workspace.subsystems_overrides(db)`.
2. Read `all_components(db)` — Salsa-memoised.
3. For each `SubsystemOverride`, resolve members:
   - For each `members` entry, decide form: contains `/` or `*` → glob, else id.
   - For globs: match against the joined `path_segments` of every non-deleted component. Multiple matches accepted.
   - For ids: lookup against the component id index. Failed lookups become a fatal validation entry returned alongside the file (see Validation).
4. Build resolved `members: Vec<String>` (sorted, deduped) and `member_evidence: Vec<MemberEvidence>` (preserving the matching order, one entry per resolved component, plus one entry per zero-match glob with `matched_via: "<glob> (no matches)"`).
5. If a subsystem has zero resolved members, append `"all members unresolved"` to its `notes`.

### Pipeline wiring

`atlas-cli/src/pipeline.rs`:

```text
load_or_default_subsystems_overrides(...)
  → validate.validate_overrides(extended)              ← pre-LLM stage
  → db.set_subsystems_overrides(...)
  → ... existing L1-L8 work ...
  → subsystems_yaml_snapshot(&db)                      ← post-L4 stage
  → cross_namespace_collision_check(...)
  → save_subsystems_atomic(path, &file)
```

### Validation surfaces

Two-stage:

**Pre-LLM (in `validate_overrides`):**

- File loads (schema_version match). Hard error on mismatch with "user-authored: migrate by hand" hint.
- Each subsystem id is unique within the file.
- Each `evidence_grade` parses (existing `EvidenceGrade::parse` semantics).
- Each subsystem's input `members` Vec is non-empty (an entirely empty member list at authoring time is a typo, not a useful subsystem).

**Post-L4 (in `subsystems_yaml_snapshot` or a sibling helper before save):**

- Each id-form member resolves to an existing component → hard error if not. Fix-it: "did you mean a glob?".
- No subsystem id collides with a component id → hard error if collision. Fix-it: "rename the subsystem".
- Glob members that match zero components → warning, surfaced in `member_evidence` (`matched_via: "<glob> (no matches)"`) and the validation report; not fatal.
- Resolved members empty (input had ≥1 members but every glob matched zero and there were no id members) → not fatal, `notes` gets `"all members unresolved"`. The distinction from pre-LLM is intentional: empty *input* is a typo; empty *resolution* is a forward-looking subsystem authored before its components exist.

### CLI

`atlas validate-overrides` extended to:

- Also load `subsystems.overrides.yaml` and run the pre-LLM validation stage.
- The subcommand prints a single combined report covering both files.

No new top-level subcommand.

### Edge participation

`Edge.participants: Vec<String>` schema unchanged. The validator that enforces participant id uniqueness (a new `validate_participant_namespace` method on `RelatedComponentsFile` in `component-ontology`, called from the cross-namespace check) catches the only schema-relevant collision concern.

LLM prompts (L6 in particular) are *not* updated in v1. The L6 batch produces edges naming component ids today; subsystems can be added as edge participants only by hand-authoring `related-components.yaml` post-LLM. A future task can extend L6 to consider subsystems.

## Testing

### Schema-level (atlas-contracts)

- `SubsystemsOverridesFile` round-trip through YAML.
- `SubsystemsFile` round-trip through YAML.
- Schema version mismatch produces user-authored error message (parallels `overrides_load_rejects_wrong_schema_version_and_warns_against_deletion`).
- `SubsystemEntry` with empty members round-trips.

### Engine-level (atlas-engine)

- `subsystems_yaml_snapshot` with empty input → empty file.
- Single subsystem with one glob, one id, one path-prefix glob → all three resolve correctly; `member_evidence` distinguishes them.
- Glob matching zero components → warning entry, subsystem still in output, `notes` includes `"all members unresolved"` only if zero total matches.
- Skip-if-equal: setting `subsystems_overrides` to an identical value doesn't bump Salsa revision (mirror the existing test for `components_overrides`).
- Subsystem-only edit doesn't invalidate `all_components`: assert that L4 work isn't redone after a subsystems-input change.

### Integration (atlas-cli)

- **Rename-stability fixture (per task exit criteria):** two-run sequence on a fixture where a directory rename moves a component under a glob. After rename, the glob still matches the new path; the id form continues matching by id. Asserts `member_evidence` reflects both forms correctly.
- **Pipeline integration:** hand-author a `subsystems.overrides.yaml`, run pipeline, assert `subsystems.yaml` contains the subsystem with resolved member ids.
- **Cross-namespace collision halt:** fixture where a subsystem id collides with a component slug. Pipeline halts with the expected error before saving any output.
- **Byte-identity re-run:** `subsystems.yaml` is byte-identical on a no-op re-run.
- **`validate-overrides` reports subsystem errors:** schema mismatch, duplicate ids, empty members are all surfaced with file path.

### Edge participation (atlas-cli + component-ontology)

- Edge with subsystem participant survives a round-trip through `related-components.yaml`.
- Subsystem id colliding with a component id triggers the cross-namespace validator regardless of whether either appears in an edge.

## Documentation

`README.md` Usage section gains a "Pre-seeding subsystems" subsection with:

- The minimal `subsystems.overrides.yaml` example shown in this spec.
- A note on the membership form heuristic (`/` or `*` → glob, else id).
- A pointer to `atlas validate-overrides`.
- One worked example showing the resulting `subsystems.yaml`.

## Exit criteria

- `subsystems.overrides.yaml` and `subsystems.yaml` schemas land in `atlas-contracts/crates/atlas-index`.
- L9 emits subsystems via `subsystems_yaml_snapshot`.
- The pipeline saves `subsystems.yaml` atomically.
- All tests above pass.
- `atlas validate-overrides` covers the new file.
- `README.md` Usage gains a "Pre-seeding subsystems" subsection.

## Open follow-ups (out of v1, recorded for triage)

- L6 prompt extension to let LLM-proposed edges name subsystems.
- Auto-discovery of subsystem candidates from L7 SCC/clique signals.
- Nested subsystems and the broader system → module → component hierarchy.
- A `kind` vocabulary for subsystems (current ComponentKind is component-granularity).
