# PR-3 continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-3** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. This prompt is self-contained — read this, then read the canon in the order below, then drive the implementation via `superpowers:executing-plans`.

## What PR-3 ships

PR-3 is **structural, LARGE** — the **largest single PR in the sprint** (LOC budget 1500–2200; stop-and-surface at 4400). It produces real outputs for the four non-dispatch stages (classify / surface / reduce / project), ships the canonical-schema shim, and migrates the runtime's intermediate projection serialization from JSON to YAML. Wave 3 of the sprint dependency graph; PR-4 + PR-5 + Phase 8 brainstorming are downstream of PR-3 shipping.

Deliverables:

1. **`crates/atlas-agents/src/runtime/outputs.rs`** — new sibling module to `dispatch.rs` holding the four typed LLM-agent output structs (`ClassifyAgentOutput`, `ReduceAgentOutput`, `ProjectAgentOutput`) + helper types (`EvidencePointer`, `ContractRef`, `EdgeRef`, `RefactoringCue`, `RefactoringCueKind`, `DocScaffoldOutline`, `DocSection`, `SubsystemSummary`, plus a `ComponentIdRef` strict-string newtype). `ComponentKind` / `Language` / `Lifecycle` are re-used from `component-ontology` if they exist; otherwise defined locally — see plan §4 Task 3 Step 3.1 for the grep-first protocol.
2. **`crates/atlas-agents/src/runtime/audit/evidence.rs`** extensions — replace PR-2's four fall-through-to-`1.0` placeholders for classify / surface / reduce / project with real per-stage scoring functions per plan Step 3.2. The dispatch-stage functions stay as-is from PR-2.
3. **`AgentOutput` accessor additions** — `primary_manifest_path()`, `declared_entrypoint_path()`, `expected_classify_tool_id()`, `declared_public_items_count()`, `declared_public_item_paths()`, `declared_child_component_ids()`, `subsystem_catalog()`, `declared_subsystem_ids()`. Evidence functions need them. Live in `crates/atlas-agents/src/runtime/audit/lane_a.rs` next to the existing accessors.
4. **Production prompts** in `crates/atlas-agents/src/runtime/mod.rs` — `build_classify_prompt` at line 919 + `build_reduce_prompt` at line 928 go from `PR-7-WIRES-REAL-PROMPT` stubs to real prompt construction; add new `build_project_prompt` (no PR-7 stub — fresh function). Each prompt embeds a fenced ```yaml worked example that deserializes via its target struct.
5. **Call-site wiring** in `mod.rs` at the classify (line 461) and reduce (line 477) invocation sites; plus a new project-stage invocation site that consumes per-subsystem reducer outputs and produces the workspace-level `ProjectAgentOutput`. Wire the canonical-schema shim into `run_workspace` so it returns the canonical artifact set alongside `L9Projection`.
6. **`crates/atlas-agents/src/runtime/projection_to_canonical.rs`** — the canonical-schema shim mapping `L9Projection` → `components.yaml` + `subsystems.yaml` + `related-components.yaml`. Hard-fails (`ShimError::MissingProjectionField`) on missing input fields; no partial-write residue on disk. Re-uses Phase 7 PR-2's `atomic_write_pair` + `atomic_write`. `ComponentsYaml` / `SubsystemsYaml` / `RelatedComponentsYaml` shapes mirror the existing canonical schema (grep-first protocol in plan Step 3.7).
7. **`crates/atlas-cli/src/pipeline.rs`** line 1177 — `agent-runtime-projection.json` → `agent-runtime-projection.yaml`. `serde_json::to_string_pretty` → `serde_yaml::to_string`. Stale `.json` files are NOT auto-deleted (forensic; greenfield-upgrade discipline).
8. **New tests:**
    - `crates/atlas-agents/tests/classify_prompt_shape.rs` — drift catcher for `build_classify_prompt`.
    - `crates/atlas-agents/tests/reduce_prompt_shape.rs` — drift catcher for `build_reduce_prompt`.
    - `crates/atlas-agents/tests/project_prompt_shape.rs` — drift catcher for `build_project_prompt`.
    - `crates/atlas-agents/tests/lane_a_classify_evidence_floor.rs` — evidence-floor clamping for `Classify`.
    - `crates/atlas-agents/tests/lane_a_surface_evidence_floor.rs` — for `Surface`.
    - `crates/atlas-agents/tests/lane_a_reduce_evidence_floor.rs` — for `Reduce`.
    - `crates/atlas-agents/tests/lane_a_project_evidence_floor.rs` — for `Project`.
    - `crates/atlas-agents/tests/projection_to_canonical_shim.rs` — synthetic `L9Projection` → canonical YAMLs round-trip.
    - `crates/atlas-agents/tests/projection_to_canonical_shim_missing_field.rs` — synthetic L9 missing required field → `ShimError`; no partial-write residue.

LOC budget: **1500–2200** (plan §4 Task 3 closing). If approaching 4400 (2× budget), **stop and surface** — brainstorm §12 risk #1.

## Scope exclusions — PR-3 does NOT do these

- **PR-3 does NOT touch dispatch prompts** at `dispatch.rs:203, :254`. PR-2 owns dispatch; PR-3 owns the four non-dispatch stages.
- **PR-3 does NOT replace PR-2's `1.0` fallback for dispatch stages** in `evidence.rs`. PR-2's dispatch-stage scoring stays. PR-3 only replaces classify / surface / reduce / project.
- **PR-3 does NOT author a `build_surface_prompt`.** Surface stage is Tool-driven (deterministic surface agent from Phase 7 PR-3); only its `surface_evidence` scoring lands in PR-3 for completeness. See plan Step 3.5.
- **PR-3 does NOT touch the auditor stub** at `mod.rs:665`. **That's PR-4.**
- **PR-3 does NOT touch the MCP framing.** **That's PR-A.** (Already landed.)
- **PR-3 does NOT add the `--disallowedTools` probe.** **That's PR-B.** (Parallel-track.)
- **PR-3 does NOT run Atlas-on-Atlas.** Calibration is PR-5.
- **PR-3 does NOT add new workspace dependencies** beyond what Phase 7 + PR-1/PR-2 already pulled in (plan §4 Task 3 pre-flight constraint).

If asked to do anything from this list, **stop and surface** — that's not PR-3 scope.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 3 (lines 1302–1975) — your scope. Steps 3.1 → 3.13 are the implementation steps; the recommended six-commit decomposition lives in the §4 Task 3 preamble. Also skim §0–§3 (reading order, deliverable restated, non-negotiables, dependency graph) and §2.1's 15 decision rows. PR-3 implements the non-dispatch half of rows 1 (per-PR ownership), 2 (drift catchers via shape tests), 3 (per-stage evidence scoring — classify / surface / reduce / project), 13 (YAML strict deserialization extended to outputs.rs structs), and **row 14** (canonical-schema shim) in its entirety.

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-1's and PR-2's per-PR notes carry forward-pointers PR-3 must honour. The two **load-bearing surface changes** PR-2 hands forward are:
   - **(a) Evidence-floor fallback policy** (PR-2 note item 1): PR-2 returns `1.0` for `Stage::Classify | Surface | Reduce | Project` as a temporary placeholder. **PR-3 MUST replace this fallback with real per-stage evidence functions at the same time as adding the typed-output structs in `outputs.rs`.** Until then, non-dispatch stages preserve hardcoded-Strong behaviour. Step 3.2 is where the replacement lands.
   - **(b) `AgentOutput::text` field** (PR-2 note item 4): PR-2 added `text: String` populated by `parse_final_output` when the LLM response carries `content[].text` blocks; dispatch parsers fence-extract from this field. **PR-3's classify / reduce / project prompts should emit fenced YAML the same way, and the parsers should read from `output.text` symmetrically.** `parse_final_output` already tries YAML fence-extract first (PR-2 note item 9); fenced YAML from PR-3 prompts will catch automatically.

   Also re-read PR-1 note items 1–4 (two-router-constructor split, `default_transport_from_config`, `provider_from_config_key`) — PR-3 runs inside the agent-runtime tool loop downstream of `BackendRouter::new_for_agent_runtime`, NOT the deterministic `new_from_config`. After your work lands, append PR-3's per-PR note with commit SHAs + deviations + forward-pointers to PR-4 (especially anything PR-4's auditor wiring needs to know about the typed output structs and the evidence functions).

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** — read §6.1 (typed outputs framing — load-bearing for PR-3's `outputs.rs`), §6.2 (per-stage evidence — load-bearing for PR-3's evidence.rs extensions), §6.3 (project stage shape), §6.4 (evidence-score functions, the per-stage scoring rubrics), §12.1 (PR-3 size risk + the recommended commit decomposition), §12.5 (`L9Projection` missing-field risk + framing #2 "shim hard-fails are prompt-correctness signals"). If plan and brainstorm disagree, **brainstorm wins**.

4. **Sprint framing memories** (durable; condition every decision):
   - `.claude/memory/feedback_yaml_canonical_interchange.md` — PR-3 is the PR that produces the bulk of the sprint's new YAML surface area; the canonical-interchange discipline is load-bearing here.
   - `.claude/memory/project_atlas_purpose_llm_consumers.md` — output quality bar is "useful as LLM context"; prompt construction for classify / reduce / project should aim there. PR-3's project-stage `DocScaffoldOutline` exists because of (c) from the three Atlas use-cases (documentation generation).
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` — the canonical-schema shim is NOT a deterministic-engine comparison harness. The shim's hard-fails are prompt-correctness signals (brainstorm framing #2), not a regression-detector against det-classifier output.
   - `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent.
   - `.claude/memory/feedback_prefer_existing_crates.md` — for any auxiliary helper, prefer maintained crates. PR-3 reuses Phase 7 PR-2's `atomic_write_pair` + `atomic_write` rather than rolling new atomic-write logic.

5. **Operational memories:**
   - `.claude/memory/project_phase7_agent_runtime_default_ratified.md` — `--agent-runtime` opt-in default; HTTP backends are the sprint's live path.
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 3** and follow Steps 3.1 → 3.13 in order. Mark `[x]` as you complete each.

3. **Pre-step grep protocol** before Step 3.1 (outputs.rs):
   ```bash
   grep -nE "pub enum (ComponentKind|Language|Lifecycle)" \
       /Users/antony/Development/Atlas/crates/component-ontology/src/*.rs 2>/dev/null
   ```
   If the enums exist in `component-ontology`, **re-use** via `use component_ontology::{ComponentKind, Language, Lifecycle};`. If not, define locally in `outputs.rs` and revisit refactor in a later phase.

4. **Pre-step grep protocol** before Step 3.7 (projection_to_canonical.rs):
   ```bash
   grep -rnE "pub struct (ComponentsYaml|SubsystemsYaml|RelatedComponentsYaml)" \
       /Users/antony/Development/Atlas/crates/ 2>/dev/null
   ```
   If the canonical-artifact structs exist (likely `crates/component-ontology/` or `crates/atlas-engine/src/canonical_artifacts.rs`), **re-use them** (`pub use component_ontology::ComponentsYaml;`). If not, define them in `projection_to_canonical.rs` and this PR becomes their canonical owner.

5. **Verify after each non-trivial step:**
   - After Step 3.1 (outputs.rs): `cargo build -p atlas-agents` clean; `cargo test -p atlas-agents --lib outputs` clean.
   - After Step 3.2 (evidence extensions): `cargo test -p atlas-agents --lib evidence` clean.
   - After Steps 3.3 / 3.4 / 3.5 (per-stage prompts): `cargo build -p atlas-agents` clean.
   - After Step 3.6 (call-site wiring): `cargo build -p atlas-agents` + `cargo test -p atlas-agents` clean.
   - After Step 3.7 (shim): `cargo build -p atlas-agents` clean.
   - After Step 3.8 (pipeline migration): `cargo build -p atlas-cli` clean.
   - After Step 3.9 (shim round-trip test): `cargo test -p atlas-agents --test projection_to_canonical_shim` clean.
   - After Step 3.10 (shim missing-field test): `cargo test -p atlas-agents --test projection_to_canonical_shim_missing_field` clean.
   - After Step 3.11 (evidence-floor tests): `cargo test -p atlas-agents --test lane_a_classify_evidence_floor --test lane_a_surface_evidence_floor --test lane_a_reduce_evidence_floor --test lane_a_project_evidence_floor` clean.

6. **Step 3.12 is the cumulative-regression gate** — run all six:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count in `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`** (memory `feedback_no_tail_pipe_for_long_tests`). **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run** (memory `feedback_atlas_test_subprocess_concurrency`). Use `--skip polyglot_phase3` substring (memory `feedback_cargo_skip_polyglot_pattern`).

7. **Step 3.13 is multi-commit + status flip.** The plan's recommended six-commit decomposition (you may consolidate or split further):
   - **Commit 1:** `outputs.rs` module + `evidence.rs` per-stage functions + AgentOutput accessors + their unit tests
   - **Commit 2:** `build_classify_prompt` production text + `classify_prompt_shape.rs` + `lane_a_classify_evidence_floor.rs`
   - **Commit 3:** `build_reduce_prompt` production text + `reduce_prompt_shape.rs` + `lane_a_reduce_evidence_floor.rs`
   - **Commit 4:** `build_project_prompt` (new function) + `project_prompt_shape.rs` + `lane_a_project_evidence_floor.rs` + `lane_a_surface_evidence_floor.rs`
   - **Commit 5:** `projection_to_canonical.rs` shim + its two test files + `pipeline.rs` JSON→YAML migration + wire-into-`run_workspace`
   - **Commit 6:** status flip (PR-3 row `[ ]` → `[x]`; "Last updated" header; PR-3 per-PR note with commit SHAs + deviations + forward-pointers to PR-4)

   Commit message convention: `sprint: PR-3 <short title>` (matches the sprint's existing `sprint:` prefix). NOT `phase7: PR-3` — that prefix is reserved for actual Phase 7 PRs and would collide forensically.

8. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your final implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 3 acceptance gate (plan §5 row PR-3). Particularly: are all three producer-prompt sites replaced (classify + reduce + project)? Is the `1.0` fallback for non-dispatch stages actually replaced (PR-2 forward-pointer)? Do the schema-drift tests cover all three new prompts? Does the shim's missing-field test cover at least three field paths (workspace_purpose, component kind, subsystem purpose)? Did the canonical-schema struct grep happen, and is the re-use decision recorded?

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues (correctness, security, broken invariants). Specific concerns for PR-3:
   - Did the strict-string adapter get applied to every identity-shaped string field in the new structs (including via the `ComponentIdRef` newtype for `Vec<String>` fields)?
   - Does the shim's `MissingProjectionField` error actually fire BEFORE any disk write (atomic-write semantics intact)?
   - Are the per-stage evidence functions handling the "vacuously satisfied" cases correctly (e.g., subsystem with zero children returns 1.0)?
   - Does the `parse_final_output` YAML fence-extract path actually catch the new prompts' output without changes?

   HIGHs fixed before status flip; MEDIUMs recorded in PR-3's per-PR note for later sweeps.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**. Don't ship broken code to flip the checkbox.

## Coordination with PR-B (parallel)

PR-B (`--disallowedTools` live-subprocess probe) is **parallel-safe** with PR-3 and may be running in another session. File sets are disjoint:
- **PR-3:** `crates/atlas-agents/src/runtime/{outputs.rs, projection_to_canonical.rs, audit/evidence.rs, audit/lane_a.rs, mod.rs}` + `crates/atlas-cli/src/pipeline.rs` + `crates/atlas-agents/tests/{classify,reduce,project}_prompt_shape.rs` + `crates/atlas-agents/tests/lane_a_{classify,surface,reduce,project}_evidence_floor.rs` + `crates/atlas-agents/tests/projection_to_canonical_shim*.rs`.
- **PR-B:** `crates/atlas-agents/tests/mcp_disallowed_tools.rs` only (single new file; `#[ignore]`-gated).

The two PRs touch disjoint subtrees — no shared edit surface. If a rebase conflict surfaces, **stop and surface** — the disjoint-files claim was load-bearing.

## Scope-creep guard

PR-3 is the largest PR in the sprint. The brainstorm §12 risk #1 mitigation (and plan §7.1) says: if the implementer reaches **4400 LOC** (2× the 1500–2200 budget) and the work is incomplete, **stop and surface** rather than continue. Surface what's done + what's outstanding + a split-PR-3 proposal (e.g., "outputs.rs + classify ships now as PR-3a; reduce + project + shim follow as PR-3b").

Indicators you may be heading there:
- The `ComponentsYaml` / `SubsystemsYaml` shape grep finds nothing and you're authoring the canonical-artifact structs from scratch (this can easily add 300+ LOC).
- The `L9Projection` shape doesn't carry every field the canonical shim needs, and you're considering extending the project prompt mid-PR.
- The `AgentOutput` accessor additions cascade into upstream refactors of dispatch parsing.

Any of those: pause, surface to the user with a "PR-3 is bigger than scoped because X; here's the split proposal" note.

## Begin at Step 3.1

Begin at **Step 3.1: Author the four typed output structs at `crates/atlas-agents/src/runtime/outputs.rs`** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 3.

Open the plan, locate the step, run the `component-ontology` enum grep first, and proceed.
