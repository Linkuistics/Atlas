# Atlas vNext — Production-prompt sprint — Status

Companion to `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`. This file tracks per-PR completion state across sessions. The active continuation prompt at `docs/superpowers/prompts/2026-05-13-pr4-continue.md` (sequential, next on the critical path; last gating PR before Phase 8 unblocks) reads this file to find the next PR to dispatch. The parallel track (PR-A + PR-B) is closed; PR-4 + PR-5 are the only remaining sequential PRs. Each session re-points this line as PRs land + new continuation prompts are authored; the expired continuation prompts for PR-1 / PR-2 / PR-3 / PR-A / PR-B were dropped after their PRs shipped (matches the cleanup pattern from commit `7d6f6f3`).

**Last updated:** 2026-05-13 (PR-3 + PR-B both landed this session — disjoint-file parallelism held per the original plan. PR-3: production classify/reduce/project prompts + 4 typed-output structs in new `runtime/outputs.rs` + canonical-schema shim + remaining 4 evidence-score functions in `evidence.rs` + projection JSON→YAML migration; implementation commit `4776013`, integration-tests commit `b31069c`, status flip `aa49ac4`. PR-B: live-subprocess `--disallowedTools` probe at `crates/atlas-agents/tests/mcp_disallowed_tools.rs`; code-commit `b8af469`, status flip `ae71dfa`. Empirical PR-B findings against upstream `claude` 2.1.140 (Claude Code) pinned in `crates/atlas-agents/src/mcp/restrictions.md`. Prior landings this session: PR-A code-commit `c07c5d5` + verification-note `d1df478`; PR-2 code-commit `876ea24`.)

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + PR-1 continuation prompt (docs only)
- [x] PR-1 — `BackendRouter::backend_for_provider` + `Arc<ForProviderFn>` closure + `--config <PATH>` flag + `.atlas/config.sprint.example.yaml` + HTTP-backend smoke test (small / structural)
- [x] PR-2 — Production dispatch prompts (replaces `PR-7-WIRES-REAL-PROMPT` stubs at `dispatch.rs:203, :254`) + Lane A YAML migration (`serde_json::from_value` → `serde_yaml::from_str` at `dispatch.rs:306, :327`) + dispatch-stage Lane A evidence scoring + `runtime/yaml_strict.rs` + `runtime/prompt_examples.rs` + `runtime/audit/evidence.rs` (medium)
- [x] PR-3 — Production classify/reduce/project prompts (replaces stubs at `mod.rs:919, :928` + new `build_project_prompt`) + 4 typed-output structs in new `runtime/outputs.rs` + remaining 4 evidence-score functions in `evidence.rs` + canonical-schema shim `runtime/projection_to_canonical.rs` + `agent-runtime-projection.json` → `.yaml` migration at `pipeline.rs:1177` (large)
- [ ] PR-4 — Cross-provider auditor (replaces `PR-7-WIRES-REAL-AUDITOR` stub at `mod.rs:665`) + `runtime/audit/audit_prompt.rs` + `runtime/audit/verdict.rs` + revision-prompt path + on-disk verdict at `.atlas/audit/<stage>/<target>.yaml` (medium)
- [ ] PR-5 — Atlas-on-Atlas calibration + intrinsic-metrics recording (cold tokens per provider; iteration count; wall time; evidence-score distribution per stage; Lane A retry counts; audit-verdict distribution; shim missing-field count) + within-LLM-spine cross-transport parity test + closeout note + memory updates (small code surface; measurement-heavy)
- [x] PR-A — `rmcp` migration of PR-1's hand-rolled MCP framing (or `jsonrpsee` + thin shim if `rmcp` fails 4-criterion verification gate) + subprocess MCP `serve_client` driver at `mcp/serve_client.rs` + `tool_loop_http.rs` subprocess-transport branch wiring (medium, parallel after PR-1)
- [x] PR-B — `--disallowedTools` live-subprocess probe at `tests/mcp_disallowed_tools.rs` (small, parallel after PR-A; `#[ignore]`-gated)

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
- **Wave 1 (after PR-0):** PR-1 — sequential; gates everything downstream. (Landed 2026-05-13; commit `a064f63`.)
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

2026-05-13 — Landed. Code-commit: `a064f63` (`sprint: PR-1 backend_for_provider + ForProviderFn closure + --config flag`). Status-flip commit follows in the same session.

PR-1 closes the PR-7 `for_provider: None` deferral with a real `BackendRouter`-backed closure, hoists `Provider` out of `atlas-agents` into `atlas-llm` to keep dep direction one-way, adds the universal `--config <PATH>` flag with env-var substitution, and ships a 340-line HTTP-backend smoke test that pins the wiring without live API calls. Workspace builds clean; HTTP smoke + clap-argument unit tests + router unit tests all green. Cumulative regression guard (polyglot fixture) unaffected by this PR's surface (full override coverage means LLM dispatch sites stay unreachable; PR-1 touched only the runtime wiring, not the dispatch-site prompts).

Key deviations + extensions from the plan PR-2+ implementers should know about:

