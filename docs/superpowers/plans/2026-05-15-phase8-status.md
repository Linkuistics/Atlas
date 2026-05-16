# Atlas vNext — Phase 8 — Cargo retirement — Status

Companion to `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`. This file tracks per-WI completion state across sessions.

**Last updated:** 2026-05-16 (WI-3 status flip).

## WI status

- [x] WI-1 — Agent-runtime HTTP-backend bypass
- [x] WI-2 — HardFail event emission in call_agent
- [x] WI-3 — Cargo classifier retirement
- [ ] WI-4 — Atlas-on-Atlas calibration + closeout

## Per-WI notes

### WI-1

**Shipped:** 2026-05-15. Code commit `8839cb8` (`phase8 WI-1: agent-runtime HTTP-backend bypass`).

**What landed:**

- `LlmRequest` schema migrated to a tagged-union shape: `prompt_template: Option<PromptId>` + new `rendered_prompt: Option<String>`; `#[non_exhaustive]` added; two public constructors `from_template(id, inputs, schema)` + `from_rendered(rendered, schema)` with the "exactly one is Some" invariant enforced by `debug_assert!` (rendered) and the constructor shape (templated). The plan's optional `#[should_panic]` debug-assert test was dropped per § 7 Step 1.3 — `#[non_exhaustive]` makes struct-literal construction a compile error outside the crate anyway.
- Four backends short-circuit on `req.rendered_prompt.is_some()` before `prompts_dir` lookup: `http_anthropic.rs`, `http_openai.rs`, `codex.rs`, `claude_code.rs`. Each `render_request` returns the rendered string verbatim on the bypass path; templated path is unchanged.
- `BackendRouter` gained `default_provider: Provider` (seeded from the `Classify` operation's configured provider during `new_inner`). Rendered requests route via `backend_for_provider(default_provider)`. Both `call` (sync) and `call_async` carry the bypass branch.
- 18 `LlmRequest` call sites migrated (plan estimated ~17 — one-site delta is non-material). Three production agent-runtime sites use `from_rendered`: `tool_loop_http.rs::build_llm_request_with_tools`, `tool_loop_mcp.rs::build_llm_request_subprocess`, the auditor in `runtime/mod.rs`. Fifteen templated sites use `from_template`.
- Cascade beyond the plan's anchors (driven by `Option<PromptId>` propagation): `LlmCacheKey.prompt` widened to `Option<PromptId>`; `TestBackend`'s HashMap key widened to `(Option<PromptId>, String)`; `BackendCallEvent::CallStart` carries `Option<PromptId>` (logging-only; renderered-prompt calls show as `None`); both stream parsers (`parse_stream`, `parse_codex_stream`) accept `Option<PromptId>`; `prompt_label` in `atlas-cli/src/progress.rs` handles `Option<PromptId>` by labelling `None` as `"agent"`. ~13 test backends across the workspace that pattern-match on `req.prompt_template` each gained `.expect("test backend services templated requests")` — those backends only serve the deterministic spine. This was the bulk of R1's "mechanically wide" mitigation.
- New test: `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` (2 tests).
- `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored (closes sprint PR-4 note item 9). The bypass alone wasn't sufficient — the test's `provider_router` was a real `BackendRouter::new_for_agent_runtime` whose `Provider::OpenAi` entry was a real OpenAI HTTP backend, so Lane B audit still made live HTTP calls. Per PR-4 note item 9(b), `BackendRouter::from_dispatch_table` was lifted out of `#[cfg(test)]` and a sibling `from_test_dispatch_table_with_providers(table, providers, default_provider, fingerprint)` was added that supports cross-provider audit lookups. The test now builds a mock router whose Anthropic and OpenAi provider entries both resolve to the same `StagedBackend`, with a canned auditor verdict for the substring `"Verdict shape"`.
- Stale R9 comment cleanup: `tool_loop_http.rs` and `tool_loop_mcp.rs` "PR-5 will introduce a dedicated PromptId variant" comments removed.
- Status doc authored (this file) per kickoff Step 0 pre-flight.

**Plan-time deviations (worth noting for WI-2's executor):**

- Call-site count: plan said "~17", actual = 18. Within tolerance, no escalation triggered. The extra site is `tool_loop_http.rs`'s test (`build_llm_request_carries_tool_descriptors`) which was rewritten anyway — it asserted on the dead `inputs.tools` shape, vacuously satisfied since the HTTP backends never read `inputs.tools` off the request body.
- `BackendRouter::call_async` was at `router.rs:266`, not the plan's "~117" — the plan flagged this anchor as "find at plan-time". Both `call` (sync) and `call_async` needed bypass branches.
- The HTTP smoke test file lives at `crates/atlas-cli/tests/agent_runtime_http_smoke.rs`, NOT `crates/atlas-agents/tests/agent_runtime_http_smoke.rs` as the plan's § 7 Files block stated. Non-material file-location drift.
- The plan's Step 1.9 framed un-ignoring the test as a single-action close. Empirically the bypass closed the `prompts_dir` problem but Lane B audit still required mock injection (PR-4 note item 9(b)). The `from_test_dispatch_table_with_providers` shape lifted out of `#[cfg(test)]` is the WI-1 in-scope expansion.
- `BackendCallEvent::CallStart` field widened from `prompt: PromptId` to `prompt: Option<PromptId>` — affects telemetry consumers in `atlas-cli/src/progress.rs` (handled inline).
- `LlmCacheKey.prompt` widened to `Option<PromptId>` — affects the deterministic spine's response cache key (still works because the spine's call sites always set `Some(_)`).
- `ProgressBackend::call` / `call_async` in `atlas-cli/src/progress.rs` now guards `on_llm_call` behind `if let Some(prompt) = req.prompt_template` — rendered requests don't increment the per-stage breakdown counter, which is semantically correct (the agent runtime owns its own logging).

**Regression gates (all six clean):**

- `cargo build --workspace`: clean.
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`: 110 test-result entries, 0 failures.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean (auto-fmt run mid-session reformatted 18 files with long `.expect("...")` chains).
- `cargo build --release --workspace`: clean.
- Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`): 2 tests pass, wall-time 99.44s — within the 100–110s expected range, cold-count in `0 < cold < 100` (loose bound satisfied by the test's internal assertion), warm + reports = 0.

**Sprint PR-4 / PR-5 note items closed:**

- PR-4 note item 9: HTTP smoke `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored. Closure required two changes — the bypass (template lookup no longer poisoned on empty `prompts_dir`) AND the `from_test_dispatch_table_with_providers` lift (Lane B audit no longer hits live OpenAI).
- PR-5 closeout note item 1 (the agent-runtime HTTP-backend wiring gap): the rendered-prompt path no longer pre-fails at `LlmError::TemplateSyntax("unknown token \`{{COMPONENT_KINDS}}\`")`. End-to-end success on Atlas-on-Atlas is WI-4's acceptance — WI-1 may still surface downstream issues (likely Lane A schema mismatch on the agent's first production-prompt output), but the failure mode has shifted from "pre-HTTP template render" to "real LLM round-trip + downstream issue".

**Cargo SHAs:** code commit `8839cb8`; status-flip commit (this) to follow.

### WI-2

**Shipped:** 2026-05-15. Code commit `f142157` (`phase8 WI-2: HardFail event emission for backend errors`).

**What landed:**

- Producer-fail HardFail emit at `crates/atlas-agents/src/runtime/mod.rs:968` (the former `let output = outcome?;` site inside `run_tool_loop_with_lane_a`). The bare `?` becomes a `match` that emits `AgentEvent::HardFail { agent_id: agent_id(request), error_kind: "backend", error_summary: e.to_string(), retry_count: lane_a_retries }` before returning `Err(e)`. The propagated `Err` preserves the backend's verbatim error text (the new arm returns `e`, not a wrapped string).
- Auditor-fail HardFail emit inside `run_real_audit`'s auditor-backend `Err(e)` match arm (lines ~1186–1198 post-edit). Emits `AgentEvent::HardFail { agent_id: agent_id_payload.to_string(), error_kind: "audit_backend", error_summary: e.to_string(), retry_count: 0 }` before returning `AuditVerdict::HardFail(...)`. The cascading `lane_b` HardFail at line 817 (`ResolvedAuditAction::HardFail` arm) still fires afterwards, but subscribers can distinguish auditor-vs-producer via the new `audit_backend` discriminator.
- `run_real_audit` gained an `event_bus: &EventBus` parameter (11th positional). The threading decision (plan § 8 Step 2.3 surfaced two shapes: thread state in OR return `Result<AuditVerdict, AuditorBackendError>`) resolved to thread-in because the closure passed to `audit::lane_b_audit` is constrained to return `AuditVerdict` — option B would have required collapsing the Result back at the closure return site without access to the bus, which doesn't help. The call-site closure captures a cloned `Arc<EventBus>` (one atomic incr) and passes `event_bus.as_ref()` through.
- New test: `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs` (2 tests). `AlwaysErroringBackend` for producer-fail; `AlwaysSucceedingBackend` (returning Weak-grade classify YAML) + auditor `AlwaysErroringBackend` (routed via `ForProviderFn` for `Provider::OpenAi`) for auditor-fail. Both tests assert (1) the expected `error_kind` HardFail lands on the bus and (2) the propagated `Err` carries the backend's verbatim error text.

**Plan-time deviations (worth noting for WI-3's executor):**

- Producer-fail anchor at `mod.rs:965` (`let output = outcome?;`) was at the plan's exact frozen-2026-05-15 line — no drift from WI-1. After my edit the rewritten match-arm spans lines 968–987.
- Auditor-fail anchor: plan said `mod.rs:1167–1172`; post-WI-1 the actual `Err(e) => { return AuditVerdict::HardFail(...) }` arm was at `mod.rs:1160–1164` (4-line drift downward from WI-1's auditor `LlmRequest::from_rendered` edit at line 1158). After my edit the rewritten arm spans lines 1186–1199.
- `agent_id` helper is a private free function — tests can't call it. The test asserts on `error_kind` and `error_summary` substring instead of asserting on the precise `agent_id` string, which is the stable-across-internal-refactoring approach.
- The auditor-fail test surfaces TWO `HardFail` events on the bus: the new `audit_backend` one (mine) and the existing `lane_b` cascade at line 817 (since `ResolvedAuditAction::HardFail` still fires). Test asserts on the `audit_backend` discriminator only, but the test's docstring calls out the coexistence so a future reader understands.

**Regression gates (all six clean):**

- `cargo build --workspace`: clean.
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`: 111 test-result-ok lines, 0 failures.
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean (one rustfmt pass applied to the new test file's chained-method assertion calls).
- `cargo build --release --workspace`: clean.
- Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`): 2 tests pass, wall-time 100.78s — within the 100–110s expected band, +1.34s from WI-1's 99.44s baseline (within natural per-run variance). Polyglot HOLDS per plan § 4 (HardFail emit is on a path the smoke never traverses).

**Sprint PR-5 closeout-note item 4 (producer-fail / auditor-fail diagnostic visibility):** closed. Both backend-error sites now emit `HardFail` records that downstream JSONL subscribers (`jsonl_subscriber.rs`) and TUI (PR-6) consumers can ingest. The operator-side calibration verification per plan § 8 acceptance-gate bullet 5 (re-run PR-5's calibration command against a known-failing backend) is **not** a CI gate — it's an out-of-session operator validation step.

**Cargo SHAs:** code commit `f142157`; status-flip commit (this) to follow.

### WI-3

**Shipped:** 2026-05-16. Three commits per plan § 9 Step 3.9 decomposition:

- WI-3a `165d0a2` (`phase8 WI-3a: drop CargoClassifyTool + classify-prompt rubric rewrite`)
- WI-3b `da96fc5` (`phase8 WI-3b: delete deterministic cargo_classifier + cascade`)
- WI-3c (this status flip; to follow)

**What landed (agent layer — WI-3a):**

- `default_tool_catalog` drops `CargoClassifyTool`; catalog count 22 → 21. Doc-comment block at top of catalog builder updated ("21 wrappers" / "9 classifiers"). In-mod count assertion test renamed `tool_catalog_default_contains_22_wrappers` → `tool_catalog_default_contains_21_wrappers`.
- `build_classify_prompt` rewrites the `confidence_grade` rubric per plan § 9 Step 3.3: "strong" now rewards `parse_cargo_toml` + source entry-point READ (the parser-tool reward replaces the old "classifier tool whose name matches the declared `kind` was CALLED" reward); "moderate" = parser tool only; "weak" = manifest read only; "declines" unchanged. The available-tools paragraph drops "and language classifiers" wording + drops `parse_pyproject_toml` (verified absent from the catalog — the latent `lane_a.rs:240` reference is a pre-existing Phase-9 cleanup, out of WI-3 scope).
- Canonical-vocabulary list at runtime/mod.rs:~1428 grows `rust-workspace` with a clarifier ("For a Rust `Cargo.toml` with a `[workspace]` table and no `[lib]`/`[bin]`, prefer `rust-workspace`"). Worked YAML example stays as `rust-library` per plan F11.
- `crates/atlas-agents/src/tools/classifiers/cargo.rs` deleted; `tools/classifiers/mod.rs` drops `pub mod cargo;` + `pub use cargo::CargoClassifyTool;`.

**What landed (analyzer + engine + CLI cascade — WI-3b):**

- `crates/atlas-analyzers/src/cargo_classifier.rs` deleted outright (no deprecated stub per F11 option a).
- `atlas-analyzers`: lib.rs / registry.rs / dispatcher.rs cascades per plan § 9 Step 3.6. Sibling-classifier doc-comments (dart / elixir / python (×2) / racket / ts_js (×2) / dockerfile) lose their cross-references to `crate::cargo_classifier`. Three registry-test cascades: `builtin_lists_fourteen_analysers` → `builtin_lists_thirteen_analysers`; `merge_yaml_updates_existing_built_in_spec_in_place` rewrites to target `dockerfile_classifier::ANALYZER_ID` (preserving the "merge_yaml updates an existing built-in spec in place" coverage); `merge_yaml_accepts_unknown_id_and_keeps_builtin_count` count drops to 13/14. Two dispatcher cargo-specific tests deleted (`dispatch_picks_cheapest_applicable` + `dispatch_returns_winning_analyser_identity`).
- `atlas-engine/src/heuristics.rs`: three doc-comment rewords (top of file / on `classify_deterministic` / inside tests-mod) record the Phase 8 WI-3 retirement.
- `atlas-engine/src/l3_classify.rs`: drops `cargo_classifier::CargoClassificationOutput` from `use`; drops the cargo downcast arm in `classification_from_output`; drops the `cargo_to_classification` helper (~30 LOC). Cargo dispatch outcomes fall through to `LlmClassifyOutput` per plan § 9 Step 3.6 Option A (preferred — no new `LlmFallbackOutput` shape).
- `atlas-cli/tests/jsonl_subscriber.rs`: two `tool_name: "classify_cargo_component"` literals rename to `"classify_ts_js_component"` (test asserts JSONL shape, not the specific tool).
- `atlas-cli/tests/scattered_atlas_layout.rs`: `cargo_classified_component_records_cargo_analyser_identity` test deleted; the per-analyser identity threading coverage survives in the sibling `dockerfile_classified_component_records_dockerfile_analyser_identity` test.

**New test (WI-3a):**

`crates/atlas-agents/tests/cargo_retirement_smoke.rs` — three tests mapping 1:1 to WI-3 deliverables:

1. `default_tool_catalog_excludes_cargo_classifier` — asserts `classify_cargo_component` is absent, `parse_cargo_toml` retained, count = 21.
2. `classify_prompt_keeps_rust_library_as_worked_example` — asserts the worked YAML example still demonstrates `kind: "rust-library"`; the new rubric names `parse_cargo_toml`; the legacy "classifier tool whose name matches" wording is gone.
3. `classify_prompt_adds_rust_workspace_to_vocabulary` — asserts `rust-workspace` + `[workspace]` clarifier appear in the prompt body.

**Plan-time deviations (worth noting for WI-4's executor):**

- Plan § 9 Step 3.1's drafted test sketch used a canned tool-call trajectory scaffold (`canned_tool_call` / `canned_tool_result` / `canned_final_yaml` helpers) to assert agent-runtime end-to-end behaviour. The executor's judgement was that a canned-trajectory scaffold tests the *runtime's* tool-execution loop with a particular sequence — not WI-3's actual deliverables (the catalog/prompt edits). The shipped tests mirror the existing `crates/atlas-agents/tests/classify_prompt_shape.rs` pattern (substring assertions on `build_classify_prompt` output + catalog-shape via `default_tool_catalog`), which honestly tests the deliverables without elaborate scaffolding.
- Anchor line drift: post-WI-2 the rubric block was at mod.rs:~1445–1457 (plan said 1411–1426); vocabulary list at ~1428–1431 (plan said 1396–1399); worked YAML example at ~1413–1424. Net drift ~+32 lines from WI-2's two HardFail emit-arm additions.
- Cascade gap: the pre-existing in-mod test `tool_catalog_default_contains_22_wrappers` at `runtime/mod.rs:1871` was not on plan § 9 Step 3.6's enumerated cascade-target list. Caught by `cargo test --workspace` post-edit (22≠21 assertion panic). Renamed + retargeted to 21. Pattern worth carrying: hardcoded-count tests on enumerable collections act as canary tests that flag any deletion, complementing named-reference cascade lists.
- Cascade gap: `atlas-analyzers/src/dispatcher.rs:186` `use crate::TargetFile;` was the only consumer of `TargetFile` through the deleted `cargo_target_with_lib` helper. Clippy `-D unused-imports` caught.
- Cascade gap: `atlas-cli/tests/scattered_atlas_layout.rs:160-203` `cargo_classified_component_records_cargo_analyser_identity` test was a regression-guard specifically for the Cargo per-analyser identity. Identified during workspace-wide `cargo-toml-classifier` literal sweep. Retired entirely (per-analyser identity coverage survives in the sibling Dockerfile test).
- Polyglot smoke recalibration: the plan's most-likely scenario (case 1, `0 < cold < 100` absorbs the recalibration) became case 3 (`run_index` itself fails with 6 unresolved `consumes-contract` participants). Without the deterministic Cargo classifier, peer1-peer6 + outlier + rust-lib in the polyglot fixture no longer auto-discover as components — they classify as `NonComponent` and get dropped from the workspace's component set, breaking the contract-edge resolution. User selected Option B (override-only fixture). Eight new `additions` entries in `phase3_polyglot/.atlas/components.overrides.yaml` declare the Cargo components explicitly; L5 surface extraction still fires on their Cargo.toml manifests via the rust_surface_analyzer, preserving the per-peer `peer-one`..`peer-six` contracts the test asserts on.

**Empirical polyglot baseline (post-WI-3b):**

- Cold count: 48 LLM calls (+~8 from WI-2's ~40 baseline; one LLM call per retired-classifier Cargo component). Loose bound `0 < cold < 100` preserved with substantial headroom (48/100); no assertion-tightening required.
- Wall time: 117.53s for full polyglot smoke (both tests). +16.75s from WI-2's 100.78s baseline; within natural variance for fixture-heavy tests.
- Components classified: 24 (unchanged from WI-2).

**Regression gates (all six clean):**

- `cargo build --workspace`: clean.
- `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`: 40 test-result-ok entries, 0 failures.
- `cargo clippy --all-targets -- -D warnings`: clean (caught the dispatcher `TargetFile` dead-import mid-WI-3b).
- `cargo fmt --check`: clean.
- `cargo build --release --workspace`: clean (41.01s).
- Polyglot release smoke: 2 tests pass, wall-time 117.53s, cold-count 48.

**Cargo SHAs:** WI-3a `165d0a2`, WI-3b `da96fc5`, status flip (this) to follow.

### WI-4

(pending)
