# Spike: L3 / L8 prompt shape vs per-candidate HTTP backend

**Date:** 2026-05-02
**Driver task:** `spike-validate-l3-l8-prompt-shape-for-per-candidate-http-backend`
**Unblocks:** `map-reduce-llm-architecture-with-backend-routing-for-context-light-analysis`

## Question

Are the existing L3 (`is_component`) and L8 (`subcarve`) prompts scoped
correctly for the proposed per-candidate / per-subdir HTTP-call model?
A per-subdir map call sees only that subdirectory's local context. If
either prompt implicitly leans on cross-directory signals (sibling
shape, parent-workspace layout, multi-crate awareness), the narrowed
shape will produce lower-quality verdicts than today's agentic call.

## Method

Static analysis of the prompt templates and the engine's
`LlmRequest.inputs` builders. The driver code is more authoritative
than the prompt body alone: a prompt token is only as informative as
the JSON the driver supplies for it. Live LLM sampling was considered
but is unnecessary to answer the structural questions; see
[Why no live calls](#why-no-live-calls).

Reference points (Atlas as the target):

- L3 prompt: `defaults/prompts/classify.md`.
- L3 driver / input builder: `crates/atlas-engine/src/l3_classify.rs`
  (`build_llm_inputs`, line 264).
- L8 prompt: `defaults/prompts/subcarve.md`.
- L8 driver / input builder: `crates/atlas-engine/src/l8_recurse.rs`
  (`build_subcarve_inputs`, line 213).
- Backend rendering: `crates/atlas-llm/src/claude_code.rs::extract_tokens`
  (line 131) — the agentic backend has no input channel beyond
  `req.inputs`. The same `extract_tokens` is reused by
  `AnthropicHttpBackend` (`http_anthropic.rs:45`).

## Findings

### Backend infrastructure is already in place

The map/reduce task description treats `AnthropicHttpBackend` as a
deliverable, but it already lives in
`crates/atlas-llm/src/http_anthropic.rs` and is wired through
`BackendRouter` (`router.rs`) with provider/model selection driven by
`.atlas/config.yaml`. Switching a prompt's backend from `claude-code`
to `anthropic` is a single config-string change — no Rust diff.

This sharpens the spike: the question is purely about prompt
suitability, not infrastructure readiness.

### L3 — prompt is shape-compatible with the per-candidate map step

The classify prompt's input shape is exactly what a per-candidate map
call would carry:

- `dir_relative` — the candidate's own path.
- `rationale_bundle.manifests` — manifests at or near *this* candidate.
- `rationale_bundle.is_git_root` — local property.
- `rationale_bundle.doc_headings` — under *this* candidate only
  (`l3_classify.rs:103`).
- `rationale_bundle.shebangs` — likewise.
- `manifest_contents` — first 16 KB of each manifest at this candidate
  (`MANIFEST_SNIPPET_LIMIT`).

There is **no input field that references siblings, the parent
workspace, or other candidates**. The prompt body matches: it opens
with "you are classifying a single candidate directory" and never
asks the agent to look elsewhere. The catalogue of `kind` values
(`{{COMPONENT_KINDS}}`) and `lifecycle_roles`
(`{{LIFECYCLE_SCOPES}}`) are static vocabulary, not cross-component
context.

Crucially, the agentic backend's Read/Grep/Glob tools are *latent* on
this prompt: nothing in `classify.md` directs the model to read
files. Whatever cross-file signal a Claude Code agent might pick up
on its own is incidental — not part of the prompt's contract.
Switching to HTTP loses nothing in the contract.

A second-order consideration: in practice most Atlas L3 calls never
reach the LLM — `classify_deterministic` (`heuristics.rs:33`) covers
the manifest-bearing cases (Cargo workspaces, `[[bin]]`, `[lib]`,
`package.json`, `pyproject.toml`, bare-git). The LLM is consulted
only for ambiguous candidates *with no manifest*, which by
construction have no cross-candidate signal to lose.

**Verdict: GO without prompt changes.** L3 is already a per-candidate
prompt; the swap to HTTP is a backend-routing edit.

### L8 — naive narrowing does not work; redesign does

L8's input shape:

- `COMPONENT_ID`, `COMPONENT_KIND` — the parent component being
  considered.
- `COMPONENT_PATHS` — the *parent's* path segments (its outer
  directories), not the immediate sub-directories under consideration.