1. **Two router constructors, not one.** `BackendRouter::new_from_config` keeps Phase 7's `reject_http_for_filesystem_required_prompt` discipline unchanged. A sibling `BackendRouter::new_for_agent_runtime` was added that relaxes the rule (HTTP backends are valid for every stage when the AgentRuntime owns the tool loop). Both delegate to a private `new_inner` with an `allow_http_filesystem_prompts: bool` flag. PR-2's prompt-template work should plug into the agent-runtime path; PR-2 has no business reaching the deterministic `new_from_config` path.

2. **`atlas_cli::backend::build_agent_runtime_backend_with_counter`** is the matching helper at the call-site layer. The deterministic-index path (`build_production_backend_with_counter`) is untouched; the `--agent-runtime` path in `main.rs::run_index_cmd` and the reports commands (`run_modularity_cmd_with_config`, `run_divergence_cmd_with_config`) call the new agent-runtime variant. The split lives at `crates/atlas-cli/src/backend.rs`; both helpers share the underlying config-load path.

3. **`default_transport_from_config` helper in `pipeline.rs`** derives `AgentRuntime::default_transport` from `defaults.model`'s provider prefix instead of hard-coding `TransportFlavour::ClaudeCode`. Mapping: `anthropic` → `HttpAnthropic`; `openai` / `openrouter` → `HttpOpenai`; `claude-code` → `ClaudeCode`; `codex` → `Codex`. This means `--config .atlas/config.sprint.yaml` is enough to switch the runtime between the canonical subprocess pair and the sprint's HTTP pairing — no code changes needed per backend variant. Unknown providers return `IndexError::Other` (caught by the existing error-surfacing path). Brainstorm decision-table row 6 resolution carries through cleanly.

4. **`provider_from_config_key` helper maps config-keys to enum:** `anthropic` / `claude-code` → `Provider::Anthropic`; `openai` / `codex` → `Provider::OpenAi`. This extends cross-provider audit to the **canonical subprocess pair** (`claude_code + codex`), not just HTTP backends. PR-4's auditor will get cross-provider routing for free when a user runs the canonical subprocess config — a meaningful capability extension beyond the plan's HTTP-only framing. Memory `project_atlas_common_backend_config` is honoured.

5. **`Provider::cross()` lives on the enum itself** at `crates/atlas-llm/src/lib.rs` (one-line `match`: Anthropic↔OpenAi). PR-4 should call `Provider::cross()` to look up the sibling, then `BackendRouter::backend_for_provider(provider)` to materialize the backend. Decision-table row 5 resolution wired and tested (`provider_cross_returns_opposite_vendor` in `router.rs::tests`).

6. **`ConfigError::EnvVarUnset` renamed to `ConfigError::MissingEnvVar { var_name }`** per Step 1.5 spec. The interpolation site at `config.rs::interpolate_segment` and the existing unit test both reflect the rename. Any future grep over `EnvVarUnset` will miss — use `MissingEnvVar` going forward.

7. **`--config <PATH>` lives directly in `main.rs::Cli`** as a `#[arg(long, value_name = "PATH", global = true)]` field. The plan's tentative `cli_args.rs` filename did not materialize — `main.rs` owns the universal flag definition; `resolve_config_path(output_dir, override) -> PathBuf` is the small helper that picks override-or-default. Three clap parser tests at `main.rs::tests` cover position-before/after subcommand + override-replaces-default behaviour.

8. **HTTP smoke test fingerprint:** `crates/atlas-cli/tests/agent_runtime_http_smoke.rs` (340 lines). Uses an in-memory `StagedBackend` for the actual LLM calls (canned responses keyed by prompt substring) and a real `BackendRouter::new_for_agent_runtime` for Lane B provider lookup. No live API keys required. `ENV_LOCK` mutex serialises env-var-touching subtests (config-loader substitution checks). `EnvGuard` captures + restores so suites don't leak across tests.

9. **Cascade from `Provider` hoist** touched four files beyond the plan-named set: `atlas-agents/src/runtime/audit/lane_b.rs` (import path), `atlas-agents/src/runtime/mod.rs` (comment update + import), `atlas-agents/src/runtime/tool_loop_http.rs` (import path), `atlas-agents/tests/audit_lane_b.rs` (test import), `atlas-cli/src/tui/state.rs` + `tui/token_panel.rs` (TUI per-provider rendering). `atlas-agents::transport::Provider` is now a `pub use` re-export so legacy import paths still compile.

10. **Plan items NOT exercised by PR-1** (deliberately deferred to their owning PRs): the Lane A YAML migration at `dispatch.rs:306, :327` is **untouched** (PR-2 owns it); the `agent-runtime-projection.json` → `.yaml` rename at `pipeline.rs:1177` is **untouched** (PR-3 owns it); the production prompt-template content at `dispatch.rs:203, :254` and `mod.rs:919, :928` is **untouched** (PR-2 + PR-3 own it). PR-1 strictly shipped the structural wiring + the config-flag plumbing.

