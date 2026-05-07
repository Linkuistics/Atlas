# Atlas vNext Phase 2 — Status

Companion to `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`.
This file tracks per-PR completion state across sessions. The continuation
prompt at `docs/superpowers/prompts/2026-05-07-vnext-continue.md` reads
this file (via the `*phase2-plan*` wildcard match) to find the next PR
to dispatch.

**Last updated:** 2026-05-07 (PR-5 + PR-13 landed; Wave 1 in progress).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Plan + status file (docs only)
- [ ] PR-1  — TypeScript / JavaScript surface analyser (in-process)
- [ ] PR-2  — Subprocess analyser transport (stdio JSON)
- [ ] PR-3  — Python surface analyser (first subprocess analyser)
- [ ] PR-4  — Per-analyser `analyser_id` / `analyser_version` plumbing through L3 dispatch
- [x] PR-5  — Rust binding extractor: regex → `syn`
- [ ] PR-6  — C# surface analyser (subprocess)
- [ ] PR-7  — Dart / Flutter surface analyser (subprocess)
- [ ] PR-8  — Elixir surface analyser (subprocess)
- [ ] PR-9  — Racket surface analyser (subprocess)
- [ ] PR-10 — LispKit surface analyser (subprocess)
- [ ] PR-11 — Compose composition-edge analyser (deterministic, in-process)
- [ ] PR-12 — Shell-script LLM-fallback analyser (in-process)
- [x] PR-13 — Phase 1 hangover bundle (L8 phantoms + PR-12-of-Phase-1 polish)
- [ ] PR-14 — Acceptance: polyglot dull-shaped fixture (smoke test)

When every box is `[x]`, Phase 2 is complete and the continuation prompt
should report success and stop.

## Dependency graph (informational; canonical in plan §4 + plan §9)

```
PR-0 ──┬──> PR-1  (TS/JS in-process)            ──┐
       │                                            │
       ├──> PR-2  (subprocess transport) ──> PR-3   ├──> Wave 2 (parallel):
       │                                            │      PR-6  (C#)
       ├──> PR-4  (id/ver plumbing)        ─────────┤      PR-7  (Dart/Flutter)
       │                                            │      PR-8  (Elixir)
       ├──> PR-5  (rust binding → syn)              │      PR-9  (Racket)
       │                                            │      PR-10 (LispKit)
       └──> PR-13 (L8 + polish bundle)              │      PR-11 (Compose)
                                                    │      PR-12 (shell-script)
                                                    │
                                                    ▼
                                      PR-14 (acceptance smoke test)
```

**Parallel-safe waves:**
- Wave 0: PR-0 (this commit).
- Wave 1 (after PR-0): PR-1, PR-2, PR-4, PR-5, PR-13 — all five concurrently. (PR-13 is independent of analyser work.)
- Wave 2 (after PR-2 + PR-4): PR-3 (must precede Wave 3 to settle the `Visibility::Conventional` / `module_path` / `attributes` schema mutations on `surfaces.rs`).
- Wave 3 (after PR-3): PR-6, PR-7, PR-8, PR-9, PR-10, PR-11, PR-12 — all seven concurrently.
- Wave 4 (after Wave 3): PR-14 (smoke test).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of
what's worth recording: deviations from the plan that the next session
needs to know, surprising fixture quirks, manual verification steps that
succeeded, follow-up cleanup deferred, schema-mutation trail (which PR
added which field to `surfaces.rs`).

### PR-0
2026-05-07 — Landed in same commit as the plan. The Phase 2 plan
(`2026-05-07-atlas-vnext-phase2-plan.md`) and this status file are the
two artefacts. The continuation prompt
(`docs/superpowers/prompts/2026-05-07-vnext-continue.md`) is unchanged —
its Step 1 wildcard `*phase2-plan*` matches the new plan filename and
auto-routes future sessions into Step 3 (execution).

Load-bearing context for Wave 1 reviewers:

- **Greenfield carries forward across phases.** No on-disk format
  compatibility with Phase 1; no migration. A user upgrading deletes
  `.atlas/` and re-runs.
