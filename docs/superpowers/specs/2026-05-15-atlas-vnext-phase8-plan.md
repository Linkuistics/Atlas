# Atlas vNext — Phase 8 — Cargo retirement — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan work-item-by-work-item. Steps use checkbox (`- [ ]`) syntax for tracking. Each WI is its own fresh session per the recast spec §10.2 context-rot discipline.

**Status:** Plan-authoring complete 2026-05-15; WI-1 executor session pending.
**Date:** 2026-05-15.
**Companion to:** brainstorm `docs/superpowers/brainstorms/2026-05-14-atlas-phase8-cargo-retirement-brainstorm.md` (locked decisions inherited verbatim — see §2). Status doc shipped by WI-4 closeout at `docs/superpowers/plans/2026-05-15-phase8-status.md`.

**Goal:** Retire the deterministic Cargo L3 classifier as Atlas's first language migration onto the LLM-spine runtime, after closing two infra unblockers (agent-runtime HTTP-backend wiring gap surfaced by sprint PR-5; `HardFail` event-bus emission for backend errors).

**Architecture:** Four sequential work items. WI-1 grows `LlmRequest` with an `Option<String>` rendered-prompt path and short-circuits the four backends + router on it; WI-2 emits `AgentEvent::HardFail` at two `call_agent` backend error sites; WI-3 deletes `CargoClassifyTool` from the agent tool catalog, rewrites the classify-prompt rubric, deletes `cargo_classifier.rs` outright, and recalibrates the polyglot smoke; WI-4 re-runs the Atlas-on-Atlas calibration sprint PR-5 left as "n/a — pre-HTTP hard-fail", un-ignores the cross-provider parity test, amends recast spec §11.2, and ships closeout.

**Tech Stack:** Rust workspace (`crates/atlas-llm`, `crates/atlas-agents`, `crates/atlas-analyzers`, `crates/atlas-cli`); Anthropic Messages API + OpenAI Chat Completions (via `http_anthropic.rs` + `http_openai.rs`); Tokio `broadcast` event bus; `serde_yaml` envelope parsing; polyglot test fixture at `crates/atlas-cli/tests/fixtures/phase3_polyglot/` + `tiny/` (regression detector).

---

## 0. Reading order

Read in this order; each section assumes the previous is internalised.

1. §1 — Phase deliverable, restated.
2. §2 — Framings table (locked decisions inherited from brainstorm; NOT re-litigated).
3. §3 — Terminology: "work item" globalised going forward.
4. §4 — Dependency graph + cumulative regression-guard expectation per WI.
5. §5 — LOC envelope per WI.
6. §6 — Test coverage table (new test files locked).
7. §§7–10 — Per-work-item deliverables (WI-1 → WI-2 → WI-3 → WI-4).
8. §11 — Recast spec §11.2 amendment text (drafted).
9. §12 — Risk register R1–R12 (from brainstorm §9) + plan-time additions.
10. §13 — Memory updates checklist (WI-4 lands these).
11. §14 — References.

---

## 1. Phase deliverable, restated

After Phase 8 ships, all of the following hold:

- `atlas index . --agent-runtime --config .atlas/config.sprint.yaml --log-events <path> --no-tui --no-budget` against the Atlas workspace completes end-to-end on the HTTP backend pair (`http_anthropic` producer / `http_openai` auditor). Sprint PR-5's pre-HTTP `LlmError::TemplateSyntax("unknown token `{{COMPONENT_KINDS}}` in template")` diagnostic is closed.
- `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` runs un-ignored in CI against real LLM output; the three asserts (component-id set, subsystem-id set, edge multiset; with `± 1` subsystem-count drift accepted) fire on actual cross-provider traffic.
- The deterministic Cargo classifier (`crates/atlas-analyzers/src/cargo_classifier.rs`, ~320 LOC including tests) and its agent-tool wrapper (`crates/atlas-agents/src/tools/classifiers/cargo.rs`, ~300 LOC including tests) are deleted. The agent tool catalog drops from 22 to 21 tools (`classify_cargo_component` gone; `parse_cargo_toml` stays).
- The L3 classify prompt's `confidence_grade` rubric rewards `parse_cargo_toml` + source entry-point READ ("strong" requires the parser tool + source-read; no longer rewards calling a deterministic classifier tool).
- Recast spec §11.2 (`docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`) replaces "vs deterministic" framing with "within-LLM-spine cross-provider parity + intrinsic-metrics baseline" per `[[feedback_no_deterministic_engine_comparison]]`.
- An intrinsic-metrics baseline lands in the closeout status doc: cold token totals per provider, iteration count, wall time, components classified, subsystems partitioned, evidence-score distribution, Lane A retry counts, audit verdict distribution, `ShimError::MissingProjectionField` count.
- Memories `project_phase4_plus_roadmap.md` + `MEMORY.md` reflect Phase 8 SHIPPED + Phase 9 (a/b/c) flagged as next-unblocked.
- Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release`) stays green across all four WIs, with cold-count recalibration recorded inline inside WI-3 if the deterministic Cargo deletion shifts the loose-bound count.

---

## 2. Framings table — locked decisions inherited from brainstorm (NOT re-litigated)

These framings entered this plan pre-locked from `docs/superpowers/brainstorms/2026-05-14-atlas-phase8-cargo-retirement-brainstorm.md`. The plan author surfaces and re-reads them verbatim; any executor session that surfaces a question that would change one of these escalates to the user before changing it.

| # | Framing | Source | Applies to |
|---|---|---|---|
| F1 | **LLM is the spine; deterministic engine is legacy.** No "compare with deterministic output" success criteria. Phase 8's Cargo classifier deletion follows from this. | `[[feedback_no_deterministic_engine_comparison]]` + `[[feedback_atlas_llm_spine_intent]]` | WI-3 + WI-4 |
| F2 | **YAML is canonical interchange.** Any new artefact shape (Phase 8 status file) is YAML. | `[[feedback_yaml_canonical_interchange]]` | WI-3 (rubric) + WI-4 (status doc) |
| F3 | **Cross-provider audit beats same-model audit.** WI-4's run script enforces both `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` rather than allowing tautological same-model degradation. | `[[feedback_cross_provider_llm_audit]]` | WI-4 |
| F4 | **Subprocess pair = `claude_code + codex`.** HTTP backends are signal-gathering opt-ins; Phase 8 calibrates against the HTTP pair (`http_anthropic` producer / `http_openai` auditor). | `[[project_atlas_common_backend_config]]` | WI-4 |
| F5 | **`--agent-runtime` is default-false.** Phase 8 doesn't flip the default — it makes the agent-runtime path actually usable end-to-end. | `[[project_phase7_agent_runtime_default_ratified]]` | WI-1, WI-2, WI-4 |
| F6 | **Prefer existing crates.** Bypass shape lives inside `atlas-llm`; no new transport crates introduced. | `[[feedback_prefer_existing_crates]]` | WI-1 |
| F7 | **Phase ordering after Phase 8:** Phase 9 (a/b/c) → Phase 10 → Phase 11. | `[[project_phase4_plus_roadmap]]` | WI-4 (memory updates) |
| F8 | **Phase shape = four work items, sequential.** WI-1 → WI-2 → WI-3 → WI-4. No parallel track. | brainstorm §3 | All |
| F9 | **Bypass shape (WI-1):** `LlmRequest { prompt_template: Option<PromptId>, rendered_prompt: Option<String>, inputs, schema }` with two public constructors (`from_template`, `from_rendered`) and a `debug_assert!`-enforced "exactly one is `Some`" invariant. Backends short-circuit on `rendered_prompt` before `prompts_dir` lookup. `BackendRouter` routes rendered requests via `default_provider`. | brainstorm §4.2 | WI-1 |
| F10 | **HardFail emission sites (WI-2):** producer-fail at `crates/atlas-agents/src/runtime/mod.rs:965` (rewrite the bare `?` propagation of `outcome` to a match-arm that emits `HardFail` then propagates). Auditor-fail at `crates/atlas-agents/src/runtime/mod.rs:1167–1172` (existing match arm gains an `event_bus.emit` before its `return`). Distinct `error_kind` discriminators: `"backend"` for producer-fail, `"audit_backend"` for auditor-fail. | brainstorm §5.2 + plan-time verification | WI-2 |
| F11 | **Cargo retirement scope (WI-3):** classifier-only. `CargoClassifyTool` dropped from `default_tool_catalog`; classify-prompt rubric rewritten so "strong" requires `parse_cargo_toml` + source entry-point READ (no longer rewards a deterministic classifier call); `crates/atlas-analyzers/src/cargo_classifier.rs` deleted outright (option a — deterministic spine is legacy); `crates/atlas-agents/src/tools/classifiers/cargo.rs` deleted. The surface analyzer `rust_surface_analyzer.rs` is **untouched** in Phase 8; surface retirement folds into Phase 9 or 8b. | brainstorm §6.1 + §6.4 | WI-3 |
| F12 | **Recast spec §11.2 amendment (WI-4):** lands inside WI-4's commit, replacing "vs deterministic" framing with "within-LLM-spine cross-provider parity + intrinsic-metrics baseline" per `[[feedback_no_deterministic_engine_comparison]]` and brainstorm §12.4 path (a). Amendment text drafted in brainstorm §8 + reproduced in §11 of this plan. | brainstorm §8 | WI-4 |
| F13 | **Cross-provider parity test design (WI-4):** strict component-id-set equality + strict edge-multiset equality; lenient subsystem-count (`± 1` legitimate provider drift). Disagreement is signal, not failure. | brainstorm §7.4 | WI-4 |

---

## 3. Terminology — "work item" globalised going forward

Decision (plan-author this session): **adopt "work item" (WI-N) as the going-forward decomposition unit** for Phase 8 and beyond. Do NOT retroactively rename PR-N labels in prior-phase memories.

**Why:**
- The user reframed "PR" → "work item" mid-brainstorm 2026-05-14. The reframe is an explicit signal.
- "Work item" is more accurate semantically: a PR is the *delivery vehicle* (git merge artefact); a work item is the *scope unit* (what is being delivered). Phase 8 WIs may collapse to one commit or split across multiple commits (§4.6 per-WI commit-decomposition guidance), so the 1:1 PR↔scope-unit mapping is wrong.
- Phase 1–7 shipped as PRs and the PR-N labels in `project_phase4_plus_roadmap.md` describe what literally landed; falsifying that history would be intrusive and wrong.

**How to apply:**
- This plan uses "WI-N" throughout for Phase 8 work items.
- Future phase docs (Phase 9 onward) adopt "WI-N" for their decomposition.
- `project_phase4_plus_roadmap.md` gets a new "Phase 8 — SHIPPED" entry in WI-N notation (WI-4 commits this); Phase 1–7 entries retain PR-N labels (no edits).
- `MEMORY.md`'s index-line description for the roadmap memory gets a hook-text refresh in WI-4 (mirrors the pattern from sprint PR-5).

---

## 4. Dependency graph + cumulative regression-guard expectation per WI

```
WI-1 (infra)    HTTP-backend agent-mode bypass
       ↓
WI-2 (infra)    HardFail event emission in call_agent
       ↓
WI-3 (retire)   Cargo classifier retirement
       ↓