PR-2 reading order before dispatch: this PR-1 note, then the plan §4 Task 2 block, then brainstorm §5.5 + §6.1 + §6.2 + §6.4 + §12.8 (Norway-problem risk). The two-router-constructor split (item 1 above) is the most load-bearing surface change: PR-2's modifications to `dispatch.rs` parse paths run inside the agent-runtime tool loop, so they live downstream of `new_for_agent_runtime`. PR-2 should NOT touch the deterministic-path rejection rule.

PR-1 commit SHA: `a064f63` (single code-commit; status flip in a separate commit per the two-commit pattern).

### PR-2

2026-05-13 — Landed. Code-commit: `876ea24` (`sprint: PR-2 production dispatch prompts + Lane A YAML migration + dispatch-stage evidence scoring`). Status-flip commit follows in the same session.

PR-2 replaced the two `PR-7-WIRES-REAL-PROMPT` stubs with production dispatch prompts that advertise a fenced ```yaml envelope, migrated Lane A's deserialiser from `serde_json::from_value` to `serde_yaml::from_str`, and introduced the dispatch-stage half of the evidence-floor scoring system. All six cumulative regression gate commands pass: workspace build, workspace tests, clippy with -D warnings, fmt --check, release workspace build, polyglot fixture release run (2 tests; cold count in loose-bound `0 < cold < 100`).

LOC: ~1546 insertions / 88 deletions across 13 files (7 modified, 6 new). Within the "structural medium" budget; below the brainstorm §12 risk #1 "stop and surface at 2x" threshold.

Key deviations + extensions from the plan PR-3+ implementers should know about:

1. **Evidence-floor fallback for not-yet-implemented stages returns 1.0, NOT 0.0.** The plan's §2.4 pseudo-code suggests returning `0.0` for classify/surface/reduce/project to "fail-loud until PR-3", but the existing `lane_b_wired_into_call_agent_skips_on_strong_grade` integration test exercises end-to-end `run_workspace` which classifies after dispatch — clamping non-dispatch grades to Declines would mis-fire Lane B for every classify call. The PR-2 implementation returns `1.0` for `Stage::Classify | Surface | Reduce | Project` so the LLM's self-grade flows through unchanged. **PR-3 MUST replace this fallback with real evidence functions** at the same time as adding the typed-output structs at `outputs.rs`. Until then, non-dispatch stages do NOT exercise the evidence-floor clamping; they preserve the pre-PR-2 hardcoded-Strong behaviour.

2. **`lane_a_validate` signature changed**: `(output, stage, candidate_ids) -> Result<(), SchemaError>` became `(output, stage, candidate_ids, transcript) -> Result<Grade, SchemaError>`. The schema-check layer is preserved verbatim (unknown edge kinds, unknown component ids, surface-count check); a second layer reads `output.confidence_grade()`, computes the evidence ratio via `compute_evidence_score`, and clamps with `claimed.min(grade_ceiling(score))`. The call site in `runtime/mod.rs::run_tool_loop_with_lane_a` uses the returned grade instead of hardcoding `Grade::Strong`. PR-4's auditor wiring will pick up the now-real grade flow.

3. **`Grade` variant order is now load-bearing.** Reordered to `Declines < Weak < Moderate < Strong` so `derive(PartialOrd, Ord)` produces the natural confidence ordering used by `claimed.min(evidence_max)`. Comment in `events.rs` warns future editors not to alphabetise. PR-4 auditor verdict comparisons should also use the natural order.

4. **AgentOutput gained `text: String` field** populated by `parse_final_output` when the LLM response carries `content[].text` blocks. Dispatch parsers fence-extract from this field. Backward-compat: the `from_value` constructor still works; the new `from_value_and_text` constructor is what `parse_final_output` calls. For PR-3 classify/surface/reduce/project YAML migrations, parsers should pull from `output.text` the same way.

5. **`Transcript::TranscriptRecord::ToolResult` gained an `args` field** with `#[serde(default)]` so pre-PR-2 cached transcripts deserialise unchanged. The evidence-floor scorer needs the per-call args to match dispatched candidate manifest paths. New accessors: `read_file_paths`, `tool_called`, `tool_calls_for`, `push_synthetic_tool_call` (test-only, `#[doc(hidden)]`).

6. **`SubsystemsOverrideFile` / `ComponentsOverrideFile` extended with `candidates_considered: Vec<L1CandidateRef>` + `confidence_grade: Option<String>`.** Both fields are `#[serde(default)]` so user-authored override files don't need to include them (the override-shortcircuit path leaves them empty). LLM dispatch output populates them; Lane A's evidence-floor scorer reads them via `AgentOutput::l1_candidates_referenced()`. The `L1CandidateRef` struct lives in `runtime/audit/lane_a.rs` and is re-exported through `runtime::audit`.

7. **YAML-1.2 reality vs. plan's YAML-1.1 framing.** The plan's Norway-problem test names (e.g. `component_id: NO` → bool false) reflect YAML 1.1 semantics; serde_yaml 0.9 follows YAML 1.2, in which `NO` / `yes` / `on` remain strings. The PR-2 regression suite at `yaml_envelope_norway_problem.rs` (9 tests) is calibrated for YAML-1.2 reality: the adapter catches the active coercion hazards (`id: true`, `id: 1.10`, `id: 42`, `id: null`), and the suite pins that `NO` / `yes` / `on` naturally stay strings. The strict adapter retains the "Norway-problem" framing in error messages for actionable retry prompts.

