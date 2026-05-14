# Atlas vNext — Phase 8 — Cargo retirement — Brainstorm

**Status:** Brainstorm complete, plan-authoring session pending.
**Date:** 2026-05-14.
**Companion to:** `docs/superpowers/prompts/2026-05-14-phase8-brainstorm.md` (kickoff prompt; dropped in the status-flip commit that closes this brainstorm per the cleanup precedent at `7d6f6f3` / `f9315f6`).

---

## 0. Reading order

Read in this order; downstream artefacts (plan, status) inherit decisions locked here.

1. §1 Summary — the decision distillation.
2. §2 Already-decided framings — binding constraints that did NOT re-litigate in this brainstorm.
3. §3 Phase shape — the four-work-item decomposition.
4. §§4–7 Per-work-item detail — WI-1 bypass, WI-2 HardFail, WI-3 Cargo retirement, WI-4 calibration + closeout.
5. §8 Spec amendment — recast spec §11.2 in-line edit.
6. §9 Open risks — risk register the plan author inherits as plan §12.
7. §10 Recommendation for next session — handoff to `superpowers:writing-plans`.
8. §11 References — canonical sources.

---

## 1. Summary

Phase 8 retires the **Cargo deterministic L3 classifier** as Atlas's first language migration onto the LLM-spine runtime. Before that retirement can happen, Phase 8 must close the **agent-runtime → HTTP-backend wiring gap** surfaced by PR-5 of the production-prompt sprint: today every agent-runtime call into an HTTP backend hits `LlmError::TemplateSyntax("unknown token \`{{COMPONENT_KINDS}}\` in template")` because the HTTP backend's `render_request` (`crates/atlas-llm/src/http_anthropic.rs:49–57`) reads a deterministic-spine prompt template from `prompts_dir` and renders it with substitution tokens that the agent runtime does not supply.

Phase 8 sequences as **four work items**:

- **WI-1** — Agent-runtime HTTP-backend bypass. `LlmRequest` grows `prompt_template: Option<PromptId>` + `rendered_prompt: Option<String>`; backends short-circuit on `rendered_prompt`. Unblocks every agent-runtime feature, not just Cargo.
- **WI-2** — `HardFail` event emission at `call_agent`'s backend error catch. Closes the PR-5 calibration "swallowed diagnostic" observation.
- **WI-3** — Cargo classifier retirement. `CargoClassifyTool` removed from `default_tool_catalog`; classify-prompt rubric rewritten to reward `parse_cargo_toml` + source-read instead of the deterministic classifier; `cargo_classifier.rs` deleted outright (per [[feedback_atlas_llm_spine_intent]] — deterministic spine is legacy).
- **WI-4** — Atlas-on-Atlas calibration + closeout. Re-run PR-5's calibration invocation; record real values for the intrinsic-metrics table; un-ignore the cross-provider parity test; amend recast spec §11.2 to drop "vs deterministic" framing; ship Phase 8 SHIPPED.

Phase 8 acceptance: `atlas index --agent-runtime` against the Atlas workspace completes end-to-end on the HTTP backend pair; intrinsic baseline locked in; cross-provider parity test exercises real LLM output; cumulative regression guard (polyglot release smoke) holds across WI-1, WI-2 (and recalibrates if necessary inside WI-3).

The user reframed "PR" terminology to "work item" during this brainstorm. Within this doc that's the only naming change; whether to globalise the rename is an open question carried to the plan session.

---

## 2. Already-decided framings (binding, NOT re-litigated)

These framings entered this brainstorm pre-locked and are recorded here verbatim so the plan author inherits them as binding:

