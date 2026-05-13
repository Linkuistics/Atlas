# PR-2 continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-2** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. This prompt is self-contained — read this, then read the canon in the order below, then drive the implementation via `superpowers:executing-plans`.

## What PR-2 ships

PR-2 is **structural, medium-sized**. It replaces the two `PR-7-WIRES-REAL-PROMPT` stubs at `crates/atlas-agents/src/runtime/dispatch.rs:203, :254` with production prompts, migrates Lane A's deserializer from JSON to YAML, and introduces the **dispatch-stage half** of Lane A evidence scoring (classify/reduce/project scoring lands in PR-3).

Deliverables:

1. **`crates/atlas-agents/src/runtime/prompt_examples.rs`** — `extract_yaml_fence(text: &str) -> Result<&str, FenceExtractError>` helper. Shared across every PR-2+ prompt-shape test.
2. **`crates/atlas-agents/src/runtime/yaml_strict.rs`** — `deserialize_string_strict` adapter for `#[serde(deserialize_with = ...)]`. Protects string-typed fields from YAML's Norway problem + implicit-typing failure modes.
3. **`crates/atlas-agents/src/runtime/audit/evidence.rs`** — `compute_evidence_score(stage, transcript, output) -> f32` dispatcher + `grade_ceiling(score) -> Grade` + the two **dispatch-stage** per-stage scoring functions (`dispatch_subsystems_evidence`, `dispatch_components_evidence`). The four remaining per-stage functions (classify/surface/reduce/project) land in PR-3 as additive extensions.
4. **Production dispatch prompts** in `crates/atlas-agents/src/runtime/dispatch.rs` — `build_dispatch_subsystems_prompt` + `build_dispatch_components_prompt` go from `PR-7-WIRES-REAL-PROMPT` stubs to real prompt construction.
5. **Lane A deserializer migration** in `dispatch.rs:306, :327` — `serde_json::from_value` → `serde_yaml::from_str`. `parse_subsystems_from_output_value` and `parse_components_from_output_value` change signatures from `Value -> Result` to `&str -> Result`. Call sites that previously fed `Value` extract the fenced-yaml body and pass `&str`.
6. **`#[serde(deserialize_with = "deserialize_string_strict")]`** applied to `SubsystemsOverrideFile`'s and `ComponentsOverrideFile`'s string-shaped fields at `dispatch.rs:103, :131`.
7. **`lane_a_validate`** extended at `crates/atlas-agents/src/runtime/audit/lane_a.rs:123` from schema-only validation to two-layer (schema + evidence floor). Calls `evidence::compute_evidence_score` + `evidence::grade_ceiling` to clamp the LLM's claimed grade.
8. **`Transcript` accessor additions** (verify which exist at plan-time): `read_file_paths() -> HashSet<PathBuf>`, `tool_called(tool_id: &str) -> bool`, `tool_calls_for(tool_id: &str) -> impl Iterator<Item=&ToolCall>` — evidence scoring needs them.
9. **Test fixture migrations** in `crates/atlas-agents/tests/dispatch_shortcircuit.rs` + `crates/atlas-agents/tests/audit_lane_b.rs` — canned `Value` outputs → canned YAML strings.
10. **New tests:**
    - `crates/atlas-agents/tests/dispatch_prompt_shape.rs` — drift catcher (brainstorm decision row 2): each `build_dispatch_*_prompt` emits a fenced ```yaml block that deserializes into the target struct.
    - `crates/atlas-agents/tests/lane_a_dispatch_evidence_floor.rs` — evidence-floor clamping for `DispatchSubsystems` and `DispatchComponents`.
    - `crates/atlas-agents/tests/yaml_envelope_norway_problem.rs` — `component_id: NO` round-trips as `"NO"`; sibling assertions for `yes` / `on` / version-shaped strings.

LOC budget: see plan §4 Task 2 closing. If approaching 2× budget, **stop and surface** — brainstorm §12 risk #1.

## Scope exclusions — PR-2 does NOT do these

- **PR-2 does NOT touch classify/reduce/project prompts** at `mod.rs:919, :928` or `build_project_prompt` (which doesn't exist yet). **That's PR-3.**
- **PR-2 does NOT add the four remaining per-stage evidence-score functions** (`classify_evidence`, `surface_evidence`, `reduce_evidence`, `project_evidence`). Only dispatch-stage in PR-2; rest in PR-3.
- **PR-2 does NOT touch the four typed-output structs** in the new `runtime/outputs.rs`. **That's PR-3.**
- **PR-2 does NOT migrate `agent-runtime-projection.json` → `.yaml`** at `pipeline.rs:1177`. **That's PR-3.**
- **PR-2 does NOT touch the `PR-7-WIRES-REAL-AUDITOR` stub** at `mod.rs:665`. **That's PR-4.**
- **PR-2 does NOT touch the MCP framing.** **That's PR-A.**
- **PR-2 does NOT run Atlas-on-Atlas.** Calibration is PR-5.

If asked to do anything from this list, **stop and surface** — that's not PR-2 scope.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 2 (lines 568–1294) — your scope. Also skim §0–§3 (reading order, deliverable restated, non-negotiables, dependency graph) and §2.1's 15 decision rows. PR-2 implements rows 1 (per-PR ownership), 2 (drift catchers via shape tests), 13 (YAML strict deserialization), and the dispatch-stage half of row 3 (per-stage evidence scoring).

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-1's per-PR note has 10 load-bearing call-outs forwarded to PR-2. In particular, the two-router-constructor split (item 1) and `default_transport_from_config` (item 3) shape PR-2's call sites: PR-2's modifications run downstream of `BackendRouter::new_for_agent_runtime`, NOT the deterministic `new_from_config`. After your work lands, append PR-2's per-PR note with commit SHA + deviations + forward-pointers to PR-3.

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** — read §5.5 (Lane A YAML migration), §6.1 (typed outputs framing — for context on what PR-3 builds on), §6.2 (per-stage evidence), §6.4 (evidence-score functions), §12.8 (Norway-problem risk). If plan and brainstorm disagree, **brainstorm wins**.

4. **Sprint framing memories** (durable; condition every decision):
   - `.claude/memory/feedback_yaml_canonical_interchange.md` — PR-2 is the first PR that produces real YAML-shape regressions; the canonical-interchange discipline is load-bearing here.
   - `.claude/memory/project_atlas_purpose_llm_consumers.md` — output quality bar is "useful as LLM context"; prompt construction should aim there.
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` — calibration anchors on intrinsic properties only.
   - `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent.
   - `.claude/memory/feedback_prefer_existing_crates.md` — for any auxiliary helper, prefer maintained crates.

5. **Operational memories:**
   - `.claude/memory/project_phase7_agent_runtime_default_ratified.md` — `--agent-runtime` opt-in default; HTTP backends are the sprint's live path.
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 2** and follow Steps 2.1 → 2.14 in order. Mark `[x]` as you complete each.

3. **Verify after each non-trivial step** (per-step `cargo build -p atlas-agents` + targeted `cargo test`). Specifically:
   - After Step 2.4 (evidence.rs): `cargo test -p atlas-agents --lib evidence` clean.
   - After Step 2.6 (dispatch prompts): `cargo build -p atlas-agents` clean.
   - After Step 2.7 (deserializer migration): `cargo test -p atlas-agents --lib dispatch` clean.
   - After Step 2.9 (drift catcher): `cargo test -p atlas-agents --test dispatch_prompt_shape` clean.
   - After Step 2.10 (evidence floor): `cargo test -p atlas-agents --test lane_a_dispatch_evidence_floor` clean.
   - After Step 2.11 (Norway-problem): `cargo test -p atlas-agents --test yaml_envelope_norway_problem` clean.

4. **Step 2.13 is the cumulative-regression gate** — run all six:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count in `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`** (memory `feedback_no_tail_pipe_for_long_tests`). **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run** (memory `feedback_atlas_test_subprocess_concurrency`). Use `--skip polyglot_phase3` substring (memory `feedback_cargo_skip_polyglot_pattern`).

5. **Step 2.14 is the two-commit pattern.** First commit: code changes (commit message per plan §4 Task 2 Step 2.14 HEREDOC). Second commit: status file — flip PR-2 row from `[ ]` to `[x]`, update "Last updated" header, fill in PR-2's per-PR note section with your commit SHA + deviations + forward-pointers to PR-3 (especially anything that conditions PR-3's evidence-function extensions or its typed-output struct module placement).

6. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 2 acceptance gate. Particularly: Norway-problem regression coverage; drift-catcher coverage of both dispatch prompts; evidence-floor clamping behaviour matches plan §4 Task 2 Step 2.10 spec.

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues (correctness, security, broken invariants — e.g., did the strict-string adapter inadvertently break a legitimate non-string field?). HIGHs fixed before status flip; MEDIUMs recorded in PR-2 per-PR note for later sweeps.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**. Don't ship broken code to flip the checkbox.

## Coordination with PR-A

PR-A (`rmcp` migration + subprocess MCP `serve_client` driver) is **parallel-safe** with PR-2 and may be running in another session. File sets are disjoint (PR-A lives under `crates/atlas-agents/src/mcp/` + `tool_loop_http.rs`; PR-2 lives under `crates/atlas-agents/src/runtime/{dispatch,audit}` + new tests). The one shared edit surface is `crates/atlas-agents/src/runtime/mod.rs` (PR-2 adds `pub mod prompt_examples; pub mod yaml_strict;` declarations; PR-A doesn't touch mod-level declarations). If a rebase conflict surfaces beyond `mod.rs`, **stop and surface** — the disjoint-files claim was load-bearing, and an unexpected conflict is a signal worth raising.

## Begin at Step 1

Begin at **Step 2.1: Author the fence-extraction helper at `crates/atlas-agents/src/runtime/prompt_examples.rs`** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 2.

Open the plan, locate the step, and proceed.