8. **Strict-string adapter applied only to `SubsystemOverrideEntry::id`** for PR-2 minimal scope. The component-side string fields (`component_id` keys in `ComponentsOverrideFile.components` BTreeMap; `ComponentFieldOverrides::language` etc.) are NOT yet adapter-guarded because (a) BTreeMap keys are themselves deserialised as strings via the inner type's `Deserialize` impl and the adapter doesn't trivially wire there, (b) the field-override values are `Option<String>` which serde handles via `#[serde(default)]`. **PR-3 should sweep the new typed-output structs in `outputs.rs` and apply the adapter to every identity-shaped string field.**

9. **`parse_final_output` now tries YAML fence-extract first** before falling back to JSON-parse. Order matters: text that contains a fenced ```yaml block parses as YAML; raw-JSON text still works via the existing branch; everything else wraps as `{"text": <raw>}`. PR-3's classify/surface/reduce/project prompts should also emit fenced YAML; this path will catch them automatically.

10. **Default dispatch caps**: `DEFAULT_DISPATCH_SOFT_CAP = 15`, `DEFAULT_DISPATCH_HARD_CAP = 30`. Exported `pub const` from `runtime/dispatch.rs` so future callers can reference them without re-hardcoding. The prompts embed caller-supplied values verbatim, not the constants — the drift catcher in `tests/dispatch_prompt_shape.rs` exercises non-default caps (7/42) to confirm the prompt isn't accidentally using a hardcoded literal.

11. **Test fixture migration scope was minimal**: only `dispatch_shortcircuit.rs::dispatch_without_override_file_fires_llm_agent` needed its canned response migrated from JSON to fenced YAML. The `audit_lane_b.rs` `ClassifyBackend` emits classify-stage JSON which still flows through `parse_final_output`'s JSON-fallback branch (no fence present → JSON-parse succeeds → backward-compat preserved).

PR-3 reading order before dispatch: this PR-2 note, then PR-1's note, then the plan §4 Task 3 block, then brainstorm §6.1 + §6.2 + §6.4 + §12.8. The two load-bearing surface changes PR-3 must honour: (a) the evidence-floor fallback policy — PR-3 MUST replace the `1.0` fallback for classify/surface/reduce/project with real evidence functions, AND (b) the `text` field on `AgentOutput` — PR-3's classify/surface/reduce/project prompts should emit fenced YAML and the parsers should read from `output.text` the same way dispatch does.

PR-2 commit SHA: `876ea24` (code-commit); status flip in a separate commit per the two-commit pattern.

### PR-3

2026-05-13 — Landed. Implementation commit: `4776013` (`sprint: PR-3 typed outputs + production prompts + canonical-schema shim`). Integration-tests commit: `b31069c` (`sprint: PR-3 drift-catchers + evidence-floor + shim integration tests`). Status-flip commit follows in the same session.

PR-3 replaced the two `PR-7-WIRES-REAL-PROMPT` stubs at `mod.rs:919, :928` with production classify + reduce prompts, added a new `build_project_prompt` (no PR-7 placeholder predecessor), authored 4 typed LLM-agent output structs in new `runtime/outputs.rs`, wired the remaining 4 per-stage evidence-score functions in `evidence.rs` (replacing PR-2's `1.0` placeholder for non-dispatch stages), and shipped the canonical-schema shim at new `runtime/projection_to_canonical.rs` plus the `agent-runtime-projection.json` → `.yaml` migration at `pipeline.rs:1177`. All six cumulative regression gate commands pass: workspace build, workspace tests (`--skip polyglot_phase3`), clippy with `-D warnings`, fmt `--check`, release workspace build, polyglot fixture release run (2 tests cold-count window upheld).

LOC: ~2051 insertions / ~63 deletions across 9 files in commit 1 + ~1228 insertions across 9 new test files in commit 2 = ~3279 LOC. Within the 1500–2200 budget when scoped to implementation only; the test footprint pushed total beyond the upper bound but stayed well under the 4400 stop-and-surface threshold. Brainstorm §12 risk #1 not triggered.

Key deviations + extensions from the plan PR-4+ implementers should know about:

1. **Canonical struct ownership decision: PR-3 authored new `*Canonical` shapes locally.** The plan-time grep for `pub struct (ComponentsYaml|SubsystemsYaml|RelatedComponentsYaml)` returned empty. The deterministic engine's `atlas_index::{ComponentsFile, SubsystemsFile}` and `component_ontology::RelatedComponentsFile` carry engine-only fields (`path_segments`, `cache_fingerprints`, `doc_anchors`, `evidence_grade: EvidenceGrade` 3-variant vs. `Grade` 4-variant, required `rationale`) that the LLM-spine has no business populating. Per memory `feedback_no_deterministic_engine_comparison`, the shim isn't a deterministic-engine parity harness — so PR-3 owns the LLM-spine variants (`ComponentsCanonical`, `SubsystemsCanonical`, `RelatedComponentsCanonical` + their row structs in `projection_to_canonical.rs`). A future refactor may unify when the engine path retires.

2. **Three new fields on output structs to make per-stage evidence scoring work.** The plan's evidence-function pseudo-code references `output.declared_child_component_ids()` / `output.declared_subsystem_ids()` accessors on `AgentOutput`; PR-3 makes those load-bearing fields in the typed structs (`ReduceAgentOutput.declared_child_component_ids: Vec<ComponentIdRef>`, `ProjectAgentOutput.declared_subsystem_ids: Vec<ComponentIdRef>`). The prompt rubrics tell the reducer / project agent to ECHO BACK the input list as the denominator of the coverage ratio. The LLM can game the metric by setting `declared == component_ids`; PR-5 calibration will measure how often this happens, and a future refactor may thread dispatch-side context to the evidence functions for an unforgeable denominator.

3. **`evidence_pointers` ordering convention is load-bearing for classify_evidence.** The classify prompt rubric tells the LLM: "`evidence_pointers` is REQUIRED and ORDERED: index 0 is the primary manifest path you read; index 1 (when present) is the source entry-point you read." `AgentOutput::primary_manifest_path()` reads `evidence_pointers[0].path`; `AgentOutput::declared_entrypoint_path()` reads `[1]`. PR-4's auditor wiring should treat this convention as locked — re-ordering would silently break the 4-rung classify-evidence ladder. The drift-catcher test (`classify_prompt_shape.rs`) pins the embedded YAML example's ordering.

4. **`expected_classify_tool_id` derives from `kind` via a static prefix-match table.** Rather than adding a `classify_tool_id` field to `ClassifyAgentOutput`, the runtime infers the expected tool from the LLM's declared `kind` (e.g. `"rust-library"` → `"parse_cargo_toml"`; `"docker-image"` → `"parse_dockerfile"`). Unknown / missing kind returns the generic `"classify"` sentinel which trivially matches nothing in the transcript — so a missing kind clamps the grade lower via the ladder. PR-4's audit prompt should NOT assume the LLM declared its expected tool; it derives the same mapping from `kind`.

5. **Grade case-insensitive deserializer adapter lives in `outputs.rs`.** `Grade` enum's default serde shape is PascalCase (`"Strong"`); the dispatch prompts and PR-3's new prompts advertise lowercase (`"strong"`). PR-3 introduced a `deserialize_grade_lowercase` helper (used as `#[serde(deserialize_with = ...)]` on every `confidence_grade: Grade` field) that accepts case-insensitive input and rejects unknown variants. PR-4's verdict struct should mirror the pattern if it embeds `Grade` from YAML.

