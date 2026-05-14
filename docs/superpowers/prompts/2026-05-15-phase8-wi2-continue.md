# Phase 8 WI-2 — kickoff prompt

Use this prompt to open the **Phase 8 WI-2 executor session**. The plan at `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` (committed in `78bb22c`) is the authoritative input. WI-1 shipped before this session opens — its code commit migrated `LlmRequest` to `Option<PromptId>` + `Option<String>`, short-circuited the four backends + router, un-ignored the HTTP smoke, and authored the Phase 8 status doc.

---

## Invocation

Invoke the `superpowers:executing-plans` skill, then hand it the body below.

## Body

Execute **Phase 8 WI-2** (HardFail event emission in `call_agent`) — the second of the four sequential work items defined in the plan. WI-2 rewrites two backend-error sites in `crates/atlas-agents/src/runtime/mod.rs` so per-agent `HardFail` events land on the event bus before the backend error propagates: producer-fail at the `let output = outcome?;` site (today ~mod.rs:885 — find at step time, plan flagged drift), and auditor-fail at the existing `Err(e) => { return AuditVerdict::HardFail(...) }` arm. Distinct `error_kind` discriminators: `"backend"` (producer) + `"audit_backend"` (auditor). New test: `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs`.

### Reading order

Read in this order; don't transitively read references unless a step forces it.

