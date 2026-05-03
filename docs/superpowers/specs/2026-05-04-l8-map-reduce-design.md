# L8 sub-carve as map/reduce

**Date:** 2026-05-04
**Driver task:** `map-reduce-llm-architecture-with-backend-routing-for-context-light-analysis`
**Spike:** `docs/spikes/2026-05-02-l3-l8-prompt-shape-spike.md`

## Status

Implemented in `crates/atlas-engine/src/l8_recurse.rs::map_reduce_subcarve`,
landing alongside an L3-call cache fix in `l3_classify::classify_via_llm`
(now routes through `db.call_llm_cached`). The standalone `Subcarve`
LLM prompt is no longer rendered, but `defaults/prompts/subcarve.md`
is retained in `EMBEDDED_PROMPTS` so its `template_sha` keeps
contributing to the run-wide fingerprint during the transition.
Slated for deletion after one shipped release.

## Problem

The pre-redesign L8 path issued one large agentic Claude Code call
per component being carved. The agent enumerated the component's
internal directory structure with Read/Glob, weighed each candidate
sub-dir, and returned a curated `sub_dirs` array. Three structural
problems made this expensive:

1. **Context rot.** Each agent call grew its context monotonically as
   it explored. Decisions degraded as the prompt size climbed.
2. **No cross-component reuse.** Two near-identical Rust crates each
   paid full agent-call cost; the prompt body always differed enough
   that no two requests cache-shared.
3. **No parallelism.** The fixedpoint driver iterates components
   serially, with each call taking minutes of wall time.

A real-monorepo first iteration on dull was observed at >15 min for
74 sub-carve decisions.

## Design

L8 splits into a deterministic *map* step and a deterministic *reduce*
step. The map step asks the L3 question of each immediate sub-dir of
the component being carved; the reduce step assembles per-sub-dir
verdicts into the final `SubcarveDecision`.

### Map step (engine, no LLM in the engine layer)

```text
for each immediate sub-directory dir of component.path_segments:
    if dir is pin-suppressed (basename or slug match): skip
    classification = is_component(db, workspace, dir)
    if classification.is_boundary:
        emit dir as a sub_dir
    else:
        record rejection
```

Pin-suppression compares both raw basename and the slugified form so
`crates/Atlas-Engine` and `atlas-engine` collapse to one match.

The verdict on each `dir` reuses the entire L3 cascade:

1. Pin short-circuit (free).
2. Deterministic rule from `crate::heuristics` (free, fires on
   manifest-bearing dirs).
3. LLM fallback through `db.call_llm_cached(&request)` —
   `LlmResponseCache` keyed on `(LlmFingerprint, PromptId, canonical-JSON(inputs))`.

This means the same `dir` asked twice (once from L8 map, once when L2
re-enumerates the dir as a candidate after the back-edge fires) is one
LLM call, not two.

### Reduce step (engine, deterministic)

```text
should_subcarve = (any sub_dir verdict was a boundary)
sub_dirs        = [dir for dir, verdict in verdicts if verdict.is_boundary]
rationale       = summary of accepted/rejected counts and kind labels
```

Modularity hint stays a *prior*, not a gate: when the engine has a
`modularity_hint` but the verdicts found no boundary, the rationale
records the disagreement, but the verdicts win.

### What is no longer needed

- `defaults/prompts/subcarve.md` is no longer rendered. It stays in
  `EMBEDDED_PROMPTS` until the next release cycle so its sha keeps
  contributing to the run-wide template fingerprint, then is deleted.
- `l8_recurse::build_subcarve_inputs` (and `parse_subcarve_response`)
  removed. The bidirectional token-coverage matrix in
  `prompt_token_coverage.rs` no longer includes `subcarve.md`.
- The fixedpoint driver's `PathologicalBackend` test —
  pathologically-novel sub_dirs from an LLM are no longer possible
  under map/reduce; sub_dirs are enumerated from registered files,
  bounded by filesystem reality and the depth cap.

## Backend routing

L3 (`Classify`) and L8 (now nothing routed; map step uses Classify)
are eligible for HTTP backends — both prompts are local-context-only.
L5 (`Stage1Surface`) and L6 (`Stage2Edges`) still need filesystem
access and stay subprocess-backed unless the filesystem-tool-use loop
in `filesystem-tool-use-loop-for-http-llm-backends-provider-agnostic`
ships.

`BackendRouter` reads `.atlas/config.yaml` per-operation `model:`
strings and dispatches by `PromptId`. Configuring HTTP for Classify
is a config-only change:

```yaml
operations:
  classify:
    model: anthropic/claude-haiku-4-5
    params:
      max_tokens: 1024
```

No code path changes are needed to take advantage of HTTP routing
for the map step.

## Concurrency

The current implementation is **serial**: the map step calls
`is_component` in a `for` loop over `candidates`. Bounded concurrency
(rayon parallel iterator with a configurable pool size, or a Tokio
runtime with a `Semaphore`) is a follow-up — Salsa 0.26 supports
parallel tracked-query execution and the `LlmResponseCache` is already
thread-safe via `AtomicU64` CAS, so the engine layer is ready.

The wall-time exit criterion (≤5 min for a 74-component first
iteration on a real monorepo) is reachable with serial map calls
when each call is HTTP (~1s round-trip) versus agent (~20s+ with tool
exploration). Concurrency is the lever for further speedup, not a
correctness requirement.

## Quality bar

The map/reduce path is **not strictly equivalent** to the prior
agentic path on the "library with `src/auth/` and `src/billing/`"
shape: today's enumeration is one level deep, so it sees `src` and
asks L3 a question whose answer is typically "non-component" (no
manifest, no sub-dir signal). A future iteration that descends
through "container" dirs (paths whose only children are other dirs,
e.g. `src/`, `lib/`, `app/`) will surface `src/auth, src/billing` as
candidates. Tracked as a follow-up.

For the targets currently in scope (the dev-workspace top level and
`~/Development/dull/`), one-level enumeration is sufficient because
sub-component candidates are always direct children of the
component's own root.

## Cache compatibility

- `template_sha` changes (subcarve.md still hashed → no change yet).
- `backend_version` may change if the operator flips the routing for
  Classify from `claude-code/...` to `anthropic/...` — that
  invalidates the LLM cache entries for that operation, as expected.
- `llm-cache.json` byte-identity on no-op re-runs is preserved —
  L3's verdicts are now also cached (previously bypassed the cache),
  so a re-run with no source changes makes zero LLM calls instead of
  re-firing every L3 fallback.

## Tests

- Engine unit tests in `l8_recurse.rs` cover the policy
  short-circuits, the empty-candidates case, the rationale shape,
  pin-suppression slug expansion, and the `is_pin_suppressed` /
  `build_rationale` helpers.
- Integration tests in `crates/atlas-engine/tests/l7_l8_fixedpoint.rs`
  exercise the back-edge closure end-to-end with a `ScriptedBackend`
  that returns `is_boundary: true` for every Classify call.
- The `pipeline_integration` no-op-re-run tests
  (`second_run_on_unchanged_fixture_is_byte_identical`,
  `llm_cache_json_is_written_and_read_across_invocations`) verify
  that the new L3-routed-through-cache wiring keeps the
  zero-LLM-call-on-no-op-re-run contract.
