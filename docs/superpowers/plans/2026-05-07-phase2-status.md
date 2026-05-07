# Atlas vNext Phase 2 — Status

Companion to `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`.
This file tracks per-PR completion state across sessions. The continuation
prompt at `docs/superpowers/prompts/2026-05-07-vnext-continue.md` reads
this file (via the `*phase2-plan*` wildcard match) to find the next PR
to dispatch.

**Last updated:** 2026-05-08 (Phase 2 COMPLETE — PR-14 acceptance smoke test landed; all 15 PRs [x]).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Plan + status file (docs only)
- [x] PR-1  — TypeScript / JavaScript surface analyser (in-process)
- [x] PR-2  — Subprocess analyser transport (stdio JSON)
- [x] PR-3  — Python surface analyser (first subprocess analyser)
- [x] PR-4  — Per-analyser `analyser_id` / `analyser_version` plumbing through L3 dispatch
- [x] PR-5  — Rust binding extractor: regex → `syn`
- [x] PR-6  — C# surface analyser (subprocess)
- [x] PR-7  — Dart / Flutter surface analyser (subprocess)
- [x] PR-8  — Elixir surface analyser (subprocess)
- [x] PR-9  — Racket surface analyser (subprocess)
- [x] PR-10 — LispKit surface analyser (subprocess)
- [x] PR-11 — Compose composition-edge analyser (deterministic, in-process)
- [x] PR-12 — Shell-script LLM-fallback analyser (in-process)
- [x] PR-13 — Phase 1 hangover bundle (L8 phantoms + PR-12-of-Phase-1 polish)
- [x] PR-14 — Acceptance: polyglot dull-shaped fixture (smoke test)

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
2026-05-07 — Landed as Atlas commits `76bf531` (main change) +
`abbb30b` (spec-review fix wiring extract_ts_js_surface into the engine
L5 path + extending the integration test) + `d5f69d1` (code-quality
fix adding `main+tsconfig` integration coverage); atlas-contracts
commit `a6f98e2` on the contracts main (kind enum docstring extension).

**Schema-mutation contribution:** atlas-contracts `crates/atlas-index/src/schema.rs`
docstring updated to mention the new per-language kinds. The `kind`
field is `String` (open vocabulary by design), so the typed enum lives
in `atlas-engine/src/types.rs` — `ComponentKind::TypescriptPackage` and
`ComponentKind::JavascriptPackage` variants added there, plus
`defaults/component-kinds.yaml` vocabulary entries.

**Implementation:** `swc_ecma_parser` + `swc_ecma_ast` for TS/JS
parsing (pinned at swc_common 21 / swc_ecma_ast 23 / swc_ecma_parser
39). Two new files in `crates/atlas-analyzers/src/`:
`ts_js_classifier.rs` (L3 deterministic classifier — package.json +
tsconfig.json → `typescript-package`; bare package.json with no
`bin`/`main`/`exports` → `javascript-package`; otherwise declines and
the legacy `node-cli`/`node-library` rules win to preserve Phase 1
vocabulary) and `ts_js_surface_analyzer.rs` (`TsJsSurfaceAnalyzer`,
`TsJsSurfaceOutput`, `extract_ts_js_surface`).

**Schema workaround:** the structured `Binding.attributes` field
that PR-3 will introduce does not exist yet. PR-1 encodes module-system
metadata (commonjs vs esm) and TS-type-only flag as a `language`-field
suffix: `typescript-type`, `javascript-cjs`, `javascript-esm`. PR-3
will migrate to the structured `attributes` slot when it lands the
schema. The suffix scheme is documented at the top of
`ts_js_surface_analyzer.rs`.

**Engine wiring (`abbb30b`):** spec review caught that
`l5_surface.rs::surface_artefacts_of` hard-gated on Rust, so TS/JS
components went through L5 with empty surfaces. Fix adds a TS/JS
branch BEFORE the Rust gate. The branch probes 8 well-known paths
(`src/{index,main}.{ts,tsx,js,jsx}`) plus `package.json`, calls
`extract_ts_js_surface`, and projects into `SurfaceArtefacts`. This
is a Phase-2 minimum that supports the integration fixture; deeper
nesting + `package.json#main`/`module`/`exports` resolution is
Phase 3 (full tree walk).

**`analyse()` returns `Declines`:** mirrors the `RustSurfaceAnalyzer`
pattern. The engine drives surface extraction directly via
`surface_artefacts_of`, not via the dispatcher. The trait method is
documented to explain this; a future driver may populate `Target`
with source bytes and route through here instead.

**Tests:** unit-level `ts_extracts_named_exports`,
`ts_extracts_type_only_export`, `js_extracts_commonjs_exports`,
plus `ts_extracts_default_export` and others; classifier tests cover
the TS / JS / legacy-fall-through rule table. Two integration tests
in `crates/atlas-engine/tests/l2_l3_queries.rs`:
`l3_package_json_with_tsconfig_classifies_as_typescript_package_without_llm_call`
(L3 only); `l5_typescript_package_surface_artefacts_include_exported_hello_symbol`
(drives L5 end-to-end and asserts the `hello` symbol is in the surface).
The code-quality fix added
`l3_main_plus_tsconfig_classifies_as_typescript_package_without_llm_call`
to pin the precedence rule at the dispatcher level.

**Cherry-pick conflict resolution at merge:** PR-1 was committed before
PR-2 / PR-4 had landed on main, so the cherry-pick had to resolve
adds in `lib.rs`, `registry.rs`, and `Cargo.toml` (additive merges of
PR-2's subprocess module + PR-1's TS/JS module imports), and update
`ts_js_to_classification` in `l3_classify.rs` to take and propagate
`analyser_id` / `analyser_version` per PR-4's `Classification` struct
shape.

### PR-2
2026-05-07 — Landed as Atlas commits `9413ffb` (main change) +
`293dc31` (SIGTERM fix) + `a5779a1` (shutdown-doc + stderr-on-crash
fix); atlas-contracts commit `89ed6cd` on the contracts main
(`SubprocessConfig::binary_sha` becomes load-bearing).

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/analyzers.rs` —
`SubprocessConfig::binary_sha: Option<String>` is now a load-bearing
field that subprocess analysers populate at registration time. No
surfaces.rs / schema.rs changes.

**Wire protocol:** Length-prefixed framing — 4-byte big-endian u32
length + UTF-8 JSON bytes; 16 MiB cap. Request kinds: `applies`,
`fingerprint_inputs`, `analyse` (the `applies` variant is reserved on
the wire; the proxy short-circuits locally against the predicate
verified at handshake — kept on the wire for Phase 3+ dynamic
predicates). Response kinds: `confident`, `graded`, `declines`,
`error`. Manifest bytes carried as base64 to keep non-UTF-8 inputs
safe.

**Process-pool semantics:** one process per analyser (Phase 2;
concurrent dispatch is Phase 3+). Lazy spawn on first dispatch. Pool
ownership lives in `SubprocessAnalyzerProxy`; registry construction
implicitly creates the pool, registry drop tears it down.
Per-analyse-call timeout (60s default; configurable per analyser).
Implementation: `Mutex<Option<ChildProcess>>` + worker thread +
`mpsc::recv_timeout` for cancellable waits (Atlas is fully sync; no
async dependency added).

**Shutdown sequence:** `SIGTERM` (via `libc::kill` under
`#[cfg(unix)]`), then EOF on stdin (via stdin drop), then `SIGKILL`
after a 5-second grace. The fix-up commit `293dc31` added the
explicit SIGTERM (the original implementation skipped it and went
EOF → SIGKILL); on Windows the SIGTERM step is a no-op and shutdown
falls through to forceful termination.

**Crash-path stderr capture (`a5779a1`):** child's stderr handle is
piped at spawn but not actively drained; on the crash path
(read error / timeout / disconnected), up to ~4 KB of stderr tail is
read synchronously (the kernel has already closed the pipe write end
by then) and appended to the `CallFailed` `message` string. Improves
PR-3 debuggability without restructuring `AnalyzerError`.

**Tag-byte 0x06:** `FingerprintBuilder::add_analyzer_binary_sha`
contributes the subprocess analyser binary's content sha to L-stage
fingerprints. Tag table: 0x01..0x05 unchanged; 0x06 new.

**`Box::leak` in `parse_fingerprint_inputs`:** the
`FingerprintInput::Custom { tag: &'static str }` shape forces the
parser to leak the tag string. Bounded per unique tag in practice
(subprocess analysers declare a small fixed set), but unbounded if a
hostile or buggy subprocess returns novel tags per call. TODO at
the leak site marks `Cow<'static, str>` migration as Phase 3
cleanup.