WI-4 (close)    Atlas-on-Atlas calibration + closeout
```

**WI-1 depends on:** nothing (start state).
**WI-2 depends on:** WI-1 (the bypass is the substrate for any end-to-end run that would exercise HardFail).
**WI-3 depends on:** WI-1 + WI-2 (else any classifier-deletion regression that surfaces during dispatch is invisible without HardFail diagnostics).
**WI-4 depends on:** WI-1 + WI-2 + WI-3 (calibration cannot occur until the wiring is fixed and Cargo is on the LLM-spine path).

**Cumulative regression-guard expectation per WI:**

| WI | Polyglot release smoke (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`) | Workspace test (`cargo test --workspace --no-fail-fast -- --skip polyglot_phase3`) | Clippy + fmt |
|---|---|---|---|
| WI-1 | **HOLD** — 2 tests pass, cold count in loose bound `0 < cold < 100` (~40 baseline since Phase 6 PR-5), warm + reports = 0, wall-time 100–110s. Bypass changes invisible to deterministic-spine smoke. | clean | clean |
| WI-2 | **HOLD** — same; HardFail emit is on a path the smoke never traverses. | clean | clean |
| WI-3 | **RECALIBRATE expected.** Plan-time grep (§4.5 below) shows polyglot fixture has 10 Cargo.toml files. Cargo classifier deletion shifts the deterministic Cargo path to `llm_classify.rs` fallback; cold count likely rises. The loose-bound `0 < cold < 100` should absorb it. WI-3's first post-deletion step is to measure the new cold count and record it inline in this plan's §8.6. If the loose bound no longer holds, the implementer escalates to the user before tightening. | clean | clean |
| WI-4 | **HOLD** at WI-3's recalibrated baseline (since WI-4 changes no Cargo path). | clean | clean |

### 4.5 Plan-time empirical pre-flight (resolves brainstorm §6.5 + R2)

Per brainstorm §6.5's contingency framing, plan-time grep result (`find crates/atlas-cli/tests/fixtures -name "Cargo.toml"`):

```
crates/atlas-cli/tests/fixtures/tiny/mycli/Cargo.toml
crates/atlas-cli/tests/fixtures/tiny/mylib/Cargo.toml
crates/atlas-cli/tests/fixtures/phase2_polyglot/rust_lib/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/rust_lib/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer1/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer2/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer3/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer4/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer5/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/peer6/Cargo.toml
crates/atlas-cli/tests/fixtures/phase3_polyglot/outlier_cluster/outlier/Cargo.toml
```

**11 Cargo.toml files** in `tests/fixtures/` total (2 in `tiny/` + 1 + 8 in `phase3_polyglot/`). Brainstorm §6.5's contingency case ("Cargo content, deterministic path drops to `llm_classify.rs` fallback") is the active scenario, not the hypothetical one. WI-3 must measure the new cold count and may need to tighten the loose-bound assertion.

**Mitigation locked into WI-3:** the first post-deletion step is `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` and recording the new cold count inline. If `0 < cold < 100` still holds → leave the bound; if not → escalate to user before tightening.

---

## 5. LOC envelope per WI

Tightened from brainstorm §3's rough table after plan-time call-site enumeration.

| WI | atlas-llm Δ | atlas-agents Δ | atlas-analyzers Δ | atlas-cli Δ | atlas-engine Δ | Tests Δ | Direction |
|---|---|---|---|---|---|---|---|
| WI-1 | +120 ± 20 | +40 ± 10 | 0 | 0 | +20 (test-helpers updated for LlmRequest schema change; ~10 call sites) | +200 | additive |
| WI-2 | 0 | +30 ± 10 | 0 | 0 | 0 | +150 | additive |
| WI-3 | 0 | −330 (cargo.rs delete + mod.rs +30 for rubric rewrite) | −620 (cargo_classifier.rs delete + registry.rs cascade + dispatcher.rs cascade + lib.rs cascade) | −10 (jsonl_subscriber.rs test name string update) | −15 (heuristics.rs comment cleanup + l3_classify.rs `use` cleanup) | +150 | deletion-heavy |
| WI-4 | 0 | +20 (parity test un-ignore + spec amendment) | 0 | 0 | 0 | +50 | minimal |
| **Total** | **+120** | **−240** | **−620** | **−10** | **+5** | **+550** | **net −195 LOC** |

The WI-1 atlas-llm Δ is driven by: `LlmRequest` schema change (~5 LOC); two constructors (`from_template`, `from_rendered`) with `debug_assert!` invariant checks (~25 LOC); four backends' `render_request` short-circuit (~15 LOC × 4 = 60 LOC); `BackendRouter::call_async` rendered-prompt routing branch (~25 LOC).

The WI-1 atlas-engine Δ is the LlmRequest call-site migration across ~10 sites (`l3_classify.rs:845`, `l5_surface.rs:108`, `l6_edges.rs:131`, `llm_cache.rs:879-880`, `testing.rs:161/162/238`, etc.). Each site rewrites from struct-literal to `LlmRequest::from_template(...)` constructor call. The aggregate +20 LOC reflects the constructor's slightly more verbose call shape minus the struct-literal boilerplate.

The WI-3 atlas-analyzers Δ is dominated by `cargo_classifier.rs` deletion (~320 LOC). The cascade in `registry.rs` (drop `CargoClassifier` use + registration + 4 test sites referencing `ANALYZER_ID`), `dispatcher.rs` (drop 2 test references), and `lib.rs` (drop `pub mod cargo_classifier;` + `pub use cargo_classifier::CargoClassifier;` re-exports + the `cargo_classifier::CargoClassificationOutput` reference in the prelude block) contributes the remaining ~300 LOC of net deletion.

---

## 6. Test coverage table — new test files locked

| WI | File | What it asserts | Gating |
|---|---|---|---|
| WI-1 | `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` (NEW) | Synthetic backend; two requests — one templated (fails when `prompts_dir = TempDir::new()`), one rendered (succeeds). Asserts the bypass works for all four backend implementations: anthropic + openai + claude_code + codex. | CI-active |
| WI-1 | `crates/atlas-agents/tests/agent_runtime_http_smoke.rs` (EXTENSION — existing file at sprint plan Task 1.8; verify path at plan-time) | Exercises full agent runtime against synthetic Anthropic-shaped HTTP server with `prompts_dir = TempDir::new()`. The bypass means the agent runtime no longer requires the dir to be populated. | CI-active (today `#[ignore]`-gated per sprint PR-4 note item 9 — WI-1 closes the underlying issue + un-ignores the test) |
| WI-2 | `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs` (NEW) | Synthetic `LlmBackend` errors on every `call_async`. One stage driven via `call_agent`. Asserts: (1) `HardFail` event lands on the bus before the future resolves; (2) `error_kind = "backend"` for producer-fail; (3) `error_summary` carries the backend's error text verbatim. Auditor-fail variant: producer backend returns `Ok`; auditor backend errors. Asserts: (1) `HardFail` with `error_kind = "audit_backend"`; (2) producer's `AgentComplete` fires beforehand. | CI-active |
| WI-3 | `crates/atlas-agents/tests/cargo_retirement_smoke.rs` (NEW) | Synthetic Cargo workspace fixture (one `Cargo.toml` with `[lib]`). Drives agent runtime via test backend with canned responses simulating a `parse_cargo_toml` + source-read trajectory. Asserts: classify output `kind: "rust-library"`, `language: "rust"`; evidence pointers include `Cargo.toml` at index 0 and `src/lib.rs` at index 1; `confidence_grade: "strong"` per new rubric. Negative test: `default_tool_catalog().iter().map(|t| t.id()).collect::<HashSet<_>>()` does NOT contain `"classify_cargo_component"`. Workspace variant: synthetic `Cargo.toml` with `[workspace]`; canned response emits `kind: "rust-workspace"`; asserts new canonical-vocab term parses. | CI-active |
| WI-4 | `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` (EXISTING — sprint PR-5 ship at commit `b8aa1d0`, `#[ignore]`-gated) | Un-ignored. Three asserts on real LLM output: component-id set equality (strict); edge multiset equality (strict); subsystem-count equality (`± 1` legitimate drift). | CI-active after WI-4 (still requires both API keys; runs via `--ignored` gate flipped to non-ignored OR via a `#[cfg(feature = "cross_provider")]` gate — implementer decides at WI-4 step time) |

---

## 7. Work item 1 — Agent-runtime HTTP-backend bypass

**Files:**

- Modify: `crates/atlas-llm/src/lib.rs:102–108` — `LlmRequest` schema. Add `prompt_template: Option<PromptId>` (change from non-optional `PromptId`); add `rendered_prompt: Option<String>` field; add `#[non_exhaustive]` attribute on the struct.
- Modify: `crates/atlas-llm/src/lib.rs` — add two public constructors `LlmRequest::from_template` + `LlmRequest::from_rendered` with `debug_assert!`-enforced invariant.
- Modify: `crates/atlas-llm/src/http_anthropic.rs:49–57` — `render_request` short-circuits on `req.rendered_prompt.is_some()`.
- Modify: `crates/atlas-llm/src/http_openai.rs:76–83` — same short-circuit pattern.
- Modify: `crates/atlas-llm/src/claude_code.rs:609` (test struct-literal); plus the production `render_request` (find at plan-time — analogous to http_anthropic.rs:49) gains the same short-circuit.
- Modify: `crates/atlas-llm/src/codex.rs:88–96` — same short-circuit pattern.
- Modify: `crates/atlas-llm/src/router.rs` — `BackendRouter::call_async` (find dispatch site at plan-time; line ~117) routing: when `req.rendered_prompt.is_some()`, route via `default_provider`. Templated requests use the existing per-`PromptId` table lookup unchanged.
- Modify: `crates/atlas-agents/src/runtime/tool_loop_http.rs:215–246` — `build_llm_request_with_tools` switches from struct-literal to `LlmRequest::from_rendered(conversation.to_string(), ResponseSchema::accept_any())`. Drop the misleading `prompt_template: PromptId::Classify` shim + the stale PR-4 comment at lines 234–238.
- Modify: `crates/atlas-agents/src/runtime/tool_loop_mcp.rs:62–63` — `build_llm_request_subprocess` switches to `LlmRequest::from_rendered`.
- Modify: `crates/atlas-agents/src/runtime/mod.rs:1158–1165` — auditor's `LlmRequest` construction switches to `from_rendered` (the audit prompt is already a fully-rendered string).
- Modify: every other `LlmRequest` struct-literal call site listed in §7.5 below — migrate from `LlmRequest { prompt_template: X, inputs: Y, schema: Z }` to `LlmRequest::from_template(X, Y, Z)`.
- Create: `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` — synthetic test backend; two requests; bypass verification.
- Modify: `crates/atlas-agents/tests/agent_runtime_http_smoke.rs` — drop `#[ignore]` per sprint PR-4 note item 9; the test today writes a populated `prompts_dir` only because of the wiring gap; with the bypass, `TempDir::new()` is sufficient.

**Per-step granularity below; ~10 steps to keep WI-1 reviewable as one commit.**

- [ ] **Step 1.1: Write the failing rendered-prompt-bypass test against the test backend**

Create `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs`:

