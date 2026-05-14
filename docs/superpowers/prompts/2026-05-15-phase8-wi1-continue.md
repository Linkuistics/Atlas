# Phase 8 WI-1 — kickoff prompt

Use this prompt to open the **Phase 8 WI-1 executor session**. The plan at `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` (committed in `78bb22c`) is the authoritative input.

---

## Invocation

Invoke the `superpowers:executing-plans` skill, then hand it the body below.

## Body

Execute **Phase 8 WI-1** (agent-runtime HTTP-backend bypass) — the first of the four sequential work items defined in the plan. WI-1 grows `LlmRequest` with an `Option<String>` rendered-prompt path, short-circuits the four backends on it, migrates ~17 call sites, and un-ignores the HTTP smoke test that sprint PR-4 left pending.

### Reading order

Read in this order; don't transitively read references unless a step forces it.

1. `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` § 0 (reading order) → § 1 (phase deliverable) → § 2 (framings table) → § 3 (terminology) → § 4 (regression-guard table + plan-time grep) → § 5 (LOC envelope) → § 6 (test coverage table) → **§ 7 (WI-1 file list + Step 1.1 → 1.11)**. §§ 8–10 (WI-2 / WI-3 / WI-4) are **out of scope** for this session.
2. Plan-locked code-side anchors (§ 14). Before editing each, verify the line numbers haven't drifted from the plan's frozen-2026-05-15 state:
   - `crates/atlas-llm/src/lib.rs:102–108` (LlmRequest schema)
   - `crates/atlas-llm/src/http_anthropic.rs:49–57` (render_request)
   - `crates/atlas-llm/src/http_openai.rs:76–83` (render_request)
   - `crates/atlas-llm/src/codex.rs:88–96` (render_request)
   - `crates/atlas-llm/src/claude_code.rs` (render_request analogue — find via `prompt_template_filename`)
   - `crates/atlas-llm/src/router.rs:~117` (`BackendRouter::call_async` dispatch)
   - `crates/atlas-agents/src/runtime/tool_loop_http.rs:215–246` (build_llm_request_with_tools)
   - `crates/atlas-agents/src/runtime/tool_loop_mcp.rs:62–63` (build_llm_request_subprocess)
   - `crates/atlas-agents/src/runtime/mod.rs:1158–1165` (auditor LlmRequest)
3. Plan § 7 Step 1.7's migration matrix — the ~17 LlmRequest call sites. Re-grep at session start to confirm the count (`grep -rn "LlmRequest\s*{\|LlmRequest::new" crates/`) — if the count differs materially from 17, surface and confirm before proceeding.
4. `.claude/memory/MEMORY.md` for the active framings the plan inherits (`[[feedback_atlas_llm_spine_intent]]`, `[[project_atlas_common_backend_config]]`, `[[project_phase7_agent_runtime_default_ratified]]`, `[[feedback_prefer_existing_crates]]`, `[[feedback_no_tail_pipe_for_long_tests]]`, `[[feedback_release_workspace_build_for_polyglot]]`, `[[feedback_cargo_skip_polyglot_pattern]]`, `[[feedback_atlas_test_subprocess_concurrency]]`).
5. Sprint PR-5 closeout note items 1–4 in `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` — the wiring-gap diagnostic WI-1 closes. Skim only; the plan's § 1 already summarises.

### Locked decisions (inherited from the plan; NOT re-litigated)

These framings entered this session pre-locked. If you surface a question that would change one, escalate to the user before changing.