- **LLM is the spine.** [[feedback_no_deterministic_engine_comparison]] + [[feedback_atlas_llm_spine_intent]]. No "compare with deterministic output" success criteria. Phase 8's deterministic Cargo classifier deletion follows from this.
- **YAML is canonical interchange.** [[feedback_yaml_canonical_interchange]]. Any new artefact shape (e.g., a Phase 8 status file) is YAML.
- **Cross-provider audit beats same-model audit.** [[feedback_cross_provider_llm_audit]]. PR-4 of the prior sprint shipped the auditor; Phase 8 inherits it; WI-4's run script enforces both `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` rather than allowing tautological same-model degradation.
- **Subprocess pair = `claude_code + codex`.** [[project_atlas_common_backend_config]]. HTTP backends are signal-gathering opt-ins; Phase 8 calibrates against the HTTP pair (`http_anthropic` producer / `http_openai` auditor) because that's the sprint-config target.
- **`--agent-runtime` is default-false.** [[project_phase7_agent_runtime_default_ratified]]. Phase 8 doesn't flip the default — it makes the agent-runtime path actually usable end-to-end for Cargo.
- **Prefer existing crates over hand-rolled code.** [[feedback_prefer_existing_crates]]. Bypass shape lives inside `atlas-llm`; no new transport crates introduced.
- **Phase ordering after Phase 8.** Phase 9 (a/b/c) remaining language retirements → Phase 10 LLM-driven analyses → Phase 11 server mode. [[project_phase4_plus_roadmap]].

These memories ARE the architectural framing. None changed during this brainstorm; the design flows from them.

---

## 3. Phase shape — four work items

```
WI-1  (infra)   HTTP-backend agent-mode bypass
        ↓
WI-2  (infra)   HardFail event emission in call_agent
        ↓
WI-3  (retire)  Cargo classifier retirement
        ↓
WI-4  (close)   Atlas-on-Atlas calibration + closeout
```

Each work item is sequential — WI-2 needs WI-1 (the bypass is the substrate for any end-to-end run); WI-3 needs both infra fixes shipped (else regression-prone); WI-4 needs everything before it. No parallel track.

The wiring fixes (WI-1, WI-2) are pure infra unblockers — they enable every future agent-runtime feature, not just Cargo. The same shape will repeat in Phase 9 for each retiring language, minus the infra cleanups.

**LOC envelope (rough total).**

| Work item | Code Δ | Tests Δ | Direction |
|---|---|---|---|
| WI-1 | +250 | +200 | additive |
| WI-2 | +30 | +150 | additive |
| WI-3 | −650 | +150 | deletion-heavy |
| WI-4 | +50 | +50 | minimal |
| **Total** | **−320** | **+550** | **net negative LOC** |

**Plan-doc kickoff item.** A WI-0 plan doc (`docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`) is recommended as the first deliverable of the plan-authoring session, mirroring the Phase 7 plan-doc convention. It's not a separate work item in the LOC envelope above — it's a docs prerequisite the plan author writes.

---

## 4. WI-1 — Agent-runtime HTTP-backend bypass

### 4.1 The wiring gap (root cause recap)

PR-5 of the production-prompt sprint surfaced this 2026-05-14. `crates/atlas-llm/src/http_anthropic.rs::render_request` (lines 49–57):

```rust
fn render_request(&self, req: &LlmRequest) -> Result<(String, Option<String>), LlmError> {
    let path = self.prompts_dir.join(prompt_template_filename(req.prompt_template));
    let template = std::fs::read_to_string(&path)
        .map_err(|e| LlmError::Invocation(format!("failed to read {:?}: {e}", path)))?;
    let tokens = extract_tokens(&req.inputs)?;
    crate::prompt::render_split(&template, &tokens)
}
```

Reads `<prompts_dir>/classify.md` (or `subcarve.md` / `stage1-surface.md` / `stage2-edges.md`) from disk; renders with `req.inputs` as substitution tokens. The deterministic-spine prompt files expect `{{COMPONENT_KINDS}}`, `{{LIFECYCLE_SCOPES}}`, etc. — tokens supplied by the deterministic engine's `l3_classify.rs`. The agent runtime supplies `conversation` (its already-rendered prompt) instead, so render fails on the first unknown token.

`crates/atlas-agents/src/runtime/tool_loop_http.rs:233` literally hard-codes the workaround that creates the gap:

```rust
LlmRequest {
    // PR-4 places the runtime's tool-loop calls under `Classify`
    // for routing purposes. PR-5 will introduce a dedicated
    // PromptId variant for the multi-step agent path; for now
    // the routing table is reused so the test backend's existing
    // canned-response surface covers us.
    prompt_template: PromptId::Classify,
    inputs: json!({ "conversation": conversation, "tools": tool_descriptors }),
    schema: ResponseSchema::accept_any(),
}
```