6. **PR-2 forward-pointer addressed: `lane_b_wired_into_call_agent_skips_on_strong_grade` renamed + assertion flipped** to `lane_b_wired_into_call_agent_fires_when_evidence_floor_clamps_below_strong`. PR-2 note item 1 explicitly flagged this would invert under PR-3's real evidence scoring (the synthetic ClassifyBackend emits no evidence_pointers + transcript is empty → classify_evidence = 0.0 → ceiling = Declines → Lane B fires). The wiring is the constant; the polarity reflects the regime. PR-4's verdict-round-trip tests should also use the new "evidence-clamped grade" assumption rather than the pre-PR-3 "hardcoded Strong" assumption.

7. **Test-backend canned-response keys changed.** Pre-PR-3 keys (`"classify component"`, `"reduce subsystem"`, `"project the workspace"`) were legacy placeholder substrings. PR-3's production prompts open with `"You are Atlas's <stage> agent"`, so test backends now key on `"<stage> agent"` (`"classify agent"`, `"reduce agent"`, `"project agent"`). PR-4's audit-prompt tests should similarly key on `"audit"` or whatever phrase the audit prompt opens with.

8. **`run_iteration` rewires the inter-stage data flow.** Classify outputs now produce a per-component `(component_id, kind, language)` rollup that's threaded into `build_reduce_prompt`. Reduce outputs produce a per-subsystem `(subsystem_id, purpose, component_count)` rollup that's threaded into `build_project_prompt`. The reducer / project agent sees its predecessor's outputs in the prompt body. PR-4's auditor will need access to the same rollups when it audits a producer's output against its inputs — the rollups are currently local to `run_iteration`; if PR-4 needs them at the auditor layer, plumb them through `AgentRequest` or a runtime-side per-call context map.

9. **6 new default cap constants** in `runtime/mod.rs`: `DEFAULT_CLASSIFY_SOFT_CAP=6` / `DEFAULT_CLASSIFY_HARD_CAP=12` / `DEFAULT_REDUCE_SOFT_CAP=4` / `DEFAULT_REDUCE_HARD_CAP=8` / `DEFAULT_PROJECT_SOFT_CAP=4` / `DEFAULT_PROJECT_HARD_CAP=8`. Exported `pub const` so downstream callers can reference them without re-hardcoding. PR-2 set the same pattern with `DEFAULT_DISPATCH_SOFT_CAP=15` / `DEFAULT_DISPATCH_HARD_CAP=30`.