- **Bypass shape (plan F9 + § 7 Step 1.2):** `LlmRequest { prompt_template: Option<PromptId>, rendered_prompt: Option<String>, inputs, schema }` with `#[non_exhaustive]`. Two public constructors `from_template(id, inputs, schema)` + `from_rendered(rendered, schema)`. `debug_assert!`-enforced exactly-one-Some invariant.
- **Routing rule (plan § 7 Step 1.6):** rendered requests route via `BackendRouter::default_provider`; templated requests use the existing per-PromptId table unchanged.
- **Migration shape (plan § 7 Step 1.7):** ~17 LlmRequest call sites migrate from struct-literal to constructors. Production agent-runtime sites (tool_loop_http.rs:233, tool_loop_mcp.rs:62, mod.rs:1158) use `from_rendered`; everything else (deterministic-spine production + tests) uses `from_template`.
- **Test gating (plan § 7 Step 1.9):** `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored at WI-1 closure (closes sprint PR-4 note item 9).
- **Commit decomposition (plan § 7 Step 1.11):** one code commit (schema + 4 backends + router + ~17 call sites + new smoke test + un-ignore) + one status-flip commit. Don't split the code commit; the migration is one architectural change.
- **R6 / R14 (plan § 12):** the `prompts_dir` parameter on `BackendRouter::new_for_agent_runtime` remains required for shape-compatibility; the bypass means it's unused on rendered-prompt paths. If a backend still touches `prompts_dir` on a rendered request, the new `llm_request_rendered_prompt_smoke.rs` test catches it.

### Operating discipline

- **No scope creep outside WI-1.** Do NOT touch:
  - WI-2 deliverables (HardFail emission at mod.rs:965 producer-fail + mod.rs:1167 auditor-fail) — that's the next session.
  - WI-3 deliverables (Cargo classifier deletion, prompt rubric rewrite, default_tool_catalog edit).
  - WI-4 deliverables (calibration run, parity test un-ignore, recast spec §11.2 amendment, memory updates).
- **Status doc pre-flight.** Before starting Step 1.1, check whether `docs/superpowers/plans/2026-05-15-phase8-status.md` exists. The plan-shipping commit (`78bb22c`) did not create it — that's a plan-author miss noted in the prior session. Create it as Step 0 with the structure below, before the WI-1 code commit:

  ```markdown
  # Atlas vNext — Phase 8 — Cargo retirement — Status

  Companion to `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md`. This file tracks per-WI completion state across sessions.

  **Last updated:** YYYY-MM-DD (status doc authored alongside WI-1 code commit per plan §7 Step 0 pre-flight).

  ## WI status

  - [ ] WI-1 — Agent-runtime HTTP-backend bypass
  - [ ] WI-2 — HardFail event emission in call_agent
  - [ ] WI-3 — Cargo classifier retirement
  - [ ] WI-4 — Atlas-on-Atlas calibration + closeout

  ## Per-WI notes

  ### WI-1

  (pending)

  ### WI-2

  (pending)

  ### WI-3

  (pending)

  ### WI-4

  (pending)
  ```

  Land the empty status doc as part of WI-1's code commit (preferred — one fewer commit), or as a separate small "phase8: author status doc" commit immediately before WI-1's code commit. Don't land it as its own status-flip commit — the status flip is WI-1's second commit, not the doc-authoring one.
- **Cumulative regression guard (plan § 4):** polyglot release smoke MUST stay green across WI-1 — 2 tests pass, cold count in `0 < cold < 100` (~40 calibrated baseline since Phase 6 PR-5), warm + reports = 0, wall-time 100–110s. The bypass is invisible to the deterministic-spine path the smoke exercises; cold-count drift here is a bug. Use `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` after `cargo build --release --workspace` per `[[feedback_release_workspace_build_for_polyglot]]`. Do NOT pipe through `tail` per `[[feedback_no_tail_pipe_for_long_tests]]`. Run dev workspace tests and release polyglot sequentially per `[[feedback_atlas_test_subprocess_concurrency]]`.
- **Inherit the plan's risk register.** R1 (mechanically wide call-site migration), R6 (invariant enforcement), R9 (stale comment cleanup), R14 (parity-test prompts_dir) are WI-1's mitigation owners. Plan § 12 names the mitigation for each.
- **No new memory writes this session.** WI-1 makes no architectural framings; it implements a locked one. Memory updates land in WI-4 per plan §13.

### Deliverable shape

Two commits (plus an optional Step 0 docs commit if the status doc isn't bundled into the code commit):

1. **(optional) Status doc authoring commit:** create `docs/superpowers/plans/2026-05-15-phase8-status.md` per Step 0 above. Title: `phase8: author Phase 8 status doc`.
2. **Code commit:** LlmRequest schema migration + 4 backends short-circuit + router routing + ~17 call-site migrations + new `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` + un-ignore of `agent_runtime_http_smoke_completes_with_config_loaded_from_env`. Title: `phase8 WI-1: agent-runtime HTTP-backend bypass`.
3. **Status-flip commit:** WI-1 row `[ ]` → `[x]` in the status doc; WI-1 per-WI note (what shipped, regression gates summary, any plan-time deviations like the actual call-site count if it differs from ~17, cargo commit SHA); "Last updated" header refresh. Title: `phase8 WI-1: status flip`.

### Acceptance gate (mirrors plan § 7's "Acceptance gate (WI-1)")

Verify every bullet before flipping the status row:

- `LlmRequest::from_template` + `from_rendered` exist with the signatures in plan § 7 Step 1.2.
- Struct schema is `{ prompt_template: Option<PromptId>, rendered_prompt: Option<String>, inputs, schema }` + `#[non_exhaustive]`.
- All four backends (`http_anthropic.rs`, `http_openai.rs`, `codex.rs`, `claude_code.rs`) short-circuit on `req.rendered_prompt.is_some()` before `prompts_dir` lookup.
- `BackendRouter::call_async` routes rendered requests via `default_provider` (plan § 7 Step 1.6 pseudo-shape).
- All ~17 LlmRequest call sites migrated; `cargo build --workspace` clean (compile-error catches missed sites).
- `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs` green (2 of 3 tests; the `#[should_panic]` debug-assert test may be gated per plan § 7 Step 1.3 if fragile under `--release`).
- `agent_runtime_http_smoke_completes_with_config_loaded_from_env` un-ignored + green.
- Stale-comment cleanup per plan § 7 Step 1.8 (R9 mitigation): `tool_loop_http.rs:234–238`'s "PR-5 will introduce a dedicated PromptId variant" comment removed.
- Six regression gates clean (`cargo build --workspace`, `cargo test --workspace -- --skip polyglot_phase3`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build --release --workspace`, polyglot release smoke).
- The plan's WI-1 acceptance command sanity-check (NOT a CI step — operator-run if convenient): `atlas index . --agent-runtime --config .atlas/config.sprint.yaml --log-events /tmp/atlas-wi1-events.jsonl --no-tui --no-budget`. Expected post-WI-1 behaviour: no longer pre-fails with `LlmError::TemplateSyntax("unknown token `{{COMPONENT_KINDS}}`")`. End-to-end success is WI-4's acceptance, not WI-1's — WI-1 may still hard-fail at a later stage (likely Lane A schema mismatch on the agent's first production-prompt output), but the failure mode has shifted from "pre-HTTP template render" to "real LLM round-trip + downstream issue."

### Drop on completion

When WI-1's status-flip commit lands, `git rm docs/superpowers/prompts/2026-05-15-phase8-wi1-continue.md` in that same commit per the drop-on-completion convention (precedent: sprint PR-1 → PR-5 continuation prompts at `7d6f6f3` / `f9315f6` / `ca93814`).

Author the next continuation prompt (`docs/superpowers/prompts/<DATE>-phase8-wi2-continue.md`) alongside WI-1's status flip, so the WI-2 fresh session has a kickoff to point at. Mirror this prompt's structure; swap the body for plan § 8 (WI-2 deliverables + the producer-fail + auditor-fail sites at mod.rs:965 + :1167).

### Begin at plan § 7 Step 1.1

Open the plan; read §§ 1–7; verify the line-number anchors haven't drifted; check whether the status doc exists (create if missing per Step 0 above); then begin **Step 1.1** — write the failing rendered-prompt-bypass test at `crates/atlas-llm/tests/llm_request_rendered_prompt_smoke.rs`.

---

## Why this prompt exists in `docs/superpowers/prompts/`

The `prompts/` directory holds in-flight invocation prompts that bootstrap the next session. They are dropped (precedent `7d6f6f3` / `f9315f6` / `ca93814`) in the status-flip commit of the work they kick off. This is WI-1's kickoff; when WI-1's status-flip commit lands, this prompt gets dropped in that same commit, and the WI-2 kickoff prompt is authored alongside it.