The "PR-4 will introduce a dedicated PromptId variant" comment marked this exact spot as known tech debt. Phase 8 WI-1 pays it off — not via a new `PromptId` variant (which preserves the wrong abstraction) but by giving the agent runtime a first-class rendered-prompt path.

### 4.2 Bypass shape (locked decision)

`LlmRequest` schema change:

```rust
#[non_exhaustive]
pub struct LlmRequest {
    pub prompt_template: Option<PromptId>,
    pub rendered_prompt: Option<String>,
    pub inputs: Value,
    pub schema: ResponseSchema,
}
```

Invariant: exactly one of `prompt_template` / `rendered_prompt` is `Some`; never both, never neither. Enforced via two public constructors:

```rust
impl LlmRequest {
    pub fn from_template(id: PromptId, inputs: Value, schema: ResponseSchema) -> Self { ... }
    pub fn from_rendered(rendered: String, schema: ResponseSchema) -> Self { ... }
}
```

`debug_assert!` in each constructor catches violations in dev/test runs. Struct-literal construction is allowed for back-compat but is grep-able and migrateable if it later becomes a problem.

### 4.3 Backend short-circuit

Each backend's `render_request` (or equivalent) gets a fast-path at the top:

```rust
fn render_request(&self, req: &LlmRequest) -> Result<(String, Option<String>), LlmError> {
    if let Some(rendered) = &req.rendered_prompt {
        return Ok((rendered.clone(), None));
    }
    let path = self.prompts_dir.join(prompt_template_filename(req.prompt_template.expect("invariant: template or rendered")));
    // ... existing prompts_dir + token render
}
```

Applied to all four backends: `http_anthropic.rs`, `http_openai.rs`, `claude_code.rs`, `codex.rs`.

`BackendRouter::call_async` routing: when `rendered_prompt.is_some()`, routing key cannot use `prompt_template`. New rule: rendered requests route via `default_provider` from the router config. Templated requests use the existing per-`PromptId` table lookup unchanged.

### 4.4 Agent runtime call-builder migration

- `crates/atlas-agents/src/runtime/tool_loop_http.rs::build_llm_request_with_tools` switches to `LlmRequest::from_rendered(conversation.to_string(), schema)`. Drop the misleading `prompt_template: PromptId::Classify` shim + the stale comment.
- `crates/atlas-agents/src/runtime/tool_loop_mcp.rs::build_llm_request_subprocess` switches the same way.
- The auditor's `LlmRequest` construction at `crates/atlas-agents/src/runtime/mod.rs:1158` switches to `from_rendered` (the audit prompt builder already produces a rendered string).

### 4.5 Tests

- `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` (new). Synthetic test backend. Two requests — one templated, one rendered. Templated path fails when `prompts_dir = TempDir::new()` (no `classify.md`); rendered path succeeds. Asserts the bypass works.
- `crates/atlas-agents/tests/agent_runtime_http_smoke.rs` (extension). Exercises the full agent runtime against a synthetic Anthropic-shaped HTTP server with `prompts_dir = TempDir::new()`. Today's smoke uses a populated dir; the new shape verifies the agent runtime no longer depends on it.
- `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` (PR-5 ship, `#[ignore]`-gated) becomes runnable in CI at WI-1 closure — un-ignoring is a WI-4 deliverable, but WI-1 verifies the test can at least construct the runtime without the `prompts_dir` error.

