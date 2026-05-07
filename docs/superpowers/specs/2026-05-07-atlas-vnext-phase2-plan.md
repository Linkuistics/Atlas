# Atlas vNext Phase 2 — Implementation Plan

**Status:** Plan (forward-looking; Phase 2 of the Atlas vNext system-model
redesign). Companion to `2026-05-06-atlas-system-model-design.md`. Sequel to
`2026-05-06-atlas-vnext-phase1-plan.md` (Phase 1 closed; status in
`docs/superpowers/plans/2026-05-06-phase1-status.md`).
**Date:** 2026-05-07.
**Treatment:** Greenfield, carried forward from Phase 1. No on-disk format
compatibility with Phase 1 outputs. No migration command. A user upgrading
deletes `.atlas/` and re-runs. Schema number stays at `1` across the entire
phase — no users exist yet, so version-bump ceremony is unnecessary; the v1
*shape* mutates freely as each language analyser lands.

**Scope:** Decomposes Phase 2 (§10.2 of the design spec, reshaped to fit
dull/'s polyglot reality plus Linkuistics' Lisp dialects) into an ordered
sequence of independently-mergeable PRs. Phase 2's deliverables: subprocess
analyser transport, seven non-Rust language analysers, one deploy-format
analyser (Compose), a shell-script LLM-fallback analyser, plus paying off
Phase 1's analyser-shape hangover. Phase 2 still ships as a one-shot CLI;
server mode is Phase 4.

The "polyglot dull-shaped fixture analysed end-to-end" outcome is the
acceptance smoke test (PR-14). Five non-Rust languages ship in the broad
parallel-wave middle of the phase; subprocess transport is established
ahead of them by Python, the abstraction-confirmation language whose
no-`pub`/`priv` shape forces the protocol to confront genuinely-non-Rust
binding semantics.

---

## 0. Reading order

Before this plan, read:

1. `2026-05-06-atlas-system-model-design.md` §0, §3, §4, §5, §6, §8, §10.2
   (Phase 2 scope), §11.2 (open questions; Phase 2 absorbs §11.2.1 by
   extending the v1 shape rather than bumping schema_version), §12 (the
   Phase 1 schema-churn risk row's "Phase 2 first non-Rust analyser =
   abstraction-confirmation milestone" — the milestone in this plan is
   PR-3 (Python), not PR-1 (TS/JS), per Q5 of the brainstorm).
2. `2026-05-06-atlas-vnext-phase1-plan.md` §3 (v1 mechanisms — many remain
   reusable as Phase 2 starting points), §7 (Phase 1's out-of-scope list,
   the Phase 2 backlog).
3. `2026-05-06-phase1-status.md` per-PR notes — especially PR-1 (analyser
   schema and the new types Phase 2 extends), PR-5 (registry +
   analyzer_registry_sha + LlmHook design), PR-7 (rust_surface_analyzer
   regex limitation that PR-5 of this plan replaces), PR-12 (multi-root
   cross-tree fixes Phase 2 will exercise harder).
4. Memory entries `feedback_phase1_open_questions`,
   `feedback_toml_parsing`, `tombstone_emit_once_design`,
   `all_components_not_salsa_tracked`, `feedback_fix_all_lints`.

This plan does *not* re-derive the architecture; it operationalises Phase
2's slice of it. Where this plan and the design spec disagree, the design
spec wins; where the spec is silent on sequencing, this plan is canonical.

---

## 1. Phase 2 deliverable, restated

End of Phase 2, an Atlas user running `atlas index .` from a polyglot
workspace (e.g. dull/, which contains C#, Dart/Flutter, TypeScript,
Python, Rust, Compose, and Buildkite-named Dockerfiles) shall see:

- Every component classified to its language (`rust`, `typescript`,
  `javascript`, `python`, `csharp`, `dart`, `flutter`, `elixir`,
  `racket`, `lispkit`).
- Each component's per-component `surfaces.yaml` populated with binding
  records, library_apis, and where applicable contracts_defined /
  contracts_consumed. `schema_version` stays integer `1`; the v1 shape
  is mutated freely by the Phase 2 PRs.
- `related-components.yaml` carrying `bundled-into` and `deployed-with`
  edges from `docker-compose*.yml` files alongside Phase 1's
  Dockerfile-derived composition edges.