**Echo fixture:** lives as a workspace `[[bin]]` declared on
`atlas-analyzers/Cargo.toml`; tests resolve via
`env!("CARGO_BIN_EXE_echo_subprocess")` so cargo handles build
ordering. Configurable via CLI flags
(`--stage`, `--crash-before-handshake`, `--hang-after-handshake`,
etc.) — chosen over env-vars to avoid clobber under parallel test
runs.

**Tests:** all 5 §4 acceptance tests present and passing —
`transport_round_trip`, `handshake_rejects_mismatched_capabilities`
(unit + integration variants), `subprocess_crash_returns_call_failed`,
`subprocess_timeout_returns_call_failed`,
`binary_sha_change_invalidates_cache`. Plus 7 additional integration
tests covering pool re-spawn after failure, process-pool drop,
malformed JSON, etc. Total 12 integration tests in
`crates/atlas-analyzers/tests/subprocess_transport.rs`.

**Side effect during PR-2 verification:** clippy 1.93's
`needless_lifetimes` lint surfaced pre-existing `'db` annotations in
`l1_queries.rs` (5×), `l2_candidates.rs` (1×), and `l9_projections.rs`
(1×). Auto-fix applied as a separate hygiene commit — see git log
`phase2: clippy 1.93 needless_lifetimes hygiene sweep`. Per
`feedback_fix_all_lints` rule.

### PR-3
2026-05-07 — Landed as Atlas commits `c8afe2a` (migrate existing
analysers to structured Visibility/attributes) + `6238214`
(python-analyzer crate, classifier, L5 subprocess wiring) + `d05082d`
(integration tests + L5 fingerprint binary_sha contribution) +
`75aba19` (spec-compliance fix: F1 single-test, F2 Visibility
discriminator, F3 end-to-end cache, F4 L6 cross-tree, F5 module_path
schema correction) + `21e4eab` (code-quality fix: F-CQ-1 cached
subprocess proxy, F-CQ-2 dedup pub_item_kind_str, F-CQ-3 lossless
JSON→YAML, F-CQ-4 ATTR_* consts adoption, F-CQ-5 missing-binary
warning); atlas-contracts commits `0c2621b` (Visibility enum +
module_path + attributes on Binding) + `6d88650` (re-export Visibility
from crate root) + `ec3f535` (module_path docstring excludes symbol)
+ `90d5916` (declare ATTR_* const keys for Binding.attributes).

