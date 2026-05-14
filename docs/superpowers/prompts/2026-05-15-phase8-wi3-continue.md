# Phase 8 WI-3 — kickoff prompt

Use this prompt to open the **Phase 8 WI-3 executor session**. The plan at `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` (committed in `78bb22c`) is the authoritative input. WI-1 + WI-2 shipped before this session opens — WI-1's code commit migrated `LlmRequest` to `Option<PromptId>` + `Option<String>`, short-circuited the four backends + router, un-ignored the HTTP smoke; WI-2's code commit added `AgentEvent::HardFail` emission to producer-fail (`runtime/mod.rs` `let output = outcome?;` site) and auditor-fail (`run_real_audit`'s auditor-backend-call arm) with distinct `error_kind` discriminators `"backend"` + `"audit_backend"`.

---

## Invocation

Invoke the `superpowers:executing-plans` skill, then hand it the body below.

## Body

Execute **Phase 8 WI-3** (Cargo classifier retirement) — the third of the four sequential work items defined in the plan. WI-3 deletes `CargoClassifyTool` from `default_tool_catalog`, rewrites the classify-prompt rubric so "strong" rewards `parse_cargo_toml` + source-read instead of a deterministic classifier call, deletes `crates/atlas-analyzers/src/cargo_classifier.rs` + `crates/atlas-agents/src/tools/classifiers/cargo.rs` outright, cascades the deletion across `lib.rs` / `registry.rs` / `dispatcher.rs` / `heuristics.rs` / `l3_classify.rs` / sibling-classifier doc-comments / `jsonl_subscriber.rs` test tool-name strings, and recalibrates the polyglot release smoke against the new deterministic-spine baseline (Cargo now falls through to `llm_classify.rs`). New test: `crates/atlas-agents/tests/cargo_retirement_smoke.rs`.

### Reading order

Read in this order; don't transitively read references unless a step forces it.

1. `docs/superpowers/specs/2026-05-15-atlas-vnext-phase8-plan.md` § 0 (reading order) → § 1 (phase deliverable) → § 2 (framings table — F11 is WI-3's framing lock) → § 4 (regression-guard table — polyglot **RECALIBRATE expected** for WI-3; § 4.5 has plan-time empirical pre-flight) → § 6 (test coverage; WI-3 row names the new test file) → **§ 9 (WI-3 file list + Step 3.1 → 3.9)**. § 10 is out of scope.
2. `docs/superpowers/plans/2026-05-15-phase8-status.md` — confirm WI-1 + WI-2 rows are `[x]` and read the per-WI notes for any plan-time deviations carried forward (notably WI-1's `Option<PromptId>` cascade across test backends + WI-2's `event_bus` threading into `run_real_audit`'s signature).
3. Plan-locked code-side anchors. Before editing each, verify the line numbers haven't drifted from the plan's frozen-2026-05-15 state (post-WI-1 + WI-2 anchors may have shifted further):
   - `crates/atlas-agents/src/runtime/mod.rs:249–253` (the `use crate::tools::classifiers::{...}` import — plan § 9 Step 3.2; actual line may differ post-WI-2's ~25-line addition near mod.rs:965 + ~15 lines near the audit closure).
   - `crates/atlas-agents/src/runtime/mod.rs:1411–1426` (the `confidence_grade` rubric inside `build_classify_prompt` — plan § 9 Step 3.3; verify at step-time).
   - `crates/atlas-agents/src/runtime/mod.rs:1396–1399` (the canonical-vocabulary list — plan § 9 Step 3.4).
   - `crates/atlas-analyzers/src/lib.rs:6, 37, 68, 400` (cargo_classifier re-exports — plan § 9 Step 3.6).
   - `crates/atlas-analyzers/src/registry.rs:30, 116, 377, 500, 572, 590, 623` (registry + test references — plan § 9 Step 3.6).
4. `.claude/memory/MEMORY.md` for the active framings the plan inherits (`[[feedback_no_deterministic_engine_comparison]]`, `[[feedback_atlas_llm_spine_intent]]`, `[[feedback_yaml_canonical_interchange]]`, `[[project_atlas_common_backend_config]]`, `[[project_phase7_agent_runtime_default_ratified]]`, `[[feedback_prefer_existing_crates]]`, `[[feedback_no_tail_pipe_for_long_tests]]`, `[[feedback_release_workspace_build_for_polyglot]]`, `[[feedback_cargo_skip_polyglot_pattern]]`, `[[feedback_atlas_test_subprocess_concurrency]]`).

### Locked decisions (inherited from the plan; NOT re-litigated)

- **Cargo retirement scope (plan F11):** classifier-only. `CargoClassifyTool` dropped from `default_tool_catalog`; `cargo_classifier.rs` deleted outright (no "deprecated" stub — F1 framing: deterministic spine is legacy). `parse_cargo_toml` STAYS (it's the LLM's manifest parser, distinct from the classifier). `rust_surface_analyzer.rs` is **untouched** in Phase 8; surface retirement folds into Phase 9 or 8b.
- **Classify-prompt rubric rewrite (plan § 9 Step 3.3):** "strong" now requires `parse_cargo_toml` (or analogous parser tool) + source entry-point READ. "moderate" = parser tool only. "weak" = manifest read only (no parser). "declines" unchanged. The rewrite removes the "classifier tool whose name matches the declared `kind` was CALLED" reward from the rubric.
- **`rust-workspace` canonical-vocabulary addition (plan § 9 Step 3.4):** add to the open-vocabulary list at mod.rs:~1396–1399. Distinguishes a `Cargo.toml` with `[workspace]` table (no `[lib]`/`[bin]`) from `rust-library` / `rust-binary`. The worked YAML example stays as `rust-library`; `rust-workspace` is only in the vocabulary list.
- **Commit decomposition (plan § 9 Step 3.9):** **three commits.** WI-3a = `crates/atlas-agents/` edits (catalog drop + prompt rubric + classifiers/cargo.rs delete + new test file); WI-3b = `crates/atlas-analyzers/` deletions + `atlas-engine` cascade + `atlas-cli/tests` cascade; WI-3c = status flip with empirically-measured cold-count baseline. The intermediate WI-3a state should pass polyglot smoke unchanged (the deterministic spine + `cargo_classifier.rs` still exist); WI-3b is where the smoke MAY shift.
- **Polyglot smoke recalibration (plan § 4.5 + § 9 Step 3.7):** § 4.5's plan-time grep showed 11 `Cargo.toml` files in `tests/fixtures/` (2 in `tiny/`, 9 in `phase3_polyglot/`). After WI-3b lands, deterministic Cargo dispatch falls through to `llm_classify.rs`. Measure the new cold count empirically; record inline in plan § 9 Step 3.7's empirical-result line. If `0 < cold < 100` still holds → leave assertion; if not → escalate to user before tightening.

### Operating discipline

- **No scope creep outside WI-3.** Do NOT touch:
  - WI-2 deliverables (the producer-fail / auditor-fail HardFail emit sites are out of scope; `run_real_audit`'s `event_bus` parameter stays as WI-2 left it).
  - WI-4 deliverables (calibration run, parity test un-ignore, recast spec §11.2 amendment, memory updates).
  - `rust_surface_analyzer.rs` — surface retirement is post-Phase-8 per F11.
  - `parse_cargo_toml` — the manifest parser stays; only the classifier is retired.
- **Cumulative regression guard (plan § 4):** polyglot release smoke is expected to **RECALIBRATE** in WI-3b — measure the new cold count and record inline. Workspace test (`cargo test --workspace -- --skip polyglot_phase3`) MUST stay green across all three commits; clippy + fmt MUST stay clean. Use `cargo build --release --workspace` before polyglot per `[[feedback_release_workspace_build_for_polyglot]]`. Do NOT pipe through `tail` per `[[feedback_no_tail_pipe_for_long_tests]]`. Run dev workspace tests and release polyglot sequentially per `[[feedback_atlas_test_subprocess_concurrency]]`.
- **Cascade risk (plan § 9 Step 3.6 cascade-in-`atlas-engine` callout):** `l3_classify.rs:27` drops `cargo_classifier::CargoClassificationOutput` from `use`. If the rest of `l3_classify.rs` pattern-matches on this type, the dispatch path must fall through to `llm_classify.rs` cleanly (Option A per the plan — preferred). Don't introduce a new `LlmFallbackOutput` shape (Option B) unless Option A breaks tests or compile.
- **Inherit the plan's risk register.** R2 (polyglot cold count exceeds 100) is the active WI-3 risk; § 4.5's plan-time empirical pre-flight surfaced this as live, not hypothetical. R5 (`ShimError::MissingProjectionField` resurface in WI-4) is out of scope for WI-3.
- **No new memory writes this session.** WI-3 implements a locked framing; it makes no architectural decisions. Memory updates land in WI-4 per plan § 13.

### Deliverable shape

Three commits:

1. **Code commit 1 (agent layer):** `crates/atlas-agents/` only — `default_tool_catalog` drops `CargoClassifyTool`; `tools/classifiers/cargo.rs` deleted; `tools/classifiers/mod.rs` re-export dropped; `build_classify_prompt` rubric + canonical-vocabulary rewrite; new `crates/atlas-agents/tests/cargo_retirement_smoke.rs`. Title: `phase8 WI-3a: drop CargoClassifyTool + classify-prompt rubric rewrite`.
2. **Code commit 2 (analyzer + engine + CLI cascade):** `crates/atlas-analyzers/src/cargo_classifier.rs` deleted; `lib.rs` / `registry.rs` / `dispatcher.rs` cascades; sibling classifier doc-comments; `atlas-engine/src/heuristics.rs` + `l3_classify.rs` cleanup; `atlas-cli/tests/jsonl_subscriber.rs` tool-name string rename. Title: `phase8 WI-3b: delete deterministic cargo_classifier + cascade`.
3. **Status-flip commit:** WI-3 row `[ ]` → `[x]` in the status doc; WI-3 per-WI note (what shipped, regression gates summary, empirical polyglot cold-count + wall-time baseline, any plan-time deviations); "Last updated" header refresh. Title: `phase8 WI-3: status flip`.

### Acceptance gate (mirrors plan § 9's "Acceptance gate (WI-3)")

Verify every bullet before flipping the status row:

- `CargoClassifyTool` absent from `default_tool_catalog`; catalog count drops from 22 to 21.
- Classify-prompt `confidence_grade` rubric matches plan § 9 Step 3.3.
- `rust-workspace` added to canonical-vocabulary list at `runtime/mod.rs:~1396`.
- `crates/atlas-analyzers/src/cargo_classifier.rs` deleted.
- `crates/atlas-agents/src/tools/classifiers/cargo.rs` deleted.
- All cascade references cleaned (lib.rs / registry.rs / dispatcher.rs / heuristics.rs / l3_classify.rs / sibling-classifier doc-comments / jsonl_subscriber.rs).
- `cargo_retirement_smoke.rs` all three tests green (catalog-absence + rust-library + rust-workspace).
- Polyglot smoke recalibrated cold-count recorded; either passes within `0 < cold < 100` or widened bound (with user authorisation).
- All six regression gates clean (`cargo build --workspace`, `cargo test --workspace -- --skip polyglot_phase3`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build --release --workspace`, polyglot release smoke).

### Drop on completion

When WI-3's status-flip commit lands, `git rm docs/superpowers/prompts/2026-05-15-phase8-wi3-continue.md` in that same commit per the drop-on-completion convention.

Author the next continuation prompt (`docs/superpowers/prompts/<DATE>-phase8-wi4-continue.md`) alongside WI-3's status flip, so the WI-4 fresh session has a kickoff to point at. Mirror this prompt's structure; swap the body for plan § 10 (WI-4 deliverables: Atlas-on-Atlas calibration run, parity test un-ignore, recast spec §11.2 amendment, intrinsic-metrics baseline, memory updates, Phase 8 SHIPPED closeout).

### Begin at plan § 9 Step 3.1

Open the plan; read §§ 1–2 + 4 + 6 + 9; verify the anchor line numbers haven't drifted (post-WI-2 the catalog import + classify-prompt anchors may have shifted by 20–40 lines); then begin **Step 3.1** — write the failing cargo-retirement smoke test at `crates/atlas-agents/tests/cargo_retirement_smoke.rs`.

---

## Why this prompt exists in `docs/superpowers/prompts/`

The `prompts/` directory holds in-flight invocation prompts that bootstrap the next session. They are dropped (precedent `7d6f6f3` / `f9315f6` / `ca93814`) in the status-flip commit of the work they kick off. This is WI-3's kickoff; when WI-3's status-flip commit lands, this prompt gets dropped in that same commit, and the WI-4 kickoff prompt is authored alongside it.