### 4.6 Cumulative regression guard

Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release`) holds across WI-1: 2 tests pass, cold count in `0 < cold < 100`, warm + reports = 0, wall-time 100–110s. The smoke runs the deterministic spine which uses the templated path — bypass changes are invisible to it.

### 4.7 Acceptance

`atlas index . --agent-runtime --config .atlas/config.sprint.yaml --log-events <path> --no-tui --no-budget` completes (success OR failure) with `RuntimeComplete` on the bus. WI-1 alone doesn't guarantee success; it guarantees the wiring no longer pre-fails at render.

---

## 5. WI-2 — HardFail event emission in `call_agent`

### 5.1 The diagnostic-visibility gap

PR-5 closeout note item 4 records: when `call_agent`'s `backend.call_async` errors with `AgentError::Backend`, the error propagates via `?` *without* emitting a `HardFail` event onto the bus. Subscribers see `IterationBoundary` + `AgentStart`, then silence — no per-agent diagnostic.

Note: `RuntimeComplete` actually DOES fire from `crates/atlas-agents/src/runtime/mod.rs:454`'s `let result = ...await; emit(RuntimeComplete); result` shape — that part of PR-5's note overstated. The actual issue is that `HardFail` is never emitted for backend errors. WI-2's framing is per-agent `HardFail` visibility, not `RuntimeComplete` resurrection.

### 5.2 Emission site (locked decision)

`crates/atlas-agents/src/runtime/mod.rs::call_agent` backend call site:

```rust
let response = match backend.call_async(&req).await {
    Ok(v) => v,
    Err(e) => {
        self.event_bus.emit(AgentEvent::HardFail {
            agent_id: agent_id.clone(),
            stage,
            target: target.clone(),
            error_kind: "backend".into(),
            error_summary: e.to_string(),
        });
        return Err(AgentError::Backend(e.to_string()));
    }
};
```

Single emit site. No control-flow change beyond the `?` → `match` rewrite. Mirrors how Lane B's verdict-translation already emits `HardFail` at mod.rs:817 and :1008.

Same treatment for the auditor's backend call at mod.rs:1167 — `AgentError::Backend` from the auditor emits `HardFail` with `error_kind: "audit_backend"` (distinct discriminator so subscribers can distinguish producer-fail from auditor-fail).

### 5.3 Tests

`crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs` (new). Synthetic `LlmBackend` that errors on every `call_async`. Drive one stage via `call_agent`. Assert:

1. `HardFail` event lands on the bus before the future resolves.
2. `RuntimeComplete` lands after.
3. The `error_summary` carries the backend's error text verbatim.

Auditor-fail variant: producer backend returns `Ok`; auditor backend errors. Assert `HardFail` with `error_kind: "audit_backend"` and that producer's `AgentComplete` fires beforehand.

### 5.4 Pipeline.rs

No change needed. `run_index_agent_runtime`'s drain handshake at `crates/atlas-cli/src/pipeline.rs:1169–1171` already joins subscribers unconditionally before checking the result error. The bus-side fix is sufficient.

### 5.5 Acceptance

Re-running PR-5's calibration command after WI-1 + WI-2 land, against an artificially-failing backend or against a real failure, produces an event-log file with `HardFail` records carrying diagnostic context — not silent termination.

---

## 6. WI-3 — Cargo classifier retirement

### 6.1 Scope (locked decision)

Classifier-only. The L5 surface analyzer (`rust_surface_analyzer.rs`, 862 LOC) stays in this phase; surface retirement folds into Phase 9 alongside the other languages, OR into a Phase 8b only if Phase 9's scope is later judged too wide. Phase 8 does not add a `Stage::Surface` to the agent runtime.

### 6.2 Catalog removal

`crates/atlas-agents/src/runtime/mod.rs::default_tool_catalog` (around line 263) drops `Arc::new(CargoClassifyTool)`. Catalog moves from 22 tools to 21:

- 9 classifiers (down from 10): compose, csharp, dart, dockerfile, elixir, lispkit, python, racket, ts_js. Cargo gone.
- 4 manifest parsers unchanged: `parse_cargo_toml`, `parse_compose`, `parse_dockerfile`, `parse_package_json`.
- 8 surfaces unchanged.

`parse_cargo_toml` stays — it's the LLM's primary tool for reading Cargo manifests; only the *deterministic classifier* tool is being retired.

### 6.3 Classify prompt rubric rewrite

`crates/atlas-agents/src/runtime/mod.rs::build_classify_prompt`'s `confidence_grade` rubric (lines ~1411–1425) gets rewritten:

| Grade | Today's text | Phase 8 text |
|---|---|---|
| strong | primary manifest READ + source entry-point READ + **classifier tool CALLED** | primary manifest READ + **appropriate parser tool CALLED** (`parse_cargo_toml` for Rust, etc.) + source entry-point READ |
| moderate | primary manifest READ + classifier tool CALLED, no source read | primary manifest READ + parser tool CALLED, no source read |
| weak | primary manifest READ, no classifier tool called | primary manifest READ only |
| declines | unchanged | unchanged |

The available-tools list in the prompt body (around lines 1366–1371) drops `classify_cargo_component` from its implicit catalog. The worked YAML example for `kind: "rust-library"` stays — it's a shape illustration, not a tool affordance.

**New entry in the canonical-vocabulary list** (line ~1397): add `rust-workspace` so the Atlas root `Cargo.toml`'s `[workspace]` shape has an obvious canonical kind. The worked example gets a `[workspace]` variant fragment (just the `kind:` line, since the rest of the YAML is shape-illustrative).

### 6.4 Deletions

- **Delete** `crates/atlas-agents/src/tools/classifiers/cargo.rs` entirely (~300 LOC including tests). Drop `pub mod cargo; pub use cargo::CargoClassifyTool;` from `crates/atlas-agents/src/tools/classifiers/mod.rs`.
- **Delete** `crates/atlas-analyzers/src/cargo_classifier.rs` entirely (~320 LOC including tests). Drop `pub mod cargo_classifier;` + re-export from `crates/atlas-analyzers/src/lib.rs`.
- **Update** `crates/atlas-analyzers/src/registry.rs` + `crates/atlas-analyzers/src/dispatcher.rs` to drop `CargoClassifier` registration.

Per the locked decision (option a from §6 of the design walkthrough): the deterministic Cargo path breaks. The deterministic spine is legacy per [[feedback_atlas_llm_spine_intent]]; option (a) is consistent with the framing.

### 6.5 Polyglot smoke recalibration

The polyglot smoke fixture may or may not contain a Cargo component. WI-3's first step: `grep -r "Cargo.toml" crates/atlas-cli/tests/fixtures/ 2>/dev/null` (or wherever the polyglot fixture lives). Three cases:

- **No Cargo content in fixture:** smoke is unaffected. Proceed.
- **Cargo content, deterministic path drops to `llm_classify.rs` fallback:** the LLM-fallback path is the documented escape hatch for declining analyzers. Cold count likely rises (`llm_classify.rs` does at least one LLM call for the Cargo component). Loose-bound assertion `0 < cold < 100` likely absorbs the change; tighten only after empirical recalibration.
- **Cargo content + `llm_classify.rs` does not handle Cargo correctly:** WI-3 either teaches `llm_classify.rs` Cargo (small extension), or the polyglot fixture's Cargo component gets swapped for an override-only entry. This is the contingency case — record empirically and adjust.

### 6.6 Tests

`crates/atlas-agents/tests/cargo_retirement_smoke.rs` (new). Synthetic Cargo workspace fixture (one `Cargo.toml` with `[lib]`). Drive the agent runtime via test backend with canned responses simulating a `parse_cargo_toml` + source-read trajectory. Assert:

- classify output `kind: "rust-library"`, `language: "rust"`.
- evidence pointers include `Cargo.toml` at index 0 and `src/lib.rs` at index 1.
- `confidence_grade: "strong"` (canned trajectory satisfies the new rubric).

Negative test: `default_tool_catalog().iter().map(|t| t.id()).collect::<HashSet<_>>()` does NOT contain `"classify_cargo_component"`.

### 6.7 Acceptance

- `CargoClassifyTool` absent from default catalog.
- Classify prompt rubric matches §6.3 above.
- `cargo_classifier.rs` and `crates/atlas-agents/src/tools/classifiers/cargo.rs` deleted.
- Polyglot smoke still passes (with cold-count recalibration if §6.5 triggered).
- `cargo build --workspace`, `cargo test --workspace -- --skip polyglot_phase3`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, polyglot release smoke all clean.

---

## 7. WI-4 — Atlas-on-Atlas calibration + closeout

### 7.1 Trigger

WI-1, WI-2, WI-3 all landed.

### 7.2 Calibration invocation

```
./target/release/atlas index . --agent-runtime \
    --config .atlas/config.sprint.yaml \
    --log-events /tmp/atlas-phase8-events.jsonl \
    --no-tui --no-budget