**Two-stage review outcome:** Spec-compliance review caught 5 partial
criteria → all five fixed in `75aba19`. Code-quality review verdict
🟡 ACCEPT WITH FIX-UP, six findings → five fixed in `21e4eab`,
finding F-CQ-6 (`LenientBackend` test-helper duplication across two
test files) deliberately deferred until Wave 3 actually copy-pastes a
third time (per the user's "don't abstract on speculation" rule).
Independent verification: `cargo test --workspace --no-fail-fast` (42
test-result groups all green), `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --check` — all clean on both repos.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/surfaces.rs`:
- New `Visibility` enum with `Explicit { keyword: String }` /
  `Conventional` variants. Tagged-discriminator wire form
  (`kind: explicit | conventional`).
- `Binding` gains `visibility: Visibility` (required), `module_path:
  Vec<String>` (skip-empty), `attributes: BTreeMap<String,
  serde_yaml::Value>` (skip-empty).
- `schema.rs` docstring extended to mention `python-package` in the
  open `kind` vocabulary.

`schema_version` stays integer `1` (greenfield shape mutation per
plan §2.1).

**Parser library:** `rustpython-parser` 0.4.0 (the plan's named
default). Used via `Suite::parse(text, &path)` from the `Parse`
trait; `text-size`-based `range()` provides byte-accurate spans for
`Binding.span` without character-vs-byte conversion.

**Implementation map:**
- New workspace member `crates/analyzers/python` (NOT
  `crates/atlas-analyzers/`, per the plan's path).
- `atlas_python_analyzer::extract_python_surface` (the pure-function
  analyser). Decoders for `[tool.poetry.dependencies]`,
  `[tool.poetry.dev-dependencies]`, `[tool.poetry.group.<n>.dependencies]`,
  `[tool.uv.sources]` for path-deps; PEP-621 `[project].name` and
  Poetry `[tool.poetry].name` for project-id resolution.
- `python-analyzer` binary (`crates/analyzers/python/src/main.rs`):
  speaks PR-2's wire protocol verbatim. Walks `Target.dir` directly
  for `*.py` / `*.pyi` files (skipping hidden, virtualenv,
  `__pycache__`); no LLM access (Phase 2 deterministic-only).
- `atlas_analyzers::python_classifier` (in-process L3 deterministic
  analyser). Recognises pyproject.toml / setup.py / requirements.txt
  → `kind: python-package`.
- `atlas_analyzers::python_surface_analyzer` —
  `python_subprocess_spec(binary_path)` builds the
  `SubprocessAnalyzerSpec`; `locate_python_analyzer_binary()` walks up
  from `current_exe()` for the runtime sibling-binary lookup.
- `crates/atlas-engine/src/l5_surface.rs` extended with a Python
  branch in `surface_artefacts_of` that builds a minimal `Target`,
  constructs a `SubprocessAnalyzerProxy` on demand, drives the
  subprocess transport, and decodes the JSON response payload back
  into typed `Binding` / `LibraryApi` values.
- `surface_of`'s L5 fingerprint contributes the python-analyzer
  binary's content sha (tag 0x06) when the component is Python; a
  rebuilt binary therefore invalidates the LLM-cached SurfaceRecord.
- `crates/atlas-engine/src/manifest_parse.rs` gains
  `extract_pyproject_path_deps()`; `root_expansion::expand_roots`
  now walks both `Cargo.toml` and `pyproject.toml` manifests for
  path-deps. `enclosing_manifest_root` recognises a
  `pyproject.toml` ancestor as a project root.
- `ComponentKind::PythonPackage` variant + `python-package` entry
  in `defaults/component-kinds.yaml`. `subcarve_policy` adds
  `PythonPackage` to the library-shaped kinds.

**PR-1 schema-suffix workaround retired:** `ts_js_surface_analyzer`
migrated from the `language: "typescript-type"` / `"javascript-cjs"` /
`"javascript-esm"` suffix scheme to structured slots:
- `language` is plain `"typescript"` / `"javascript"`.
- ESM exports → `Visibility::Explicit { keyword: "export" }`,
  `attributes.module_system: "esm"`, `attributes.type_only: true` for
  type-only exports.
- CJS object-shorthand (`module.exports = { foo }`) →
  `Visibility::Conventional` (no per-binding keyword in the
  shorthand shape), `attributes.module_system: "commonjs"`.
- CJS property-style (`exports.foo = …`) →
  `Visibility::Explicit { keyword: "exports" }`,
  `attributes.module_system: "commonjs"`.
PR-1's tests updated to assert against the new structured slots.

**Rust analyser migration:** `rust_surface_analyzer` populates
`Visibility::Explicit { keyword: "pub" }`, empty `module_path`,
empty `attributes` for every binding. Version constant unchanged
(PR-5 already bumped to 2.0.0).

**Tests:** all 5 §4 acceptance criteria + 22 lib unit tests + 7
classifier unit tests + 5 integration tests + 3 cross-tree path-dep
tests. Listed by criterion:
1. `l3_pyproject_toml_classifies_as_python_package_without_llm_call`
   (in `tests/l2_l3_queries.rs`) — classify w/o LLM.
2. `python_underscore_prefix_function_records_conventional_private_attribute`
   (in `tests/l5_python_surface.rs`) — conventional visibility +
   `attributes.private: true`.
3. `python_dataclass_decorator_recorded_in_attributes_decorator_chain`
   (in `tests/l5_python_surface.rs`) — `@dataclass` decorator
   capture.
4. `python_binary_sha_change_invalidates_l5_cache`
   (in `tests/l5_python_surface.rs`) — binary content change
   reshapes L5 fingerprint.
5. `pyproject_path_dep_expands_to_peer_root` (and the two sibling
   variants for `dev-dependencies` and `[tool.uv.sources]`)
   (in `tests/multi_root_path_deps.rs`) — cross-tree path-dep flows
   into the consumer's L6 cache key via the fixed-point walker.

**Decisions / deviations:**

- **Wire types not factored into a shared crate.** The plan
  mentioned this as an option; PR-3 keeps them in `atlas-analyzers`
  and the python-analyzer binary takes a path-dep on
  `atlas-analyzers` for them. Rationale: low-cost dep arrow, no
  cycle (atlas-analyzers does not depend on atlas-python-analyzer),
  and Phase 3's "every analyser is a subprocess" sweep would re-shape
  this anyway.
- **L5 per-language probe NOT extracted into a helper trait.** The
  plan's wording authorised this if the LOC count justified it; with
  TS/JS + Rust + Python branches the inline form is ~250 LOC and
  the language-shape diffs are large enough that an extracted trait
  would have to carry a wide interface. Kept inline; revisit in
  Phase 3 when the C# / Dart / Elixir / Racket / LispKit branches
  land.
- **Python subprocess does its own filesystem walk.** The wire
  protocol's `Target.manifests` only carries pre-loaded manifests;
  Python source files are not pre-loaded. The subprocess walks
  `Target.dir` directly via `std::fs::read_dir`. Mirrors the TS/JS
  in-process pattern; a future driver may stream source bytes
  through the wire envelope to keep the analyser sandboxable.
- **`PythonPackage` is a new ComponentKind variant** alongside the
  pre-existing `PythonLibrary` / `PythonApp`. The legacy heuristic
  rule still emits `PythonLibrary` when this analyser declines (e.g.
  on a manifest the registry hasn't been populated against);
  Phase 3 may unify the vocabulary.
- **`extract_pyproject_path_deps` recognises Poetry + uv only.** PEP
  621's `[project.dependencies]` is a string-array form that does
  not standardise local-path deps, so the engine's path-dep walker
  does not consult it. Recognised conventions: Poetry's
  `dependencies` / `dev-dependencies` / groups, plus uv's
  `[tool.uv.sources]`.

**Cleanups deferred:**
- The python-analyzer binary's runtime fs walk uses `std::fs::read_dir`
  with manual pruning (skipping `__pycache__`, `.venv`, etc.). Phase
  3 may swap in `ignore` for full gitignore-aware walks if dull/'s
  Python components live alongside `.gitignore` files that exclude
  source dirs.
- The `module_path` slot on the TS/JS analyser is empty for now;
  resolving `package.json#name` + relative import paths into a
  dotted module path is left for a future PR.
- `PythonPackage` does not yet flow into `pipeline.rs`'s
  language-tag inference; a polyglot Python+Rust component still
  appears as Rust-only at L1. Not a regression — pre-PR-3 the same
  was true.
- `LenientBackend` test-helper is duplicated between
  `tests/multi_root_path_deps.rs` and `tests/l5_python_surface.rs`.
  Stage-2 review (F-CQ-6) flagged this; deliberately deferred until
  Wave 3 copy-pastes it a third time, then extract to
  `tests/common/mod.rs`.

**Wave 3 readiness — load-bearing patterns established by PR-3:**

- **`cached_subprocess_proxy(spec)`** in
  `crates/atlas-analyzers/src/python_surface_analyzer.rs` is the
  canonical pattern for caching a `SubprocessAnalyzerProxy` keyed on
  binary path. Wave 3's C# / Dart / Elixir / Racket / LispKit
  subprocess analysers should adopt the same pattern (one helper per
  analyser crate is fine) so an N-component workspace incurs one
  spawn per analyser, not N×analysers spawns.
- **`ATTR_*` const keys** in `atlas-contracts` (`ATTR_PRIVATE`,
  `ATTR_DECORATOR_CHAIN`, `ATTR_MODULE_SYSTEM`, `ATTR_TYPE_ONLY`)
  define the canonical attribute-key vocabulary on `Binding.attributes`.
  Wave 3 PRs that introduce new attribute keys (C# `[Attribute]`,
  Elixir `@spec`, Dart `@annotations`, etc.) should append new
  `ATTR_*` consts in the same file rather than emitting bare-string
  keys.
- **`Visibility::Conventional`** is the load-bearing variant for
  languages without explicit visibility keywords (Python, Dart,
  Racket, Elixir `defp`). Use with an `attributes.private: true`
  flag (via `ATTR_PRIVATE`) when the language has a
  conventional-private form (leading `_`, `defp`, etc.). Use
  `Visibility::Explicit { keyword: ... }` for `pub` / `public` /
  `export` / `provide` etc.
- **JSON→YAML conversion** in subprocess analyser response decoding:
  use `serde_json::from_value::<serde_yaml::Value>(v.clone())` (the
  idiom validated in `decode_python_surface_payload`), NOT
  `serde_yaml::from_str(&v.to_string())` (lossy on YAML-special
  characters in attribute values).
- **`module_path` semantics** are file-path-derived only (excluding
  symbol). `pkg/sub/mod.py` → `["pkg", "sub", "mod"]`. The dotted
  identifier is `module_path.join(".") + "." + symbol`.

### PR-4
2026-05-07 — Landed as Atlas commits `a1b8f20` (main change) +
`b705926` (code-quality fix preserving OVERRIDE sentinel for
additions); atlas-contracts commit `d1e7ade` on the contracts main
(per_component docstring listing the sentinel ids).

**Schema-mutation contribution:** atlas-contracts `per_component.rs`
docstring extended to enumerate the analyser ids the field can hold
(`cargo-toml-classifier`, `dockerfile-l3`, `python-surface-analyzer`,
`typescript-package-classifier`, plus `"none"` and `"override"` sentinels).

**Implementation:** `dispatch` and `dispatch_with_filter` now return
`(DispatchOutcome, &'a str /* analyser_id */, &'a str /* analyser_version */)`
with the slices borrowed from the registry's analyser instances. New
public sentinels in `atlas-analyzers`: `NONE_ANALYZER_ID = "none"` /
`NONE_ANALYZER_VERSION = "0.0.0"` (returned on `AllDeclined`). New
engine-side sentinels in `l3_classify`: `OVERRIDE_ANALYSER_ID = "override"`
/ `OVERRIDE_ANALYSER_VERSION = "0.0.0"` (used by `addition_to_classification`
in `l4_tree.rs` for hand-authored additions and by `pins_to_classification`
for explicit pins). New engine-side sentinels in `heuristics`:
`LEGACY_ANALYSER_ID = "legacy-deterministic-rules"` /
`LEGACY_ANALYSER_VERSION = "1.0.0"` (used by the npm / pyproject /
bare-git rule table).

**Code-quality fix `b705926`:** the original `lookup_analyser_identity`
in `l9_projections.rs` re-invoked `is_component`, which followed the
deterministic / heuristic / LLM path and missed the `addition_to_classification`
path that already stamped the OVERRIDE sentinel. Hand-authored
`overrides.additions` entries without corresponding pins were silently
recorded as `analyser_id: "none"`. Approach A: a new `AnalyserIdentityMap`
type alias and `all_component_analyser_identities` query in `l4_tree.rs`
expose the live tree's classification identities; `lookup_analyser_identity`
now consults that map first and only falls back to `is_component` for
components not in the map. The `is_component` fallback is defensive only
— the live tree is the canonical source. Also added a TODO comment at
`DispatchOutcome::analyzer_id` noting it is now redundant with the tuple
return and can be removed in a future cleanup.

**Tests:** `dispatch_returns_winning_analyser_identity` and
`dispatch_all_declined_returns_none_identity` added to `dispatcher.rs`'s
test module. Two new integration tests in
`crates/atlas-cli/tests/scattered_atlas_layout.rs`:
`cargo_classified_component_records_cargo_analyser_identity` (asserts
`"cargo-toml-classifier"`) and `dockerfile_classified_component_records_dockerfile_analyser_identity`
(asserts `"dockerfile-l3"`). The fix-commit added
`overrides_addition_records_override_analyser_id` pinning the corrected
addition path. The Python-classified-component criterion in §5 is
correctly absent (deferred to PR-3 / PR-14, since the Python analyser
does not exist until PR-3). The L3 stage fingerprint's `"l3-driver"` /
`"1.0.0"` placeholder was replaced with the LLM analyser's id/version
(the fingerprint only gates the LLM-classify branch). The
`L3_DRIVER_VERSION` const is deleted from both `l9_projections.rs` and
the `lib.rs` re-export.

**Cleanups beyond §4 (greenfield):** `addition_to_classification` and
the four heuristics rules learned the new id/version fields by virtue
of `Classification` growing them; `parse_llm_response`,
`pins_to_classification`, `unknown_classification` updated similarly.

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
2026-05-07 — Landed as Atlas commits `792c40d` (csharp crate scaffold) +
`14da618` (L3 classifier + L5 engine wiring) + `3c31edf` (integration
tests + fmt) + `b437e8c` (spec fix F1 sln-with-two-csproj integration
test) + `c76d74e` (spec fix F2 csproj path-dep recognition) + `19669a5`
(rebase fix propagating analyser count 8→9 in three sibling registry
tests after the csharp addition); atlas-contracts commit `b1f3fb2`
(kind docstring gains csharp-project / csharp-solution paragraph).

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring extended with a
PR-6 paragraph (csharp-project / csharp-solution). No type-level
schema changes — the `kind` field stays `String` (open vocabulary).

**Parser library:** `tree-sitter-c-sharp` via the `tree-sitter` Rust
binding (the plan's named default).

**Implementation map:**
- New workspace member `crates/analyzers/csharp` (lib + `csharp-analyzer`
  subprocess binary), speaking PR-2's wire protocol verbatim.
- `atlas_analyzers::csharp_classifier` (in-process L3 deterministic):
  recognises `*.csproj` → `csharp-project` and `*.sln` → `csharp-solution`,
  with `*.sln` taking precedence when both appear at the same path
  (per the F1 sln-vs-csproj integration test).
- `atlas_analyzers::csharp_surface_analyzer` —
  `csharp_subprocess_spec(binary_path)` builds the
  `SubprocessAnalyzerSpec`; `cached_csharp_subprocess_proxy(spec)`
  caches the proxy keyed on binary path (mirrors PR-3's
  `cached_subprocess_proxy` pattern); `locate_csharp_analyzer_binary`
  walks up from `current_exe()` for the runtime sibling-binary lookup.
- `crates/atlas-engine/src/l5_surface.rs` extended with a C# branch in
  `surface_artefacts_of` that builds a minimal `Target`, drives the
  subprocess, and decodes the JSON response into typed `Binding` /
  `LibraryApi` values via the generalised `decode_subprocess_surface_payload`.
- `crates/atlas-engine/src/manifest_parse.rs` learns to read
  `*.csproj`'s `<ProjectReference Include="../path">` for cross-tree
  path-dep recognition (F2 spec fix); `enclosing_manifest_root`
  recognises a `*.csproj` ancestor as a project root.
- `ComponentKind::CsharpProject` / `ComponentKind::CsharpSolution`
  variants + `csharp-project` / `csharp-solution` entries in
  `defaults/component-kinds.yaml`. `subcarve_policy` adds both kinds
  to the library-shaped kinds.

**Binding extraction shape:**
- Top-level `public class/struct/interface/record/enum` → `Binding`
  with `Visibility::Explicit { keyword: "public" }`, `module_path`
  rooted at the namespace (e.g. `Foo.Bar.Models`).
- `public` methods on public types → `Binding` with the method name
  appended to the parent type's `module_path`.
- C# `internal` / `private` members are excluded from surface (Phase 2
  default; opt-in via `attributes.internal_visible: true` is Phase 3+).
- C# attributes (`[Authorize]`, `[Serializable]`) captured as
  `attributes.cs_attributes: ["Authorize", "Serializable"]`.

**Tests:** all 5 §4 acceptance criteria covered + 23 lib unit tests +
classifier unit tests + 7 integration tests in
`crates/atlas-engine/tests/l5_csharp_surface.rs`. Listed by criterion:
1. `l3_csproj_classifies_as_csharp_project_without_llm_call` (no LLM).
2. `csharp_namespace_module_path_records_dotted_namespace_for_nested_class` (module_path).
3. `csharp_serializable_attribute_recorded_in_attributes_cs_attributes_list` (attribute capture).
4. `csharp_internal_class_excluded_from_surface_only_public_class_appears` (internal exclusion).
5. `l3_sln_with_two_csproj_emits_two_csharp_project_components` (sln→multi-component, F1 fix).

**Decisions / deviations:**
- **F-CQ-6 LenientBackend duplication addressed**: the test-helper
  duplication PR-3 deferred (between `multi_root_path_deps.rs` and
  `l5_python_surface.rs`) is NOT yet extracted in PR-6 — `l5_csharp_surface.rs`
  is the third copy site. Per the user's "don't abstract on
  speculation" rule the deferral was tied to "Wave 3 actually copy-pastes
  it a third time, then extract"; PR-6's third copy-paste means
  extraction is now warranted but was kept inline in PR-6 to stay
  within the §4 scope. Defer the extraction to a Wave 4 cleanup PR
  (post PR-14).
- **`*.sln` precedence** when both `*.sln` and `*.csproj` co-exist:
  the sln wins because it nominates the multi-project boundary;
  the csproj entries become subcomponents under the sln-classified
  parent. F1 spec fix added an integration test pinning this rule.
- **Subprocess does its own filesystem walk.** Mirrors PR-3's pattern;
  the wire protocol's `Target.manifests` carries pre-loaded manifests
  but C# source files are walked by the subprocess directly via
  `std::fs::read_dir` on `Target.dir`.

**Cleanups deferred:**
- `LenientBackend` test-helper extraction to `tests/common/mod.rs`
  (now warranted; Wave 4).
- C# `internal_visible: true` opt-in attribute (Phase 3+).
- Module-path resolution for `using` aliases / type-forwarded namespaces
  (Phase 3+).

### PR-7
2026-05-08 — Landed as Atlas commit `a153753` (squashed integration of
`phase2-pr7-impl`: scaffold → wiring → tests → spec fixes F1/F2 →
quality fixes C-1/C-2); atlas-contracts commit `fe2f4dd` (kind
docstring gains dart-package / flutter-package paragraph). Per-stage
commits preserved on the branch ref `phase2-pr7-impl`.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring extended with a
PR-7 paragraph (`dart-package` and `flutter-package`, discriminated by
the `flutter:` top-level block in `pubspec.yaml`). No type-level
schema changes — `kind` stays `String`; the typed enum lives in
`atlas-engine`'s `ComponentKind`.

**Parser library:** the dart-analyzer crate uses a hand-rolled
tokeniser + minimal Dart-source scanner (the §4 PR-7 plan named
`tree-sitter-dart` as the default; the subagent swapped to a focused
in-house tokeniser to keep the workspace dep tree slim — Dart's
top-level public-symbol shape is regular enough that a full grammar
parser is overkill for the surface extraction this PR needs).

**Implementation map:**
- New workspace member `crates/analyzers/dart` (lib + `dart-analyzer`
  subprocess binary), speaking PR-2's wire protocol verbatim.
- `atlas_analyzers::dart_classifier` (in-process L3 deterministic):
  recognises `pubspec.yaml` and discriminates on the `flutter:`
  top-level key — present → `flutter-package`, absent → `dart-package`.
- `atlas_analyzers::dart_surface_analyzer` —
  `dart_subprocess_spec(binary_path)` builds the
  `SubprocessAnalyzerSpec`; `cached_dart_subprocess_proxy(spec)`
  caches the proxy keyed on binary path (mirrors PR-3's
  `cached_subprocess_proxy` pattern); `locate_dart_analyzer_binary`
  walks up from `current_exe()` for the runtime sibling-binary
  lookup.
- `crates/atlas-engine/src/l5_surface.rs` extended with a Dart branch
  in `surface_artefacts_of` that builds a minimal `Target`
  pre-loading `pubspec.yaml`, drives the subprocess, and decodes the
  JSON response into typed `Binding` / `LibraryApi` values via
  `decode_dart_surface_payload` (mirrors `decode_csharp_surface_payload`
  with `"dart"` as the default language tag).
- `crates/atlas-engine/src/manifest_parse.rs` gains
  `extract_pubspec_path_deps`: walks both `dependencies:` and
  `dev_dependencies:` for the `path:` short-form, skipping
  version-string / SDK / git forms.
- `crates/atlas-engine/src/root_expansion.rs` walks `pubspec.yaml`
  alongside `Cargo.toml` / `pyproject.toml` / `*.csproj`;
  `enclosing_manifest_root` recognises a `pubspec.yaml` ancestor as
  a project root.
- `ComponentKind::DartPackage` / `ComponentKind::FlutterPackage`
  variants (alongside the pre-existing `DartLibrary` / `DartApp` /
  `FlutterApp`) + `dart-package` / `flutter-package` entries in
  `defaults/component-kinds.yaml`.

**Binding extraction shape:**
- Top-level `class`/`mixin`/`enum`/`extension` declarations →
  `Binding` with `Visibility::Conventional` and an
  `attributes.private: true` flag if the symbol starts with `_`
  (Dart's leading-underscore privacy convention; uses PR-3's
  `ATTR_PRIVATE` const).
- Top-level `function`/`getter`/`setter` → same shape.
- Dart `@annotations` → `attributes.dart_annotations: ["override",
  "deprecated", ...]` ordered by source occurrence.
- Quality fix C-2 rejects top-level variable initialisers that look
  like function declarations (covers the `final x = () { ... }`
  edge case where the identifier on the LHS would otherwise be
  emitted twice).
- Quality fix C-1 captures the declaration-line span for binding
  `content_sha` (matches the C# / Python pattern).

**Tests:** all §4 PR-7 acceptance criteria + unit tests in the
analyser crate + classifier unit tests + integration tests under
`crates/atlas-engine/tests/l2_l3_queries.rs`. Listed by criterion:
1. `l3_pubspec_yaml_with_flutter_block_classifies_as_flutter_package_without_llm_call`
   (no LLM dispatch).
2. `l3_pubspec_yaml_without_flutter_block_classifies_as_dart_package_without_llm_call`
   (the discriminator).
3. `dart_underscore_prefix_function_records_conventional_private_attribute`
   (Phase 2 PR-3 conventional-private pattern reused).
4. `dart_override_annotation_recorded_in_attributes_dart_annotations_list`
   (annotation capture).
5. `pubspec_path_dep_expands_to_peer_root` and
   `pubspec_multiple_path_deps_expand_to_multiple_peer_roots`
   (cross-tree path-dep, F2 spec fix).

**Decisions / deviations:**
- **Parser library swap.** Plan named `tree-sitter-dart`; subagent
  used a hand-rolled tokeniser. Within the per-PR §4 latitude
  (mature pure-Rust alternatives are explicitly authorised). Trade-off:
  no full-grammar coverage of edge syntax (e.g. records, function
  types) but the surface symbols this PR cares about — top-level
  declarations — are scanned reliably. Phase 3 may revisit if the
  surface shape grows.
- **Subprocess does its own filesystem walk.** Mirrors PR-3 / PR-6
  pattern; `Target.manifests` carries `pubspec.yaml` only. Source
  file walk via `std::fs::read_dir` on `Target.dir`.
- **`flutter:` block discrimination is presence-based.** The
  classifier emits `flutter-package` whenever `flutter:` is a
  top-level mapping key in `pubspec.yaml`, regardless of contents.
  Distinguishing Flutter applications (`flutter-app`) from packages
  is left to the legacy heuristic / LLM path; the subagent
  considered overloading on `flutter.uses-material-design` etc. and
  declined as out of scope.

**Cleanups deferred:**
- Full `tree-sitter-dart` migration (Phase 3+) when surface shape
  needs records / function-type coverage.
- The `module_path` slot on Dart bindings is empty for now; resolving
  Dart's library + part declarations into a dotted module path is
  Phase 3+.
- `LenientBackend` test-helper extraction to `tests/common/mod.rs`
  (still warranted; cumulative deferral from PR-3 / PR-6 / PR-7;
  Wave 4 cleanup PR).

### PR-8
2026-05-08 — Landed as Atlas commit `d2afa16` (squashed integration of
`phase2-pr8-impl`: scaffold → wiring → tests → quality fixes
F-CQ-1/2/4/5 → cargo fmt). Atlas-contracts companion was landed before
the orchestrator merge as commits `2b3f558` (`ContractKind::Behaviour`
+ `ATTR_SPEC` / `ATTR_DOC` + `elixir-project` kind docstring) and
`2f7d41d` (re-export `ATTR_SPEC` and `ATTR_DOC` from atlas-index root)
on the atlas-contracts main; the orchestrator did NOT author a
PR-8 atlas-contracts companion. Per-stage commits preserved on the
branch ref `phase2-pr8-impl`.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/surfaces.rs` —
- `ContractKind::Behaviour` variant added to the enum (Phase 2's
  greenfield `kind` open vocabulary). Behaviour-shaped contracts
  (Elixir's `defprotocol` + `@callback` decls) flow through the
  existing `Contract` type with the new variant.
- `ATTR_SPEC` and `ATTR_DOC` const keys for `Binding.attributes`
  (re-exported from the atlas-index crate root). Wave 3 PRs that
  touch the same attribute namespace adopt the `ATTR_*` const-key
  convention PR-3 established.
- atlas-index `crates/atlas-index/src/schema.rs` `kind` docstring
  extended with a PR-8 paragraph (`elixir-project`).

**Parser library:** `tree-sitter` + `tree-sitter-elixir` 0.3.5
(latest published at the time of PR-8). The crate's Cargo.toml
originally pinned `tree-sitter = "0.24"` directly; the orchestrator
switched to `tree-sitter = { workspace = true }` (0.25 from PR-6) —
`tree-sitter-elixir-0.3.5` only takes a dev-dep on `tree-sitter`, so
the 0.25 runtime is fine and the `links = "tree-sitter"` duplicate
goes away.

**Implementation map:**
- New workspace member `crates/analyzers/elixir` (lib + `elixir-analyzer`
  subprocess binary), speaking PR-2's wire protocol verbatim.
- `atlas_analyzers::elixir_classifier` (in-process L3 deterministic):
  recognises `mix.exs` → `elixir-project`. Phase 2 lands the
  shape-agnostic `elixir-project` kind; finer sub-kinds (e.g.
  `phoenix-app`) are Phase 3+.
- `atlas_analyzers::elixir_surface_analyzer` —
  `elixir_subprocess_spec(binary_path)` builds the spec;
  `cached_elixir_subprocess_proxy(spec)` caches the proxy keyed on
  binary path; `locate_elixir_analyzer_binary()` walks up from
  `current_exe()`.
- `crates/atlas-engine/src/l5_surface.rs` extended with an Elixir
  branch in `surface_artefacts_of` that pre-loads `mix.exs`, drives
  the subprocess, and decodes the JSON response into typed `Binding`
  / `LibraryApi` / `Contract` values via
  `decode_elixir_surface_payload` — the only Wave-3 decoder so far
  that returns three vecs (the Elixir `defprotocol` / `@callback`
  pairs surface as `Contract`s with `ContractKind::Behaviour`).
- `crates/atlas-engine/src/manifest_parse.rs` gains
  `extract_mix_exs_path_deps`: regex-based scanner for the
  `path: "../sibling"` form inside `deps()` blocks. Pure-Rust;
  no Mix runtime invoked.
- `crates/atlas-engine/src/root_expansion.rs` walks `mix.exs`
  alongside the other path-dep-bearing manifests;
  `enclosing_manifest_root` recognises a `mix.exs` ancestor as a
  project root via the new `elixir_root` candidate (tail of the
  `workspace_root.or(crate_root).or(python_root).or(csharp_root)
  .or(dart_root).or(elixir_root)` precedence chain).
- `ComponentKind::ElixirProject` variant + `elixir-project` entry
  in `defaults/component-kinds.yaml`. `subcarve_policy` adds
  `ElixirProject` to the library-shaped kinds.

**Binding extraction shape:**
- `def name`/`defp name` → `Binding` with `Visibility::Explicit
  { keyword: "def" }` for public, `Visibility::Conventional` +
  `attributes.private: true` for `defp`.
- Elixir `@spec` / `@doc` module attributes captured as
  `attributes.spec: "..."` / `attributes.doc: "..."` via the new
  `ATTR_SPEC` / `ATTR_DOC` const keys (atlas-contracts).
- F-CQ-1 quality fix: `@spec` / `@doc` parsing handles the
  unary_operator AST node shape (where the LHS of `@` is wrapped in
  a unary tree); previously misparsed.

**Tests:** §4 PR-8 acceptance criteria 1–4 covered + classifier unit
tests + integration tests in
`crates/atlas-engine/tests/l5_elixir_surface.rs`.

**Decisions / deviations:**
- **`ad0cdbf` "DO NOT CHERRY-PICK" Cargo.toml repoint reverted.**
  The PR-8 worktree carried a transient repoint of the atlas-contracts
  workspace path-deps to a sibling worktree (`atlas-contracts-phase2-pr8`).
  The squash absorbed that diff; the orchestrator manually reverted to
  the canonical `../atlas-contracts/...` path-deps before committing.
- **No L6 `implemented_contracts` projection yet.** The elixir analyser
  emits `WireBinding.implemented_contracts` (populated by `@behaviour
  Foo.Bar` declarations) but the engine-side `SurfaceArtefacts` does
  not yet carry a structured `implemented_contracts` field. Decode
  silently drops the data; a future PR adds the field and the
  decode logic. Documented in the decoder body as a Phase-2 scope
  boundary.
- **Tree-sitter dep alignment.** Branch pinned `tree-sitter = "0.24"`;
  workspace is on 0.25 (PR-6). Switched the elixir crate to
  `tree-sitter = { workspace = true }` to satisfy the
  links="tree-sitter" single-copy constraint.

**Cleanups deferred:**
- L6 `implemented_contracts` projection (Phase 3+).
- Phoenix sub-kind discrimination (`phoenix-app` etc.) — Phase 3+.
- Mix umbrella-app shape (multiple sub-projects under `apps/`)
  is currently treated as a single `elixir-project`; Phase 3 may
  decompose into per-sub-app components.
- `LenientBackend` test-helper extraction (still warranted; cumulative
  deferral from PR-3 / PR-6 / PR-7 / PR-8; Wave 4 cleanup PR).

### PR-9
2026-05-08 — Landed as Atlas commit `cd2ee24` (squashed integration of
`phase2-pr9-impl`: scaffold → wiring → tests → spec fix F1
binary_sha contribution → quality fixes F-RKT-1/2/4 → cargo fmt);
atlas-contracts commit `86b5a28` (kind docstring gains racket-package
paragraph). Per-stage commits preserved on the branch ref
`phase2-pr9-impl`.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring extended with a
PR-9 paragraph (`racket-package`). No type-level schema changes.

**Parser library:** hand-rolled S-expression tokeniser shipped in
`atlas-racket-analyzer::lib`. The Phase 2 plan named no specific
library for Racket; the subagent chose a minimal byte-scan reader
because Racket's `provide`/`require` surface is regular enough that
a full parser is overkill. F-RKT-1 quality fix corrects the
tokeniser's handling of `#t`/`#f`/`#:keyword`/`#'` reader macros
(previously misparsed).

**Implementation map:**
- New workspace member `crates/analyzers/racket` (lib + `racket-analyzer`
  subprocess binary), speaking PR-2's wire protocol verbatim.
- `atlas_analyzers::racket_classifier` (in-process L3 deterministic):
  recognises `info.rkt` → `racket-package`.
- `atlas_analyzers::racket_surface_analyzer` —
  `racket_subprocess_spec(binary_path)` builds the
  `SubprocessAnalyzerSpec`. F-RKT-2 quality fix swapped the
  racket-specific cached-proxy helper for the shared
  `cached_subprocess_proxy` PR-3 introduced (eliminates per-language
  duplication).
- `crates/atlas-engine/src/l5_surface.rs` extended with a Racket
  branch in `surface_artefacts_of` that pre-loads `info.rkt`, drives
  the subprocess, and decodes the JSON response into typed `Binding`
  / `LibraryApi` values via `decode_racket_surface_payload`.
- `crates/atlas-engine/src/manifest_parse.rs` gains
  `extract_info_rkt_path_deps`: byte-scan finds the `(define deps
  ...)` form and extracts string literals starting with `.` or `/`
  (registry package names without separators are excluded).
- `crates/atlas-engine/src/root_expansion.rs` walks `info.rkt`
  alongside the other path-dep-bearing manifests;
  `enclosing_manifest_root` recognises an `info.rkt` ancestor as a
  project root via the new `racket_root` candidate.
- F-RKT-4 quality fix renamed `resolve_python_component_dir` to the
  language-agnostic `resolve_component_dir_first_segment` (re-used
  by both Python and Racket); the racket-specific
  `resolve_racket_component_dir` mirrors the csharp/dart/elixir
  pattern.
- `ComponentKind::RacketPackage` variant + `racket-package` entry
  in `defaults/component-kinds.yaml`. `subcarve_policy` adds
  `RacketPackage` to the library-shaped kinds.

**Binding extraction shape:**
- Top-level `(provide ...)` clauses → `Binding`s with
  `Visibility::Explicit { keyword: "provide" }`.
- `(define name ...)` → `Binding` with the symbol name; visibility
  derived from whether the symbol is in any `provide` form.
- `(struct name ...)` → recorded as `PubItemKind::Struct`.

**F1 spec fix (`19104b8`):** L5 fingerprint now contributes the
racket-analyzer binary's content sha via tag 0x06 — a rebuilt
binary therefore invalidates Racket-component cached
`SurfaceRecord`s, mirroring the Python / C# / Dart / Elixir
patterns.

**Tests:** §4 PR-9 acceptance criteria covered + classifier unit
tests + integration tests in
`crates/atlas-engine/tests/l5_racket_surface.rs`. F-RKT-1 added
regression tests for `#t`/`#f`/`#:keyword`/`#'` tokeniser correctness.

**Decisions / deviations:**
- **Hand-rolled reader** preferred over a tree-sitter-racket
  dependency. Within per-PR §4 latitude.
- **`provide` / `require` are the surface boundary.** Internal
  `define` forms not exported via `provide` are treated as
  conventional-private (Racket has no language-level private keyword;
  `provide` is the export gate).
- **No `raco` invocation.** The analyser does not shell out to the
  Racket toolchain; everything is pure-Rust byte-scanning. Phase 3
  may add raco-driven dependency resolution if the surface accuracy
  warrants it.

**Cleanups deferred:**
- Tree-sitter-racket migration (Phase 3+) if surface shape needs
  full grammar coverage.
- `(struct-out)` / `rename-out` / contract-out forms in `provide`
  blocks are currently treated as plain symbol exports; Phase 3 may
  capture the modifier semantics.
- `LenientBackend` test-helper extraction (still warranted; cumulative
  deferral; Wave 4 cleanup PR).

### PR-10
2026-05-08 — Landed as Atlas commit `16bbdb8` (squashed integration of
`phase2-pr10-impl`: scaffold → wiring → quality fixes
F-CQ-1 / F-CQ-2); atlas-contracts commit `f5f16c0` (kind docstring
gains lispkit-package paragraph). Per-stage commits preserved on
the branch ref `phase2-pr10-impl`.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring extended with a
PR-10 paragraph (`lispkit-package`).

**Parser library:** hand-rolled S-expression scanner in the
lispkit-analyzer crate. Plan §4 PR-10 noted the manifest convention
was TBD; subagent settled on R7RS-standard `*.sld` (Scheme Library
Definition) files containing `(define-library ...)` forms. The
classifier matches on file extension; the surface analyser parses
the `(define-library ...)` form to extract `(export ...)` clauses
as the public surface.

**Implementation map:**
- New workspace member `crates/analyzers/lispkit` (lib + `lispkit-analyzer`
  subprocess binary).
- `atlas_analyzers::lispkit_classifier` (in-process L3 deterministic):
  recognises `*.sld` → `lispkit-package`.
- `atlas_analyzers::lispkit_surface_analyzer` —
  `lispkit_subprocess_spec(binary_path)` builds the
  `SubprocessAnalyzerSpec`; `cached_lispkit_subprocess_proxy(spec)`
  caches the proxy keyed on binary path; `locate_lispkit_analyzer_binary`
  walks up from `current_exe()` for the runtime sibling-binary
  lookup.
- `crates/atlas-engine/src/l5_surface.rs` extended with a LispKit
  branch in `surface_artefacts_of` that pre-loads any `*.sld` file
  in the candidate dir, drives the subprocess, and decodes the JSON
  response into typed `Binding` / `LibraryApi` values.
- `crates/atlas-engine/src/manifest_patterns.rs` adds `*.sld` to
  the recognised manifest suffixes.
- `ComponentKind::LispkitPackage` variant + `lispkit-package` entry
  in `defaults/component-kinds.yaml`. `subcarve_policy` adds
  `LispkitPackage` to the library-shaped kinds.

**F-CQ-1 quality fix (`ed38d5a`):** dropped a dead `BASE64` import
and the unused `base64` dep from the lispkit crate's Cargo.toml
(the subprocess binary doesn't carry binary blobs in the wire
payload — only JSON-serialisable text).

**F-CQ-2 quality fix (`03c1186`):** generalised
`decode_python_surface_payload` to
`decode_subprocess_surface_payload(payload, component_id,
default_language)`, factoring out the language-defaulting parameter.
The Python and LispKit branches now both call the shared decoder
(passing `"python"` and `"scheme"` respectively). The C# / Dart /
Elixir / Racket decoders kept their language-specific names — Wave
4 cleanup may consolidate them onto the shared helper.

**Tests:** §4 PR-10 acceptance criteria covered + classifier unit
tests + integration tests in
`crates/analyzers/lispkit/tests/integration_test.rs`.

**Decisions / deviations:**
- **Manifest convention is `*.sld`.** Plan §4 PR-10 left the
  convention TBD between `package.scm` and `*.sld`; subagent picked
  the R7RS-standard `*.sld` form because LispKit (the Linkuistics
  project's Swift Scheme interpreter) follows R7RS. Recognised by
  file extension rather than basename.
- **No `info.rkt`-style path-dep walker.** `*.sld` files don't
  declare external path-deps in a regular form; LispKit's
  `(import ...)` clauses name libraries by symbol, not path. The
  cross-tree fixed-point walker is therefore a no-op for LispKit
  components — they live in their primary root only. Phase 3 may
  add a symbolic-import resolver if real-world LispKit projects
  need cross-tree composition.
- **Hand-rolled scanner** rather than a tree-sitter-scheme dep.
  Within per-PR §4 latitude.

**Cleanups deferred:**
- Cross-tree `(import ...)` resolution (Phase 3+).
- C# / Dart / Elixir / Racket decoder consolidation onto the shared
  `decode_subprocess_surface_payload` helper (F-CQ-2 generalisation;
  Wave 4 cleanup PR).
- `LenientBackend` test-helper extraction (still warranted; cumulative
  deferral; Wave 4 cleanup PR).

### PR-11
2026-05-07 — Landed as Atlas commits `4da5209` (main implementation) +
`7aacbea` (spec fix F1: lex-sort composition edges in `canonicalise_edges`) +
`69b2309` (quality fix F-CQ-1: extract shared L6 path utilities into
`l6_paths.rs`) + `e5f1260` (quality fix F-CQ-2: trim trailing hyphen after
slug truncation).

**2026-05-08 deferred-companion update:** atlas-contracts commit
`6708155` lands the `compose-orchestration` paragraph in the `kind`
field's open-vocabulary docstring (deferred from PR-11's original
session per the original status note's "schema-mutation contribution
proposal (orchestrator integrates)" framing).

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring:
- `compose-orchestration` — A Docker Compose file declaring an orchestrated
  set of services. Produces `bundled-into` (image/build → this) and
  `deployed-with` (between co-declared service sources) edges at L6.

**Implementation:** in-process deterministic analyser (no LLM). Two new
files: `crates/atlas-analyzers/src/compose_classifier.rs` (`ComposeClassifier`,
L1 enumeration to seed deliverable candidates; L3 classification; `ComposeShape` /
`ComposeService` types; `parse_compose()`) and `crates/atlas-engine/src/l6_compose_edges.rs`
(`composition_edges_from_compose`: emits `bundled-into` from `image:`/`build:`
to compose-orchestration; `deployed-with` between co-declared services).
Glob coverage for `docker-compose.yml`, `docker-compose.*.yml`, `compose.yml`,
`compose.*.yml`. External-component id scheme: `external-<slug>` with
non-alphanumeric→hyphen, 64-char truncation, leading/trailing hyphen trim
(F-CQ-2 fix moves trim to *after* truncation). Local docker-image matching
by component-id leaf with tie-break to external fallback on collision.

**F1 spec fix (`7aacbea`):** spec required compose- and Dockerfile-derived
composition edges to be "interleaved … in lexicographic order". Original
`all_proposed_edges` extended in fixed order (Dockerfile-then-Compose).
Fix added a stable sort by canonical key `(kind, lifecycle, participants)`
to `canonicalise_edges` *before* the dedupe loop, using the same comparator
as `l9_projections.rs::related_components_yaml_snapshot`. Stable sort
preserves first-insertion-wins for the dedupe (so contract edges still
beat composition edges, composition edges still beat LLM edges, where
their canonical keys collide).

**F-CQ-1 quality fix (`69b2309`):** five path-utility helpers
(`component_id_leaf`, `build_component_segment_dirs`, `absolute_under_any_root`,
`absolute_component_dir`, `path_prefix_lookup`, `normalise_path`) were
duplicated verbatim between `l6_compose_edges.rs` and `l6_composition.rs`.
Extracted into a new `pub(crate)` module `crates/atlas-engine/src/l6_paths.rs`;
both call sites now import from there. Net −187 / +130 lines (57 lines of
duplication eliminated).

**Tests:** 5 integration tests in `crates/atlas-engine/tests/compose_edges.rs`
covering all spec acceptance criteria + 2 negative-path tests (no compose
files; single-service no `deployed-with`). Plus a `external_component_id_no_trailing_hyphen_after_truncation`
regression test pinning F-CQ-2.

**Edge interleaving (post-F1):** all proposed edges flow through `canonicalise_edges`
which now lex-sorts by `(kind, lifecycle, participants)` before dedupe.
Output ordering is deterministic and matches §4 PR-11's "interleaved … in
lexicographic order" requirement.

**Deferred / follow-ups:**
- `DockerComposeBundle` (existing Phase 1 kind) coexists with the new
  `ComposeOrchestration`. No producer for `DockerComposeBundle` after
  PR-11 — it's reachable only via pins/overrides/LLM responses. Safe
  coexistence; cleanup deferred to Phase-3 sweep.
- `WireLibraryApi.kind` field is hardcoded `"library-api"` on the wire
  but ignored by the engine decoder (pre-existing PR-3 debt; not PR-11).

### PR-12
2026-05-08 — Landed as Atlas commit `1a08c4a` (squashed integration of
`phase2-pr12-impl`: vocabulary → analyser → registry+lib.rs →
engine wiring → tests/validate → spec fix F1 (Confidence::Declines
threshold gate at 0.6) → quality fix F-CQ-1 (hand-written Default
delegates to `new()` to preserve the threshold gate));
atlas-contracts commit `b2b53d8` (kind docstring gains
shell-script / makefile-orchestration paragraph). Per-stage commits
preserved on the branch ref `phase2-pr12-impl`.

**Schema-mutation contribution:** atlas-contracts
`crates/atlas-index/src/schema.rs` `kind` docstring extended with a
PR-12 paragraph (`shell-script` and `makefile-orchestration`).

**Implementation map:**
- `atlas_analyzers::shell_script_llm_analyzer` (new module): the
  `ShellScriptLlmAnalyzer` registers as the 8th built-in (now 14th
  after Wave-3 PRs 6/7/8/9/10 added their classifiers);
  `extract_shell_surface` is the L5-side deterministic extractor
  (function definitions / Make targets) used by the
  `entry_is_shell` branch in `surface_artefacts_of`.
- `atlas_analyzers/Cargo.toml` gains a regex dep used by the shell
  parser.
- `crates/atlas-engine/src/l3_classify.rs` extended with a
  `shell_to_classification` adapter that materialises the L3 LLM
  output into a `Classification` with `EvidenceGrade::Medium`
  (conservative for LLM verdicts) and `analyser_id` /
  `analyser_version` plumbing per PR-4.
- `crates/atlas-engine/src/l5_surface.rs` extended with a Shell
  branch in `surface_artefacts_of` that probes well-known shell /
  Makefile filenames (`Makefile`, `GNUmakefile`, `*.sh`, plus
  `*.mk` from `entry.manifests`), passes them to
  `extract_shell_surface`, and returns the binding / library-api
  vecs unchanged.
- `crates/atlas-cli/src/validate.rs` learns about the new
  ComponentKind variants.
- `ComponentKind::ShellScript` / `MakefileOrchestration` variants +
  `shell-script` / `makefile-orchestration` entries in
  `defaults/component-kinds.yaml`. `subcarve_policy` adds both as
  Stop-leaf kinds (one component end-to-end, never carve inside).

**F1 spec fix (`3c31afe`):** the analyser previously emitted
`Graded` outputs unconditionally; spec required gating on the LLM
confidence (`Confidence::Declines` below 0.6). The fix splits the
threshold check into a `should_emit_graded` helper and routes
sub-threshold outputs into the `Declines` arm of `AnalyzerResult`,
which the dispatcher then falls through.

**F-CQ-1 quality fix (`aed7295`):** the hand-written `impl Default
for ShellScriptLlmAnalyzer` previously short-circuited the threshold
gate (it set `confidence_threshold` to `0.0`, which silently
emitted Graded outputs for every LLM verdict). The fix delegates
`Default::default()` to `Self::new()` so the gate stays at the
class-default `0.6`.

**Tests:** §4 PR-12 acceptance criteria covered + integration
tests in `crates/atlas-engine/tests/l5_shell_surface.rs`.

**Decisions / deviations:**
- **In-process, not subprocess.** Phase 2 contract: subprocess
  analysers are deterministic-only. The shell-script analyser
  needs LLM access (Phase 1 PR-5's `LlmHook`), so it stays
  in-process. Phase 3+ may add a bidirectional callback channel
  for subprocess analysers that need LLM dispatch.
- **`EvidenceGrade::Medium` for LLM verdicts.** The other Wave-3
  classifiers emit `EvidenceGrade::Strong` (their inputs are
  deterministic manifest signals). PR-12's verdict comes from the
  LLM, so the conservative `Medium` grade tells downstream stages
  that a hand-authored override / pin should win over this verdict
  at parity weight.
- **Probe-list rather than full directory walk.** `entry_is_shell`
  components' surface candidates are enumerated from a fixed list
  of well-known shell / make filenames plus any `*.mk` files
  pre-loaded into `entry.manifests`. Phase 3 may swap to a full
  walk if real-world fixtures need it.

**Cleanups deferred:**
- Full directory walk for shell components (Phase 3+).
- `LenientBackend` test-helper extraction (still warranted; cumulative
  deferral; Wave 4 cleanup PR).

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
2026-05-08 — Landed as Atlas commit `718ca88` (squashed integration of
worktree branch `phase2-pr14-impl`). Per-stage history preserved on the
branch ref:
- `2ece907` PR-14 deviation — Dockerfile.<suffix> recognition for *.buildkite.
- `5fba23c` PR-14 acceptance — polyglot smoke test (Wave 4 final).

No atlas-contracts companion (PR-14 is end-to-end test only; every kind
already in the contracts vocabulary from prior PRs).

**Schema-mutation contribution:** none (test-only PR plus the
Dockerfile.<suffix> production fix; no schema changes).

**Implementation map:**
- New file `crates/atlas-cli/tests/phase2_polyglot_fixture.rs` (715
  LOC) — three end-to-end acceptance tests + the `PR14Backend` lenient
  recording backend. Tests:
  1. `polyglot_fixture_classifies_all_components_and_emits_expected_edges`
     — covers AC#1 (every component classified to its kind), AC#2
     (non-empty surfaces.yaml), AC#3 (edge counts: ≥2 bundled-into from
     compose, ≥2 deployed-with, ≥3 bundled-into from Dockerfiles, ≥1
     defines-contract from Elixir behaviour, ≥1 consumes-contract from
     path-deps).
  2. `polyglot_no_op_rerun_is_zero_llm_calls` — covers AC#4 (warm rerun
     produces zero LLM calls; cold run records 26 LLM calls, all
     L6 Stage2Edges + Subcarve, none Classify/Stage1Surface).
  3. `polyglot_targeted_edit_invalidates_only_affected_entries` — covers
     AC#5 (editing one component's source invalidates only its L5 +
     consumers' L6; cold=26 calls, post-edit=2 calls).
- New 30-file hermetic fixture under
  `crates/atlas-cli/tests/fixtures/phase2_polyglot/`: csharp_lib (csproj +
  Csharp/Lib.cs), dart_lib (pubspec + lib/dart_lib.dart), flutter_app
  (pubspec with `flutter:` block + lib/main.dart), ts_pkg + js_pkg, py_pkg
  (pyproject + pkg/__init__.py + pkg/mod.py), ex_app (mix.exs +
  lib/ex_app.ex declaring a `defprotocol`), rkt_pkg (info.rkt + main.rkt),
  lk_pkg (package.scm + main.sld), rust_lib (Cargo.toml + src/lib.rs with
  nested `pub mod foo { pub struct Bar; }` to verify PR-5's syn
  extractor), three Dockerfiles (`Dockerfile`, `Dockerfile.frontend.buildkite`,
  `Dockerfile.backend.buildkite`), two compose files (each in its own
  dir to satisfy the ≥2 deployed-with edges requirement: `compose/` and
  `compose-proxy/`), and `build_glue/Makefile` + `scripts/deploy.sh`.

**Production fixes (intrinsically tied to the integration test, ~131
insertions / 14 deletions across 3 files):**
- `crates/atlas-engine/src/manifest_patterns.rs` — new
  `is_dockerfile_basename` recognising `Dockerfile`, `Dockerfile.<suffix>`,
  and `Containerfile`; plumbed into `is_manifest_file`.
- `crates/atlas-analyzers/src/dockerfile_classifier.rs` — `applies` /
  `fingerprint_inputs` / `analyse` widened to find any Dockerfile-shaped
  manifest (lex-first order for determinism), not just the bare basename.
- `crates/atlas-engine/src/l6_composition.rs` — new `locate_dockerfile_in_dir`
  helper so composition-edge emission flows through `*.buildkite`-suffix
  files. All five existing `compose_edges` tests + five `composition_edges`
  tests stay green; no regressions.

**LLM call accounting:**
- Cold run: 26 LLM calls (Stage2Edges batch + Subcarve calls per L8
  candidate). Zero Classify or Stage1Surface calls — the 10 deterministic
  language analysers + Dockerfile + Compose all classify without LLM
  consultation, and the surface analysers are deterministic.
- Warm rerun: **0 LLM calls** (100% persistent-cache hit) — the load-bearing
  invariant.
- Post-edit run (touching `ex_app/lib/ex_app.ex`): 2 LLM calls (only
  ex-app's slice + L6 batch reshape).

**Component count: 17 (vs the brief's 16):**
- 10 language components: csharp-project, dart-package, flutter-package,
  typescript-package, javascript-package, python-package, elixir-project,
  racket-package, lispkit-package, rust-library.
- 3 docker-image components (one per Dockerfile, including the two
  `*.buildkite`-suffix ones).
- 2 compose-orchestration components (one per compose dir; PR-11's
  classifier emits one orchestration per dir, not per file).
- 1 makefile-orchestration + 1 shell-script (both injected via
  `OverridesFile.additions`, matching PR-12's own integration-test pattern
  in `l5_shell_surface.rs` — PR-12's `applies()` predicate gates on the
  file living in `target.manifests`, which requires the basename in
  `manifest_patterns::is_manifest_file`; neither `Makefile` nor `*.sh`
  are listed there. The deeper fix is Phase 3 scope.)

**Decisions / deviations:**
- **Dockerfile.<suffix> production fix (~131 LOC).** ~3x the Phase 1
  PR-12 deviation precedent of 37 LOC, but well within the brief's
  "2x of PR estimate" scope-blowup limit (PR-14 total: 846 LOC vs
  1000-1500 estimate). Intrinsically required: without it, no
  `bundled-into` edges flow from `*.buildkite` Dockerfiles, which is a
  PR-14 acceptance criterion.
- **17 components vs brief's 16.** Documented in the test's module
  docstring; root cause is PR-11's compose-orchestration-per-dir
  semantic (not per-file).
- **Shell-script + makefile-orchestration via `OverridesFile.additions`,
  not auto-discovery.** Matches PR-12's own integration-test pattern.
  Phase 3 may extend `is_manifest_file` to recognise Makefile / *.sh.
- **JS source moved to `js_pkg/src/index.js`** from the brief's
  `js_pkg/index.js` — PR-1's L5 driver only probes
  `src/index.{ts,tsx,js,jsx}`. Test-side adjustment, no production
  change.

**Cleanups deferred (cumulative across Wave 3 + Wave 4):**
- `LenientBackend` test-helper extraction (deferred from PR-3 → PR-6 →
  PR-7 → PR-8 → PR-9 → PR-10 → PR-14; now seven copy sites).
- C# / Dart / Elixir / Racket subprocess decoders consolidation onto
  the shared `decode_subprocess_surface_payload` helper (introduced in
  PR-10's F-CQ-2 generalisation).
- `is_manifest_file` extension to recognise `Makefile`/`GNUmakefile`/`*.mk`/`*.sh`/`*.bash`/`*.zsh`
  so PR-12's auto-discovery path covers shell-script components without
  `OverridesFile.additions` injection.
- Per-language Phase 3 cleanups (full tree-sitter-dart, raco-driven
  Racket dep resolution, Phoenix sub-kinds for Elixir, Mix umbrella
  decomposition, LispKit `(import ...)` symbolic resolution, etc. —
  enumerated in each language PR's per-PR note).

**Phase 2 closeout:** all 15 PRs (PR-0..PR-14) [x]. The continuation
prompt's Step 3 termination condition is met — next paste enters Step 1
again, detects Phase 2 closed, and either reports completion (if no
Phase 3 plan exists) or pivots to Phase 3 planning.
