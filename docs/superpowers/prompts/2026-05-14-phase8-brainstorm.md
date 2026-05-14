# Phase 8 brainstorm — kickoff prompt

Use this prompt to open the **Phase 8 brainstorm** session. Drop this file in the same status-flip commit that closes Phase 8 PR-0 (per the cleanup precedent at `7d6f6f3` / `f9315f6`).

---

## Invocation

Invoke the `superpowers:brainstorming` skill, then hand it the body below.

## Body

Brainstorm **Phase 8** of the Atlas vNext roadmap. Phase 8 is **Cargo retirement** — the first language analyzer to migrate from the deterministic spine to the LLM-spine runtime, per recast spec §11.2 (`docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`). The production-prompt sprint (Phase-7-completion) **SHIPPED 2026-05-14**, formally unblocking Phase 8.

### Highest-priority constraint — fold this in before any Cargo-specific work

A **new prerequisite** surfaced 2026-05-14 by PR-5's Atlas-on-Atlas calibration: an **agent-runtime → HTTP backend wiring gap**. Today every agent-runtime call into an HTTP backend hits `LlmError::TemplateSyntax("unknown token \`{{COMPONENT_KINDS}}\` in template")` (or a missing-file error) because `crates/atlas-llm/src/http_anthropic.rs::render_request` reads a deterministic-spine prompt template (`classify.md` / `subcarve.md` / `stage1-surface.md` / `stage2-edges.md`) from `prompts_dir` and renders it with `req.inputs` — but the agent runtime supplies a fully-rendered prompt via `build_dispatch_subsystems_prompt` / `build_classify_prompt` / `build_reduce_prompt` / `build_project_prompt` and does NOT supply the deterministic engine's substitution tokens. **No agent-runtime + HTTP combination can complete an Atlas run today** — Cargo retirement can't ship until this is closed.

Required brainstorm output: a Phase 8 plan whose **first PR** (after PR-0 plan) is the wiring-gap fix, with Cargo-analyzer work following. The fix likely needs:

- A new `LlmBackend::call_async_with_rendered_prompt(rendered: String, params: Value)` entry point OR an "agent-mode" `LlmRequest` variant that bypasses `prompts_dir` lookup + token substitution.
- Re-routing `AgentError::Backend(...)` hard-fails through the `HardFail` event-bus emission so `RuntimeComplete` still fires on backend errors (today `crates/atlas-cli/src/pipeline.rs::run_index_agent_runtime` swallows the runtime `Err` *before* the drain-handshake — see PR-5 status note item 4).

Brainstorm should explore: is this one PR or two? Does the fix touch `BackendRouter` constructors? Does it require schema changes to `LlmRequest`? Are the `prompts_dir` files still needed for the deterministic spine, or can the field become optional?

### Phase 8 framing (recast spec §11.2)

Phase 8 retires the **Cargo analyzer** (Rust crates / `Cargo.toml` parsing currently in `crates/atlas-analyzers`) by replacing its deterministic logic with LLM-driven analysis routed through the agent runtime. The deterministic Cargo analyzer is the obvious first language to retire because:

1. Atlas itself is a Rust workspace (Cargo is the dogfooding target — Atlas-on-Atlas runs exercise Cargo first).
2. Cargo's analyzer is the most mature deterministic implementation, giving the strongest baseline to grade LLM output against.
3. PR-5's calibration already attempted a Cargo-shaped Atlas-on-Atlas run; once the wiring gap closes, that calibration becomes the Phase 8 quality bar.

Brainstorm should explore (non-exhaustive): which analyzer entry points are crossing into the runtime today; what the LLM prompt shape looks like for Cargo (component identification, edge extraction, override application); how the canonical-schema shim from sprint PR-3 plugs in; how Phase 8's deliverable interacts with the four production prompts that shipped in PR-2 / PR-3.

### Already-decided framings (do NOT re-litigate)

- **LLM is the spine** — deterministic engine is legacy. [[feedback_no_deterministic_engine_comparison]] + [[feedback_atlas_llm_spine_intent]] are the binding framings. No "compare with deterministic output" success criteria.
- **YAML is canonical interchange** — [[feedback_yaml_canonical_interchange]] applies to any new artifact shape.
- **Cross-provider audit** — same-model audit is tautological; the sprint's PR-4 auditor shipped, Phase 8 inherits it. [[feedback_cross_provider_llm_audit]].
- **Subprocess pair = `claude_code + codex`** — typical Atlas runtime; HTTP backends are signal-gathering opt-ins. [[project_atlas_common_backend_config]].
- **`--agent-runtime` is default-false** — ratified [[project_phase7_agent_runtime_default_ratified]]. Phase 8 doesn't flip the default; it makes the agent-runtime path actually usable end-to-end for Cargo.
- **Prefer existing crates** over hand-rolled code [[feedback_prefer_existing_crates]].
- **Phase ordering after Phase 8**: Phase 9 (a/b/c) remaining language retirements → Phase 10 LLM-driven analyses → Phase 11 server mode + web-app subscriber. Out of scope for this brainstorm. [[project_phase4_plus_roadmap]].

### Reading list (canonical sources for the brainstorm)

Read in this order; do NOT re-read files transitively unless a question forces it.

1. `.claude/memory/MEMORY.md` + `.claude/memory/project_phase4_plus_roadmap.md` — current phase ordering + Phase 8's already-recorded unblocked status.
2. `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`:
   - § "Sprint — complete" → "Sprint → Phase 8 handoff" — the explicit handoff item list.
   - § "Per-PR notes" → PR-5 note items 2 + 4 — the wiring-gap root-cause analysis and the `HardFail`/`RuntimeComplete` issue.
   - § "Atlas-on-Atlas baseline section" — the recorded diagnostic that is Phase 8's first-PR target.
3. `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` — §11.2 (Phase 8 framing) + §10.7 (LLM-spine runtime shape) + §13 (binding decisions, already-locked).
4. `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` — sprint scope reference for what Phase 8 inherits vs. what was Phase-7-completion.
5. `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` — canonical system model (`§10` retexted by recast spec §13).
6. `docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md` — §8.5 framing for "acceptable hard-fail with specific diagnostic" + §12 risk #5 (the case that materialised); useful prior art for Phase 8's risk register.

Do **not** read the deleted Phase 7 brainstorm/plan/status — those were dropped in this prompt's authoring commit. Their content is fully absorbed by the sprint status's per-PR notes and by `project_phase4_plus_roadmap`.

### Operating discipline

- The brainstorm session should produce: `docs/superpowers/brainstorms/2026-05-14-atlas-phase8-cargo-retirement-brainstorm.md` and end by recommending the spec/plan author. No code in this session.
- Do **not** start writing the Phase 8 plan in the brainstorm. The plan is the **next** session's deliverable (use `superpowers:writing-plans`).
- If the brainstorm surfaces a scoping question that materially shifts the wiring-gap-vs-Cargo split, escalate to the user before locking the split.
- Save any new framing memory at the end of the brainstorm if the brainstorm produces a durable decision not already captured.

---

## Why this prompt exists in `docs/superpowers/prompts/`

The `prompts/` directory holds in-flight invocation prompts that bootstrap the next session. They are dropped (per precedent `7d6f6f3` and `f9315f6`) in the status-flip commit of the work they kick off. This prompt has no PR number — it's a phase-kickoff prompt, not a per-PR continuation prompt — but the same drop-on-completion convention applies.