```

(Same as PR-5's invocation. The config file and the env-var requirements (`ANTHROPIC_API_KEY` + `OPENAI_API_KEY`) are unchanged. WI-4's run script asserts both keys present and fails fast with a setup-error if either is missing — per [[feedback_cross_provider_llm_audit]], same-model degradation defeats the auditor's purpose.)

### 7.3 Intrinsic metrics recorded

Re-populate the table PR-5 left as "n/a — pre-HTTP hard-fail":

| Metric | Recorded value |
|---|---|
| Cold token total (producer-Anthropic) | TBD WI-4 |
| Cold token total (auditor-OpenAI) | TBD WI-4 |
| Iteration count to convergence | TBD WI-4 |
| Wall time | TBD WI-4 |
| Number of components classified | TBD WI-4 (Atlas workspace has ~12–14 crates) |
| Number of subsystems partitioned | TBD WI-4 |
| Evidence-score distribution per stage (p25 / p50 / p90) | TBD WI-4 |
| Lane A retry count per stage | TBD WI-4 |
| Lane B revision count per stage | TBD WI-4 |
| Audit verdict distribution (Accept / RequestRevision / HardFail / Skipped) | TBD WI-4 |
| `ShimError::MissingProjectionField` count + field names | TBD WI-4 |

These become the Phase-8 baseline regression detector. They land in `docs/superpowers/plans/2026-05-15-phase8-status.md` (or whatever the plan author names it). The plan asserts NO thresholds; future PRs assert against these recorded values.

### 7.4 Cross-provider parity re-run

`crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` un-ignored. The three asserts (component-id set equality, subsystem-id set equality, edge multiset equality) fire on real LLM output for the first time. Test design accepts disagreements on subsystem partitioning as signal (assert `subsystem_count_anthropic ≈ subsystem_count_openai ± 1` rather than equality); component-id and edge-multiset equalities remain strict.

If asserts fail with structural disagreement, that's signal — captured in the closeout note as cross-provider drift, NOT as a phase-blocking failure. Matches prior brainstorm §8.5 framing.

### 7.5 Spec amendment

In-line amendment to `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §11.2 — see §8 of this brainstorm.

