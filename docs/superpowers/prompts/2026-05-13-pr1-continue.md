# PR-1 continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-1** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. This prompt is self-contained — read this, then read the canon in the order below, then drive the implementation via `superpowers:executing-plans`.

## What PR-1 ships

PR-1 is **small and structural** (LOC budget 200–350). It unblocks every downstream PR but contains no prompt-engineering work.

The deliverables:

1. **`BackendRouter::backend_for_provider(Provider) -> Option<&Arc<dyn LlmBackend>>`** as a new (non-test-gated) impl block in `crates/atlas-llm/src/router.rs`, alongside the existing test-gated `from_dispatch_table`.

2. **`Provider` enum hoisted** from `crates/atlas-agents/src/transport.rs` to `crates/atlas-llm/src/` so the new router method can return a per-provider lookup without inverting the workspace dep direction. `atlas-agents::transport::Provider` becomes a re-export.

3. **`Provider::cross() -> Provider`** method added (one-line: Anthropic↔OpenAi).

4. **`Arc<ForProviderFn>` closure constructed** in `crates/atlas-cli/src/pipeline.rs::run_index_agent_runtime` (replaces today's `for_provider: None`). The closure delegates to the built `BackendRouter`.

5. **`--config <PATH>` universal CLI flag** wired into `crates/atlas-cli/src/main.rs` (or wherever the central args definition lives — verify at plan-time). Applies to all subcommands. Overrides the default `<workspace_root>/.atlas/config.yaml` resolution.

6. **Config loader env-var substitution** in `crates/atlas-cli/src/config.rs` (verify location). `${VAR_NAME}` substitution at load time; missing variable → `ConfigError::MissingEnvVar { var_name }`, **not** silent empty string.

7. **Checked-in `.atlas/config.sprint.example.yaml`** with the canonical sprint backend pairing (`claude-opus-4-7` over `http_anthropic` + `gpt-5-codex` over `http_openai`); no real keys, only `${ANTHROPIC_API_KEY}` / `${OPENAI_API_KEY}` placeholders.

8. **`.gitignore`** extended with `!.atlas/config.sprint.example.yaml` so the example file is tracked while `.atlas/` stays gitignored. Pattern mirrors the existing `!.claude/memory/` exception.

9. **HTTP-backend smoke test** at `crates/atlas-cli/tests/agent_runtime_http_smoke.rs` exercising `--agent-runtime --config <path>` against `test_backend` canned responses. No real API keys; verifies wiring + env-var substitution + Lane B routing.

## Scope exclusions — PR-1 does NOT do these

- **PR-1 does NOT touch the production prompt templates.** The three `PR-7-WIRES-REAL-PROMPT` stubs at `crates/atlas-agents/src/runtime/dispatch.rs:203, :254` and `crates/atlas-agents/src/runtime/mod.rs:919, :928` stay as-is. **That's PR-2.**
- **PR-1 does NOT migrate Lane A's deserializer.** The `serde_json::from_value` call sites at `dispatch.rs:306, :327` stay as JSON. **That's PR-2.**
- **PR-1 does NOT extend Lane A to two-layer validation.** The `lane_a_validate` function at `lane_a.rs:123` stays schema-only. **That's PR-2.**
- **PR-1 does NOT add per-stage evidence scoring.** The new `runtime/audit/evidence.rs` module doesn't exist until PR-2.
- **PR-1 does NOT touch the `PR-7-WIRES-REAL-AUDITOR` stub** at `mod.rs:665`. **That's PR-4.**
- **PR-1 does NOT migrate `agent-runtime-projection.json` → `.yaml`** at `pipeline.rs:1177`. **That's PR-3.**
- **PR-1 does NOT touch the MCP framing.** `crates/atlas-agents/src/mcp/{mod.rs, server.rs, descriptors.rs}` stays as hand-rolled. **That's PR-A.**
- **PR-1 does NOT run Atlas-on-Atlas.** Calibration is PR-5; PR-1 is structural-wiring-only.

If the user asks PR-1 to do anything from the scope-exclusion list above, **stop and surface** — that's not PR-1 scope.

## Reading order

Read these in order before starting work:

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** — the canonical sprint plan; §4 Task 1 is your immediate scope. Read §0–§3 (reading order; deliverable restated; non-negotiables; dependency graph) for context. Skim §4 Tasks 2–7 to know what's downstream so PR-1 doesn't accidentally pre-empt their scope. Don't skip §2.1's 15 decision rows — PR-1 implements row 9 (`for_provider` sibling method) and row 10 (`--config <PATH>` infrastructure).

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — the sprint status file. PR-0's per-PR note has 10 design call-outs forwarded to PR-1 — these are load-bearing. After your work lands, you append PR-1's per-PR note to this file with your commit SHA + any deviations from the plan.

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** — the canonical design artefact (commit `436fdb2`). The plan operationalises it. If the plan and the brainstorm disagree on scope, **the brainstorm wins**. PR-1 implements brainstorm §4 (Wave 1 — Foundation).

4. The five **sprint framing memories** (durable; outlive the sprint; condition every decision):
   - `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent.
   - `.claude/memory/project_atlas_purpose_llm_consumers.md` — Atlas's outputs feed other LLM tools.
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` — no det-engine baseline rhetoric.
   - `.claude/memory/feedback_prefer_existing_crates.md` — prefer maintained crates.
   - `.claude/memory/feedback_yaml_canonical_interchange.md` — YAML is canonical interchange.
   - `.claude/memory/feedback_cross_provider_llm_audit.md` — Lane B uses different-provider auditor.

5. **Sprint-scoped operational memories:**
   - `.claude/memory/project_atlas_common_backend_config.md` — canonical user runtime is `claude_code + codex`; HTTP backends are signal-gathering.
   - `.claude/memory/project_phase7_agent_runtime_default_ratified.md` — `--agent-runtime` is opt-in; HTTP backends are the live path for the sprint.
   - `.claude/memory/project_phase4_plus_roadmap.md` — Phase 7 SHIPPED 2026-05-12; Phase 8 (Cargo retirement) gated on this sprint's items 1–4.
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md`, `.claude/memory/feedback_no_tail_pipe_for_long_tests.md`, `.claude/memory/feedback_atlas_test_subprocess_concurrency.md`, `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints carry forward from Phase 7.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline (the skill expects the plan to be canonical; do not re-explore design space during execution; if something feels under-specified or contradictory, return to the user — that's a plan-reopen, not an execution-time fix).

2. **Open the plan at §4 Task 1** and follow the steps 1.1 → 1.10 in order. Each step has a checkbox `- [ ]`; mark `[x]` as you complete each. Steps use the same checkbox tracking your subagent-driven workflow expects.

3. **Verify after each non-trivial step.** Specifically:
   - After Step 1.1 (router.rs edit): `cargo build -p atlas-llm` clean.
   - After Step 1.2 (Provider hoist verify): `git grep -nE 'atlas_agents::transport::Provider' crates/` shows the cascade is complete (call sites updated to `atlas_llm::Provider` or use the re-export).
   - After Step 1.3 (pipeline.rs edit): `cargo build -p atlas-cli` clean.
   - After Step 1.5 (config.rs env-var substitution): `cargo test -p atlas-cli config_loader` clean.
   - After Step 1.8 (smoke test): `cargo test -p atlas-cli --test agent_runtime_http_smoke` clean.

4. **Step 1.9 is the cumulative-regression gate** — run all six commands in order:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count in loose-bound `0 < cold < 100` (~40 calibrated); warm + reports = 0. Do **NOT** pipe through `tail` (memory `feedback_no_tail_pipe_for_long_tests`).

5. **Step 1.10 is the two-commit pattern.** First commit: the code changes (see plan §4 Task 1 Step 1.10 for the exact `git commit -m` HEREDOC). Second commit: status-file flip — change PR-1's row in `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` from `[ ]` to `[x]`, fill in the PR-1 per-PR note (currently `*(Empty — to be filled by PR-1's session.)*`) with your commit SHA + any deviations + forward-pointers to PR-2.

6. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development` (which `executing-plans` integrates with):

1. **Spec compliance review** — a fresh `feature-dev:code-reviewer` subagent checks the implementation against the plan's §4 Task 1 acceptance gate. Surfaces any spec deviations.

2. **Code quality review** — a fresh `feature-dev:code-reviewer` subagent checks for HIGH-severity issues (correctness bugs, security regressions, broken invariants) and MEDIUM-severity issues (deferrable cleanup). HIGHs are fixed before the status flip; MEDIUMs may be deferred and recorded in the PR-1 per-PR note for later sweeps.

If the implementer-side review flags an issue you can't resolve in one fix-cycle, **stop and surface** to the user. Don't ship broken code to get the checkbox flipped.

## Begin at Step 1

Begin at **Step 1.1: Add `BackendRouter::backend_for_provider` to `crates/atlas-llm/src/router.rs`** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 1.

Open the plan, locate the step, and proceed.
