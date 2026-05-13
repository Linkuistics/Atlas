# Atlas vNext — Production-prompt sprint — Status

Companion to `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`. This file tracks per-PR completion state across sessions. The PR-1 continuation prompt at `docs/superpowers/prompts/2026-05-13-pr1-continue.md` reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-13 (PR-2 landed — production dispatch prompts + Lane A YAML migration + dispatch-stage evidence-floor scoring; status-flip commit follows the PR-2 code-commit `876ea24`).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know) in the per-PR notes block below.

- [x] PR-0 — Plan + status + PR-1 continuation prompt (docs only)
- [x] PR-1 — `BackendRouter::backend_for_provider` + `Arc<ForProviderFn>` closure + `--config <PATH>` flag + `.atlas/config.sprint.example.yaml` + HTTP-backend smoke test (small / structural)
- [x] PR-2 — Production dispatch prompts (replaces `PR-7-WIRES-REAL-PROMPT` stubs at `dispatch.rs:203, :254`) + Lane A YAML migration (`serde_json::from_value` → `serde_yaml::from_str` at `dispatch.rs:306, :327`) + dispatch-stage Lane A evidence scoring + `runtime/yaml_strict.rs` + `runtime/prompt_examples.rs` + `runtime/audit/evidence.rs` (medium)
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

*(Empty — to be filled by PR-3's session.)*

### PR-4

*(Empty — to be filled by PR-4's session.)*

### PR-5

*(Empty — to be filled by PR-5's session. This is the sprint-closeout PR; its note carries the recorded Atlas-on-Atlas baseline numbers + cross-provider parity outcome + sprint SHIPPED summary.)*

### PR-A

*(Empty — to be filled by PR-A's session. PR-A's first commit is the `rmcp` maturity-verification note at `crates/atlas-agents/src/mcp/rmcp_verification.md` documenting the 4-criterion decision.)*

### PR-B

*(Empty — to be filled by PR-B's session.)*