1. `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` § 0 (reading order) → § 1 (phase deliverable) → § 2 (framings table — F10 is WI-2's framing lock) → § 4 (regression-guard table — polyglot HOLDS for WI-2) → § 6 (test coverage; WI-2 row names the new test file) → **§ 8 (WI-2 file list + Step 2.1 → 2.6)**. §§ 9 + 10 are out of scope.
2. `docs/superpowers/plans/2026-05-15-phase8-status.md` — confirm WI-1 row is `[x]` and read the per-WI WI-1 notes for any plan-time deviations carried forward.
3. Plan-locked code-side anchors. Before editing each, verify the line numbers haven't drifted from the plan's frozen-2026-05-15 state (and post-WI-1 anchors may have shifted further):
   - `crates/atlas-agents/src/runtime/mod.rs:965` (producer-fail; plan §8 Step 2.2 — actual line may differ post-WI-1)
   - `crates/atlas-agents/src/runtime/mod.rs:1167–1172` (auditor-fail; the rewrite is to the `Err(e) =>` arm, NOT the `let llm_request = LlmRequest::from_rendered(...)` line WI-1 changed)
   - `crates/atlas-agents/src/runtime/mod.rs:817 + 1008` (existing Lane B + Lane A HardFail emit sites — WI-2 mirrors their `agent_id` + `retry_count` shape)
4. `.claude/memory/MEMORY.md` for the active framings the plan inherits (`[[feedback_atlas_llm_spine_intent]]`, `[[project_atlas_common_backend_config]]`, `[[project_phase7_agent_runtime_default_ratified]]`, `[[feedback_prefer_existing_crates]]`, `[[feedback_no_tail_pipe_for_long_tests]]`, `[[feedback_release_workspace_build_for_polyglot]]`, `[[feedback_cargo_skip_polyglot_pattern]]`, `[[feedback_atlas_test_subprocess_concurrency]]`).
5. Sprint PR-5 closeout note item 4 in `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` — the producer-fail / auditor-fail diagnostic-visibility gap WI-2 closes. Skim only; the plan's § 1 already summarises.

### Locked decisions (inherited from the plan; NOT re-litigated)

- **HardFail emission sites (plan F10 + § 8):** producer-fail at the `let output = outcome?;` propagation site (rewrite to a match-arm); auditor-fail at the existing `Err(e) => { return AuditVerdict::HardFail(...) }` arm. Both emit `AgentEvent::HardFail` *before* returning.
- **`error_kind` discriminators (plan F10):** `"backend"` for producer-fail; `"audit_backend"` for auditor-fail. The two discriminators are load-bearing — tests assert on them.
- **`retry_count` shape (plan § 8 Step 2.2):** producer-fail uses `lane_a_retries` (the loop's local counter; can be 0 or 1). Auditor-fail uses `0` (Lane B doesn't retry).
- **Test design (plan § 8 Step 2.1):** synthetic `AlwaysErroringBackend` for producer-fail variant; producer = `AlwaysSucceedingBackend` returning a canned classify YAML envelope + erroring auditor for auditor-fail variant. Both assertions check (1) `HardFail` event lands on the bus, (2) Err propagation preserves the backend's verbatim error text.
- **Commit decomposition (plan § 8 Step 2.6):** one code commit (mod.rs 2-site rewrite + new test file) + one status-flip commit. Don't split — producer + auditor are coupled by the same diagnostic-visibility framing.

### Operating discipline

- **No scope creep outside WI-2.** Do NOT touch:
  - WI-3 deliverables (Cargo classifier deletion, prompt rubric rewrite, `default_tool_catalog` edit).
  - WI-4 deliverables (calibration run, parity test un-ignore, recast spec §11.2 amendment, memory updates).
- **Cumulative regression guard (plan § 4):** polyglot release smoke MUST stay green across WI-2 — 2 tests pass, cold count in `0 < cold < 100`, warm + reports = 0, wall-time 100–110s. WI-2 doesn't touch the deterministic-spine path, so HardFail emission is invisible to the smoke; drift here is a bug. Use `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` after `cargo build --release --workspace` per `[[feedback_release_workspace_build_for_polyglot]]`. Do NOT pipe through `tail` per `[[feedback_no_tail_pipe_for_long_tests]]`. Run dev workspace tests and release polyglot sequentially per `[[feedback_atlas_test_subprocess_concurrency]]`.
- **Inherit the plan's risk register.** No WI-2-specific risks beyond R5 (`ShimError::MissingProjectionField` may surface in WI-4 — out of scope here). The threading-cost question in plan § 8 Step 2.3 (auditor function may need `event_bus` + `audit_agent_id` threaded in) is the only step-time judgment call; the plan names both shapes (thread state in OR return `Result<AuditVerdict, AuditorBackendError>`) and lets the implementer pick.
- **No new memory writes this session.** WI-2 implements a locked framing; it makes no architectural decisions. Memory updates land in WI-4 per plan § 13.

### Deliverable shape

Two commits:

1. **Code commit:** mod.rs producer-fail rewrite + auditor-fail rewrite + new `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs`. Title: `phase8 WI-2: HardFail event emission for backend errors`.
2. **Status-flip commit:** WI-2 row `[ ]` → `[x]` in the status doc; WI-2 per-WI note (what shipped, regression gates summary, any plan-time deviations); "Last updated" header refresh. Title: `phase8 WI-2: status flip`.

### Acceptance gate (mirrors plan § 8's "Acceptance gate (WI-2)")

Verify every bullet before flipping the status row:

- Producer-fail and auditor-fail sites emit `AgentEvent::HardFail` before returning.
- Distinct `error_kind` discriminators: `"backend"` (producer) + `"audit_backend"` (auditor) — assertion-load-bearing.
- `agent_runtime_hardfail_emission.rs` both tests green.
- All six regression gates clean (`cargo build --workspace`, `cargo test --workspace -- --skip polyglot_phase3`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build --release --workspace`, polyglot release smoke).
- Re-running PR-5's calibration command (operator-run if convenient, NOT a CI step) after WI-1 + WI-2 against a known-failing backend OR against a real classify-stage failure produces an event-log file with `HardFail` records carrying diagnostic context.

### Drop on completion

When WI-2's status-flip commit lands, `git rm docs/superpowers/prompts/2026-05-15-phase8-wi2-continue.md` in that same commit per the drop-on-completion convention.

Author the next continuation prompt (`docs/superpowers/prompts/<DATE>-phase8-wi3-continue.md`) alongside WI-2's status flip, so the WI-3 fresh session has a kickoff to point at. Mirror this prompt's structure; swap the body for plan § 9 (WI-3 deliverables: Cargo classifier retirement, classify-prompt rubric rewrite, polyglot smoke recalibration).

### Begin at plan § 8 Step 2.1

Open the plan; read §§ 1–2 + 4 + 6 + 8; verify the anchor line numbers haven't drifted (post-WI-1 the producer-fail anchor may have shifted); then begin **Step 2.1** — write the failing producer-fail + auditor-fail tests at `crates/atlas-agents/tests/agent_runtime_hardfail_emission.rs`.

---

## Why this prompt exists in `docs/superpowers/prompts/`

The `prompts/` directory holds in-flight invocation prompts that bootstrap the next session. They are dropped (precedent `7d6f6f3` / `f9315f6` / `ca93814`) in the status-flip commit of the work they kick off. This is WI-2's kickoff; when WI-2's status-flip commit lands, this prompt gets dropped in that same commit, and the WI-3 kickoff prompt is authored alongside it.