- `STRUCTURAL_SIGNALS` — `seam_density`, `modularity_hint`,
  `cliques_touching`, `current_depth`, `max_depth`. Three of those
  five (`seam_density`, `modularity_hint`, `cliques_touching`) are
  **inherently parent-scoped** — they describe partitions or
  edge-density across the parent's interior. They have no per-subdir
  analog.
- `EDGE_NEIGHBOURHOOD` — currently always supplied as the empty array
  (`l8_recurse.rs:249`). Reserved channel.
- `PIN_SUPPRESSED_CHILDREN` — user-pinned ids that must not appear as
  children.

Two structural issues block naive narrowing into a per-subdir map call:

1. **The output is a curated list, not a per-subdir verdict.** The
   prompt's contract is "return `sub_dirs`: which sub-directories to
   open up". A per-subdir map step produces N independent
   `is_component` answers; the curation (which to actually open) is
   the multi-subdir comparison the current prompt performs.

2. **The immediate-subdirectory listing is not in `req.inputs`.** The
   driver supplies `COMPONENT_PATHS` (the parent's outer dirs), not
   the children inside them. Today the agent enumerates them via tool
   use (Glob/Read on `COMPONENT_PATHS`). A non-agentic backend has no
   such option — the engine would have to enumerate them and ship
   them in the request.

3. **Three structural signals only make sense at the parent.**
   `seam_density`, `modularity_hint`, and `cliques_touching` describe
   the parent's interior partition and edge density. They cannot be
   meaningfully sliced per-subdir. Either each map call carries the
   parent's full signal payload (cheap but redundant) or the signals
   migrate to the deterministic reduce (correct but a behaviour
   change).

What works instead is a **redesign**: the map step asks the L3-style
question of each immediate sub-directory ("is this a component, given
it sits inside parent P of kind K?"), and the reduce is deterministic
Rust that:

- Collects per-subdir verdicts.
- Applies `modularity_hint` as a tie-breaker / prior on which subdirs
  to keep when the L3 verdict is ambiguous.
- Honours `PIN_SUPPRESSED_CHILDREN` by dropping matches.
- Emits `should_subcarve = (any verdict was true)` and `sub_dirs`
  from the surviving verdicts.

Net effect: the standalone `subcarve.md` prompt could be replaced by
a small variant of `classify.md` ("classify this subdirectory of
parent X"), and `subcarve_decision`'s LLM call disappears in favour
of N parallel `Classify` calls plus deterministic post-processing.
This is materially simpler than the current design and aligns with
the map/reduce task's own framing of "L8 subcarve becomes
map/reduce" — it just additionally implies that L8's own prompt
template ceases to exist as a separate thing.

**Verdict: GO via redesign — not via narrowing the existing prompt.**

## Recommendations

### L3 — go, no prompt changes

Action items, only what the map/reduce task already calls for:

- Route `PromptId::Classify` through the existing `AnthropicHttpBackend`
  via `.atlas/config.yaml` operation overrides.
- Verify `llm-cache.json` byte-identity for cache-hit re-runs (the
  `LlmFingerprint` differs per backend, so the first run with the new
  backend will be a full miss; subsequent runs should be free).

No template edits required. No structural changes required to
`build_llm_inputs`.

### L8 — go via redesign; revise the map/reduce task description

Action items to add to the map/reduce task before implementation:

1. **Engine-side change:** add an immediate-subdirectory enumeration
   step in `compute_decision` (`l8_recurse.rs:89`) — read the
   contents of each `COMPONENT_PATHS` entry, filter to directories,
   exclude pin-suppressed names. This is the deterministic
   pre-step.

2. **Prompt strategy:** introduce a `PromptId::ClassifySubdir`
   variant (or reuse `PromptId::Classify` with a parent-context
   addendum) whose `LlmRequest.inputs` carries:
   - The subdir's `dir_relative` (relative to repo root).
   - The same per-candidate rationale bundle the existing L3 builds.
   - A new `parent_context` field: `{ id, kind, modularity_partition_membership? }`.
     The membership flag lets the LLM see "this subdir is on the A
     side of the modularity partition" when the engine has a hint;
     it is omitted otherwise.

3. **Reduce step (deterministic Rust):**
   - Emit `should_subcarve = true` iff at least one subdir verdict
     is `is_boundary: true`.
   - `sub_dirs = [subdir for verdict if verdict.is_boundary]`.
   - When `modularity_hint` is present and verdicts disagree with
     it, log but do not override — verdicts win, the hint is a prior.
   - Drop pin-suppressed names defensively (they should already be
     filtered pre-map).

4. **Decommission the standalone subcarve prompt** once the
   map/reduce path lands. Keep `subcarve.md` in the corpus during
   the transition for cache-fingerprint reasons (its `template_sha`
   contributes to the run-wide fingerprint), then delete it after
   one shipped release.

5. **Quality bar:** for the dev-workspace and synthetic-second
   targets, the new `sub_dirs` set must equal the pre-conversion set
   modulo deterministic ordering. Treat any deviation as a behaviour
   change requiring golden updates and a written justification.

These items expand the map/reduce task description without changing
its exit criteria.

## Bug discovered and fixed during the spike

### Symptom

`prompt::render` errors with `unknown token `{{COMPONENT_KINDS}}` in
template` whenever L3 escalates to the LLM. `classify_via_llm`
swallows the error into `unknown_classification("LLM call failed:
template syntax error: …")` and returns `is_boundary: false`,
indistinguishable from a normal `non-component` verdict.

### Root cause

`classify.md` references `{{COMPONENT_KINDS}}` and
`{{LIFECYCLE_SCOPES}}` for the kind catalogue and lifecycle-scope
catalogue blocks the model picks from. L5 and L6 inject their
analogous vocab tokens in their `build_inputs` builders
(`l5_surface.rs:127` → `CATALOG_COMPONENTS`, `l6_edges.rs:103` →
`ONTOLOGY_KINDS`). L3 forgot to. The helpers
`render_kinds_for_prompt` and `render_lifecycle_scopes_for_prompt`
were exported from `atlas-engine` but never called outside drift
tests.

The bug stayed hidden because:

- `classify_deterministic` (`heuristics.rs:33`) handles the common
  manifest-bearing cases (Cargo workspace/lib/bin, package.json,
  pyproject.toml, bare-git-no-readme) — a high enough hit rate that
  the LLM-fallback path rarely fired in practice.
- L3 unit tests use `TestBackend`, which keys on the raw `LlmRequest.inputs`
  JSON and never invokes `prompt::render`.
- L8's `subcarve.md` has no vocab tokens, so its render path works
  fine — there was no "L8 broke too" symptom to draw attention to
  the asymmetry.

### Fix

`l3_classify.rs::build_llm_inputs` now mirrors the L5/L6 pattern:
parse `defaults/component-kinds.yaml`, render the kinds and
lifecycle-scope blocks, and add `COMPONENT_KINDS` and
`LIFECYCLE_SCOPES` keys to the JSON returned to `LlmRequest.inputs`.
A new regression test `classify_prompt_renders_with_build_llm_inputs`
calls `prompt::render` against the real template with the real
inputs builder and panics on any unknown-token error — so the next
prompt vocabulary divergence fails the build instead of degrading
silently.

The integration test `l3_ambiguous_candidate_calls_llm_fallback` was
updated to add the same two keys to its canned-input JSON, since
TestBackend requires byte-identical inputs to match a canned response
(memory: `testbackend-requires-byte-identical-llmrequest-inputs-to-match-cache`).

### Implication for the spike's recommendation

Unchanged. The bug affects both backends identically — neither
agentic Claude Code nor Anthropic HTTP can render a template with
unsupplied tokens. The structural analysis stands: L3's prompt is
shape-compatible with the per-candidate map step, and the swap to
HTTP is safe.

## Live validation

### L3 — bug-fix mechanically proven; pipeline-level smoke test was inconclusive

The new `classify_prompt_renders_with_build_llm_inputs` regression
test exercises exactly the path the bug broke: render `classify.md`
with the inputs `build_llm_inputs` produces. The test passes
post-fix and would fail without it, so the fix is mechanically
verified.

A pipeline-level smoke test (`atlas index` against a tempdir of
markdown-only directories with a `.git` marker, configured to use
`claude-code/claude-sonnet-4-6`) ran to completion in 3:11 wall, but
fired **0 LLM calls** despite generating 5 classify-stage operations:
the bare-git-no-manifests rule plus deduplication of equivalent
candidates short-circuited every candidate at the deterministic
layer. Forcing the LLM-fallback path through the full pipeline
would require hand-authored override `additions` or a fixture with
specifically chosen non-manifest signals (a README at root,
shebangs in scripts, etc., without any of the recognised manifest
shapes) — out of scope for this spike. The regression test covers
the failure mode that mattered.

### L8 — the existing prompt is conservative enough that the synthetic test produced no subcarve

A synthetic single-crate Rust library was constructed under
`/tmp/atlas-spike-l8/`: `Cargo.toml` with `[package]` + `[lib]`
(no `[workspace]`, no `[[bin]]`), `src/lib.rs` re-exporting four
modules, and `src/{agent,discover,state,survey}/mod.rs` mirroring
the responsibility-line split of Ravel-Lite's top-level package.
This is the closest available approximation to a "library with a
clear sub-carve case" — Atlas's own three crates are flat
`src/*.rs` layouts, and every other Rust project under
`~/Development/` is either a workspace (Workspace classification
absorbs the top level), a CLI (policy-stop), or a single-crate
library with no internal sub-directory structure.

Running `atlas index` against this target produced:

- 1 component classified deterministically as `rust-library` via the
  `cargo-lib` rule (`Cargo.toml:[lib]`, no `[[bin]]`).
- 1 LLM call total — `Stage1Surface`, which produced a coherent
  surface record (purpose paragraph, `consumes_files`, role hints,
  cross-component mentions) cached at `llm-cache.json`.
- `subcarve=1` in the stage breakdown but no `Subcarve` entry in
  `llm-cache.json`.
- Final tree: 1 root component, no children, `iterations=0`.

The implication: either the subcarve LLM call returned
`should_subcarve: false` (and was filtered before cache write — the
cache only persists *successful* responses that influence outputs),
or the call errored and was swallowed into a `stopped` decision.
Either way, the existing L8 prompt **did not propose carving the
four obvious sub-modules** even though the structural setup is the
canonical example the prompt body itself names ("a Rust library with
`src/auth/` and `src/billing/` directories that touch disjoint parts
of the code"). This is consistent with the spike's structural
verdict: today's L8 leans on tool-discovered evidence beyond the
structured inputs, and on a cold synthetic tree with stub function
bodies the agent reasonably found no compelling reason to recurse.

### What this evidence supports

- **The L3 fix is correct and necessary.** Without it, every
  ambiguous L3 candidate was silently downgraded to "unknown" — a
  bug that the map/reduce task would have inherited had the spike
  not surfaced it.
- **The L8 prompt is brittle on minimal synthetic content.** A
  redesigned per-subdir map step (Classify-style on each subdir)
  would have asked four independent "is this a component?"
  questions, each with the *subdir's* doc-headings/shebangs/etc.,
  and would not depend on the agent inferring sub-component
  intent from stub `mod.rs` bodies. This is additional structural
  evidence for the L8-redesign recommendation.

### What this evidence does not establish

- It does not directly compare today's L8 verdict to a per-subdir
  map step's verdict on a real-world target with rich sub-module
  content. That comparison is the map/reduce task's own exit
  criterion ("`kind_accuracy` against the dev-workspace golden is
  within 2pp of the pre-conversion baseline") and the right place
  to gather it.
- It does not validate the HTTP backend transport itself, only the
  prompt suitability. `AnthropicHttpBackend` has its own unit test
  coverage (`http_anthropic.rs::tests`); its production behaviour
  is a separate operational concern.

## Decision summary

| Prompt | Verdict | Action |
|---|---|---|
| L3 `is_component` | GO — vocab-injection bug fixed in this spike; no further prompt changes needed | Backend swap via `.atlas/config.yaml`. The vocab fix lands as part of this spike. |
| L8 `subcarve` | GO via redesign | Replace per-component agent call with engine-enumerated subdirs + N parallel `Classify`-style map calls + deterministic reduce. Revise the map/reduce task description with the items above. |

The map/reduce task can be promoted from MEDIUM to HIGH priority on
the L3 swap alone. The L8 work is larger than the task description
implied — but cleaner, since it eliminates a prompt rather than
adding one.