- Six of the seven non-Rust language surface analysers running as
  subprocesses (Python, C#, Dart/Flutter, Elixir, Racket, LispKit).
  TypeScript/JavaScript ships in-process in Phase 2; its migration to
  subprocess is deferred to Phase 3 alongside the Rust surface, Cargo
  classifier, Dockerfile classifier, and LLM-classify sweep that
  fulfils the "single mechanism" end-state.
- Phase 1 PR-7's regex-based Rust binding extractor replaced with `syn`
  (PR-5 of this plan). `nested_pub_inside_pub_mod_is_phase1_known_limitation`
  closes.

Out of scope, deferred to later phases: server mode; drift / impact /
modularity reports; migration of existing in-process analysers to
subprocess; bidirectional LLM callback channel for subprocess analysers;
`--strict-overrides`; LLM threshold calibration; contract rename-match;
k8s and Helm analysers (dropped — neither dull/ nor Linkuistics use them).

---

## 2. Open-question pre-conditions

Phase 1 resolved §11.2.2 and §11.2.3 (companion specs landed alongside
Phase 1 PR-0). Phase 2 absorbs the remaining schema-related open question
without a separate spec PR.

### 2.1 §11.2.1 — Surface schema for non-Rust languages (RESOLVED IN-PHASE)

**Phase 2 resolution:** The v1 shape is the canonical schema across
Phase 2. There is no `schema_version: 2`. Each language analyser PR
extends `atlas_index::surfaces` (in atlas-contracts) with the variants and
fields its language requires; the in-tree readers stay coherent at every
commit because `cargo test --workspace` exercises every consumer of the
schema. Greenfield + no-users-yet means schema mutation is a free
operation: when a PR changes a struct's shape, every fixture that breaks
gets regenerated in the same PR.

The v1 shape adjustments expected during Phase 2 (specified per-PR in §4):

- A `Visibility` enum with `Explicit { keyword: String }` (Rust `pub`,
  C# `public`, TS `export`) and `Conventional` (Python no-leading-`_`,
  Dart no-leading-`_`, Racket `provide`, Elixir `def` vs `defp`)
  variants. Lands in PR-3 (Python forces it).
- An `attributes: BTreeMap<String, serde_yaml::Value>` field on
  `Binding` to absorb language-specific decorations (C# `[Attribute]`,
  Python decorators, Dart `@annotations`, Elixir `@spec`). Lands in
  PR-3 or PR-6, whichever needs it first.
- A `module_path: Vec<String>` field on `Binding` to disambiguate same-
  named symbols in different modules (Python's `pkg.mod.fn`, C#
  `Namespace.Class.Method`). Lands in PR-3.
- A new `ContractKind::Behaviour` variant for Elixir's `behaviour`
  protocol pattern. Lands in PR-8.

Other §11.2 open questions remain Phase 3+ deferrals (§11.2.4 contract
rename-match, §11.2.5 server auth, §11.2.6 LLM threshold calibration,
§11.2.7 cache compression, §11.2.8 worktree consistency).

---

## 3. Phase 1 mechanisms reused as starting points

These Phase 1 mechanisms are extended rather than rewritten. They are
*starting points*, not compatibility constraints — under greenfield
treatment, any Phase 2 PR is free to refactor them when the new code
demands it. Listing them avoids duplicate work and makes the "where to
look" question cheap.

| Mechanism | Location | How Phase 2 uses it |
|---|---|---|
| Analyser trait + registry + dispatcher | `crates/atlas-analyzers/src/{lib,registry,dispatcher}.rs` | Extended in PR-2: registry learns to register and dispatch subprocess analysers via the same `Analyzer` trait. Every Phase 2 analyser plugs in here. |
| `FingerprintBuilder` | `crates/atlas-engine/src/cache/fingerprint.rs` | Extended in PR-2: new `add_analyzer_binary_sha` method (tag `0x06`) so subprocess analyser binaries contribute to L-stage fingerprints. |
| `PersistentCache` | `crates/atlas-engine/src/cache/{mod,layout,fingerprint}.rs` | Unchanged. Subprocess analyser outputs cache identically to in-process outputs (the cache stores the JSON-serialised `StageOutput` blob). |
| `SurfacesFile` / `Contract` / `Binding` / `LibraryApi` | `atlas-contracts/crates/atlas-index/src/surfaces.rs` | Mutated freely each PR (no schema bump). PR-3 introduces `Visibility::Conventional`; PR-3 or PR-6 introduces `attributes`; PR-3 introduces `module_path`; PR-8 introduces `ContractKind::Behaviour`. |
| Multi-root `Workspace` + path-dep expansion | `crates/atlas-engine/src/{db,root_expansion,manifest_parse}.rs` | Engine-side fixed-point walker is unchanged. Each language analyser PR extends `manifest_parse.rs` with its language's path-dep recognition (`pubspec.yaml dependencies: ... path:`, `package.json dependencies: ... file:`, `mix.exs ... path:`). |
| `extract_rust_surface` regex byte-walker | `crates/atlas-analyzers/src/rust_surface_analyzer.rs` | Replaced wholesale in PR-5 with a `syn`-based AST walker. The brace-counting state machine is deleted under greenfield. |
| Scattered per-component `.atlas/` writers | `crates/atlas-engine/src/l9_projections.rs` + `crates/atlas-cli/src/pipeline.rs` | Unchanged. PR-4 (per-analyser id/version plumbing) feeds these writers honest analyser identity instead of the `l3-driver` placeholder. |
| Contract participant validator | `atlas-contracts/crates/component-ontology/src/lib.rs` | Unchanged. Each new language analyser's contracts pass through the same validator. |
| Composition edges from Dockerfiles | `crates/atlas-engine/src/l6_edges.rs` + `crates/atlas-analyzers/src/dockerfile_classifier.rs` | Unchanged in shape. PR-11 adds Compose composition edges via the same edge-emission path; PR-14 verifies `Dockerfile.*.buildkite` files (dull's CI naming) flow through PR-9's classifier without modification. |
| L6 participant-surface fingerprint | `crates/atlas-engine/src/l6_edges.rs` | Unchanged. Each Phase 2 language analyser's surface fingerprint contributes to L6 invalidation just like Rust surfaces did in Phase 1. |

The Phase 1 status note for PR-7 documenting `nested_pub_inside_pub_mod_is_phase1_known_limitation`
is closed by PR-5. The Phase 1 status note for PR-12 documenting L8 phantom
subcomponents and three polish items is closed by PR-13.

---

## 4. PR sequence

PRs are numbered in dependency order. Sizes are estimates excluding tests
and excluding generated code. Each PR ends with passing
`cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
and `cargo fmt --check`.

### PR-0 — Plan + status + verification of continuation-prompt re-use (no code)

**Intent:** Land this plan and the Phase 2 status file; verify the
existing `docs/superpowers/prompts/2026-05-07-vnext-continue.md` correctly
detects "Phase 2 plan exists → enter Step 3 execution" via the wildcard
match (`*phase2-plan*`). No new prompt file required; the existing one is
already idempotent across phases.

**Files:**
- Create: `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md` (this file).
- Create: `docs/superpowers/plans/2026-05-07-phase2-status.md` (PR checklist + dependency graph + per-PR notes section).

**Acceptance criteria:**
- Both documents land in their respective directories.
- The status file contains 15 PR checkboxes (PR-0 through PR-14), all `[ ]` except PR-0 which is `[x]` after this commit.
- The continuation prompt's Step 1 wildcard (`*phase2-plan*`) matches the new plan filename (verified by re-reading the prompt).

**LOC:** 0 code, ~700-900 lines of plan + ~80-150 lines of status.

---

### PR-1 — TypeScript / JavaScript surface analyser (in-process)

**Intent:** First non-Rust analyser. Lands in-process to validate the
analyser-shape independently of subprocess transport (PR-2). TS and JS
share one analyser; module-system detection (CommonJS, ESM, declaration
files) is internal.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `ts_js_surface_analyzer.rs` — `TsJsSurfaceAnalyzer` (registered via `AnalyzerRegistry::builtin()`); `TsJsSurfaceOutput { contracts, bindings, library_apis }`; `extract_ts_js_surface(component_id, source_files)` deterministic extractor.
- Modify: `cargo_classifier.rs` (or a sibling new file) — extend L3 deterministic classification to detect `package.json` + `tsconfig.json` and emit `kind: typescript-package` / `kind: javascript-package` accordingly. (Cargo classifier becomes a sibling of TS-classifier; both are deterministic at L3.)
- Modify: `lib.rs` — re-export new types.
- Modify: `registry.rs` — `builtin()` registers the TS/JS analyser at L5.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `typescript-package`, `javascript-package`. (Atlas-contracts side commit.)

**Parser library:** `swc_ecma_parser` + `swc_ecma_ast` (mature, fast,
TS+JSX support; pure Rust). Add to `crates/atlas-analyzers/Cargo.toml`.

**Binding extraction shape:**
- For TS: `export` declarations (named, default, type-only) → `Binding` with `Visibility::Explicit { keyword: "export" }`. Type aliases and interfaces → `LibraryApi` entries.
- For JS: same as TS but type-only fields are `None`. Module system inferred (CommonJS `module.exports` vs ESM `export`); both emit `Binding` records, with `attributes: { "module_system": "commonjs" | "esm" }`.
- `package.json#main` / `module` / `exports` resolve to a `LibraryApi` whose `entrypoint` is the resolved path.

**Acceptance criteria:**
- New unit test: `ts_extracts_named_exports` — `export function foo() {}` and `export class Bar {}` produce two `Binding` records.
- New unit test: `ts_extracts_type_only_export` — `export type Foo = string` produces a `Binding` with the type-only attribute.
- New unit test: `js_extracts_commonjs_exports` — `module.exports = { foo, bar }` produces two `Binding` records.
- New integration test: a `package.json` + `tsconfig.json` + `src/index.ts` fixture is classified `typescript-package` at L3 with no LLM call, and its surfaces.yaml lists every exported symbol.
- The analyser's `analyse` returns `Confident` outputs for valid inputs; malformed source is `Error`.

**LOC:** ~1100-1500 (analyser + tests + fixture).

---

### PR-2 — Subprocess analyser transport

**Intent:** Establish the stdio-JSON wire protocol for out-of-process
analysers. The existing `Analyzer` trait stays as the unified interface;
subprocess analysers are wrapped in a `SubprocessAnalyzerProxy` that
implements `Analyzer` by speaking JSON to a child process. PR-2 wires the
infrastructure; PR-3 is the first consumer.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `subprocess/mod.rs` — `SubprocessAnalyzerProxy { id, version, stage, cost_class, binary_path, binary_sha, applicability }`; implements `Analyzer`.
- Create: `subprocess/transport.rs` — length-prefixed JSON framing (4-byte big-endian u32 length, then UTF-8 JSON bytes). `read_frame` / `write_frame` helpers over `std::process::ChildStdin/Stdout`.
- Create: `subprocess/handshake.rs` — capability negotiation. On spawn, the analyser process emits a `Capabilities` envelope (`{ id, version, stage, cost_class, applicability_predicate }`); the parent verifies it matches the registered spec.
- Create: `subprocess/process_pool.rs` — per-analyser process pool. Lifetime: spawned on first dispatch, killed at pipeline shutdown. Pool size: 1 process per analyser (Phase 2; concurrent dispatch is Phase 3+).
- Create: `subprocess/wire_types.rs` — `Request { kind: "applies" | "fingerprint_inputs" | "analyse", target, context }`; `Response { kind: "confident" | "graded" | "declines" | "error", payload }`.
- Modify: `registry.rs` — `register_subprocess(SubprocessAnalyzerSpec)` constructs and stores a `SubprocessAnalyzerProxy`; `dispatch` and `dispatch_with_filter` are unchanged at the `Analyzer` trait level (subprocess proxies are dispatched identically).
- Modify: `lib.rs` — re-export subprocess types.

**Files (in `crates/atlas-engine/src/cache/fingerprint.rs`):**
- Modify: `FingerprintBuilder::add_analyzer_binary_sha(&Sha256Hex)` — new tag byte `0x06`. Subprocess analysers contribute their binary content sha (computed once at registry construction, cached).

**Files (in `atlas-contracts/crates/atlas-index/src/analyzers.rs`):**
- Modify: `Transport::Subprocess { binary_path, binary_sha }` — confirm the existing `SubprocessConfig` shape (PR-1 of Phase 1 already defined `Transport`). The `binary_sha` field becomes load-bearing here.

**Subprocess error model:**
- Subprocess exits non-zero → `AnalyzerError::CallFailed { exit_code, stderr_tail }`.
- Subprocess hangs > 60 seconds (configurable per-analyser, default 60s) → `AnalyzerError::CallFailed { reason: "timeout" }`.
- Subprocess emits JSON the parent can't parse → `AnalyzerError::MalformedInput { ... }`.
- Pipeline shutdown sends SIGTERM, then SIGKILL after 5s grace.
- A subprocess that fails or times out does NOT poison the registry; later dispatches respawn it.

**Subprocess LLM access:** out of scope for Phase 2. Subprocess analysers
must not require LLM access. The shell-script LLM-fallback (PR-12) stays
in-process. If a Phase 3+ subprocess analyser needs LLM access, the
bidirectional callback channel is designed then.

**Acceptance criteria:**
- New unit test: `transport_round_trip` — write a frame, read it back, assert byte equality.
- New unit test: `handshake_rejects_mismatched_capabilities` — capabilities envelope declaring `stage: l3` against a spec that declared `stage: l5` errors with a clear message.
- New unit test: `subprocess_crash_returns_call_failed` — a fixture binary that exits 1 produces `AnalyzerError::CallFailed`.
- New unit test: `subprocess_timeout_returns_call_failed` — a fixture binary that sleeps > timeout produces `AnalyzerError::CallFailed { reason: "timeout" }`.
- New unit test: `binary_sha_change_invalidates_cache` — replacing the analyser binary changes the L-stage fingerprint.
- A reference echo-binary fixture lives under `crates/atlas-analyzers/tests/fixtures/echo_subprocess/` to back the integration tests.

**LOC:** ~1400-1800 (transport + handshake + pool + tests + reference fixture).

---

### PR-3 — Python surface analyser (first subprocess analyser)

**Intent:** First consumer of PR-2's subprocess transport. Python's
no-`pub`/`priv` shape forces `SurfacesFile` / `Binding` / `LibraryApi` to
absorb a `Visibility::Conventional` variant — this is the
abstraction-confirmation milestone (design §12).

**New crate (workspace member):**
- Create: `crates/analyzers/python/Cargo.toml`, `src/main.rs` (subprocess entry point implementing the wire protocol), `src/lib.rs` (analyser logic).

**Parser library:** `rustpython-parser` (pure Rust, no external Python
dependency on the user's box). Subagent has discretion to swap to a
different mature pure-Rust Python parser if rustpython-parser proves
inadequate during implementation.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `python_surface_analyzer.rs` — registers a `SubprocessAnalyzerSpec` against the `python-analyzer` binary; wraps it as a `SubprocessAnalyzerProxy` at `AnalyzerRegistry::builtin()`.
- Modify: `registry.rs` — `builtin()` registers the python subprocess analyser at L5.
- Modify: `cargo_classifier.rs` (or sibling) — extend L3 deterministic classification: `pyproject.toml` / `setup.py` / `requirements.txt` → `kind: python-package`.

**Files (in `atlas-contracts/crates/atlas-index/src/surfaces.rs`):**
- Modify: `Visibility` enum gains `Conventional`; existing code that emitted Rust `pub` migrates to `Visibility::Explicit { keyword: "pub".into() }`. PR-5 (rust binding extractor → syn) re-emits Rust bindings under the new variant.
- Modify: `Binding` gains `module_path: Vec<String>` (Python `pkg.mod.fn` and later C# / Elixir need this).
- Modify: `Binding` gains `attributes: BTreeMap<String, serde_yaml::Value>` (Python decorators, later C# attributes etc.).

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `python-package`.

**Binding extraction shape:**
- Top-level `def foo(...)` and `class Bar(...)` → `Binding` with `Visibility::Conventional` (no leading `_`) or `Visibility::Conventional` with attribute `private: true` (leading `_`).
- Decorators captured as `attributes`: `{"decorator_chain": ["dataclass", "frozen"]}`.
- `module_path` derived from filename relative to the package root: `pkg/sub/mod.py` → `["pkg", "sub", "mod"]`.
- `pyproject.toml#project.name` + `[tool.poetry.dependencies]` / `[project.dependencies]` → path-dep recognition for Python.

**Acceptance criteria:**
- New integration test: a `pyproject.toml` + `pkg/__init__.py` + `pkg/mod.py` fixture is classified `python-package` at L3 with no LLM call, and its surfaces.yaml lists `pkg.mod.foo` and `pkg.mod.Bar` as bindings.
- New integration test: a Python file with `def _private()` and `def public()` produces two bindings, distinguished by the conventional-private attribute.
- New integration test: a `@dataclass` decorator on a class produces a binding whose `attributes.decorator_chain` includes `dataclass`.
- New persistent-cache test: re-running with the same binary produces a cache hit; touching the python-analyzer binary content invalidates the L5 cache.
- New cross-tree test (echoing PR-12 of Phase 1): a Python component path-dep'd via `pyproject.toml`'s `[tool.poetry.dependencies]` from another root contributes its surface to the consumer's L6 cache key.

**LOC:** ~1200-1600 (analyser binary + wrapper + schema mutation + tests + fixture).

---

### PR-4 — Per-analyser id/version plumbing through L3 dispatch

**Intent:** Replace the `l3-driver` / `L3_DRIVER_VERSION` placeholders in
per-component file metadata with the dispatching analyser's actual id and
version. Required before the parallel wave (PR-6 to PR-12) so each new
analyser's identity propagates correctly into `<component>/.atlas/component.yaml`.

**Files (in `crates/atlas-analyzers/src/`):**
- Modify: `dispatcher.rs` — `dispatch` and `dispatch_with_filter` return `(DispatchOutcome, &str /* analyser_id */, &str /* analyser_version */)`. The dispatcher tracks which analyser produced the winning outcome; for `AllDeclined` the returned id/version is `("none", "0.0.0")`.
- Modify: `registry.rs` — `iter_dispatch_order` already exposes ordering; the dispatcher reads id/version from the winning analyser ref.

**Files (in `crates/atlas-engine/src/`):**
- Modify: `l3_classify.rs` — capture the analyser id/version returned by the dispatcher into a new field on the L3 result.
- Modify: `l9_projections.rs` — `per_component_yaml_snapshot` populates `analyser_id` and `analyser_version` from the captured values; the `l3-driver` placeholder is deleted.

**Files (in `atlas-contracts/crates/atlas-index/src/per_component.rs`):**
- Modify: docstring on `analyser_id` / `analyser_version` clarifies they reflect the L3 analyser that classified the component (e.g. `cargo-toml-classifier`, `dockerfile-l3`, `python-surface-analyzer`).

**Acceptance criteria:**
- New unit test: `dispatch_returns_winning_analyser_identity` — given Cargo and LLM both apply at L3, the dispatcher returns `("cargo-toml-classifier", "<version>")`.
- New unit test: `dispatch_all_declined_returns_none_identity` — when every analyser declines, the dispatcher returns `("none", "0.0.0")`.
- New integration test: a Cargo crate's `<component>/.atlas/component.yaml` lists `analyser_id: cargo-toml-classifier`, not `l3-driver`.
- A Dockerfile-classified component's `analyser_id: dockerfile-l3`.
- A Python-classified component's `analyser_id: python-surface-analyzer` (verifies subprocess analysers contribute identity correctly).

**LOC:** ~400-600.

---

### PR-5 — Rust binding extractor: regex → `syn`

**Intent:** Replace PR-7-of-Phase-1's regex byte-walker
(`extract_rust_surface`) with a `syn`-based AST walker. Closes the
`nested_pub_inside_pub_mod_is_phase1_known_limitation`. PR-7-of-Phase-1's
1266-line analyser shrinks substantially because syn does the heavy
lifting.

**Parser library:** `syn` crate (with `full` feature for parsing whole
files, not just tokens). Pinned version recorded in
`crates/atlas-analyzers/Cargo.toml`.

**Files (in `crates/atlas-analyzers/src/`):**
- Modify: `rust_surface_analyzer.rs` — `extract_rust_surface` rewritten around `syn::parse_file`. Walks `syn::File::items` recursively; for each `syn::Item::*` with `Visibility::Public`, emits a `Binding` with span computed from `proc_macro2::Span` line/column → byte range.
- Delete: the byte-by-byte state-machine walker (string literals, char literals, comments, raw-string prefixes, brace nesting). Greenfield delete; no-users-yet means span backwards-compat is not required.
- Preserve: span semantics from spec §2.1 PR-7 (block items: `pub` start to `}` end; statement items: `pub` start to `;` end).
- Preserve: `RUST_SURFACE_ANALYZER_ID` and `_VERSION` constants (version bumps to `2.0.0` to mark the breaking change).
- Preserve: the `extract_rust_surface(component_id, lib_rs_bytes, main_rs_bytes, lib_rs_relpath, main_rs_relpath)` signature so PR-7-of-Phase-1's callers don't churn.

**Files (in `crates/atlas-engine/src/contract_canonicalisation.rs`):**
- Unchanged. The `code_derived_content_sha(bytes, span)` function takes a span produced by either implementation; new spans from `syn` produce different content shas than the regex extractor produced for the same files (greenfield: regenerate fixtures).

**Acceptance criteria:**
- New unit test: `syn_extracts_nested_pub_inside_pub_mod` — `pub mod foo { pub struct Bar; }` produces a binding for `Bar` with span inside the module. The Phase 1 regex extractor missed this; the test asserts the new behaviour.
- All 23 existing tests in `rust_surface_analyzer.rs` continue to pass (with fixture regeneration where span byte ranges shifted).
- Integration test fixture in `surfaces_emission_rust.rs` regenerated; PR-7-of-Phase-1's six tests stay green.
- `nested_pub_inside_pub_mod_is_phase1_known_limitation` memory note removed (or updated to `closed_in_phase2_pr5`).

**LOC:** ~600-900 (vs Phase 1's ~1266 — significant reduction).

---

### PR-6 — C# surface analyser (subprocess)

**Intent:** Wave-2 language analyser. C# brings attribute-as-metadata,
namespace-not-file-based modules, partial classes, and sealed types — a
genuine stress test for the `attributes` and `module_path` fields PR-3
introduced.

**New crate:**
- Create: `crates/analyzers/csharp/Cargo.toml`, `src/main.rs` (subprocess entry point), `src/lib.rs`.

**Parser library:** `tree-sitter-c-sharp` via the `tree-sitter` Rust
binding. Subagent has discretion to swap to a hand-rolled C# parser or
shell out to `roslyn` (heavyweight; requires .NET) if tree-sitter proves
inadequate.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `csharp_surface_analyzer.rs` — registers the subprocess analyser.
- Modify: deterministic L3 classifier — `*.csproj` / `*.sln` → `kind: csharp-project` / `kind: csharp-solution`.
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `csharp-project`, `csharp-solution`.

**Binding shape:**
- Top-level `public class/struct/interface/record/enum` → `Binding`.
- `public` methods on public types → `Binding` with `module_path` rooted at the class namespace.
- C# `internal` members are excluded from surface (Phase 2 default; an `attributes.internal_visible: true` opt-in is Phase 3+).
- C# attributes (`[Authorize]`, `[Serializable]`, etc.) captured as `attributes.cs_attributes: ["Authorize", "Serializable"]`.
- `*.csproj` `<PackageReference>` / `<ProjectReference>` → path-dep recognition.

**Acceptance criteria:**
- New integration test: a `*.csproj` + `Program.cs` + `Models/User.cs` fixture is classified `csharp-project`; surfaces.yaml lists `Program` and `Models.User` as bindings with module_path.
- New integration test: a `[Serializable]` attribute on a class produces `attributes.cs_attributes: ["Serializable"]`.
- New integration test: `internal class Foo` is NOT in the surface; `public class Foo` is.
- New integration test: a `*.sln` referencing two `*.csproj` files results in two components, with both surfaces analysed.

**LOC:** ~1500-2000.

---

### PR-7 — Dart / Flutter surface analyser (subprocess)

**Intent:** Wave-2 language analyser. Dart's `pubspec.yaml` shape is
manifest-driven like Cargo and pyproject; Flutter is identified by a
`flutter:` block in pubspec. Path-deps via `pubspec.yaml`'s
`dependencies: ... path:` form exercise multi-root cross-tree behaviour
(echoing Phase 1 PR-12's multi-root pattern).

**New crate:**
- Create: `crates/analyzers/dart/Cargo.toml`, `src/main.rs`, `src/lib.rs`.

**Parser library:** `tree-sitter-dart`. Fallback: hand-rolled lexer (Dart
syntax is regular enough; `class`, `mixin`, `extension`, `typedef`).

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `dart_surface_analyzer.rs`.
- Modify: deterministic L3 classifier — `pubspec.yaml` → `kind: dart-package` (no `flutter:` block) or `kind: flutter-package` (has `flutter:` block).
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `dart-package`, `flutter-package`.

**Files (in `crates/atlas-engine/src/manifest_parse.rs`):**
- Modify: `extract_path_deps` learns to read `pubspec.yaml` `dependencies: { foo: { path: "../foo" } }` form.

**Binding shape:**
- Top-level public functions, classes, mixins, extensions, typedefs → `Binding` with `Visibility::Conventional` (Dart's leading-underscore convention).
- `@deprecated`, `@override`, `@protected` annotations captured as `attributes.dart_annotations`.
- Library API derived from `pubspec.yaml#name` + the public exports of `lib/<name>.dart` (Dart's library-entrypoint convention).

**Acceptance criteria:**
- New integration test: a `pubspec.yaml` + `lib/dart_pkg.dart` fixture without `flutter:` block is classified `dart-package`.
- New integration test: a `pubspec.yaml` with `flutter:` block is classified `flutter-package`.
- New integration test: `_private` and `public` top-level functions are distinguished by visibility attribute.
- New cross-tree test: a Dart consumer with `dependencies: { lib_a: { path: "../lib_a" } }` emits the expected path-dep edge after multi-root expansion.

**LOC:** ~1200-1500.

---

### PR-8 — Elixir surface analyser (subprocess)

**Intent:** Wave-2 language analyser. Elixir introduces `behaviour` —
Erlang/Elixir's structural protocol pattern — which earns its own
`ContractKind::Behaviour` variant in the schema.

**New crate:**
- Create: `crates/analyzers/elixir/Cargo.toml`, `src/main.rs`, `src/lib.rs`.

**Parser library:** `tree-sitter-elixir`. Note: Elixir's macro system
makes parsing genuinely hard; tree-sitter covers the core grammar but
not macro-expanded code. Phase 2 documents this as `elixir_macro_expansion_phase3` — Phase 3 may invoke the Elixir compiler for full
expansion.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `elixir_surface_analyzer.rs`.
- Modify: deterministic L3 classifier — `mix.exs` → `kind: elixir-project`.
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `elixir-project`.

**Files (in `atlas-contracts/crates/atlas-index/src/surfaces.rs`):**
- Modify: `ContractKind` gains `Behaviour` variant. Behaviour contracts capture `@callback` declarations.

**Files (in `crates/atlas-engine/src/manifest_parse.rs`):**
- Modify: read `mix.exs`'s `defp deps do [{:foo, path: "../foo"}] end` for path-dep recognition. (Pragmatic regex-based; mix.exs is Elixir code, not declarative — full parse is out of scope.)

**Binding shape:**
- Top-level `defmodule Mod do ... end` → module binding.
- `def` (public) → `Binding` with `Visibility::Conventional` (visible).
- `defp` (private) → excluded from surface.
- `@spec` and `@doc` captured as `attributes`.
- `defprotocol` / `defimpl` → `Contract` with kind `Behaviour` (defines / implements).
- `behaviour M, callbacks: [...]` → `Contract` with kind `Behaviour`.

**Acceptance criteria:**
- New integration test: a `mix.exs` + `lib/foo.ex` fixture is classified `elixir-project`.
- New integration test: `def foo` and `defp bar` produce one binding (foo) and zero bindings (bar excluded).
- New integration test: `defprotocol Stringable do @callback to_string(t) :: String.t() end` produces a `Contract` with `kind: behaviour` and a `defines-contract` edge.
- New integration test: a path-dep declared in `defp deps` resolves correctly through multi-root expansion.

**LOC:** ~1300-1600.

---

### PR-9 — Racket surface analyser (subprocess)

**Intent:** Wave-2 language analyser. Racket uses `provide`/`require`
declarations for module surface, and `info.rkt` as the package manifest.
Drives Linkuistics use cases.

**New crate:**
- Create: `crates/analyzers/racket/Cargo.toml`, `src/main.rs`, `src/lib.rs`.

**Parser library:** `tree-sitter-racket` if mature; otherwise a
hand-rolled minimal s-expression reader. Subagent has discretion. The
binding extraction is shallow (top-level `provide`, `define`, `define-struct`,
`define-syntax`) so a full reader isn't required.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `racket_surface_analyzer.rs`.
- Modify: deterministic L3 classifier — `info.rkt` → `kind: racket-package`. `*.rkt` without `info.rkt` → fallback to LLM-classify (Racket source can be a library, an app, or a one-off — info.rkt is the disambiguator).
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `racket-package`.

**Binding shape:**
- `(provide name1 name2 ...)` → each `name` becomes a `Binding` with `Visibility::Conventional`.
- `(define name ...)` at top level (without explicit provide) → `Binding` with `attributes.private: true` (Racket's default is library-private without provide).
- `(require lib)` → import edge contribution (consumed by L6).
- `info.rkt`'s `deps` field → path-dep recognition for Racket.

**Acceptance criteria:**
- New integration test: an `info.rkt` + `main.rkt` with `(provide foo)` and `(define foo ...)` produces a binding for `foo`.
- New integration test: a `(define helper ...)` without provide is in the surface but flagged `private: true`.
- New integration test: a `(require 'other-pkg)` resolves through a sibling Racket package via path-dep.

**LOC:** ~1000-1300.

---

### PR-10 — LispKit surface analyser (subprocess)

**Intent:** Wave-2 language analyser. LispKit is a Swift-implemented
Scheme used by one Linkuistics project; it follows R7RS-ish conventions
with a `define-library` syntax for module declaration.

**New crate:**
- Create: `crates/analyzers/lispkit/Cargo.toml`, `src/main.rs`, `src/lib.rs`.

**Parser library:** Hand-rolled minimal s-expression reader (no mature
tree-sitter for LispKit specifically; reuse the Racket-side reader if
PR-9 hand-rolled one). Subagent discretion.

**Manifest detection:** PR-10's first task is to verify LispKit's actual
manifest convention against the Linkuistics project that uses it. Working
hypothesis: a `package.scm` or `lispkit.toml` file at the package root
declaring metadata. If the convention is "no manifest, just a directory of
.scm files", fall back to `*.sld` (R7RS library declaration files) as the
manifest signal.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `lispkit_surface_analyzer.rs`.
- Modify: deterministic L3 classifier — `package.scm` (or whatever PR-10 verifies) → `kind: lispkit-package`.
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `lispkit-package`.

**Binding shape:**
- `(define-library (lib name) (export sym1 sym2 ...) (begin ...))` → each exported `sym` is a `Binding`.
- `(define name ...)` inside library → optional binding with `attributes.private: true` if not exported.

**Acceptance criteria:**
- New integration test: a LispKit fixture (one library file with `define-library` + exports) is classified `lispkit-package` with the exports as bindings.
- New integration test: a non-exported `define` at library scope is flagged `private: true` in the surface.
- Manifest convention documented in the PR description (whatever the subagent verified against Linkuistics).

**LOC:** ~800-1100.

---

### PR-11 — Compose composition-edge analyser (deterministic, in-process)

**Intent:** Compose composition-edge analyser produces `bundled-into` and
`deployed-with` edges from `docker-compose*.yml` files. In-process
(deterministic, no LLM).

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `compose_classifier.rs` — `ComposeClassifier` (L1 file enumeration to seed deliverable candidates; L3 classification of compose files; L6 emission of composition edges).
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `compose-orchestration` (a `kind: compose-orchestration` component is what a compose file is — a deployment-orchestration deliverable).

**Files (in `crates/atlas-engine/src/l6_edges.rs`):**
- Modify: extend candidate-edge proposer to consult the Compose analyser at L6, alongside the Dockerfile analyser. Compose-derived edges are interleaved with Dockerfile-derived edges in lexicographic order.

**Edge semantics:**
- For each service in a compose file:
  - If `image:` declared → `bundled-into` edge from the source-component (resolved via `image:` matching a `kind: docker-image` component already produced by Phase 1 PR-9, or an `external-component` if the image isn't local) to the compose-orchestration deliverable.
  - If `build:` declared → resolve the Dockerfile path and emit the same `bundled-into` edge as Phase 1 PR-9 already does, plus an additional `bundled-into` edge from that source-component to the compose-orchestration deliverable.
- Between every pair of services in the same compose file → `deployed-with` edge (symmetric, lifecycle: deploy).

**Acceptance criteria:**
- New integration test: a `docker-compose.yml` with `services: { web: { image: "myrepo/web:1" }, db: { image: "postgres:15" } }` produces a `compose-orchestration` component, a `bundled-into` edge from each image's source-component (or external-component) to the orchestration, and a `deployed-with` edge between web and db.
- New integration test: a compose file with `build:` declarations correctly resolves to local Dockerfile-derived components.
- Multiple compose files in one workspace are emitted as separate orchestration components.

**LOC:** ~700-1000.

---

### PR-12 — Shell-script LLM-fallback analyser (in-process)

**Intent:** Closes the design's "fuzzy orchestration" case. In-process
(uses the existing `LlmHook` from Phase 1 PR-5). Outputs always
`Confidence::Graded`.

**Files (in `crates/atlas-analyzers/src/`):**
- Create: `shell_script_llm_analyzer.rs` — `ShellScriptLlmAnalyzer` registered at L3 (classification) and L5 (surface). Inputs: `*.sh`, `*.bash`, `*.zsh`, `Makefile`, `*.mk`.
- Modify: `registry.rs::builtin()`.

**Files (in `atlas-contracts/crates/atlas-index/src/schema.rs`):**
- Modify: `kind` enum gains `shell-script` (deliverable kind for standalone scripts) and `makefile-orchestration` (for Makefiles).

**Behaviour:**
- L3 classification: identify the script's primary purpose (build glue, deploy script, dev convenience, CI step) via LLM Stage 1 prompt against the file's first 200 lines. Output: `Confidence::Graded { kind, confidence }`.
- L5 surface: extract function definitions in the script (e.g. `function foo() { ... }` or `foo() { ... }`); emit each as a `Binding` with `Visibility::Conventional` and `attributes.shell_function: true`.
- LLM prompts pinned to Phase 2 defaults; calibration is Phase 3.

**Acceptance criteria:**
- New integration test: a `deploy.sh` fixture with a `function deploy()` declaration produces a `shell-script` component with one binding.
- New integration test: a `Makefile` with `build:` and `clean:` targets produces a `makefile-orchestration` component (target extraction is in scope; LLM-derived purpose is `Confidence::Graded`).
- Confidence threshold defaults to `0.6` (any LLM call below this returns `Confidence::Declines`); the threshold is configurable via `analyzers.yaml`.

**LOC:** ~600-900.

---

### PR-13 — Phase 1 hangover bundle (L8 phantoms + PR-12 polish)

**Intent:** Pay off the four open items from Phase 1's status file:
- L8 phantom subcomponent observation (PR-12-of-Phase-1 status note).
- `l5_surface.rs:253-262` cryptic `vec![PathBuf::new()]` sentinel (refactor to early return).
- `surface_calls_for` substring matching in `atlas_contracts_in_ravel_lite.rs` (replace with `serde_json::from_str` lookup).
- `pipeline.rs:776-812` missing `eprintln!` warning on the `roots[0]` fallback when no manifest matches.

**Files:**
- Modify: `crates/atlas-engine/src/l8_subcomponents.rs` (or wherever the L8 fixedpoint lives — investigate which file holds the subcarve enumeration). Fix the phantom-subcomponent emission for cases where a peer root and primary share parent-directory layout.
- Modify: `crates/atlas-engine/src/l5_surface.rs:253-262` — refactor the cryptic absolute-segment branch to an early return.
- Modify: `crates/atlas-cli/tests/atlas_contracts_in_ravel_lite.rs` — replace `surface_calls_for`'s substring matching against the canonical-input JSON with a proper `serde_json::from_str` lookup that asserts the structured `COMPONENT_ID` field equals the expected value.
- Modify: `crates/atlas-cli/src/pipeline.rs:776-812` — add `eprintln!("warning: ...")` on the `roots[0]` fallback when no manifest resolves under any root.

**Acceptance criteria:**
- New regression test: a fixture mirroring the PR-12-of-Phase-1 layout (peer root + primary sharing parent-dir) produces ZERO phantom subcomponents (e.g. no `atlas-contracts/consumer-crate` ghost).
- The PR-12-of-Phase-1 test continues to pass after the `surface_calls_for` rewrite.
- Manual eyeball verification that the polished `l5_surface.rs` is clearer than the sentinel form.
- `pipeline.rs` tests stay green; new test asserts the warning fires when expected.

**LOC:** ~400-700.

---

### PR-14 — Acceptance: polyglot dull-shaped fixture

**Intent:** End-to-end smoke test that exercises every Phase 2 analyser
plus the Phase 1 mechanisms they extend. Hermetic checked-in fixture, not
a live-repo dependency.

**Files:**
- Create: `crates/atlas-cli/tests/phase2_polyglot_fixture.rs` — runs `atlas index` against a hand-crafted fixture under `crates/atlas-cli/tests/fixtures/phase2_polyglot/`. The fixture mirrors dull/'s polyglot shape:
  - One C# component (`csharp_lib/Csharp.Lib.csproj` + `Csharp/Lib.cs`).
  - One Dart component (`dart_lib/pubspec.yaml` + `lib/dart_lib.dart`).
  - One Flutter component (`flutter_app/pubspec.yaml` with `flutter:` block + `lib/main.dart`).
  - One TS package (`ts_pkg/package.json` + `ts_pkg/tsconfig.json` + `ts_pkg/src/index.ts`).
  - One JS package (`js_pkg/package.json` + `js_pkg/index.js` — no tsconfig, no TS).
  - One Python package (`py_pkg/pyproject.toml` + `py_pkg/pkg/__init__.py` + `py_pkg/pkg/mod.py`).
  - One Elixir project (`ex_app/mix.exs` + `ex_app/lib/ex_app.ex` declaring a behaviour).
  - One Racket package (`rkt_pkg/info.rkt` + `rkt_pkg/main.rkt`).
  - One LispKit package (whatever PR-10 verifies — placeholder `lk_pkg/package.scm` + `lk_pkg/main.sld`).
  - One Rust crate (`rust_lib/Cargo.toml` + `rust_lib/src/lib.rs` with a `pub mod foo { pub struct Bar; }` to verify PR-5's syn-based extractor).
  - Three Dockerfiles: `Dockerfile`, `Dockerfile.frontend.buildkite`, `Dockerfile.backend.buildkite` — verifies Phase 1 PR-9's classifier handles the `*.buildkite` suffix.
  - Two compose files: `docker-compose.yml` and `docker-compose.proxy-apis.yml` — verifies PR-11.
  - One Makefile and one `deploy.sh` — verifies PR-12.

**Acceptance criteria:**
- Every component is classified to its expected `kind` with no LLM calls except for the shell-script LLM-fallback (PR-12). 12 components total (10 language + 1 compose-orchestration + 1 makefile-orchestration), plus 3 deliverable docker-image components from Dockerfiles, plus 1 shell-script component.
- Every component has a non-empty `surfaces.yaml` with at least one binding (where the component has source code; `compose-orchestration` and `shell-script` may have zero bindings if the deliverable has no public functions).
- `related-components.yaml` contains:
  - At least 2 `bundled-into` edges from the Compose files.
  - At least 2 `deployed-with` edges from the Compose files.
  - At least 3 `bundled-into` edges from the Dockerfiles (verifies `*.buildkite` suffix).
  - At least 1 `defines-contract` edge from the Elixir behaviour.
  - At least 1 `consumes-contract` edge from the cross-component path-dep wiring.
- A no-op re-run produces 100% cache hit (zero LLM calls; verified by `PR14Backend` per the PR-12-of-Phase-1 pattern).
- A targeted edit of one component's source invalidates only its L5 entry plus consumers' L6 entries — full pipeline survival of the Phase 1 PR-11 invariant under Phase 2's wider scope.

**LOC:** ~600-900 fixture + ~400-600 test.

---

## 5. Acceptance criteria summary (per-PR table)

The following table is the canonical acceptance gate. A PR may not land
until every row in its column is green.

| PR | Tests pass | New unit/integration tests | Smoke test contributes to |
|---|---|---|---|
| PR-0  | n/a (docs)        | n/a                                                                                       | n/a |
| PR-1  | workspace         | named exports, type-only export, CJS exports, classify w/o LLM                            | PR-14 (TS+JS components) |
| PR-2  | atlas-analyzers   | transport round-trip, handshake mismatch, crash, timeout, binary-sha invalidation         | PR-14 (every subprocess analyser uses transport) |
| PR-3  | workspace         | classify w/o LLM, conventional visibility, decorator capture, cache invalidation, cross-tree | PR-14 (Python component) |
| PR-4  | workspace         | dispatch identity, all-declined identity, cargo + dockerfile + python identity propagation | PR-14 (every component's analyser_id) |
| PR-5  | workspace         | nested pub inside pub mod extracted; all 23 PR-7-of-Phase-1 tests stay green             | PR-14 (Rust component) |
| PR-6  | workspace         | classify w/o LLM, attribute capture, internal exclusion, sln→multi-component             | PR-14 (C# component) |
| PR-7  | workspace         | dart vs flutter discrimination, conventional visibility, path-dep cross-tree              | PR-14 (Dart + Flutter) |
| PR-8  | workspace         | classify w/o LLM, def vs defp visibility, behaviour contract, mix.exs path-dep            | PR-14 (Elixir + behaviour edge) |
| PR-9  | workspace         | provide-extracted bindings, no-provide private flag, info.rkt path-dep                    | PR-14 (Racket component) |
| PR-10 | workspace         | define-library export extraction, non-exported define private, manifest convention doc'd  | PR-14 (LispKit component) |
| PR-11 | workspace         | bundled-into from image:, deployed-with between services, multi-compose-file              | PR-14 (compose-orchestration + 2+ deployed-with edges) |
| PR-12 | workspace         | shell function binding, Makefile target extraction, threshold-configurable                | PR-14 (shell-script + makefile-orchestration components) |
| PR-13 | workspace         | zero phantom subcomponents in regression fixture; PR-12-of-Phase-1 still green             | PR-14 (clean L8 output) |
| PR-14 | e2e               | polyglot fixture: 12 components classified, no-LLM paths verified, cache hit on rerun     | this *is* the smoke test |

---

## 6. Risks (Phase 2 specific)

These are operational risks for the Phase 2 implementation, supplementing
design §12 (architectural risks).

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Subprocess transport's wire protocol leaks Cargo idioms despite Q1's mitigation. | Medium | High | PR-2 ships against Python (PR-3) which has structurally the most-different surface shape (no pub/priv wall, decorators, dotted module paths). If the protocol survives Python, the C#/Dart/Elixir/Racket/LispKit wave succeeds mechanically. |
| Seven parallel-wave PRs (PR-6 to PR-12) cause merge churn on `surfaces.rs` and `schema.rs` in atlas-contracts. | High | Medium | The schema's load-bearing mutations (`Visibility::Conventional`, `module_path`, `attributes`) are all settled by PR-3 before Wave 3 starts. Wave 3 PRs only add new `kind` enum variants and self-contained variants, which conflict-resolve in <5 min. The status file's per-PR notes record each PR's schema contribution so the merge trail is auditable. |
| Choice of language parser library per analyser introduces native deps that complicate the build. | Medium | Medium | Each analyser PR's "Files" sub-section names its parser dep choice; native deps stay inside the subprocess analyser binary, never the engine. Cross-platform CI already exists for the Rust workspace. |
| Subprocess analyser binary distribution. | Medium | High | Phase 2: each analyser is a separate workspace member under `crates/analyzers/<lang>/`; binaries build with `cargo build --workspace` and are co-located in `target/<profile>/`. Out-of-tree analyser distribution is Phase 3+. |
| L8 phantom-subcomponent cleanup (PR-13) requires touching the L8 fixedpoint, which has subtle invariants. | Medium | Medium | PR-13 lands its own regression test based on the Phase 1 PR-12 observation; the `tombstone_emit_once_design` and `all_components_not_salsa_tracked` invariants stay green. |
| `syn`-based Rust extractor (PR-5) finds spans the regex extractor missed, churning Phase 1 surface fingerprints. | High | Low | Greenfield + no users: regenerate fixtures. Documented as an expected churn in the PR description. |
| Dart/Flutter analyser conflates pubspec dependencies and Flutter-specific manifest fields. | Medium | Low | PR-7's brief separates Dart-shape (pure pubspec) from Flutter-shape (pubspec with `flutter:` block); each emits a distinct `kind`. |
| LispKit's manifest convention is unspecified and may not match any common Scheme dialect. | Medium | Medium | PR-10's first task is to verify against the actual Linkuistics project that uses LispKit; the brief authorises a fallback to `*.sld` library declaration files if no canonical manifest exists. |
| Schema mutation across the parallel wave (PR-6 to PR-12) breaks earlier PRs' expectations of a stable shape. | Medium | Medium | Greenfield: each wave-2 PR regenerates fixtures it touches. Per-PR status notes record what schema fields it added so the next session sees the trail. |
| Subprocess analyser's first-call latency (process spawn + handshake) is high; degrades hot-path performance noticeably. | Medium | Low | Phase 2 spawns one process per analyser per pipeline (process pool with size 1 reused across calls). Worst-case spawn cost amortises across the typical 100+ component classification. Phase 3 may add concurrent pools if needed. |
| Two PRs land out of dependency order due to merge timing. | Low | High | The PR descriptions explicitly list `Depends on: PR-N`; CI / orchestrator should refuse to merge a PR whose dependency target has not yet landed. |

---

## 7. Out of scope for Phase 2

These items are deferred to later phases, per design §10.3-§10.4 and per
the brainstorm decisions. A reviewer who flags them as missing should
redirect to the relevant phase.

- **Migration of existing in-process analysers** (Cargo, Dockerfile, RustSurface, LlmClassify) **to subprocess.** Phase 3 sweep — establishes the philosophical end-state of "one mechanism only".
- **rust-analyzer integration replacing `syn`.** Phase 3 stretch; `syn` is the Phase 2 cost-quality point.
- **TS-as-subprocess.** Phase 3 sweep.
- **Bidirectional LLM callback channel** for subprocess analysers. Deferred until a Phase 3+ subprocess analyser actually needs LLM access.
- **Contract rename-match** (§11.2.4). Phase 3.
- **LLM confidence threshold calibration** (§11.2.6). Phase 3, after real-workspace data exists.
- **`--strict-overrides` flag.** Phase 3.
- **k8s manifest analyser, Helm chart analyser.** Dropped from Phase 2 (neither dull/ nor Linkuistics use them); reconsidered if a future workspace requires them.
- **Drift report, impact query, modularity report.** Phase 3+.
- **Pattern detection.** Phase 3+.
- **Composition divergence report.** Phase 3+.
- **Server mode** (file watcher, query API, subscriptions). Phase 4+.
- **Grafeo derived index.** Phase 4+.
- **Cache compression** (§11.2.7).
- **Worktree commit-sha consistency validation** (§11.2.8).
- **PR-12-of-Phase-1 manual verification** against live Ravel-Lite + atlas-contracts. Deferred until Ravel-Lite ports to vNext shape (which is its own future redesign phase per the user's statement).
- **Concurrent subprocess analyser pools.** Phase 3+ if measured spawn-cost ever becomes hot.
- **Stretch goal: Haskell analyser.** Originally listed in §10.2; deprioritised in favour of the dull/Linkuistics-driven scope.

---

## 8. References

- Design spec: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` (especially §10.2, §11.2, §12).
- Phase 1 plan: `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
- Phase 1 status (per-PR notes): `docs/superpowers/plans/2026-05-06-phase1-status.md`.
- Open-question resolutions from Phase 1:
  - `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md`
  - `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`
- Continuation prompt (idempotent across phases): `docs/superpowers/prompts/2026-05-07-vnext-continue.md`.
- Phase 1 mechanisms reused as Phase 2 starting points (see §3 above).
- Memory entries that constrain Phase 2:
  - `feedback_phase1_open_questions` — Phase 1 §11.2.2 / §11.2.3 are closed; §11.2.1 is absorbed in-phase here per §2.1 above.
  - `feedback_toml_parsing` — every TOML reader uses the `toml` crate.
  - `feedback_fix_all_lints` — every PR runs `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
  - `tombstone_emit_once_design` — PR-13 must not break L4 prior-filter when fixing L8 phantoms.
  - `all_components_not_salsa_tracked` — PR-13's L8 fix must keep tree-assembly memoisation in the CLI/L9 layer.
  - `nested_pub_inside_pub_mod_is_phase1_known_limitation` — closed by PR-5; memory entry can be removed or amended.

---

## 9. Dependency graph (canonical)

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