- **No schema_version bump in Phase 2.** `SurfacesFile.schema_version`
  stays integer `1`. Each language analyser PR mutates the v1 *shape*
  freely (PR-3 adds `Visibility::Conventional`, `module_path`,
  `attributes`; PR-8 adds `ContractKind::Behaviour`; etc.). Append the
  schema-mutation contribution to the per-PR note when the PR lands so
  the trail is auditable.
- **Subprocess analysers are deterministic-only in Phase 2.** No LLM
  access from subprocess. The shell-script LLM-fallback (PR-12) stays
  in-process. Phase 3+ may add a bidirectional callback channel if
  needed.
- **Per-analyser parser library choice is per-PR and overridable by the
  subagent.** Plan §4 names a default (e.g. `swc_ecma_parser` for
  PR-1, `rustpython-parser` for PR-3, `tree-sitter-c-sharp` for PR-6,
  `syn` for PR-5); a subagent that finds the named library inadequate
  during implementation may swap to a different mature pure-Rust
  alternative, recording the swap and its rationale in the per-PR
  status note.
- **Wave 1 first-dispatch order matters slightly:** PR-2 (subprocess
  transport) and PR-4 (id/ver plumbing) are both needed by PR-3
  (Python). PR-1 and PR-5 are independent. PR-13 is fully independent
  and can be parallel-dispatched with any of Wave 1.

### PR-1
(awaiting subagent dispatch)

### PR-2
(awaiting subagent dispatch)

### PR-3
(awaiting subagent dispatch)

### PR-4
(awaiting subagent dispatch)

### PR-5
2026-05-07 — Landed as commits `ffe26c1` (main rewrite) + `a5a503e`
(visibility-guard fix from spec re-review) + `d8549d6` (doc + unused
feature cleanup from code-quality review). No atlas-contracts changes.

**Schema-mutation contribution:** none — PR-5 only rewrites the existing
analyser; `Visibility::Conventional` / `attributes` / `module_path` /
`ContractKind::Behaviour` all remain Phase 2 forward-work (PR-3 / PR-8).
`ANALYZER_VERSION` constant bumped from `1.0.0` to `2.0.0` as the
breaking-change marker for downstream cache invalidation.

**Implementation:** `extract_rust_surface` rewritten around `syn::parse_file`;
the byte-by-byte state-machine walker, brace counter, comment / string-literal
skipper, raw-string prefix handling, and manual derive parser are all gone.
File went from 1266 → 810 lines (-36% net; -45% in implementation excluding
tests). Span byte ranges sourced from `proc_macro2::Span::byte_range`
(requires `span-locations` feature on `proc-macro2`); the start of each
binding's span is taken from `vis.span()` (the `pub` keyword) and the end
from `item.span()` so leading attributes (`#[derive(...)]`, doc comments)
do not widen the span — preserves the spec §2.1 PR-7 semantics.

**Visibility guard on mod recursion (`a5a503e`):** the spec re-review
caught that the new `walk_items` recursed into all inline mod bodies
regardless of visibility, broader than Phase 1's depth-0-only regex. Items
inside a non-pub mod are not externally reachable. Fix gates recursion on
`mod_item.vis` matching `Public(_) | Restricted(_)`; `Inherited` mods are
not walked. Note: `pub(self)` parses as `Restricted` and is therefore
recursed; this is an academic edge case (`pub(self)` is effectively never
written) and matches the syntactic-pub-ness criterion used by `is_pub`
elsewhere.

**Doc + dep cleanup (`d8549d6`):** module-level doc updated to accurately
describe span semantics (the original claim that "`pub` keyword starts the
item's span in `syn`'s representation" was wrong — `Item::span()` starts
at leading attributes); `extra-traits` feature on `syn` removed (unused;
its `Debug`/`PartialEq`/`Eq`/`Hash`/`Clone` derives on every AST node added
compile cost without benefit).

**Tests:** All 19 retained Phase 1 tests pass without fixture regeneration
(span byte ranges happened to coincide with the regex extractor's for the
covered cases). The Phase 1 `nested_pub_inside_pub_mod_is_phase1_known_limitation`
limitation-pinning test was replaced by the new
`syn_extracts_nested_pub_inside_pub_mod` acceptance test which asserts
`pub mod outer { pub struct Hidden; }` extracts both `outer` and `Hidden`.
The visibility-guard fix added `non_pub_mod_does_not_surface_inner_pub_items`
as a regression test. Total test count: 21 (19 retained + 1 acceptance + 1
regression). The `nested_pub_inside_pub_mod_is_phase1_known_limitation`
memory note is closed.

