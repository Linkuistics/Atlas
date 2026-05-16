# Phase 8 WI-4 — kickoff prompt

Use this prompt to open the **Phase 8 WI-4 executor session**. The plan at `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` (committed in `78bb22c`) is the authoritative input. WI-1 + WI-2 + WI-3 shipped before this session opens — WI-3a's code commit `165d0a2` dropped `CargoClassifyTool` from `default_tool_catalog` + rewrote the classify-prompt rubric to reward `parse_cargo_toml` + source-read instead of a deterministic classifier call + added `rust-workspace` to the canonical-vocabulary list; WI-3b's code commit `da96fc5` deleted `crates/atlas-analyzers/src/cargo_classifier.rs` outright, cascaded the deletion across `lib.rs` / `registry.rs` / `dispatcher.rs` / sibling-classifier doc-comments / `heuristics.rs` / `l3_classify.rs` (Option A — Cargo dispatch falls through to `LlmClassifyOutput`) / `jsonl_subscriber.rs` / `scattered_atlas_layout.rs`, and recalibrated the polyglot smoke via eight override `additions` entries in `phase3_polyglot/.atlas/components.overrides.yaml` declaring peer1-peer6 + outlier + rust-lib explicitly (chosen over plan's Option A per user direction at the WI-3b boundary, preserving F1 framing).

---

## Invocation

Invoke the `superpowers:executing-plans` skill, then hand it the body below.

## Body

Execute **Phase 8 WI-4** (Atlas-on-Atlas calibration + closeout) — the fourth and final work item of Phase 8. WI-4 re-runs the Atlas-on-Atlas calibration sprint PR-5 left as "n/a — pre-HTTP hard-fail", un-ignores `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` against real LLM output, amends recast spec §11.2 per plan §11, populates the intrinsic-metrics baseline in the status doc, ships memory updates (`project_phase4_plus_roadmap.md` Phase-8-SHIPPED entry + `MEMORY.md` hook-line refresh), and writes the Phase 8 closeout entry in the status doc.

### Reading order

Read in this order; don't transitively read references unless a step forces it.

1. `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` § 0 (reading order) → § 1 (phase deliverable) → § 2 (framings table — F3 + F4 + F12 + F13 are WI-4's framing locks) → § 4 (regression-guard table — polyglot HOLD at WI-3b's recalibrated baseline for WI-4) → § 6 (test coverage; WI-4 row covers `agent_runtime_cross_provider_parity.rs` un-ignore) → **§ 10 (WI-4 file list + Step 4.1 → 4.8)**. § 11 carries the recast spec §11.2 amendment text. § 12 is the risk register (R5 + R7 + R8 are active WI-4 risks). § 13 is the memory-updates checklist.
2. `docs/superpowers/plans/2026-05-15-phase8-status.md` — confirm WI-1 + WI-2 + WI-3 rows are `[x]` and read the per-WI notes for plan-time deviations carried forward (notably WI-3's polyglot recalibration empirical baseline: cold count 48, wall time 117.53s, 24 components; the override-only fixture approach + the eight `additions` entries in `phase3_polyglot/.atlas/components.overrides.yaml`).
3. Plan-locked code-side anchors. Before editing each, verify the line numbers haven't drifted from the plan's frozen-2026-05-15 state (post-WI-2 + post-WI-3a + post-WI-3b anchors may have shifted further):
   - `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` — find the `#[ignore]`-attributed test function; plan § 10 Step 4.3 calls it `cross_provider_canonical_artifact_parity_holds` (verify at step-time). The strict-vs-lenient assertion shape is plan F13: strict component-id-set equality + strict edge-multiset equality + lenient subsystem-count (`± 1` legitimate drift).
   - `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` § 11.2 — the Deliverables + Acceptance paragraphs that plan § 11 replaces.
   - `.claude/memory/project_phase4_plus_roadmap.md` — the existing roadmap entry shape (Phase 1-7 SHIPPED bullets) that the Phase 8 SHIPPED entry mirrors.
   - `.claude/memory/MEMORY.md` — the existing one-line hook-text for `project_phase4_plus_roadmap.md` that gets a description refresh.
4. `.claude/memory/MEMORY.md` for the active framings the plan inherits (`[[feedback_cross_provider_llm_audit]]` — both `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` MUST be set, no same-model degradation; `[[project_atlas_common_backend_config]]`; `[[feedback_atlas_llm_spine_intent]]`; `[[feedback_no_deterministic_engine_comparison]]`; `[[feedback_yaml_canonical_interchange]]`; `[[project_phase7_agent_runtime_default_ratified]]`; `[[feedback_no_tail_pipe_for_long_tests]]`; `[[feedback_release_workspace_build_for_polyglot]]`).

### Locked decisions (inherited from the plan; NOT re-litigated)

- **Cross-provider audit is non-negotiable (plan F3 + R7):** WI-4's Step 4.1 verifies both `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are set BEFORE running anything. If either is missing → abort the session, set both keys, restart fresh. Do NOT skip — brainstorm R7 explicitly notes that silent same-model degradation defeats the auditor's purpose.
- **HTTP backend pair (plan F4):** the calibration runs against `http_anthropic` producer / `http_openai` auditor. Subprocess pair (`claude_code + codex`) is NOT the WI-4 substrate per F4 framing (HTTP backends are signal-gathering opt-ins; Phase 8 calibrates against the HTTP pair to exercise the bypass + HardFail emit paths shipped in WI-1 + WI-2).
- **Parity-test assertion shape (plan F13):** strict component-id-set equality (`anthropic_ids == openai_ids`), strict edge-multiset equality, lenient subsystem-count (`± 1` legitimate provider drift). The `± 1` is the F13 lock; widening to `± 2` requires user authorisation. F13 framing: "disagreement is signal, not failure" — strict failures lift the un-ignore to `#[ignore = "tracked drift"]` with explicit reason text, not phase-block.
- **Recast spec §11.2 amendment (plan F12 + § 11):** WI-4's commit lands the §11.2 rewrite verbatim from plan § 11. The amendment replaces "vs deterministic" framing with "within-LLM-spine cross-provider parity + intrinsic-metrics baseline" per `[[feedback_no_deterministic_engine_comparison]]`.
- **Intrinsic-metrics baseline (plan § 1 deliverable + § 10 Step 4.4):** the plan asserts NO thresholds for the baseline; future PRs assert against the recorded values. Cold token totals per provider, iteration count, wall time, components classified, subsystems partitioned, evidence-score distribution, Lane A retry counts, audit verdict distribution, `ShimError::MissingProjectionField` count.
- **Memory updates (plan § 13):** WI-4's commit appends "Phase 8 — SHIPPED 2026-05-DD" to `project_phase4_plus_roadmap.md` in WI-N notation with baseline numbers + Phase 9 unblocked flag; refreshes `MEMORY.md`'s one-line hook for the roadmap memory. No new memory files created (WI-4 implements a locked framing per plan § 13).

### Operating discipline

- **No scope creep outside WI-4.** Do NOT touch:
  - WI-1 / WI-2 / WI-3 deliverables (the bypass, HardFail emit, cargo retirement are out of scope; they all shipped and have status-flip commits).
  - `rust_surface_analyzer.rs` — Phase 8 untouches surface retirement (folds into Phase 9 or 8b per F11).
  - The polyglot smoke (`phase3_polyglot_fixture.rs`) — its recalibrated baseline is the WI-3b empirical baseline (cold=48, wall=117.53s, 24 components) and HOLDS at WI-4.
- **Cumulative regression guard (plan § 4):** polyglot release smoke must stay at WI-3b's empirical baseline (cold count 48; wall time 117.53s; loose bound `0 < cold < 100` preserved). Workspace test (`cargo test --workspace -- --skip polyglot_phase3`) MUST stay green; clippy + fmt MUST stay clean. Use `cargo build --release --workspace` before polyglot per `[[feedback_release_workspace_build_for_polyglot]]`. Do NOT pipe through `tail` per `[[feedback_no_tail_pipe_for_long_tests]]`. Run dev workspace tests and release polyglot sequentially per `[[feedback_atlas_test_subprocess_concurrency]]`.
- **Live-LLM gate (plan § 10 Step 4.1):** WI-4 runs against real LLM endpoints. The `--ignored`-gate flip in Step 4.3 means the parity test will hit live LLM APIs every CI run. Plan § 10 Step 4.3 offers two gate shapes — `#[ignore]` drop (simpler) vs `#[cfg(feature = "cross_provider")]` cargo-feature gate (more CI-friendly). Implementer's call; the latter is recommended if CI doesn't have API keys.
- **R5 (`ShimError::MissingProjectionField`) is the active WI-4 risk.** Production prompts may emit YAML that lacks fields the canonical-schema shim requires. Per plan § 10 Step 4.2 mitigation: if a `MissingProjectionField` surfaces during the calibration run, bundle the prompt fix inside WI-4 if trivial; spin a follow-on work item if not (`docs/superpowers/specs/2026-05-DD-phase8b-shim-followup-plan.md`).
- **Authorize ONE new memory write only if R7 / R8 forces it.** WI-4 implements a locked framing; per plan § 13 it writes memory updates (`project_phase4_plus_roadmap.md` + `MEMORY.md`) but does NOT create new memory files. Exception: if cross-provider drift surfaces a *new* decisional pattern worth retaining for Phase 9, that authorizes a new feedback/project memory with the rationale documented in the WI-4 commit message.

### Deliverable shape

**Decomposition: one or two commits.** Plan § 10 Step 4.8 lists six logical artefacts (calibration run output, parity test un-ignore, recast spec amendment, intrinsic-metrics baseline, memory updates, status-doc closeout). They cohere as a single Phase-8-SHIPPED commit, OR — if the calibration surfaces a prompt fix (R5 mitigation) — two commits: the prompt fix first, then the calibration + closeout. Implementer chooses at step-time based on what the calibration run surfaces.

Commit title options:
- Single-commit shape: `phase8 WI-4: Atlas-on-Atlas calibration + Phase 8 SHIPPED`.
- Two-commit shape: `phase8 WI-4a: <prompt fix description>` + `phase8 WI-4: closeout + Phase 8 SHIPPED`.

### Acceptance gate (mirrors plan § 10's "Acceptance gate (WI-4)")

Verify every bullet before flipping the status row + writing the Phase-8-SHIPPED section:

- `./target/release/atlas index . --agent-runtime --config .atlas/config.sprint.yaml --log-events <path> --no-tui --no-budget` completes end-to-end (exit code 0). Sprint PR-5's pre-HTTP `LlmError::TemplateSyntax("unknown token \`{{COMPONENT_KINDS}}\` in template")` diagnostic is closed.
- `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs` un-ignored (or `#[cfg(feature = "cross_provider")]`-gated); three asserts (component-id set equality, edge-multiset equality, subsystem-count `± 1`) fire on real cross-provider traffic. Outcome recorded in closeout note (HOLDS / lenient-drift / strict-drift per F13).
- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` § 11.2 carries the plan-§11 amendment text verbatim.
- `docs/superpowers/plans/2026-05-15-phase8-status.md`'s intrinsic-metrics baseline section is populated with the empirical values from the calibration run (cold token totals per provider, iteration count, wall time, components classified, subsystems partitioned, evidence-score distribution per stage, Lane A retry counts per stage, Lane B revision counts per stage, audit verdict distribution, `ShimError::MissingProjectionField` count + field names if non-zero).
- `.claude/memory/project_phase4_plus_roadmap.md` appended with "Phase 8 — SHIPPED 2026-05-DD" entry in WI-N notation; `.claude/memory/MEMORY.md` hook-line description refreshed.
- WI-4 row `[ ]` → `[x]` in the status doc; "Last updated" header refreshed to "2026-05-DD (WI-4 status flip — Phase 8 SHIPPED)"; closeout `## Phase 8 — complete` section appended per plan § 10 Step 4.6 template.
- All six regression gates clean (`cargo build --workspace`, `cargo test --workspace -- --skip polyglot_phase3`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build --release --workspace`, polyglot release smoke at WI-3b baseline cold=48 / wall~117s).

### Drop on completion

When WI-4's closeout commit lands, `git rm docs/superpowers/prompts/2026-05-16-phase8-wi4-continue.md` in that same commit per the drop-on-completion convention.

No follow-on kickoff prompt is required. Phase 8 closes at WI-4; the next phase (Phase 9) is opened by a fresh brainstorm session per the recast spec §10.2 context-rot discipline.

### Begin at plan § 10 Step 4.1

Open the plan; read §§ 1–2 + 4 + 6 + 10 + 11 + 12 + 13; verify the anchor line numbers haven't drifted (post-WI-3 the cross-provider parity test file may have shifted by ~10 lines from sprint PR-5's frozen state; § 11's recast amendment text is plan-author authored, not code-anchored); then begin **Step 4.1** — verify both `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are exported (abort the session immediately if either is missing per F3 + R7).

---

## Why this prompt exists in `docs/superpowers/prompts/`

The `prompts/` directory holds in-flight invocation prompts that bootstrap the next session. They are dropped (precedent `7d6f6f3` / `f9315f6` / `ca93814` / `da96fc5`'s parent WI-3b kickoff `2026-05-15-phase8-wi3-continue.md` dropped in WI-3's status-flip commit) in the status-flip commit of the work they kick off. This is WI-4's kickoff; when WI-4's closeout commit lands, this prompt gets dropped in that same commit. No subsequent kickoff is authored — Phase 8 closes at WI-4 and Phase 9 opens with a fresh brainstorm.
