# Atlas vNext — Production-prompt sprint — Status

Companion to `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`. This file tracks per-PR completion state across sessions. The PR-1 continuation prompt at `docs/superpowers/prompts/2026-05-13-pr1-continue.md` reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-13 (PR-0 landed — plan + status + PR-1 continuation prompt; PR-0 status row pre-flipped `[x]` in the same commit per the brief's two-commit-exception for PR-0).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + PR-1 continuation prompt (docs only)
- [ ] PR-1 — `BackendRouter::backend_for_provider` + `Arc<ForProviderFn>` closure + `--config <PATH>` flag + `.atlas/config.sprint.example.yaml` + HTTP-backend smoke test (small / structural)
- [ ] PR-2 — Production dispatch prompts (replaces `PR-7-WIRES-REAL-PROMPT` stubs at `dispatch.rs:203, :254`) + Lane A YAML migration (`serde_json::from_value` → `serde_yaml::from_str` at `dispatch.rs:306, :327`) + dispatch-stage Lane A evidence scoring + `runtime/yaml_strict.rs` + `runtime/prompt_examples.rs` + `runtime/audit/evidence.rs` (medium)
- [ ] PR-3 — Production classify/reduce/project prompts (replaces stubs at `mod.rs:919, :928` + new `build_project_prompt`) + 4 typed-output structs in new `runtime/outputs.rs` + remaining 4 evidence-score functions in `evidence.rs` + canonical-schema shim `runtime/projection_to_canonical.rs` + `agent-runtime-projection.json` → `.yaml` migration at `pipeline.rs:1177` (large)
- [ ] PR-4 — Cross-provider auditor (replaces `PR-7-WIRES-REAL-AUDITOR` stub at `mod.rs:665`) + `runtime/audit/audit_prompt.rs` + `runtime/audit/verdict.rs` + revision-prompt path + on-disk verdict at `.atlas/audit/<stage>/<target>.yaml` (medium)
- [ ] PR-5 — Atlas-on-Atlas calibration + intrinsic-metrics recording (cold tokens per provider; iteration count; wall time; evidence-score distribution per stage; Lane A retry counts; audit-verdict distribution; shim missing-field count) + within-LLM-spine cross-transport parity test + closeout note + memory updates (small code surface; measurement-heavy)
- [ ] PR-A — `rmcp` migration of PR-1's hand-rolled MCP framing (or `jsonrpsee` + thin shim if `rmcp` fails 4-criterion verification gate) + subprocess MCP `serve_client` driver at `mcp/serve_client.rs` + `tool_loop_http.rs` subprocess-transport branch wiring (medium, parallel after PR-1)
- [ ] PR-B — `--disallowedTools` live-subprocess probe at `tests/mcp_disallowed_tools.rs` (small, parallel after PR-A; `#[ignore]`-gated)

When every box is `[x]`, the sprint is complete. PR-5's closeout note appended below; memory `project_phase4_plus_roadmap` is updated to mark the sprint SHIPPED + Phase 8 (Cargo retirement per recast §11.2) unblocked.

**Gating set:** PRs 1–4 unblock Phase 8 brainstorming. PR-5, PR-A, PR-B may land afterward and may overlap with Phase 8 plan-writing. If Phase 8 work begins before PR-5 ships, the brainstorm should note the Atlas-on-Atlas baseline isn't recorded yet (plan §7.9).

## Dependency graph (canonical in plan §3)

```
PR-0 (plan + status + PR-1 continuation prompt)
  │
  ▼
PR-1 (backend_for_provider + ForProviderFn closure + --config flag + HTTP smoke)
  │
  ├──► PR-A (rmcp migration + subprocess MCP serve_client driver) ──► PR-B (--disallowedTools probe)
  │       (parallel with PR-2 / PR-3 / PR-4 / PR-5)
  │
  ▼
PR-2 (dispatch prompts + Lane A YAML migration + dispatch-stage evidence scoring)
  │
  ▼
PR-3 (classify/reduce/project prompts + outputs.rs + evidence.rs extensions
      + canonical-schema shim + projection JSON→YAML)
  │
  ▼
PR-4 (cross-provider auditor + audit prompt + transcript rendering + on-disk verdict)
  │
  ▼
PR-5 (Atlas-on-Atlas calibration + intrinsic metrics + cross-transport parity + closeout)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (landed 2026-05-13).
- **Wave 1 (after PR-0):** PR-1 — sequential; gates everything downstream.
- **Wave 2 (after PR-1):** PR-2 — sequential. Gates Phase 8.
- **Wave 3 (after PR-2):** PR-3 — sequential. Gates Phase 8. Largest single PR in the sprint (1500–2200 LOC budget; brainstorm §12 risk #1 "stop and surface at >2× budget" applies).
- **Wave 4 (after PR-3 and PR-1):** PR-4 — sequential. Gates Phase 8.
- **Wave 5 (after PR-4):** PR-5 — post-gate. Calibration + closeout.
- **Parallel track:** PR-A may dispatch as soon as PR-1 lands; PR-B follows PR-A. Both run alongside PR-2/3/4/5.

The cumulative regression guard (`cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` after `cargo build --release --workspace`) holds at cold count in loose-bound `0 < cold < 100` (~40 calibrated); warm + reports = 0. The polyglot fixture has full override coverage, so production-prompt changes from this sprint remain unreachable from the smoke — by construction the smoke is unaffected by prompt-template work.

Memory pointers (must be re-read by each subagent at PR-N start):
- Sprint framings: `feedback_no_deterministic_engine_comparison`, `project_atlas_purpose_llm_consumers`, `feedback_prefer_existing_crates`, `feedback_yaml_canonical_interchange`, `feedback_cross_provider_llm_audit`.
- Operational: `project_atlas_common_backend_config`, `project_phase7_agent_runtime_default_ratified`, `project_phase4_plus_roadmap`, `feedback_worktree_base_verification`.
- Execution discipline: `feedback_no_tail_pipe_for_long_tests`, `feedback_release_workspace_build_for_polyglot`, `feedback_atlas_test_subprocess_concurrency`, `feedback_cargo_skip_polyglot_pattern`, `feedback_no_iterator_stubs_for_singletons`, `feedback_no_version_on_workspace_path_deps`.

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of what's worth recording: deviations from the plan that the next session needs to know; decision-table row interpretations confirmed; cross-cutting refactor surfaces; follow-up cleanup deferred; anything load-bearing for the cumulative regression guard.

### PR-0

2026-05-13 — Landed: the sprint plan, this status file, and the PR-1 continuation prompt. Plan: `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`. PR-1 continuation prompt: `docs/superpowers/prompts/2026-05-13-pr1-continue.md`. Design anchor: the sprint brainstorm at `docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md` (commit `436fdb2` on main; the 15-row decision table is locked + the five framings condition every decision). YAML-canonical ratification commit `a852be5` was the most recent main commit prior to PR-0.

PR-0 follows the brief's **single-commit exception** to the two-commit pattern: plan + status (with PR-0 row already `[x]`) + continuation prompt land in one commit. PRs 1–B all follow the canonical two-commit pattern (code-commit + status-flip).

Key PR-0 design call-outs to surface to PR-1+:

1. **`Provider` enum hoist from atlas-agents to atlas-llm** at PR-1 Step 1.1. The new `BackendRouter::backend_for_provider(Provider)` method lives in `atlas-llm`; the existing `Provider` enum lives in `atlas-agents::transport`. Inverting the dep direction would require either a re-export or a cycle — the plan calls for hoisting `Provider` (small enum: Anthropic | OpenAi) to `atlas-llm` and adding a re-export at `atlas-agents::transport::Provider` so existing call sites keep working. PR-1 implementer runs `git grep -nE 'atlas_agents::transport::Provider|use .*transport::Provider' crates/` to find the cascade. Plan-time call ratified.

2. **`Provider::cross() -> Provider` method** added at PR-1 (one-line implementation: Anthropic ↔ OpenAi). PR-4's audit closure uses it to look up the cross-provider auditor.

3. **YAML-canonical discipline** (`feedback_yaml_canonical_interchange`) is the durable framing for every interchange artefact this sprint introduces. JSON survives only at LLM tool-use APIs (Anthropic / OpenAI tool calls are JSON-native), JSONL event streams, and MCP/gRPC wire protocols. If a PR finds itself authoring a new `.json` file under `.atlas/` or in `crates/atlas-*` source, **stop and surface** — the only legitimate non-YAML interchange is the three exceptions above.

4. **No deterministic-engine baseline rhetoric** (`feedback_no_deterministic_engine_comparison`). Calibration anchors on intrinsic LLM-runtime properties only. If a PR's commit message or test name implies a deterministic-engine comparison, the PR has drifted. The Phase 7 `polyglot_smoke_cross_transport_parity_claude_code_vs_codex` test stays in the tree as **forensic**, not load-bearing; PR-5's `agent_runtime_cross_provider_parity.rs` replaces it for new-work regression detection.

5. **No model downgrade tier** (decision row 7). Sprint commits to `claude-opus-4-7` (Anthropic) + `gpt-5-codex` (OpenAI) for the recorded baseline. Cheaper iteration during prompt-engineering dev work is permitted (e.g., swap to `claude-haiku-4-5` via `--config` for fast feedback) but the final calibration recorded in PR-5 uses Opus 4.7.

6. **`rmcp` maturity verification gate** at PR-A plan-time (§2.2 + Task 6 Step A.1). Four concrete criteria: publishing cadence (within 12 months); repo activity (within 6 months); multi-client server abstraction documented; transitive-dep footprint (≤30 direct, no WebSocket/TLS/HTTP-server transitives). Failing any one criterion routes to `jsonrpsee` + thin shim. PR-A's first commit is the verification note + the decision.

7. **PR-3 is the scope-creep risk** (plan §7.1; LOC budget 1500–2200; stop-and-surface at 4400). PR-3's task list recommends a six-commit decomposition (outputs.rs + evidence.rs; classify; reduce; project + surface; shim + migration; status flip). Implementer surfaces if work approaches the 4400 threshold.

8. **Cumulative regression guard for every PR-1+:** `cargo build --release --workspace` + `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`. Polyglot smoke is unaffected by prompt-template changes (fixture has full override coverage; LLM dispatch sites are unreachable from it). Cold count in loose-bound `0 < cold < 100`; warm + reports = 0. Do not pipe through `tail` (memory `feedback_no_tail_pipe_for_long_tests`); do not run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run (memory `feedback_atlas_test_subprocess_concurrency`); use `--skip polyglot_phase3` substring for skipping (memory `feedback_cargo_skip_polyglot_pattern`).

9. **Commit message convention:** `sprint: PR-N <short title>` (matches main's existing `sprint:` prefix for the brainstorm + memory commits). NOT `phase7: PR-N` — that prefix is reserved for actual Phase 7 PRs and would collide forensically.

10. **Live source-file landmarks verified clean** against current main 2026-05-13 (commit `a852be5`). The full list lives in plan §8. Spot-check candidates the PR-1 implementer should re-verify: `dispatch.rs:203, :254, :274, :285, :306, :327, :339, :346`; `mod.rs:350, :356, :461, :477, :665, :919, :928, :1008`; `router.rs:14, :142, :213`; `pipeline.rs:1015, :1177`; `lane_a.rs:44, :62, :97, :123`; `atomic_write.rs:40, :134`.

PR-0 commit SHA: `b44593a` (single commit per the brief's PR-0 exception; status row pre-flipped `[x]` in the same commit).

### PR-1

*(Empty — to be filled by PR-1's session.)*

### PR-2

*(Empty — to be filled by PR-2's session.)*

### PR-3

*(Empty — to be filled by PR-3's session.)*

### PR-4

*(Empty — to be filled by PR-4's session.)*

### PR-5

*(Empty — to be filled by PR-5's session. This is the sprint-closeout PR; its note carries the recorded Atlas-on-Atlas baseline numbers + cross-provider parity outcome + sprint SHIPPED summary.)*

### PR-A

*(Empty — to be filled by PR-A's session. PR-A's first commit is the `rmcp` maturity-verification note at `crates/atlas-agents/src/mcp/rmcp_verification.md` documenting the 4-criterion decision.)*

### PR-B

*(Empty — to be filled by PR-B's session.)*