### PR-6
(awaiting subagent dispatch)

### PR-7
(awaiting subagent dispatch)

### PR-8
(awaiting subagent dispatch)

### PR-9
(awaiting subagent dispatch)

### PR-10
(awaiting subagent dispatch)

### PR-11
(awaiting subagent dispatch)

### PR-12
(awaiting subagent dispatch)

### PR-13
2026-05-07 — Landed as commits `788fc92` (main change) + `71b8019` (doc fix
addressing code-quality reviewer's request to make the absolute-path
`SubcarveDecision.sub_dirs` contract explicit). No atlas-contracts changes.

**L8 fix location and scope:** the L8 fixedpoint enumeration lives in
`crates/atlas-engine/src/l8_recurse.rs` (the plan's hint at
`l8_subcomponents.rs` was a renaming artefact; the actual file is
`l8_recurse.rs`). The phantom-subcomponent fix was bigger than the plan's
"one-line condition" framing implied and required two related changes:

1. **Manifest disambiguation in `absolutise_under_any_root`** — Pass 1 now
   prefers a root that has at least one registered manifest under the
   candidate path before falling back to the legacy "first root with a
   registered file" pass. This eliminated the
   `atlas-contracts/consumer-crate` phantom under the peer-root + primary
   parent-dir-share layout.
2. **Absolute paths in subcarve back-edge `sub_dirs`** — `map_reduce_subcarve`
   now stores `abs_dir.clone()` instead of `rel.to_path_buf()`. The relative
   form was phantom-amplified by L2's source-4 root-walk: for each relative
   `back_edge` value, L2 tried each root and accepted the first whose
   `path_is_inside(<root>/<sub_dir>, dir)` matched, so a relative `src`
   could spuriously route to `<primary>/src` and create a phantom
   candidate. Pre-PR-13 this hole existed but didn't bite because the
   relative paths happened to resolve under one root only.

The `SubcarveDecision.sub_dirs` field doc and `subcarve_plan` function doc
now explicitly state the absolute-path contract (the doc-fix commit
`71b8019` addresses the code-quality reviewer's concern that this
load-bearing invariant was implicit).

**Polish items addressed:**
- `l5_surface.rs` `vec![PathBuf::new()]` sentinel removed; absolute-segment
  branch now uses an early `continue`.
- `surface_calls_for` in `atlas_contracts_in_ravel_lite.rs` rewritten to
  use `serde_json::from_str` + structured `COMPONENT_ID` lookup (was
  substring matching against canonical input JSON).
- `pipeline.rs` lifted the inline closure into `resolve_component_abs_dir`
  helper with a `ResolvedComponentDir { path, fell_back }` return type;
  caller emits `eprintln!("warning: ...")` when `fell_back` is true.
  Five new unit tests pin the helper.

**Tests added:**
- `peer_root_with_empty_segment_does_not_phantom_emit_primary_subdirs` in
  `crates/atlas-engine/tests/l7_l8_fixedpoint.rs` — hermetic regression
  fixture mirroring the PR-12-of-Phase-1 layout; asserts zero phantom
  subcomponents.
- Five `resolve_component_abs_dir_*` unit tests in `pipeline::tests`
  exercising the manifest-disambiguation, absolute-segment short-circuit,
  fallback-to-roots[0], and missing-dir cases.
- The existing `back_edge_adds_subcarve_sub_dirs_to_workspace_carve_back_edge`
  test was updated to assert on absolute paths in `sub_dirs`.

**Memory invariants preserved:** `tombstone_emit_once_design` and
`all_components_not_salsa_tracked` are intact; L4 prior-filter was not
touched; `resolve_component_abs_dir` is a plain free function in the CLI
layer (not salsa-tracked).

**Deferred:** none — all four hangover items closed.

### PR-14
(awaiting subagent dispatch)