### 7.6 Memory updates

- `.claude/memory/project_phase4_plus_roadmap.md` — Phase 8 marked SHIPPED with baseline numbers; Phase 9a flagged as next-unblocked.
- `.claude/memory/MEMORY.md` — index line description refreshed.

### 7.7 Acceptance

- All intrinsic-metrics columns populated with real values.
- Cross-provider parity test either passes or surfaces structural-disagreement signal recorded in the closeout note.
- Recast spec §11.2 amended (see §8).
- Memories refreshed.
- Phase 9 unblocked.

---

## 8. Spec amendment — recast §11.2

The recast spec §11.2 today reads (under "Deliverables"):

> Reference-output comparison harness. Structural-equivalence (not byte-equality) is the bar: same set of components, same set of contracts, same set of edges (modulo justifiable refinements). Differences are reviewed and either accepted-as-improvement (LLM agent found a real signal the deterministic classifier missed) or flagged-as-regression (LLM agent dropped a signal the deterministic classifier had). Specific equivalence rules are a plan-time decision.

And under "Acceptance":

> Cargo-language outputs match (or improve on) reference outputs; cold token budget is locked in; warm=0 still holds.

This conflicts with [[feedback_no_deterministic_engine_comparison]] (the binding framing that says don't compare against deterministic output). Prior brainstorm §12.4 flagged the conflict and noted two resolution paths; Phase 8 picks **path (a)**: amend the spec text.

**Replacement Deliverables paragraph:**

> Within-LLM-spine cross-provider parity harness. Structural-equivalence (not byte-equality) is the bar: same set of components, same set of subsystems (± 1 subsystem of legitimate provider drift), same edge multiset (modulo justifiable provider-side refinements). Differences across providers are signal, not regression — captured in the calibration closeout note. There is no comparison against deterministic-classifier output; the deterministic spine is legacy per §4.3.

**Replacement Acceptance paragraph:**

> Cargo-language outputs land in the canonical-schema shape with non-empty intrinsic metrics (cold token totals for both providers, components classified, subsystems partitioned, audit verdict distribution); cross-provider parity holds within the structural-equivalence bar; cold token budget is recorded as the Phase 8 baseline; warm = 0 holds for repeated runs over the same fingerprint.

The amendment lands inside WI-4's commit, not as a separate work item.

---

## 9. Open risks — register inherited by the plan as §12

**R1 — `LlmRequest` schema change is mechanically wide.** Every construction site across `atlas-llm` tests, `budget.rs`, `router.rs`, `test_backend.rs`, plus polyglot-smoke fixtures, needs the constructor migration. ~10–15 call sites. **Mitigation:** WI-1 ships as one atomic commit; `cargo build --workspace` + `cargo test --workspace` is the regression catcher; clippy `-D warnings` catches missed pattern matches.

**R2 — Polyglot smoke cold-count regression on WI-3.** Option (a) deletes `cargo_classifier.rs` outright. If the polyglot fixture has Cargo content, `llm_classify.rs` fallback kicks in — cold count rises. **Mitigation:** WI-3's first task is to grep the fixture; recalibrate the smoke's loose-bound assertion inside the same work item. The `0 < cold < 100` loose bound likely absorbs it; tighten only after empirical recalibration.

**R3 — Atlas-on-Atlas Cargo workspace handling.** The Atlas root `Cargo.toml` is `[workspace]`. Today's deterministic classifier emits `kind: "workspace"`. The new LLM-spine prompt must also produce `"workspace"` (or `rust-workspace` per §6.3) for this component. **Mitigation:** WI-3 adds `rust-workspace` to the prompt's canonical-vocabulary list; the worked example gets a `[workspace]` variant fragment.

**R4 — Cross-provider parity disagreements may surface in WI-4.** Two LLMs may legitimately disagree on subsystem partitioning. **Mitigation:** parity test asserts strict component-id-set + edge-multiset equality, lenient subsystem-count equality (`± 1`). Closeout note records any disagreement narratively.

**R5 — `ShimError::MissingProjectionField` may surface in WI-4.** Prior brainstorm §12.5 risk. PR-5 couldn't measure it (the shim never ran). **Mitigation:** if missing fields surface, fix is local to the project prompt + shim (mirrors PR-3 mitigation). Fold into WI-4 itself if trivial; spin a follow-on work item only if extensive.

**R6 — `LlmRequest::from_template` / `from_rendered` invariant.** "Exactly one is `Some`" is enforced at construction, not at use. Buggy struct-literal construction with both/neither `Some` is caught at `render_request` time. **Mitigation:** two constructors are the only documented public path; `debug_assert!` catches violations in dev runs; struct-literal usage is grep-able + migrateable.

**R7 — Backend `for_provider` closure misconfiguration.** If `OPENAI_API_KEY` isn't set, the auditor closure returns `None` and Lane B degrades to same-model audit. **Mitigation:** WI-4's run script asserts both env vars exist before invoking; hard-fail with a clear setup-error rather than silent degradation.

**R8 — `dispatcher.rs` + `registry.rs` cascade.** Deleting `CargoClassifier` from the analyzer registry could break unrelated tests that enumerate registered analyzers. **Mitigation:** WI-3 runs `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` as pre-flight; any failing test gets a one-line update bundled into WI-3's commit.

**R9 — Stale comments referencing deleted code.** `tool_loop_http.rs:233–238` carries a comment about "PR-5 will introduce a dedicated PromptId variant" that's now obsolete; other call sites may carry similar stale references. **Mitigation:** WI-1 grep pass for `"PromptId::Classify"`, `"prompt_template_filename"`, and the obsolete comment fragments; clean up inline.

**R10 — Per-stage iteration cap × concurrency math (carried from prior brainstorm §12.6).** Atlas-on-Atlas's ~12–14 components × 8 per-stage semaphore = up to 96 in-flight tool calls peak. Multiplied across three stages, fan-out could be large. **Mitigation:** HTTP transport semaphore (default 8) is the real backstop — caps outbound API call rate regardless of per-stage fan-out. WI-4 records peak measured fan-out as part of the intrinsic-metrics table.

**R11 — YAML Norway problem + indentation drift (carried from prior brainstorm §12.8).** WI-3's new prompt-rubric text doesn't introduce new YAML output paths, but the canonical-vocabulary list growth (adding `rust-workspace`) needs a quoted-string emission discipline. **Mitigation:** existing per-field strict deserialization adapters cover `kind`; the `yaml_envelope_norway_problem.rs` test catches regressions.

**R12 — Upstream-version sensitivity of subprocess restrictions (carried from prior brainstorm §12.7).** Phase 8 calibrates against the HTTP pair, not subprocess. PR-A/PR-B of the prior sprint shipped subprocess support but Phase 8 doesn't exercise it. **Mitigation:** Phase 8 doesn't depend on subprocess. Subprocess exercise is deferred to a later phase.

---

## 10. Recommendation for next session

**Next session:** `superpowers:writing-plans` invoked against this brainstorm doc.

**Output:** `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` (mirroring the Phase 7 plan-doc naming convention at `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md`).

**Plan author should:**

- Reproduce the four-work-item decomposition (§§3–7) with file-by-file deliverables, exact diffs where possible.
- Cover the §11.2 spec amendment as a discrete deliverable inside WI-4 (not a separate work item).
- Carry §9's risk register forward as plan §12 with the same numbering.
- Specify the cumulative regression-guard expectation: polyglot release smoke stays green across WI-1 + WI-2 (no Cargo content touched); WI-3 may need a smoke recalibration recorded inline.
- Lock the LOC envelope per §3's table.
- Reference the bypass-shape (§4.2), HardFail-site (§5.2), Cargo-scope (§6.1) decisions as already-locked plan-time decisions. The plan's framing table inherits these verbatim; no re-litigation.

**One open framing question to resolve at the start of the plan session.** The user reframed "PR" terminology to "work item" mid-brainstorm. Should this be a global rename across Atlas's process artefacts going forward (memory `project_phase4_plus_roadmap` + future phase docs adopt "work item" / "step"), or was it a one-off framing for this brainstorm? The brainstorm doc above uses "work item" throughout per the in-session decision; the plan session should clarify whether to globalise.

---

## 11. References

- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` — recast design spec; §11.2 is amended by Phase 8 WI-4 per §8 of this brainstorm. §10.7 (LLM-spine runtime), §13 (binding decisions) inherited verbatim.
- `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` — sprint plan; Phase 8 inherits PRs 1–4 (production prompts + auditor + on-disk verdict + `for_provider` wiring), PR-A (rmcp + subprocess `serve_client`), PR-B (`--disallowedTools` probe).
- `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` — sprint status; the "Sprint → Phase 8 handoff" section and PR-5 note items 2 + 4 are the load-bearing inputs.
- `docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md` — prior brainstorm; §8.5 (acceptable hard-fail framing), §12.4 (§11.2 conflict resolution), §12.5 (L9Projection-shape risk) inherited as prior art.
- `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` — canonical system model; §10 retexted by recast spec §13.
- `.claude/memory/project_phase4_plus_roadmap.md` — Phase 8 unblocked status; will be updated WI-4.
- `.claude/memory/MEMORY.md` — index line description refreshed WI-4.
- Code-side anchors (frozen as of 2026-05-14): `crates/atlas-llm/src/http_anthropic.rs:49–57` (wiring-gap site), `crates/atlas-llm/src/lib.rs:80–108` (`PromptId` + `LlmRequest` schema), `crates/atlas-agents/src/runtime/tool_loop_http.rs:215–246` (agent-runtime call-builder), `crates/atlas-agents/src/runtime/mod.rs:444–456` (`run_workspace` + `RuntimeComplete` emission shape), `crates/atlas-agents/src/runtime/mod.rs:241–289` (`default_tool_catalog`), `crates/atlas-agents/src/runtime/mod.rs:1356–1438` (`build_classify_prompt`), `crates/atlas-analyzers/src/cargo_classifier.rs` (deletion target).