10. **Canonical shim wire-in lives at the CLI layer, not in `run_workspace`.** The plan-text suggested "wire the shim into `run_workspace` so it returns the canonical artifact set alongside `L9Projection`". PR-3 kept `run_workspace`'s signature stable (still returns `Result<L9Projection, AgentError>`) and wired `project_l9_to_canonical(&projection, &config.output_dir)` into `pipeline.rs` alongside the existing `agent-runtime-projection.yaml` emission. Rationale: the CLI owns the `output_dir` and the projection-emission filesystem layout; threading `output_dir` through the runtime layer would couple the runtime to the CLI's filesystem-layout concerns. The plan's intent ("canonical artifacts get written when the runtime finishes a workspace") is achieved via CLI-side composition. PR-4's on-disk verdict path can follow the same pattern.

11. **Three-commit decomposition** instead of the plan's recommended six. Rationale matches PR-A's precedent (note item 11): `mod.rs` is shared across PR-3's logical commit boundaries (mod-decls + caps + 3 prompt functions + call-site rewiring all in one file), so a six-commit split would either require non-compiling intermediate states or hairy `git add -p` partial staging without reviewer value. Each PR-3 commit is independently buildable + testable. Commit 1 carries all source + test-fixture updates; commit 2 carries the 9 new test files; commit 3 is this status flip.

PR-3 commit SHAs: implementation `4776013`; integration tests `b31069c`; status flip in a separate commit per the two-commit-pattern adaptation.

### PR-4