```rust
//! WI-1 bypass smoke: a request constructed via
//! `LlmRequest::from_rendered` must succeed against any backend even
//! when `prompts_dir` is empty. A request constructed via
//! `LlmRequest::from_template` must fail in that same setup (read of
//! `<prompts_dir>/classify.md` not found).
//!
//! Synthetic test backend stands in for the HTTP backends because the
//! invariant under test is in the request shape, not the wire shape.

use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn rendered_prompt_request_constructs_without_prompts_dir() {
    let req = LlmRequest::from_rendered(
        "You are a test agent. Reply with: ok.".to_string(),
        ResponseSchema::accept_any(),
    );
    assert!(req.rendered_prompt.is_some());
    assert!(req.prompt_template.is_none());
    assert_eq!(req.rendered_prompt.as_deref().unwrap(),
               "You are a test agent. Reply with: ok.");
}

#[test]
fn templated_request_carries_prompt_id() {
    let req = LlmRequest::from_template(
        PromptId::Classify,
        json!({"COMPONENT_KINDS": "[]", "LIFECYCLE_SCOPES": "[]"}),
        ResponseSchema::accept_any(),
    );
    assert!(req.prompt_template.is_some());
    assert!(req.rendered_prompt.is_none());
    assert_eq!(req.prompt_template, Some(PromptId::Classify));
}

#[test]
#[should_panic(expected = "exactly one of prompt_template / rendered_prompt")]
fn debug_assert_catches_both_some() {
    // SAFETY: bypassing the public constructors via struct-literal —
    // this is what `#[non_exhaustive]` discourages but doesn't prevent
    // inside the crate. Test exists so the invariant is doc + asserted.
    let _ = LlmRequest {
        prompt_template: Some(PromptId::Classify),
        rendered_prompt: Some("X".into()),
        inputs: serde_json::Value::Null,
        schema: ResponseSchema::accept_any(),
    };
}
```

Run: `cargo test -p atlas-llm --test llm_request_rendered_prompt_smoke`.
Expected: FAIL with "no function `from_rendered`" or similar compile error.

- [ ] **Step 1.2: Migrate `LlmRequest` schema in `crates/atlas-llm/src/lib.rs:102–108`**

Replace:

```rust
/// One engine-issued request to a backend.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub prompt_template: PromptId,
    pub inputs: Value,
    pub schema: ResponseSchema,
}
```

With:

```rust
/// One engine-issued request to a backend.
///
/// Exactly one of `prompt_template` / `rendered_prompt` is `Some`.
/// Construct via `from_template` (deterministic-spine path; reads a
/// prompt file from the backend's `prompts_dir` and substitutes
/// `{{TOKEN}}` placeholders from `inputs`) or `from_rendered`
/// (agent-runtime path; the prompt is already a complete string and
/// goes to the backend verbatim).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LlmRequest {
    pub prompt_template: Option<PromptId>,
    pub rendered_prompt: Option<String>,
    pub inputs: Value,
    pub schema: ResponseSchema,
}

impl LlmRequest {
    /// Construct a request whose prompt is loaded from
    /// `<prompts_dir>/<prompt_template_filename(id)>` and rendered
    /// with `inputs` as substitution tokens. Used by the deterministic
    /// spine (L3/L5/L6 stages).
    pub fn from_template(id: PromptId, inputs: Value, schema: ResponseSchema) -> Self {
        Self {
            prompt_template: Some(id),
            rendered_prompt: None,
            inputs,
            schema,
        }
    }

    /// Construct a request whose prompt is already a complete string
    /// (built by the agent runtime via `build_*_prompt` helpers) and
    /// goes to the backend verbatim, bypassing `prompts_dir` lookup.
    pub fn from_rendered(rendered: String, schema: ResponseSchema) -> Self {
        debug_assert!(
            !rendered.is_empty(),
            "from_rendered: rendered prompt must not be empty"
        );
        Self {
            prompt_template: None,
            rendered_prompt: Some(rendered),
            inputs: Value::Null,
            schema,
        }
    }
}
```

The `inputs: Value::Null` on `from_rendered` reflects that the agent runtime's already-rendered prompt has consumed every placeholder during its build pass; the field stays for shape-compatibility but is unused by the rendered-prompt path.

- [ ] **Step 1.3: Run Step 1.1's tests; expect PASS for first two, PASS for the third (debug assertion only fires in dev builds)**

Run: `cargo test -p atlas-llm --test llm_request_rendered_prompt_smoke`.
Expected: 2 tests pass; 1 ignored (the `#[should_panic]` debug-assert test runs only with `--features check-invariants` or similar — implementer may decide gating at step time. If the test is fragile under `--release`, skip it: the `debug_assert!` is *documentation* in code-form, not a load-bearing assertion).

- [ ] **Step 1.4: Short-circuit `render_request` in `crates/atlas-llm/src/http_anthropic.rs:49–57`**

Replace:

```rust
fn render_request(&self, req: &LlmRequest) -> Result<(String, Option<String>), LlmError> {
    let path = self
        .prompts_dir
        .join(prompt_template_filename(req.prompt_template));
    let template = std::fs::read_to_string(&path)
        .map_err(|e| LlmError::Invocation(format!("failed to read {:?}: {e}", path)))?;
    let tokens = extract_tokens(&req.inputs)?;
    crate::prompt::render_split(&template, &tokens)
}
```

With:

```rust
fn render_request(&self, req: &LlmRequest) -> Result<(String, Option<String>), LlmError> {
    // WI-1 bypass: agent runtime supplies a fully-rendered prompt;
    // skip prompts_dir lookup + token substitution.
    if let Some(rendered) = &req.rendered_prompt {
        return Ok((rendered.clone(), None));
    }
    let id = req.prompt_template.expect(
        "LlmRequest invariant: exactly one of prompt_template / rendered_prompt is Some",
    );
    let path = self.prompts_dir.join(prompt_template_filename(id));
    let template = std::fs::read_to_string(&path)
        .map_err(|e| LlmError::Invocation(format!("failed to read {:?}: {e}", path)))?;
    let tokens = extract_tokens(&req.inputs)?;
    crate::prompt::render_split(&template, &tokens)
}
```

- [ ] **Step 1.5: Apply the same short-circuit to `http_openai.rs:76`, `codex.rs:88`, and `claude_code.rs`'s analogous render_request site**

For `http_openai.rs` the return type is `Result<String, LlmError>` (single string, not split-pair). Adapt the `Ok` arm accordingly:

```rust
fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
    if let Some(rendered) = &req.rendered_prompt {
        return Ok(rendered.clone());
    }
    let id = req.prompt_template.expect(
        "LlmRequest invariant: exactly one of prompt_template / rendered_prompt is Some",
    );
    let path = self.prompts_dir.join(prompt_template_filename(id));
    // ... existing read + render_split body
}
```

For `codex.rs:88` same shape as http_openai. For `claude_code.rs`, locate its `render_request` (the `prompt_template_filename` helper lives at `crates/atlas-llm/src/claude_code.rs` per the `use crate::claude_code::prompt_template_filename` re-export pattern) — verify at plan-time.

- [ ] **Step 1.6: Update `BackendRouter::call_async` rendered-prompt routing in `crates/atlas-llm/src/router.rs`**

Find the dispatch logic that selects a backend from `req.prompt_template` (look near `router.rs:117` per the grep result). Add a branch at the top: if `req.rendered_prompt.is_some()`, route via `self.default_provider`-backed backend. Otherwise fall through to the existing per-`PromptId` table lookup. Pseudo-shape:

```rust
async fn call_async(&self, req: &LlmRequest) -> Result<Value, LlmError> {
    if req.rendered_prompt.is_some() {
        // WI-1: rendered requests bypass per-PromptId table; route via
        // default provider per the agent-runtime path.
        let provider = self.default_provider;
        let backend = self.backend_for_provider(provider).ok_or_else(|| {
            LlmError::Setup(format!(
                "rendered-prompt request: no backend registered for default \
                 provider {provider:?}"
            ))
        })?;
        return backend.call_async(req).await;
    }
    // ... existing per-PromptId routing body unchanged
}
```

`self.default_provider` exists today as part of `BackendRouter`'s config (verify at plan-time; if not, this WI adds a `default_provider: Provider` field to the router struct + threads it through `BackendRouter::new`).

- [ ] **Step 1.7: Migrate every `LlmRequest` call site from struct-literal to `from_template` / `from_rendered`**

The grep result locks 14 production call sites + ~9 test/helper sites. Migration matrix:

| File:line | Current shape | New shape |
|---|---|---|
| `crates/atlas-llm/src/budget.rs:138–139` | `LlmRequest { prompt_template: X, inputs: Y, schema: Z }` (test helper) | `LlmRequest::from_template(X, Y, Z)` |
| `crates/atlas-llm/src/test_backend.rs:99–100` | same (test helper) | same |
| `crates/atlas-llm/src/router.rs:413, 425` | same (tests) | same |
| `crates/atlas-llm/src/claude_code.rs:609` | same (test) | same |
| `crates/atlas-llm/src/codex.rs:326, 349, 409` | same (tests) | same |
| `crates/atlas-engine/src/l3_classify.rs:845` | same (production — deterministic spine) | `LlmRequest::from_template(X, Y, Z)` |
| `crates/atlas-engine/src/l5_surface.rs:108` | same (production) | same |
| `crates/atlas-engine/src/l6_edges.rs:131` | same (production) | same |
| `crates/atlas-engine/src/llm_cache.rs:879–880` | same (test helper) | same |
| `crates/atlas-engine/src/testing.rs:161/162/238` | same (helpers) | same |
| `crates/atlas-engine/tests/l5_python_surface.rs:461` | same (test) | same |
| `crates/atlas-engine/tests/l5_racket_surface.rs:119` | same (test) | same |
| `crates/atlas-agents/tests/agent_runtime_single_iteration.rs:319` | same (test) | same |
| `crates/atlas-agents/src/runtime/mod.rs:1158–1165` | production (auditor) — already a rendered prompt under the wire | `LlmRequest::from_rendered(prompt, ResponseSchema::accept_any())` (the `inputs.conversation` shimming goes away; auditor's `prompt` is a complete string) |
| `crates/atlas-agents/src/runtime/tool_loop_http.rs:233–245` | production — agent runtime | `LlmRequest::from_rendered(conversation.to_string(), ResponseSchema::accept_any())` |
| `crates/atlas-agents/src/runtime/tool_loop_mcp.rs:62–63` | production — agent runtime | same |

Approximate total: ~17 call sites. R1 estimated 10–15; plan-time count is ~17. Recommend doing the migration with a single grep + edit pass, then `cargo build --workspace` to catch any missed sites via compile error.

- [ ] **Step 1.8: Clean up stale `PromptId::Classify`-shim comments per R9**

Search worktree for:
- `"PR-5 will introduce a dedicated PromptId variant"` (tool_loop_http.rs:234–238 confirmed)
- `"PromptId::Classify"` references in code-comments (not actual code uses)
- `"prompt_template_filename"` re-export references that became dead

Replace each comment block with a one-liner explaining the bypass shape, or drop entirely if the explanation is redundant with the schema doc comments in `lib.rs`.

- [ ] **Step 1.9: Un-ignore `agent_runtime_http_smoke_completes_with_config_loaded_from_env`**

Per sprint PR-4 note item 9: this test was `#[ignore]`-gated because PR-4's real auditor calls the OpenAI HTTP backend which tries to load `classify.md` from an empty `prompts_dir`. With WI-1's bypass, the auditor's `LlmRequest::from_rendered` no longer needs the dir.

Edit `crates/atlas-agents/tests/agent_runtime_http_smoke.rs`: drop the `#[ignore]` attribute on the test in question (verify exact test name at plan-time; the sprint PR-4 note item 9 names it). Verify the test passes with the WI-1 bypass.

- [ ] **Step 1.10: Run cumulative regression gates**

```
cargo build --workspace
cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean across all six gates. Polyglot smoke: 2 tests pass, cold count in `0 < cold < 100`, warm + reports = 0, wall-time 100–110s.

- [ ] **Step 1.11: Commit WI-1**

**Commit decomposition: two commits.** WI-1 ships as a single code-commit (schema + 4 backends + router + ~17 call-site migrations + new test file + un-ignore) followed by a status-flip commit. The migration is mechanically wide but architecturally one change; splitting it creates intermediate compile-broken states.

```
git add crates/atlas-llm/ crates/atlas-engine/ crates/atlas-agents/
git commit -m "phase8 WI-1: agent-runtime HTTP-backend bypass"
# status-flip commit lands separately when the status doc gets its WI-1 entry
```

**Acceptance gate (WI-1):**

- `LlmRequest::from_template` + `from_rendered` exist; struct schema is `{ prompt_template: Option<PromptId>, rendered_prompt: Option<String>, inputs, schema }`.
- All four HTTP/subprocess backends short-circuit on `rendered_prompt`.
- `BackendRouter::call_async` routes rendered requests via `default_provider`.
- All ~17 call sites migrated; `cargo build --workspace` clean.
- `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` green.
- `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored + green.
- All six regression gates clean.
- `atlas index . --agent-runtime --config .atlas/config.sprint.yaml --log-events <path> --no-tui --no-budget` no longer fails with `LlmError::TemplateSyntax("unknown token `{{COMPONENT_KINDS}}`")`. (WI-1 alone doesn't guarantee end-to-end success; it guarantees the wiring no longer pre-fails at render. WI-4 confirms end-to-end.)

---

## 8. Work item 2 — HardFail event emission in `call_agent`

**Files:**

- Modify: `crates/atlas-agents/src/runtime/mod.rs:965` — producer-fail site. Rewrite the bare `let output = outcome?;` propagation into a match-arm that emits `AgentEvent::HardFail` with `error_kind = "backend"` before returning `Err`.
- Modify: `crates/atlas-agents/src/runtime/mod.rs:1167–1172` — auditor-fail site. The existing match arm `Err(e) => { return AuditVerdict::HardFail(...) }` gains an `event_bus.emit(AgentEvent::HardFail { error_kind: "audit_backend", ... })` before the `return`.
- Create: `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs` — synthetic backend; producer-fail + auditor-fail variants; bus subscriber assertions.

**Per-step granularity below; ~6 steps.**

- [ ] **Step 2.1: Write the failing producer-fail test**

Create `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs`:

```rust
//! WI-2: HardFail event emission for backend errors.
//!
//! Two test cases:
//!  - Producer backend errors → `HardFail { error_kind: "backend" }` fires
//!    on the bus *before* the future resolves to `Err`.
//!  - Producer Ok, auditor backend errors → `HardFail { error_kind:
//!    "audit_backend" }` fires after the producer's `AgentComplete`.
//!
//! Sprint PR-5 closeout note item 4 surfaced this gap: `RuntimeComplete`
//! fires correctly today (PR-4 note item 7 covers it), but per-agent
//! `HardFail` events for backend errors were swallowed by the bare
//! `?` propagation at runtime/mod.rs:965 (producer) + the bare
//! `return AuditVerdict::HardFail(...)` at runtime/mod.rs:1170
//! (auditor).

use atlas_agents::events::{AgentEvent, EventBus};
use atlas_agents::runtime::{AgentRuntime, AgentRequest, Workspace};
// ... synthetic-backend imports

#[tokio::test]
async fn producer_backend_error_emits_hardfail_then_propagates() {
    let bus = Arc::new(EventBus::new());
    let mut subscriber = bus.subscribe();
    let backend = Arc::new(AlwaysErroringBackend::new("synthetic upstream rejection"));
    let runtime = AgentRuntime::new_for_test(backend, bus.clone());
    let request = AgentRequest::test_default(/* stage: Classify, target: "_test_component" */);

    let result = runtime.call_agent(request).await;

    // 1. Assertion: HardFail event landed on the bus.
    let mut saw_hardfail = false;
    while let Ok(evt) = subscriber.try_recv() {
        if let AgentEvent::HardFail { error_kind, error_summary, .. } = evt {
            assert_eq!(error_kind, "backend");
            assert!(error_summary.contains("synthetic upstream rejection"));
            saw_hardfail = true;
            break;
        }
    }
    assert!(saw_hardfail, "expected HardFail event on bus");

    // 2. Assertion: future resolves to Err(AgentError::Backend(...)).
    let err = result.expect_err("producer-fail must propagate Err");
    let summary = err.to_string();
    assert!(summary.contains("synthetic upstream rejection"),
            "Err summary should carry backend's error verbatim: {summary}");
}

#[tokio::test]
async fn auditor_backend_error_emits_audit_hardfail_after_producer_complete() {
    // ... producer backend returns Ok; auditor backend errors.
    // Assert: AgentComplete event fires for producer; then HardFail
    // with error_kind = "audit_backend" fires; verdict carries audit
    // failure reason.
}
```

`AlwaysErroringBackend` is a synthetic test helper; implementer adds it to `crates/atlas-agents/tests/common/synthetic_backends.rs` (or inline in the test file if the test file is the only consumer — single-test policy per sprint PR-5 note item 8). The auditor variant requires the producer's `AlwaysSucceedingBackend` returning a canned classify YAML envelope.

Run: `cargo test -p atlas-agents --test agent_runtime_hardfail_emission`.
Expected: FAIL — `producer_backend_error_emits_hardfail_then_propagates` fails the "expected HardFail event on bus" assertion because mod.rs:965 swallows the emission today.

- [ ] **Step 2.2: Rewrite the producer-fail site at `runtime/mod.rs:965`**

Find the surrounding context (verify exact lines at step-time; line 965 today is `let output = outcome?;` inside `run_tool_loop_with_lane_a` at ~mod.rs:885):

Replace:

```rust
let output = outcome?;
```

With:

```rust
let output = match outcome {
    Ok(v) => v,
    Err(e) => {
        // WI-2: emit per-agent HardFail before propagating. Closes
        // the sprint PR-5 closeout-note item 4 gap.
        self.event_bus.emit(AgentEvent::HardFail {
            agent_id: agent_id(request),
            error_kind: "backend".to_string(),
            error_summary: e.to_string(),
            retry_count: lane_a_retries,
        });
        return Err(e);
    }
};
```

The `agent_id(request)` helper exists today (mod.rs:1009 calls it for the Lane A HardFail emit at line 1008). The `retry_count` mirrors the existing Lane A HardFail (lane_a.rs's HardFail emit at mod.rs:1012 uses `retry_count: 1`); for producer-fail the count is `lane_a_retries` because the producer-fail can happen on retry 0 OR retry 1.

- [ ] **Step 2.3: Rewrite the auditor-fail site at `runtime/mod.rs:1167–1172`**

Replace:

```rust
let response = match auditor_backend.call_async(&llm_request).await {
    Ok(v) => v,
    Err(e) => {
        return AuditVerdict::HardFail(format!("auditor backend call failed: {e}"));
    }
};
```

With:

```rust
let response = match auditor_backend.call_async(&llm_request).await {
    Ok(v) => v,
    Err(e) => {
        // WI-2: distinguish auditor-fail from producer-fail via
        // `error_kind`.
        event_bus.emit(AgentEvent::HardFail {
            agent_id: audit_agent_id.clone(),
            error_kind: "audit_backend".to_string(),
            error_summary: e.to_string(),
            retry_count: 0,
        });
        return AuditVerdict::HardFail(format!("auditor backend call failed: {e}"));
    }
};
```

The `event_bus` and `audit_agent_id` need to be threaded into the auditor function. The auditor function signature (find at plan-time around mod.rs:1100 — it's the function containing the `let llm_request = LlmRequest { ... }` at line 1158) probably takes a `&self` or `&AgentRuntime` somewhere; if not, the implementer threads them through. If threading requires too much surface change, an alternative shape is: have the auditor return a `Result<AuditVerdict, AuditorBackendError>` and emit `HardFail` at the caller. Implementer chooses based on what's least invasive at WI-2 step time; both shapes satisfy the test.

- [ ] **Step 2.4: Run Step 2.1's test; expect PASS**

```
cargo test -p atlas-agents --test agent_runtime_hardfail_emission
```

Expected: both tests pass. Verify the producer-fail Err propagation still includes the backend's error text verbatim (the new match arm returns `e`, not a wrapped string, preserving the original error).

- [ ] **Step 2.5: Run cumulative regression gates**

Same six gates as WI-1 Step 1.10. Polyglot smoke: still HOLDS.

- [ ] **Step 2.6: Commit WI-2**

**Commit decomposition: two commits.** Code commit (mod.rs 2-site rewrite + new test file) + status flip. The producer + auditor sites are coupled by the same diagnostic-visibility framing; splitting them serves no review purpose.

```
git add crates/atlas-agents/src/runtime/mod.rs \
        crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs
git commit -m "phase8 WI-2: HardFail event emission for backend errors"
# status flip separately
```

**Acceptance gate (WI-2):**

- Producer-fail and auditor-fail sites in `mod.rs` emit `AgentEvent::HardFail` before returning.
- Distinct `error_kind` discriminators: `"backend"` (producer) + `"audit_backend"` (auditor).
- `agent_runtime_hardfail_emission.rs` both tests green.
- All six regression gates clean.
- Re-running PR-5's calibration command after WI-1 + WI-2 land, against a known-failing backend OR against a real classify-stage failure, produces an event-log file with `HardFail` records carrying diagnostic context.

---

## 9. Work item 3 — Cargo classifier retirement

**Files:**

- Modify: `crates/atlas-agents/src/runtime/mod.rs:249–253` — drop `CargoClassifyTool` from the `use crate::tools::classifiers::{...}` import.
- Modify: `crates/atlas-agents/src/runtime/mod.rs:263` — drop `Arc::new(CargoClassifyTool),` from the `handles: Vec<ToolHandle>` literal. Update the layout comment at lines 241–247 from "10 classifiers" to "9 classifiers".
- Modify: `crates/atlas-agents/src/runtime/mod.rs:1356–1438` — `build_classify_prompt`. Rewrite `confidence_grade` rubric per §9.3 below; update available-tools list to drop `classify_cargo_component`; add `rust-workspace` to canonical-vocabulary list (line ~1397); extend worked example with a `[workspace]` variant fragment.
- Delete: `crates/atlas-agents/src/tools/classifiers/cargo.rs` (~300 LOC).
- Modify: `crates/atlas-agents/src/tools/classifiers/mod.rs:19` — drop `pub use cargo::CargoClassifyTool;`. Drop `pub mod cargo;` (find at top of file).
- Delete: `crates/atlas-analyzers/src/cargo_classifier.rs` (~320 LOC).
- Modify: `crates/atlas-analyzers/src/lib.rs:37` — drop `pub mod cargo_classifier;`.
- Modify: `crates/atlas-analyzers/src/lib.rs:68` — drop `pub use cargo_classifier::CargoClassifier;`.
- Modify: `crates/atlas-analyzers/src/lib.rs:400` — drop `cargo_classifier::CargoClassificationOutput,` from prelude block.
- Modify: `crates/atlas-analyzers/src/lib.rs:6` — drop the `cargo_classifier::CargoClassifier` doc-comment reference.
- Modify: `crates/atlas-analyzers/src/registry.rs:30` — drop `use crate::cargo_classifier::CargoClassifier;`.
- Modify: `crates/atlas-analyzers/src/registry.rs:116` — drop `let cargo = Arc::new(CargoClassifier::new()) as Arc<dyn Analyzer>;` + its registration call. Drop any downstream registration references to `cargo` in the same block.
- Modify: `crates/atlas-analyzers/src/registry.rs:377, 500, 572, 590, 623` — drop test/check references to `cargo_classifier::ANALYZER_ID` + `CargoClassifier.id()`.
- Modify: `crates/atlas-analyzers/src/dispatcher.rs:184, 310, 541, 542` — drop test references to `cargo_classifier` + `CargoClassifier`.
- Modify: `crates/atlas-engine/src/heuristics.rs` lines 13, 62, 245–246 — drop doc-comment references to `atlas_analyzers::cargo_classifier`.
- Modify: `crates/atlas-engine/src/l3_classify.rs:27` — drop `cargo_classifier::CargoClassificationOutput,` from `use` block (verify whether anything else in l3_classify.rs depends on this type at step-time; if so, dispatch falls through to `llm_classify.rs` per F11 / brainstorm §6.5).
- Modify: `crates/atlas-analyzers/src/{dart,elixir,python,racket,ts_js,dockerfile,csharp,lispkit,compose}_classifier.rs` — drop the "sibling of `cargo_classifier`" doc-comment references (one line each, ~9 files).
- Modify: `crates/atlas-cli/tests/jsonl_subscriber.rs:40, 45` — drop or rename the `"classify_cargo_component"` test tool-name strings (verify whether the test asserts on this specific tool name or just any tool name at step-time; if specific, swap to e.g. `"classify_typescript_component"`).
- Create: `crates/atlas-agents/tests/cargo_retirement_smoke.rs` — synthetic Cargo workspace fixture; full canned-trajectory drive; rust-library + rust-workspace assertions; negative test for catalog absence.

**Per-step granularity below; ~9 steps.**

- [ ] **Step 3.1: Write the failing cargo-retirement smoke test**

Create `crates/atlas-agents/tests/cargo_retirement_smoke.rs`:

```rust
//! WI-3 acceptance: Cargo classifier is retired; the LLM-spine agent
//! classifies Cargo components by reading manifests + source files,
//! without needing a deterministic `classify_cargo_component` tool.

use atlas_agents::runtime::default_tool_catalog;
use std::collections::HashSet;

#[test]
fn default_tool_catalog_excludes_cargo_classifier() {
    let catalog = default_tool_catalog();
    let ids: HashSet<&str> = catalog.iter().map(|t| t.id()).collect();
    assert!(
        !ids.contains("classify_cargo_component"),
        "Cargo classifier tool must not be in the default catalog after WI-3; \
         found ids: {ids:?}"
    );
    // Sanity: parse_cargo_toml stays — it's the LLM's tool for reading
    // Cargo manifests, only the deterministic classifier is retired.
    assert!(
        ids.contains("parse_cargo_toml"),
        "parse_cargo_toml must remain in the catalog (manifest parser, \
         not classifier)"
    );
    assert_eq!(
        catalog.iter().count(),
        21,
        "default catalog should drop from 22 to 21 tools after WI-3"
    );
}

#[tokio::test]
async fn rust_library_component_classifies_via_parse_cargo_toml_plus_source_read() {
    // Synthetic workspace: one Cargo.toml with [lib], one src/lib.rs.
    let fixture = make_rust_library_fixture();

    // Canned-response trajectory: agent calls parse_cargo_toml,
    // reads src/lib.rs, emits classify YAML with strong grade.
    let canned = vec![
        canned_tool_call("parse_cargo_toml", json!({"path": "Cargo.toml"})),
        canned_tool_result(json!({"name": "mylib", "kind": "[lib]"})),
        canned_tool_call("read_file", json!({"path": "src/lib.rs"})),
        canned_tool_result(json!({"text": "pub fn add(a: i32, b: i32) -> i32 { a + b }"})),
        canned_final_yaml(r#"
component_id: "mylib"
kind: "rust-library"
language: "rust"
lifecycle: "build"
subsystem_hint: "mylib_subsystem"
evidence_pointers:
  - path: "Cargo.toml"
    line_range: [1, 8]
  - path: "src/lib.rs"
confidence_grade: "strong"
"#),
    ];

    let result = run_classify_with_canned(&fixture, canned).await;
    let output = result.expect("classify should succeed with strong grade");

    assert_eq!(output.kind, "rust-library");
    assert_eq!(output.language, "rust");
    assert_eq!(output.evidence_pointers[0].path, "Cargo.toml");
    assert_eq!(output.evidence_pointers[1].path, "src/lib.rs");
    assert_eq!(output.confidence_grade, "strong");
}

#[tokio::test]
async fn rust_workspace_component_classifies_with_rust_workspace_kind() {
    // Synthetic workspace: one Cargo.toml with [workspace], no [lib]/[bin].
    let fixture = make_rust_workspace_fixture();
    let canned = vec![
        canned_tool_call("parse_cargo_toml", json!({"path": "Cargo.toml"})),
        canned_tool_result(json!({"workspace": {"members": ["crates/foo"]}})),
        canned_final_yaml(r#"
component_id: "workspace_root"
kind: "rust-workspace"
language: "rust"
lifecycle: "build"
subsystem_hint: "_workspace"
evidence_pointers:
  - path: "Cargo.toml"
    line_range: [1, 5]
confidence_grade: "strong"
"#),
    ];

    let result = run_classify_with_canned(&fixture, canned).await;
    let output = result.expect("workspace classify should succeed");
    assert_eq!(output.kind, "rust-workspace");
}
```

Helpers (`make_rust_library_fixture`, `make_rust_workspace_fixture`, `canned_tool_call`, `run_classify_with_canned`, etc.) live in `crates/atlas-agents/tests/common/cargo_retirement_helpers.rs` (new file) OR inline in this test file if the only consumer is this test (single-test policy).

Run: `cargo test -p atlas-agents --test cargo_retirement_smoke`.
Expected: FAIL — `default_tool_catalog_excludes_cargo_classifier` fails because the catalog still contains `classify_cargo_component`; the `rust-workspace` test fails because the prompt's canonical-vocabulary list doesn't include it yet; the `rust-library` test may pass or fail depending on the canned-trajectory binding.

- [ ] **Step 3.2: Drop `CargoClassifyTool` from `default_tool_catalog` at `crates/atlas-agents/src/runtime/mod.rs:249–289`**

Replace:

```rust
pub fn default_tool_catalog() -> ToolCatalog {
    use crate::tools::classifiers::{
        CargoClassifyTool, ComposeClassifyTool, CsharpClassifyTool, DartClassifyTool,
        DockerfileClassifyTool, ElixirClassifyTool, LispKitClassifyTool, PythonClassifyTool,
        RacketClassifyTool, TsJsClassifyTool,
    };
    // ...
    let handles: Vec<ToolHandle> = vec![
        // Classifiers (10).
        Arc::new(CargoClassifyTool),
        Arc::new(ComposeClassifyTool),
        // ...
```

With:

```rust
pub fn default_tool_catalog() -> ToolCatalog {
    use crate::tools::classifiers::{
        ComposeClassifyTool, CsharpClassifyTool, DartClassifyTool,
        DockerfileClassifyTool, ElixirClassifyTool, LispKitClassifyTool,
        PythonClassifyTool, RacketClassifyTool, TsJsClassifyTool,
    };
    // ...
    let handles: Vec<ToolHandle> = vec![
        // Classifiers (9).
        Arc::new(ComposeClassifyTool),
        // ...
```

Update the doc-comment block at lines 241–247:

```rust
/// Build the default tool catalog from the 21 wrappers remaining after
/// Phase 8 WI-3 (Cargo classifier retired).
///
/// Layout mirrors `crate::tools::{classifiers, manifests, surfaces}`:
///
/// - 9 classifiers (Cargo dropped Phase 8 WI-3)
/// - 4 manifest parsers
/// - 8 surface analysers
```

- [ ] **Step 3.3: Rewrite `build_classify_prompt`'s `confidence_grade` rubric at `runtime/mod.rs:1411–1426`**

Replace:

```rust
`confidence_grade` rubric:
- "strong": primary manifest READ and source entry-point READ and \
  the classifier tool whose name matches the declared `kind` was \
  CALLED.
- "moderate": primary manifest READ and the classifier tool was \
  CALLED, but no source entry-point read (or the kind was inferred \
  from the manifest alone).
- "weak": primary manifest READ but no classifier tool called (the \
  kind/language are best-guess from filename / directory structure).
- "declines": ...
```

With:

```rust
`confidence_grade` rubric:
- "strong": primary manifest READ and the appropriate parser tool was \
  CALLED (`parse_cargo_toml` for Rust, `parse_package_json` for \
  TS/JS, `parse_dockerfile` for Docker, `parse_compose` for Compose, \
  etc.) and source entry-point READ.
- "moderate": primary manifest READ and the parser tool was CALLED, \
  but no source entry-point read.
- "weak": primary manifest READ only (no parser tool called).
- "declines": the primary manifest could not be read, OR there isn't \
  enough evidence to commit to a kind/language — emit a best-guess \
  + this grade so a downstream consumer or human reviewer can \
  intervene.
```

Update the "available-tools" mention at lines ~1366–1371 — change "and language classifiers" wording to remove the dispatch-tool affordance. New text:

```rust
Use the available manifest-parser tools (parse_cargo_toml, \
parse_package_json, parse_pyproject_toml, parse_dockerfile, \
parse_compose, ...) to read the component's primary manifest BEFORE \
assigning a kind / language / lifecycle. Then read at least one \
source entry-point (lib.rs, index.ts, __init__.py, the Dockerfile's \
FROM line, ...) to confirm.
```

Note: `parse_pyproject_toml` is named here for shape-symmetry with the existing prompt; verify at step-time whether that tool actually exists in the catalog (the 4 manifest parsers in the catalog comment are `parse_cargo_toml`, `parse_compose`, `parse_dockerfile`, `parse_package_json` — `parse_pyproject_toml` may not exist). If it doesn't, drop it from the prompt's inline example list. Don't ADD it as a new manifest parser; that's out of scope for WI-3.

- [ ] **Step 3.4: Add `rust-workspace` to the canonical-vocabulary list at `mod.rs:1396–1399` + extend the worked YAML example**

Replace:

```rust
- `kind` is an open-vocabulary kebab-case string. Use the canonical \
  Atlas vocabulary when it fits (`rust-library`, `rust-binary`, \
  `typescript-package`, `python-package`, `docker-image`, \
  `csharp-project`, ...). Quote it.
```

With:

```rust
- `kind` is an open-vocabulary kebab-case string. Use the canonical \
  Atlas vocabulary when it fits (`rust-library`, `rust-binary`, \
  `rust-workspace`, `typescript-package`, `python-package`, \
  `docker-image`, `csharp-project`, ...). Quote it. For a Rust \
  `Cargo.toml` with a `[workspace]` table and no `[lib]`/`[bin]`, \
  prefer `rust-workspace`.
```

The worked YAML example (lines ~1381–1392) stays as `rust-library` — it's a shape illustration. The `rust-workspace` addition is in the vocabulary list, not a second worked example.

- [ ] **Step 3.5: Delete `crates/atlas-agents/src/tools/classifiers/cargo.rs`**

```
rm crates/atlas-agents/src/tools/classifiers/cargo.rs
```

Drop the corresponding `pub mod cargo;` declaration + `pub use cargo::CargoClassifyTool;` re-export from `crates/atlas-agents/src/tools/classifiers/mod.rs` (line 19 + a sibling `pub mod cargo;` near the top of that file — find at step-time).

- [ ] **Step 3.6: Delete `crates/atlas-analyzers/src/cargo_classifier.rs` + cascade**

```
rm crates/atlas-analyzers/src/cargo_classifier.rs
```

Cascade cleanup across `crates/atlas-analyzers/src/`:

- `lib.rs:6` — drop the doc-comment line referencing `cargo_classifier::CargoClassifier`.
- `lib.rs:37` — drop `pub mod cargo_classifier;`.
- `lib.rs:68` — drop `pub use cargo_classifier::CargoClassifier;`.
- `lib.rs:400` — drop `cargo_classifier::CargoClassificationOutput,` from the prelude `pub use` block.
- `registry.rs:30` — drop `use crate::cargo_classifier::CargoClassifier;`.
- `registry.rs:116` — drop `let cargo = Arc::new(CargoClassifier::new()) as Arc<dyn Analyzer>;` AND the registration call that follows it (verify at step-time; the registration is the call that pushes `cargo` onto the registry).
- `registry.rs:377, 500, 572, 590, 623` — drop test/integration references to `crate::cargo_classifier::ANALYZER_ID` + `CargoClassifier.id()`. Some of these are inside test functions that may now have no test bodies — delete the whole test function if so (don't leave empty `#[test] fn name() {}` stubs).
- `dispatcher.rs:184, 310, 541, 542` — same pattern.

Cascade cleanup across other `atlas-analyzers` modules — the `cargo_classifier::CargoClassificationOutput` reference shape was reused as the doc-comment baseline for sibling classifiers. Each sibling's "sibling of `cargo_classifier`" doc-comment loses the cross-reference (one-line edits):

- `dart_classifier.rs:3`
- `elixir_classifier.rs:4`
- `python_classifier.rs:3` + `:49`
- `racket_classifier.rs:4`
- `ts_js_classifier.rs:3` + `:57`
- `dockerfile_classifier.rs:32`

Cascade in `atlas-engine`:

- `heuristics.rs:13, 62, 245–246` — drop or rephrase doc-comment cross-references. Lines 245–246 are inside a comment block referring to `atlas_analyzers::cargo_classifier::CargoClassifier` for PR-5 context; the rephrase is "see prior deterministic classifier (retired Phase 8 WI-3)".
- `l3_classify.rs:27` — drop `cargo_classifier::CargoClassificationOutput,` from `use` block. Verify whether the rest of `l3_classify.rs` body still references `CargoClassificationOutput` (probable: it pattern-matches dispatcher outputs). If it does, swap to a `LlmFallbackOutput` path OR ensure the dispatcher's Cargo dispatch falls through to `llm_classify.rs` cleanly. **This is the cascade most likely to require code changes beyond `use`-line cleanup.** If l3_classify.rs has structural dependence on `CargoClassificationOutput`, WI-3 either:
  - Option A: stubs the Cargo dispatch path in `l3_classify.rs` to immediately fall through to `llm_classify.rs` (preferred — matches brainstorm §6.5 mitigation framing).
  - Option B: introduces an `LlmFallbackOutput` shape and routes Cargo through it.

Implementer chooses A unless A breaks tests or compile.

Cascade in `atlas-cli`:

- `tests/jsonl_subscriber.rs:40, 45` — change the test tool-name string from `"classify_cargo_component"` to any remaining classifier (e.g., `"classify_typescript_component"` — verify the actual tool id at step-time by reading `tools/classifiers/ts_js.rs` or similar).

- [ ] **Step 3.7: Run cumulative regression gates + measure polyglot smoke recalibration**

```
cargo build --workspace
cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast 2>&1 | tee /tmp/wi3-polyglot-smoke.log
```

Record the new cold count from the smoke output. Three possible cases:

1. **Cold count remains in `0 < cold < 100`** (most likely per §4.5's loose-bound framing). Leave the assertion as-is. Record the new empirical baseline inline below in this plan.
2. **Cold count exceeds 100.** The loose bound no longer holds; escalate to user before tightening. Likely cause: 11 Cargo.toml files × `llm_classify.rs` fallback × per-file overhead. Mitigation: widen the bound (e.g. `0 < cold < 150`) inline with empirical observation.
3. **Polyglot smoke fails for non-cold-count reasons** (compile errors propagating into the smoke, or `llm_classify.rs` not handling Cargo cleanly). This is brainstorm §6.5's third sub-case; either teach `llm_classify.rs` Cargo (small extension; in scope) OR swap the polyglot fixture's Cargo components for override-only entries (also in scope; minimal edit to `phase3_polyglot/atlas/overrides`-style files).

**Record empirically here once measured:** _____ (cold count) / _____ (wall time).

- [ ] **Step 3.8: Run Step 3.1's tests; expect PASS**

```
cargo test -p atlas-agents --test cargo_retirement_smoke
```

Expected: all three tests pass — catalog-absence + rust-library + rust-workspace.

- [ ] **Step 3.9: Commit WI-3**

**Commit decomposition: three commits.**

WI-3 is deletion-heavy across two crates. Three commits keep each layer reviewable:

1. **Commit 1 — agent layer:** `crates/atlas-agents/` edits (catalog drop + prompt rubric rewrite + `tools/classifiers/cargo.rs` delete + classifiers/mod.rs cascade + new test file). Polyglot smoke should still PASS at this intermediate state because the deterministic spine + `cargo_classifier.rs` still exist; only the LLM-spine path's catalog and prompt changed.

   ```
   git add crates/atlas-agents/
   git commit -m "phase8 WI-3a: drop CargoClassifyTool + classify-prompt rubric rewrite"
   ```

2. **Commit 2 — analyzer layer:** `crates/atlas-analyzers/` deletions + `atlas-engine` cascade. This is the commit where polyglot smoke MAY shift (the deterministic-spine Cargo path now falls through to `llm_classify.rs`).

   ```
   git add crates/atlas-analyzers/ crates/atlas-engine/ crates/atlas-cli/tests/
   git commit -m "phase8 WI-3b: delete deterministic cargo_classifier + cascade"
   ```

3. **Commit 3 — status flip:** WI-3 row `[ ]` → `[x]` in the status doc; recorded cold-count baseline inline; "Last updated" header refresh.

   ```
   git add docs/superpowers/plans/2026-05-15-phase8-status.md
   git commit -m "phase8 WI-3: status flip"
   ```

**Acceptance gate (WI-3):**

- `CargoClassifyTool` absent from `default_tool_catalog`; catalog drops from 22 to 21 tools.
- Classify prompt's `confidence_grade` rubric matches §9.3.
- `rust-workspace` in canonical-vocabulary list at mod.rs:~1397.
- `crates/atlas-analyzers/src/cargo_classifier.rs` deleted.
- `crates/atlas-agents/src/tools/classifiers/cargo.rs` deleted.
- All cascade references (lib.rs / registry.rs / dispatcher.rs / heuristics.rs / l3_classify.rs / sibling-classifier doc-comments / jsonl_subscriber.rs) cleaned.
- `cargo_retirement_smoke.rs` all tests green.
- Polyglot smoke recalibrated cold-count recorded; either passes within `0 < cold < 100` or widened bound (with user authorisation).
- All six regression gates clean.

---

## 10. Work item 4 — Atlas-on-Atlas calibration + closeout

**Files:**

- Modify: `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` — un-ignore the test OR flip the gate from `#[ignore]` to a `#[cfg(feature = "cross_provider")]` cargo-feature gate (implementer chooses at step-time; the former is simpler, the latter is more CI-friendly).
- Modify: `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §11.2 — replace Deliverables paragraph + Acceptance paragraph per §11 of this plan.
- Create: `docs/superpowers/plans/2026-05-15-phase8-status.md` — status doc with PR checklist (WI-1 through WI-4 boxes), per-WI notes, and Phase 8 closeout section. Patterned after `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`.
- Modify: `.claude/memory/project_phase4_plus_roadmap.md` — append "Phase 8 — SHIPPED YYYY-MM-DD" entry in WI-N notation with baseline numbers + Phase 9 unblocked flag.
- Modify: `.claude/memory/MEMORY.md` — refresh the roadmap-memory hook-line description.

**Per-step granularity below; ~8 steps. Implementer needs `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` exported.**

- [ ] **Step 4.1: Verify env-var preconditions**

```bash
test -n "$ANTHROPIC_API_KEY" || { echo "ANTHROPIC_API_KEY not set"; exit 1; }
test -n "$OPENAI_API_KEY" || { echo "OPENAI_API_KEY not set"; exit 1; }
```

If either is missing: **abort the WI-4 session**. Per F3 + R7, same-model degradation is non-negotiable. Set both keys, restart the WI-4 session with a fresh prompt. Do NOT skip this check — the brainstorm R7 explicitly notes that silent degradation defeats the auditor's purpose.

- [ ] **Step 4.2: Run the Atlas-on-Atlas calibration**

```bash
./target/release/atlas index . --agent-runtime \
    --config .atlas/config.sprint.yaml \
    --log-events /tmp/atlas-phase8-events.jsonl \
    --no-tui --no-budget 2>&1 | tee /tmp/atlas-phase8-stderr.log
```

(Build via `cargo build --release --workspace` first per `[[feedback_release_workspace_build_for_polyglot]]`.)

This is the same invocation sprint PR-5 ran. With WI-1 + WI-2 + WI-3 landed, the run should now reach the full agent-tree dispatch instead of hard-failing pre-HTTP. Expected wall-time: minutes to tens of minutes (Atlas workspace has ~12–14 crates).

Record stderr exit code: ___ . Wall-time: ___ . Total events: ___ .

If the run hard-fails at any stage, the `/tmp/atlas-phase8-events.jsonl` log carries `HardFail` events (per WI-2). Diagnose from the log; the most likely failure modes are:

- **`ShimError::MissingProjectionField` (R5).** Production prompts may emit YAML that lacks fields the canonical-schema shim requires. Fix: edit the relevant prompt (per the field name surfaced in the diagnostic). Bundle the prompt fix inside WI-4 if trivial; spin a follow-on work item if not (per R5's mitigation).
- **Cross-provider audit failure.** Auditor reject-rate may be high if prompts produce dispute-prone output. Recorded as signal in §10.4 below, not phase-blocking.

- [ ] **Step 4.3: Re-run the parity test against real LLM output**

Un-ignore the test (drop `#[ignore]` attribute on `cross_provider_canonical_artifact_parity_holds` in `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs`; verify the exact function name at step-time).

```bash
cargo test -p atlas-agents --test agent_runtime_cross_provider_parity --release -- --nocapture 2>&1 | tee /tmp/wi4-parity.log
```

Adapt the test's existing strict assertions per F13:

- Component-id-set equality: strict (`anthropic_ids == openai_ids`).
- Edge-multiset equality: strict (same edges, same multiplicity).
- Subsystem-count equality: lenient. Replace `assert_eq!(anthropic_count, openai_count)` with `assert!((anthropic_count as i32 - openai_count as i32).abs() <= 1, "subsystem count diverges > 1 across providers");`.

Three outcomes:

- **Test passes:** record as "cross-provider parity HOLDS". Land the parity-test un-ignore as part of WI-4's commit.
- **Test fails on lenient subsystem-count drift:** widen the bound to `± 2` only with explicit user authorisation. Otherwise this is the signal R4 flagged; record in the closeout note and escalate the disagreement narratively.
- **Test fails on strict component-id-set / edge-multiset assertions:** signal of meaningful cross-provider drift. Record in closeout note. NOT phase-blocking per F13 ("disagreement is signal, not failure") — but the un-ignore lands as a `#[ignore = "tracked drift"]` with explicit reason text rather than dropping the `#[ignore]` entirely. Escalate to user for the framing decision.

- [ ] **Step 4.4: Populate the intrinsic-metrics baseline**

Parse the `/tmp/atlas-phase8-events.jsonl` log + the `.atlas/audit/<stage>/<target>.yaml` files materialised during the run. Compute each metric:

| Metric | Source | Recorded value |
|---|---|---|
| Cold token total (producer-Anthropic) | Sum of `audit_tokens.in + audit_tokens.out` across `<audit_dir>/*` files where `auditor.provider = "anthropic"`, PLUS the producer's per-`AgentComplete` `tokens_in + tokens_out` for stages routed via Anthropic | ___ |
| Cold token total (auditor-OpenAI) | Sum across `<audit_dir>/*` files where `auditor.provider = "openai"` | ___ |
| Iteration count to convergence | Count `IterationBoundary` events in `events.jsonl` | ___ |
| Wall time | Difference between first `IterationBoundary` and `RuntimeComplete` timestamps | ___ |
| Number of components classified | Count `AgentComplete` events with `stage = "classify"` | ___ |
| Number of subsystems partitioned | Count `AgentComplete` events with `stage = "reduce"` | ___ |
| Evidence-score distribution per stage (p25 / p50 / p90) | `AgentComplete.evidence_score` distribution per `stage` | ___ |
| Lane A retry count per stage | Sum `HardFail` events with `error_kind = "lane_a"` per stage; sum `lane_a_retries` from `AgentComplete` events | ___ |
| Lane B revision count per stage | Sum `AuditVerdict` events with `verdict = "request_revision"` per stage | ___ |
| Audit verdict distribution (Accept / RequestRevision / HardFail / Skipped) | Bucket `AuditVerdict` events by `verdict` kind | ___ |
| `ShimError::MissingProjectionField` count + field names | Stderr log scan for the error type | ___ |

These values land verbatim in `docs/superpowers/plans/2026-05-15-phase8-status.md` (Step 4.6 below) as the Phase 8 baseline. The plan asserts NO thresholds; future PRs assert against the recorded values.

- [ ] **Step 4.5: Amend recast spec §11.2 per §11 of this plan**

Open `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` § 11.2. Replace the Deliverables paragraph + Acceptance paragraph with the §11 text below.

- [ ] **Step 4.6: Create the closeout status doc**

Create `docs/superpowers/plans/2026-05-15-phase8-status.md` mirroring `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`'s structure:

```markdown
# Atlas vNext — Phase 8 — Cargo retirement — Status

Companion to `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`. This file tracks per-WI completion state across sessions.

**Last updated:** YYYY-MM-DD (WI-4 landed this session — Phase 8 SHIPPED; all four work items merged. Closeout commit: `<sha>`.)

## WI status

- [x] WI-1 — Agent-runtime HTTP-backend bypass
- [x] WI-2 — HardFail event emission in call_agent
- [x] WI-3 — Cargo classifier retirement
- [x] WI-4 — Atlas-on-Atlas calibration + closeout

## Per-WI notes

### WI-1

YYYY-MM-DD — Landed. Code commit `<sha>` + status flip `<sha>`.

[brief note on what shipped, regression gates summary, any plan-time deviations]

### WI-2

[same shape]

### WI-3

YYYY-MM-DD — Landed. Three commits per plan §9.9.

**Polyglot smoke recalibration:** new cold count is ___ (was ~40 before WI-3); wall-time ___s (was ~102s). Loose bound `0 < cold < 100` [held / was widened to ___].

[other notes]

### WI-4

[same shape]

## Phase 8 — complete

**SHIPPED YYYY-MM-DD.**

### Atlas-on-Atlas baseline (intrinsic metrics)

| Metric | Recorded value |
|---|---|
| Cold token total (producer-Anthropic) | ___ |
[full table per Step 4.4]

### Cross-provider parity outcome

[Test passed / failed; narrative summary]

### Phase 8 → Phase 9 handoff

Phase 9 (a/b/c — remaining language retirements) is now unblocked. The bypass + HardFail infrastructure from WI-1 + WI-2 carries forward; each subsequent language retirement follows the WI-3 shape (tool catalog drop + prompt rubric extension + analyser deletion + smoke recalibration).
```

- [ ] **Step 4.7: Update memories**

`.claude/memory/project_phase4_plus_roadmap.md` gets a new entry between the production-prompt sprint and Phase 9 (mirror the sprint's entry shape):

```markdown
- **Phase 8** — *SHIPPED YYYY-MM-DD.* Cargo retirement (first language LLM-driven). 4 work items (WI-1 agent-runtime HTTP-backend bypass + WI-2 HardFail event emission + WI-3 Cargo classifier retirement + WI-4 Atlas-on-Atlas calibration + closeout). Plan: `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`. Status: `docs/superpowers/plans/2026-05-15-phase8-status.md`. Brainstorm: `docs/superpowers/brainstorms/2026-05-14-atlas-phase8-cargo-retirement-brainstorm.md` (commit `56d1b38`). Intrinsic-metrics baseline: cold = ___ + ___ tokens (Anthropic + OpenAI); iterations = ___; wall-time = ___. Cross-provider parity: [held / drift recorded narratively]. Polyglot smoke recalibrated: cold = ___ (was ~40). Phase 9 (a/b/c) unblocked.
```

`.claude/memory/MEMORY.md` line for the roadmap memory: refresh description text to reflect Phase 8's shipping. Current line:

```
- [Atlas vNext Phase 4+ roadmap](project_phase4_plus_roadmap.md) — Phases 4 + 5 + 6 + 7 SHIPPED + production-prompt sprint SHIPPED 2026-05-14 (logically Phase-7-completion); Phase 8 (Cargo retirement) formally unblocked, with new agent-runtime/HTTP-backend wiring-gap prerequisite surfaced by PR-5 calibration.
```

New line:

```
- [Atlas vNext Phase 4+ roadmap](project_phase4_plus_roadmap.md) — Phases 4 + 5 + 6 + 7 + production-prompt sprint + Phase 8 (Cargo retirement, first LLM-spine language migration) SHIPPED YYYY-MM-DD; Phase 9 (a/b/c remaining language retirements) unblocked.
```

No other memory writes. The thirteen framing memories operated unchanged across Phase 8; none required text updates.

- [ ] **Step 4.8: Run cumulative regression gates one final time + commit WI-4**

```
cargo build --workspace
cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean across all six gates; polyglot smoke at WI-3's recalibrated baseline.

**Commit decomposition: two commits.**

1. **Code + docs commit:** parity-test un-ignore + spec amendment + status doc + memory updates.

   ```
   git add crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs \
           docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md \
           docs/superpowers/plans/2026-05-15-phase8-status.md \
           .claude/memory/
   git commit -m "phase8 WI-4: Atlas-on-Atlas calibration + spec amendment + closeout"
   ```

2. **Status flip:** status doc's WI-4 row `[ ]` → `[x]`, "Phase 8 — complete" section's date stamp, "Last updated" header.

   ```
   git add docs/superpowers/plans/2026-05-15-phase8-status.md
   git commit -m "phase8 WI-4: status flip"
   ```

**Note on the plan-session kickoff prompt:** the kickoff prompt `docs/superpowers/prompts/2026-05-15-phase8-plan.md` is dropped in the *plan-shipping commit* (the commit that lands this plan file), per the convention recorded in that prompt's footer ("when the Phase 8 plan ships its status-flip commit, this prompt gets dropped in that same commit"). If for any reason it's still present in the worktree when WI-4 runs, drop it during WI-4's Commit 1.

**Acceptance gate (WI-4):**

- All intrinsic-metrics rows populated with real numbers (not "n/a — pre-HTTP hard-fail").
- Cross-provider parity test either un-ignored + green OR re-ignored with explicit drift-tracking reason.
- Recast spec §11.2 amended per §11 of this plan.
- Status doc shipped at `docs/superpowers/plans/2026-05-15-phase8-status.md`.
- Memories refreshed per Step 4.7.
- Plan-prompt dropped in status-flip commit.
- All six regression gates clean.
- Phase 8 — SHIPPED.

---

## 11. Recast spec §11.2 amendment text

WI-4 lands this amendment in-place inside `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` § 11.2.

**Current Deliverables paragraph (to be replaced):**

> Reference-output comparison harness. Structural-equivalence (not byte-equality) is the bar: same set of components, same set of contracts, same set of edges (modulo justifiable refinements). Differences are reviewed and either accepted-as-improvement (LLM agent found a real signal the deterministic classifier missed) or flagged-as-regression (LLM agent dropped a signal the deterministic classifier had). Specific equivalence rules are a plan-time decision.

**Replacement Deliverables paragraph:**

> Within-LLM-spine cross-provider parity harness. Structural-equivalence (not byte-equality) is the bar: same set of components (strict id-set equality across `http_anthropic` producer + `http_openai` auditor runs), same set of subsystems (`± 1` subsystem of legitimate provider drift accepted), same edge multiset (modulo justifiable provider-side refinements). Differences across providers are signal, not regression — captured in the calibration closeout note as cross-provider drift. There is no comparison against deterministic-classifier output; the deterministic spine is legacy per §4.3.

**Current Acceptance paragraph (to be replaced):**

> Cargo-language outputs match (or improve on) reference outputs; cold token budget is locked in; warm=0 still holds.

**Replacement Acceptance paragraph:**

> Cargo-language outputs land in the canonical-schema shape with non-empty intrinsic metrics (cold token totals for both providers, iteration count, components classified, subsystems partitioned, evidence-score distribution per stage, Lane A + Lane B retry counts, audit verdict distribution, `ShimError::MissingProjectionField` count). Cross-provider parity holds within the structural-equivalence bar defined in the Deliverables paragraph; cold token budget is recorded as the Phase 8 baseline; warm = 0 holds for repeated runs over the same fingerprint.

The amendment lands inside WI-4's code-commit, not as a separate work item.

---

## 12. Risk register

R1–R12 reproduced verbatim from brainstorm §9 (same numbering for cross-reference). Plan-time additions appended as R13 and R14. Each risk names the WI that owns its mitigation.

**R1 — `LlmRequest` schema change is mechanically wide.** Every construction site across `atlas-llm` tests, `budget.rs`, `router.rs`, `test_backend.rs`, plus polyglot-smoke fixtures, needs the constructor migration. Plan-time enumeration: **~17 call sites** (brainstorm estimated 10–15 — slightly low). *Mitigation:* WI-1 ships as one atomic code commit; `cargo build --workspace` + `cargo test --workspace` is the regression catcher; clippy `-D warnings` catches missed pattern matches. *Owner:* WI-1.

**R2 — Polyglot smoke cold-count regression on WI-3.** Plan-time grep (§4.5) confirms the polyglot fixture HAS 11 Cargo.toml files. The deterministic Cargo path drops to `llm_classify.rs` fallback. Cold count likely rises. *Mitigation:* WI-3 Step 3.7 measures the new cold count + records it inline; the `0 < cold < 100` loose bound likely absorbs it; tightening only after empirical recalibration; user escalation if the bound no longer holds. *Owner:* WI-3.

**R3 — Atlas-on-Atlas Cargo workspace handling.** The Atlas root `Cargo.toml` is `[workspace]`. The new LLM-spine prompt must produce `"rust-workspace"` for this component. *Mitigation:* WI-3 Step 3.4 adds `rust-workspace` to the prompt's canonical-vocabulary list + extends the wording to call out the `[workspace]` case explicitly. *Owner:* WI-3.

**R4 — Cross-provider parity disagreements may surface in WI-4.** Two LLMs may legitimately disagree on subsystem partitioning. *Mitigation:* parity test asserts strict component-id-set + edge-multiset equality, lenient subsystem-count equality (`± 1`). Closeout note records any disagreement narratively. *Owner:* WI-4.

**R5 — `ShimError::MissingProjectionField` may surface in WI-4.** Prior brainstorm §12.5 risk. PR-5 couldn't measure it (the shim never ran). *Mitigation:* if missing fields surface, fix is local to the project prompt + shim (mirrors PR-3 mitigation). Fold into WI-4 itself if trivial; spin a follow-on work item only if extensive. *Owner:* WI-4.

**R6 — `LlmRequest::from_template` / `from_rendered` invariant.** "Exactly one is `Some`" is enforced at construction, not at use. Buggy struct-literal construction with both/neither `Some` is caught at `render_request` time. *Mitigation:* two constructors are the only documented public path; `#[non_exhaustive]` makes struct-literal construction visible from outside the crate (compile error); inside the crate, `debug_assert!` catches violations in dev runs. *Owner:* WI-1.

**R7 — Backend `for_provider` closure misconfiguration.** If `OPENAI_API_KEY` isn't set, the auditor closure returns `None` and Lane B degrades to same-model audit. *Mitigation:* WI-4 Step 4.1 asserts both env vars exist before invoking; hard-fail with a clear setup-error rather than silent degradation. *Owner:* WI-4.

**R8 — `dispatcher.rs` + `registry.rs` cascade.** Deleting `CargoClassifier` from the analyzer registry could break unrelated tests that enumerate registered analyzers. *Mitigation:* WI-3 runs `cargo test --workspace --no-fail-fast -- --skip polyglot_phase3` as pre-flight; any failing test gets a one-line update bundled into WI-3's Commit 2. Specific call sites at registry.rs:377/500/572/590/623 + dispatcher.rs:184/310/541/542 are enumerated in §9 Files block — implementer addresses each. *Owner:* WI-3.

**R9 — Stale comments referencing deleted code.** `tool_loop_http.rs:233–238` carries a comment about "PR-5 will introduce a dedicated PromptId variant" that's now obsolete; other call sites carry similar stale references. *Mitigation:* WI-1 Step 1.8 grep pass for stale-comment patterns; clean inline. *Owner:* WI-1.

**R10 — Per-stage iteration cap × concurrency math.** Atlas-on-Atlas's ~12–14 components × 8 per-stage semaphore = up to 96 in-flight tool calls peak. Multiplied across three stages, fan-out could be large. *Mitigation:* HTTP transport semaphore (default 8) is the real backstop — caps outbound API call rate regardless of per-stage fan-out. WI-4 Step 4.4 records peak measured fan-out as part of the intrinsic-metrics table. *Owner:* WI-4.

**R11 — YAML Norway problem + indentation drift.** WI-3's prompt-rubric text doesn't introduce new YAML output paths, but the canonical-vocabulary list growth (adding `rust-workspace`) needs a quoted-string emission discipline. *Mitigation:* existing per-field strict deserialization adapters cover `kind`; the `yaml_envelope_norway_problem.rs` test (sprint PR-2 ship) catches regressions. *Owner:* WI-3.

**R12 — Upstream-version sensitivity of subprocess restrictions.** Phase 8 calibrates against the HTTP pair, not subprocess. Sprint PR-A / PR-B shipped subprocess support; Phase 8 doesn't exercise it. *Mitigation:* Phase 8 doesn't depend on subprocess. Subprocess exercise is deferred to a later phase (Phase 9 may revisit if needed). *Owner:* none-active. *Note:* Phase 9's brainstorm should re-check upstream `claude` / `codex` flag inventories per `restrictions.md`.

**R13 (plan-time addition) — `l3_classify.rs` cascade may exceed `use`-line cleanup.** Plan-time read of `crates/atlas-engine/src/l3_classify.rs:27` shows `cargo_classifier::CargoClassificationOutput` imported. If l3_classify.rs has structural dependence on this type (e.g., pattern-matches Cargo dispatcher output), WI-3's cascade is wider than a `use`-line drop. *Mitigation:* WI-3 Step 3.6 lists two options — (A) stub the Cargo dispatch path to fall through to `llm_classify.rs`; (B) introduce an `LlmFallbackOutput` shape. Implementer picks (A) unless tests force (B). *Owner:* WI-3.

**R14 (plan-time addition) — Parity test `prompts_dir = TempDir::new()` may still fail post-WI-1 if any code path requires a populated dir.** The bypass closes the agent-runtime path, but `BackendRouter::new_for_agent_runtime` (per sprint PR-5 note) still REQUIRES the parameter. *Mitigation:* WI-1 Step 1.6 verifies that rendered requests route via `default_provider` independent of `prompts_dir`. If any backend still touches `prompts_dir` on a rendered request, that's a WI-1 implementation bug — caught by the bypass smoke test (Step 1.1). *Owner:* WI-1.

---

## 13. Memory updates — WI-4 checklist

| File | Change | Owner | Verification |
|---|---|---|---|
| `.claude/memory/project_phase4_plus_roadmap.md` | Append "Phase 8 — SHIPPED YYYY-MM-DD" entry in WI-N notation; flag Phase 9 (a/b/c) unblocked | WI-4 Step 4.7 | grep `Phase 8 — SHIPPED` returns one hit |
| `.claude/memory/MEMORY.md` | Refresh the roadmap-memory hook-line description (single line edit) | WI-4 Step 4.7 | line ≤ 200 chars; describes Phase 8 + Phase 9 status accurately |

No other memory writes. The thirteen framing memories (F1–F7 architectural + the operational ones at `[[project_atlas_common_backend_config]]`, `[[project_phase7_agent_runtime_default_ratified]]`, the four execution-discipline memories) are durable framings; Phase 8 operates within them without amendment.

The "work item" terminology globalisation decision (§3 of this plan) does NOT trigger a memory edit. Future phase docs adopt WI-N notation; prior memories retain their original PR-N references because those describe historical artifacts.

---

## 14. References

- `docs/superpowers/brainstorms/2026-05-14-atlas-phase8-cargo-retirement-brainstorm.md` — the authoritative input for this plan. All locked framings (§2) trace there. Commit `56d1b38`.
- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` — recast design spec; §11.2 is amended by WI-4 (text in §11 of this plan). §10.7 (LLM-spine runtime shape) + §13 (binding decisions) inherited verbatim.
- `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` — prior sprint's plan-doc; template-shape reference (file-by-file deliverables, checkbox steps, cargo-gate cumulative regression discipline, two-commit pattern).
- `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` — sprint status. Load-bearing inputs for this plan:
  - § "Sprint — complete" — sprint SHIPPED 2026-05-14 baseline; cumulative regression-guard summary across 8 sprint PRs.
  - § "Atlas-on-Atlas baseline section" — the wiring-gap diagnostic that motivates WI-1 + WI-2.
  - § "Sprint → Phase 8 handoff" — the explicit handoff item naming the wiring gap as Phase 8 prerequisite #1.
  - § "Per-PR notes → PR-5 → item 4" — the producer-fail / auditor-fail diagnostic-visibility note that motivates WI-2.
- `.claude/memory/project_phase4_plus_roadmap.md` — Phase 8 unblocked status; updated by WI-4.
- `.claude/memory/MEMORY.md` — index-line description refreshed by WI-4.

**Code-side anchors (frozen 2026-05-15 worktree state; verify at executor-session step time):**

- `crates/atlas-llm/src/lib.rs:102–108` — `LlmRequest` schema (WI-1 modifies).
- `crates/atlas-llm/src/http_anthropic.rs:49–57` — render_request wiring-gap site (WI-1 short-circuits).
- `crates/atlas-llm/src/http_openai.rs:76–83` — render_request (WI-1).
- `crates/atlas-llm/src/codex.rs:88–96` — render_request (WI-1).
- `crates/atlas-llm/src/claude_code.rs` — render_request analogue (WI-1; find at step time).
- `crates/atlas-llm/src/router.rs:~117` — `BackendRouter::call_async` routing (WI-1).
- `crates/atlas-agents/src/runtime/tool_loop_http.rs:215–246` — agent-runtime call-builder (WI-1).
- `crates/atlas-agents/src/runtime/tool_loop_mcp.rs:62–63` — same for subprocess transport (WI-1).
- `crates/atlas-agents/src/runtime/mod.rs:241–289` — `default_tool_catalog` (WI-3 drops CargoClassifyTool at :250 + :263).
- `crates/atlas-agents/src/runtime/mod.rs:614` — `pub async fn call_agent` entry (WI-2 context).
- `crates/atlas-agents/src/runtime/mod.rs:817 + 1008` — existing Lane B + Lane A HardFail emit sites (WI-2 mirrors at producer + auditor sites).
- `crates/atlas-agents/src/runtime/mod.rs:965` — producer-fail propagation site (WI-2 rewrites).
- `crates/atlas-agents/src/runtime/mod.rs:1158–1172` — auditor LlmRequest + call_async + match arm (WI-1 migrates the request; WI-2 emits HardFail in the Err arm).
- `crates/atlas-agents/src/runtime/mod.rs:1356–1438` — `build_classify_prompt` (WI-3 rewrites rubric + canonical-vocab list).
- `crates/atlas-agents/src/tools/classifiers/cargo.rs` — Cargo classifier wrapper (WI-3 deletes).
- `crates/atlas-agents/src/tools/classifiers/mod.rs:19` — re-export to drop (WI-3).
- `crates/atlas-analyzers/src/cargo_classifier.rs` — deterministic Cargo classifier (WI-3 deletes).
- `crates/atlas-analyzers/src/lib.rs:6, 37, 68, 400` — cascade refs (WI-3 drops).
- `crates/atlas-analyzers/src/registry.rs:30, 116, 377, 500, 572, 590, 623` — cascade refs (WI-3 drops).
- `crates/atlas-analyzers/src/dispatcher.rs:184, 310, 541, 542` — cascade refs (WI-3 drops).
- `crates/atlas-engine/src/heuristics.rs:13, 62, 245–246` — doc-comment refs (WI-3 cleans).
- `crates/atlas-engine/src/l3_classify.rs:27` — `use` of `cargo_classifier::CargoClassificationOutput` (WI-3 drops; potential cascade per R13).
- `crates/atlas-cli/tests/jsonl_subscriber.rs:40, 45` — test tool-name strings (WI-3 rename).
- 11 LlmRequest call sites across `atlas-llm` + `atlas-engine` + `atlas-agents` tests + production — full list in §7 Step 1.7 table (WI-1 migrates each).