*(Empty — to be filled by PR-4's session.)*

### PR-5

*(Empty — to be filled by PR-5's session. This is the sprint-closeout PR; its note carries the recorded Atlas-on-Atlas baseline numbers + cross-provider parity outcome + sprint SHIPPED summary.)*

### PR-A

2026-05-13 — Landed. Verification-note commit: `d1df478` (`sprint: PR-A rmcp maturity verification`). Code-commit: `c07c5d5` (`sprint: PR-A rmcp migration + subprocess serve_client driver`). Status-flip commit follows in the same session.

PR-A migrated PR-1's hand-rolled MCP JSON-RPC framing (~250 LOC of envelope types + dispatch loop in `mcp/{mod.rs, server.rs, descriptors.rs}`) onto the `rmcp` crate (Rust MCP SDK), and shipped the structural subprocess `serve_client` driver so `TransportFlavour::ClaudeCode | Codex` no longer hard-errors at first `call_agent`. All six cumulative regression gates clean; `mcp_multiplex.rs` regression-green with test-logic preserved (assertions adapted to standard MCP wire shape).

Key PR-A decisions + follow-ups PR-B + future PRs should know:

1. **rmcp 1.6 PASSED all four maturity criteria** (verification note at `crates/atlas-agents/src/mcp/rmcp_verification.md`): last publish 2026-05-01 (12 days; monthly cadence); repo activity 2026-05-12 (1 day; stdio-parse-error robustness fixes in the last week); multi-client server abstraction documented (`serve_server` is per-transport; multi-client = spawn one per duplex transport with handlers sharing `Arc<Inner>` state); 14 direct deps at default features `[base64, macros, server]` (zero WS/TLS/HTTP-server crates pulled). PR-A proceeds with `rmcp`; the `jsonrpsee` fallback path documented in the plan was not exercised.

2. **MCP wire shape standardised.** Atlas's hand-rolled framing emitted a non-standard `{"type":"json","json":<result>}` content variant for tool-call outputs; rmcp emits standard MCP via `CallToolResult::structured(value)` which produces `{"content":[{"type":"text","text":"<json-string>"}], "structuredContent":<value>, "isError":false}`. `mcp_multiplex.rs` assertions adapted to use `structuredContent`; the multi-client isolation contract (per-pipe id round-trip + payload-per-client) is preserved verbatim.

3. **MCP lifecycle enforcement.** rmcp's `serve_server` requires the initialize-first handshake before dispatching `tools/call` or `tools/list`. Atlas's hand-rolled server was lenient about lifecycle. The `mcp_multiplex.rs` test now performs a proper initialize handshake before each test's first request; this is correct MCP, not a test-logic change. PR-B's live-subprocess probe inherits the same lifecycle requirement.

4. **The `unknown-method` test now sends `atlas/this_method_does_not_exist`** instead of the prior `resources/list`. Under rmcp, `resources/list` is a standard MCP method that defaults to returning empty `Ok(ListResourcesResult::default())`; the test would no longer surface `METHOD_NOT_FOUND`. Custom methods route to `ServerHandler::on_custom_request`, which defaults to `METHOD_NOT_FOUND`. The semantic intent of the test (assert -32601 for unknown methods) is preserved.

5. **`AgentRuntime` grew `mcp_server: Option<Arc<McpServer>>` field.** All seven construction sites (CLI pipeline + dispatch.rs unit-test helper + 5 integration-test files) now pass `mcp_server: None`. The subprocess branch returns a clear error when None; PR-7 will populate when subprocess transports become the live exercise path. PR-2/PR-3/PR-4 do NOT need to touch this field — their `AgentRuntime` construction is via existing call sites that already carry `None`.

6. **`AgentError` grew four granular subprocess variants:** `SubprocessSpawn(io::Error)`, `SubprocessWait(io::Error)`, `SubprocessFailed { exit_status }`, `NoFinalOutput`. PR-B's probe pattern-matches on these. PR-2/PR-3/PR-4 don't interact with these variants; they're scoped to the subprocess path.

7. **`serve_client.rs` is a structural skeleton, not a complete subprocess driver.** PR-A ships spawn/wait/drain plumbing exercised against POSIX `cat` / `false` stubs. The dual-channel question (whether MCP wire and LLM prompt share `child.stdin`/`child.stdout` or sit on disjoint channels — CLI arg vs stdio) is genuinely upstream-version-dependent for both claude-code and codex. The `claude_code_config` preset uses `--print <prompt>` (current claude-code CLI shape); the `codex_config` preset has a TODO marker for the verified flag set. **PR-B is the empirical validation** that pins down the exact wire shape per upstream — pin the targeted upstream version in `restrictions.md` when PR-B runs against the live binary.

8. **`restrictions.md` updated.** `claude-code` section unchanged from PR-1 (`--disallowedTools=Read,Grep,Glob,Bash,Write,Edit`). `codex` section now records the current state: as of 2026-05-13 codex has no dedicated `--disallowedTools`-equivalent flag; tool-availability is controlled implicitly by what MCP servers in the config advertise (Atlas's MCP server exposes only the Atlas tool catalog, so the restriction is enforced by omission). PR-B's probe will validate this empirically + update if upstream adds an explicit flag.

9. **Test-content shape changes are confined to `mcp_multiplex.rs`.** The test's structural assertions (concurrent multi-client isolation, id round-trip, payload-per-client) are the load-bearing contract and are preserved. The wire-shape changes (initialize handshake; `structuredContent` assertion field; custom-method-for-unknown-method test) reflect standard MCP and are necessary downstream of the framing migration.

10. **Cargo.lock growth: ~10 new transitive crates** introduced by rmcp default features (rmcp-macros, schemars, pastey, tokio-util, plus a small handful of helper crates). Many of rmcp's 14 direct deps overlap Atlas's existing workspace deps (tokio, serde, serde_json, async-trait, thiserror, tracing, chrono, base64). No WebSocket/TLS/HTTP-server crates pulled.

11. **Pragmatic commit decomposition.** The plan's recommended 4-commit decomposition (verification, migration, serve_client + wiring, status flip) was simplified to 3 commits: verification note (`d1df478`) + combined code commit (`c07c5d5`) + status flip. The migration and the subprocess wiring are tightly coupled — splitting them creates an intermediate compile-clean state without reviewer value. This matches PR-1's actual 2-commit pattern (code + status-flip).

PR-A commit SHAs: verification `d1df478`; code `c07c5d5` (single code-commit; status flip follows per the two-commit pattern).

### PR-B

2026-05-13 — Landed. Code-commit: `b8af469` (`sprint: PR-B subprocess --disallowedTools probe`). Status-flip commit follows in the same session.

PR-B shipped `crates/atlas-agents/tests/mcp_disallowed_tools.rs` — an `#[ignore]`-gated live-subprocess probe that asserts the **Atlas server-side MCP transcript records zero `Read` tool calls** when `claude --disallowedTools=Read,Grep,Glob,Bash,Write,Edit` is spawned with a Read-provoking prompt. Two `#[tokio::test]`s ship in the file: the claude probe + a codex stub. Both run with `cargo test -p atlas-agents --test mcp_disallowed_tools --release -- --ignored`. The forensic `eprintln!` captures upstream version + observed response shape for traceability.

All six cumulative regression gates clean. Polyglot fixture: 2 tests passed, ~110s wall-time, cold count within loose bound. The `--ignored` claude probe ran live twice against the operator's machine: both passes, response shape stable across runs.

Key PR-B decisions + follow-ups future PRs (PR-7, PR-3, PR-4) should know:

1. **Empirical pin against `claude` 2.1.140 (Claude Code), verified 2026-05-13.** `claude --version` output is the source of truth. The brief and earlier sprint docs assumed the binary was named `claude-code`; the actual upstream binary is `claude`. `claude_code_config` in `crates/atlas-agents/src/mcp/serve_client.rs:69` now spawns `claude` (not `claude-code`); `restrictions.md` documents the verified binary name + flag set.

2. **`--disallowedTools` companion-flag inventory verified.** `--disallowedTools` and `--disallowed-tools` both accepted (CamelCase + kebab-case forms work). Value accepts comma- OR space-separated tool-name lists. `--mcp-config <configs...>` accepts JSON files or strings (space-separated). `-p`/`--print` is the non-interactive entry point. `--strict-mcp-config` forces MCP sourcing exclusively from `--mcp-config` (no per-user MCP config inheritance). Recorded in `restrictions.md`.

3. **Observed response shape (Ok arm) under 2.1.140:** subprocess succeeded; the LLM emitted a refusal text along the lines of *"I can't fulfill this request as stated. The `Read` tool is not currently available in my toolset..."* + lists the tools it *does* see (which are claude's own internal CLI tools — `Agent`, `AskUserQuestion`, `ScheduleWakeup`, `Skill`, `ToolSearch` — NOT Atlas's MCP catalog). Server-side transcript: 0 entries. Wall-time: ~21s including LLM inference. PR-B's assertion (zero Read calls) held cleanly.

4. **Atlas's in-process `McpServer` cannot today be reached by a `claude --mcp-config` subprocess.** The `mcp_config.json` PR-B writes points at a `/bin/echo` placeholder; claude's MCP client fails to handshake with it, so no MCP traffic crosses to Atlas's server. The transcript stays empty regardless of whether `--disallowedTools` works. **The probe's assertion is therefore forward-looking** — it catches regressions where future claude-code routes built-in `Read` requests through MCP servers OR where Atlas registers an MCP-exposed tool named `Read`, but it does NOT catch regressions in claude's built-in tool-availability gating (which would surface in the forensic `eprintln!` response text instead). PR-7 is expected to materialise a standalone `atlas-mcp-server` binary that `claude --mcp-config` can spawn; once shipped, this test gains operational teeth. The test's module docstring explicitly calls this out (recast §5.4 invariant; "what the assertion catches today vs after PR-7" section).

5. **`serve_client` return type extended** (`crates/atlas-agents/src/mcp/serve_client.rs:121`). PR-A's `serve_client` auto-drained the per-client MCP transcript internally and never exposed it, and the `ClientId` was generated by a private static counter — leaving PR-B's test with no way to observe the server-side transcript. With explicit user approval (over alternatives: McpServer cache, defer PR-B, drop the transcript assertion), the return type changed from `Result<AgentOutput, AgentError>` to `Result<(AgentOutput, Vec<serde_json::Value>), AgentError>`. Sole production caller at `crates/atlas-agents/src/runtime/mod.rs:833` updated with `.map(|(output, _subprocess_mcp_transcript)| output)` to preserve existing semantics (the subprocess MCP transcript is discarded at the production call site for now; PR-7 may merge it with the HTTP-side `Transcript` if needed).

6. **`serve_client` drain-order bug fixed.** Pre-PR-B, the per-client transcript was drained *after* the `if !exit_status.success()` early-return check at `serve_client.rs:181-192`. Subprocesses that exited non-zero after making tool calls would leak transcript entries in `McpServer.transcript: Mutex<HashMap<ClientId, Vec<Value>>>` (the `HashMap` entry was never removed). The drain is now unconditional and precedes the exit-status check; the warn-log on non-zero exit includes the transcript-entry count for forensic visibility. Caught by the PR-B quality reviewer; fix landed in the same code-commit.

7. **`AgentRuntime.mcp_server: Option<Arc<McpServer>>` semantics preserved.** PR-B does NOT touch `AgentRuntime` or its construction sites. The seven existing call sites still pass `mcp_server: None`; the subprocess-transport branch in `runtime/mod.rs::run_tool_loop_with_lane_a` still hard-errors with the same diagnostic when `None`. PR-7 owns the population.

8. **Codex sibling stub** (`codex_subprocess_cannot_invoke_disallowed_read_equivalent`) ships as a `#[ignore]`-gated `#[tokio::test]` with `eprintln!` + `return` body documenting the unblocking conditions (codex upstream version + introduced disallow flag + the three update sites). Per `restrictions.md` (2026-05-13 against codex 0.x), codex has no `--disallowedTools`-equivalent flag — restriction is enforced by *omission* (Atlas's MCP catalog has no Read-equivalent). When a future codex upstream introduces an explicit disallow flag, fill in the stub mirroring the claude probe + update `codex_config` in `serve_client.rs` + extend `restrictions.md` § codex.

9. **LOC budget overage acknowledged.** Plan §4 Task 7 budget: 100–200 LOC for the test file. Shipped: 258 LOC (claude probe + codex stub + 4 helpers + module docstring). The 58-LOC overage is entirely doc-comments — the "what the assertion catches today vs after PR-7" framing, the why-`#[ignore]`-gated explanation, and the forensic-output documentation. The 2× stop-and-surface threshold (400 LOC) was not approached. Spec reviewer flagged the overage and judged the overage was "entirely doc-comments" — kept on that basis.

10. **MEDIUM issues recorded for later sweeps** (none HIGH unresolved): the spec reviewer's Issue 1 (the assertion is structurally vacuous today because no MCP wire flows between claude subprocess and Atlas's in-process server) is acknowledged in the test's module docstring as a forward-looking-only assertion until PR-7 materialises a standalone `atlas-mcp-server` binary. The brief explicitly accepted this framing (Step B.1 inline scaffold lines 3098-3112: "The load-bearing assertion is the server-side transcript"; PR-A note item 7: "PR-B is the empirical validation that pins down the exact wire shape per upstream").

11. **Two-commit pattern verified.** Commit 1: code + test + restrictions.md + serve_client return-type extension + runtime/mod.rs adapter (`sprint: PR-B subprocess --disallowedTools probe`). Commit 2: status flip (this file's PR-B row from `[ ]` to `[x]` + "Last updated" header refresh + this per-PR note).

PR-B commit SHAs: code `b8af469` (single code-commit); status flip in a separate commit per the two-commit pattern.
