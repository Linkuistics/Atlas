# PR-4 continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-4** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. This prompt is self-contained — read this, then read the canon in the order below, then drive the implementation via `superpowers:executing-plans`.

## What PR-4 ships

PR-4 is **structural, MEDIUM** — Wave 4 of the sprint dependency graph; the last gating PR before Phase 8 (Cargo retirement) unblocks. It replaces the `PR-7-WIRES-REAL-AUDITOR` stub with a real cross-provider audit-prompt round-trip, wires the revision-prompt path that re-invokes the producer with the auditor's reason as a system-prompt addendum, and persists the verdict on disk at `.atlas/audit/<stage>/<target>.yaml` so re-runs replay rather than re-audit.

Deliverables:

1. **`crates/atlas-agents/src/runtime/audit/audit_prompt.rs`** — new module. `build_audit_prompt(producer_provider, auditor_provider, stage, producer_output_rendered, transcript_tuples) -> String` + `render_transcript_for_audit(transcript) -> String` rendering the producer's tool-call trail as ordered `(tool_name, args_summary, result_summary)` tuples + `summarise_args` / `summarise_result` byte-budgeted truncators (200 / 400 bytes; PR-5 calibration may surface a need to adjust). Truncation hint: `"[N bytes truncated]"` so the auditor knows it isn't seeing the full result. Embeds the fenced ```yaml verdict example per brainstorm §7.1.

2. **`crates/atlas-agents/src/runtime/audit/verdict.rs`** — new module. `AuditVerdictOnDisk` struct (YAML shape per brainstorm §7.4 verbatim: `agent_id`, `stage`, `producer:{provider,model,output_sha}`, `auditor:{provider,model,verdict,reason}`, `audit_tokens:{in,out}`, `audited_at`) + `VerdictKind` enum (`Accept | RequestRevision | HardFail | Skipped`, `#[serde(rename_all = "snake_case")]`) + `write_verdict_pair(audit_dir, stage, target_id, &verdict, transcript_text) -> Result<()>` using Phase 7 PR-2's `atomic_write_pair` + `read_verdict_if_complete(audit_dir, stage, target_id) -> Result<Option<AuditVerdictOnDisk>>` for the re-run replay path. Half-pair on disk (one file present, the other absent) surfaces as `Ok(None)` → re-audit; the orphan file is left in place (forensic). Strict-string adapter (`deserialize_string_strict` from `runtime::yaml_strict`) on every identity-shaped string field (`agent_id`, `output_sha`, etc.) per PR-2 / PR-3 pattern.

3. **Auditor stub replacement at `mod.rs:746`** — **VERIFY THE LINE NUMBER FIRST**. The plan says `:665`; PR-2/PR-3 work has shifted it. As of 2026-05-13 post-PR-3, the comment `PR-7-WIRES-REAL-AUDITOR` lives at `mod.rs:746`; `resolve_audit_verdict` at `:973`; `now_iso` at `:1502`; `call_agent` at `:590`. Re-grep at PR start to re-pin:
   ```bash
   grep -nE "PR-7-WIRES-REAL-AUDITOR|fn resolve_audit_verdict|pub\(super\) fn now_iso|pub async fn call_agent" \
       crates/atlas-agents/src/runtime/mod.rs
   ```
   The replacement closure: pre-flight verdict cache lookup → cross-provider auditor backend lookup via `Provider::cross()` + `for_provider(auditor_provider)` → render transcript + build audit prompt → call auditor `.call_async(...)` → fence-extract YAML body via `prompt_examples::extract_yaml_fence` → `serde_yaml::from_str::<AuditorVerdictYaml>` → emit `AuditFire` + `AuditVerdict` events → persist verdict to disk → return `AuditOutcome`. The `Provider::cross()` symmetry (Anthropic↔OpenAI) is the canonical cross-provider recipe per memory `feedback_cross_provider_llm_audit`.

4. **Revision-prompt path** — `build_revision_addendum(producer_previous_output, auditor_reason, retries_remaining) -> String` per brainstorm §7.3 + extend `resolve_audit_verdict` (currently at `mod.rs:973`) to invoke `runtime.call_agent` recursively with the revision-augmented prompt on the `RequestRevision` branch instead of merely accepting the producer result. The recursive call increments `lane_a_retries` (or a sibling `lane_b_revisions` field on `ToolLoopOutcome`); cumulative cap = 2 per agent. Phase 7 PR-5's existing escalation rule (`lane_a_retries >= 1` → `RequestRevision` becomes `HardFail`) carries forward — PR-4 wires the **counter increment**, not the cap policy.

5. **`AgentRuntime` gains `audit_dir: PathBuf` field** threaded from `crates/atlas-cli/src/pipeline.rs::run_index_agent_runtime` (currently constructs the runtime around line 1100; verify) as `<workspace_root>/.atlas/audit/`. **All seven `AgentRuntime` construction sites** (CLI pipeline + dispatch.rs unit-test helper + 5 integration-test files; mirror PR-A's `mcp_server` cascade) need the new field. Tests use `tempfile::TempDir`.

6. **Strengthened `cross_provider_audit_routing.rs`** — Phase 7 PR-5 ship-test asserted `AuditDegraded` fires on single-provider config but never invoked the real audit code path (stub returned `Accept`). PR-4 makes the assertions stronger: real audit round-trip via SettableTestBackend cross-provider pair; single-provider config → `AuditDegraded` + same-model fallback exercising the **real audit code path**; `OpenAi` producer → `Anthropic` auditor symmetry.

7. **New tests:**
    - `crates/atlas-agents/tests/audit_prompt_shape.rs` — embedded verdict-rubric kinds present; embedded YAML example deserializes; transcript renderer truncates long values with the byte-budget hint.
    - `crates/atlas-agents/tests/audit_revision_round_trip.rs` — synthetic producer + auditor; auditor emits `request_revision`; producer's retry sees `AUDITOR'S CRITIQUE` substring + reason in the system-prompt addendum; cumulative budget escalates to `HardFail` on the second revision.
    - `crates/atlas-agents/tests/audit_verdict_atomic_write.rs` — round-trip through disk; half-pair triggers re-audit; YAML shape contains the brainstorm-§7.4 keys verbatim.
    - `crates/atlas-agents/tests/cross_provider_audit_routing.rs` — Phase 7 PR-5 test extended (this file already exists; verify path); strengthened assertions per above.

LOC budget: **400–700** (plan §4 Task 4 closing). If approaching **1400** (2× budget), **stop and surface** — brainstorm §12 risk #1 framing applies to any PR in this sprint.

## Scope exclusions — PR-4 does NOT do these

- **PR-4 does NOT touch dispatch / classify / reduce / project prompts.** PR-2 + PR-3 own those.
- **PR-4 does NOT introduce an auditor-side confidence grade.** Decision row 8 (brainstorm §2.2) locks: auditor emits verdict only (`Accept | RequestRevision | HardFail | Skipped`), no grade — avoids auditor-of-auditor regress.
- **PR-4 does NOT add a model downgrade tier.** Sprint commits to `gpt-5-codex` as the auditor for the recorded baseline; dev-iteration may swap via `--config` (memory `project_atlas_common_backend_config`) but the calibration in PR-5 uses GPT-5-Codex.
- **PR-4 does NOT record calibration metrics.** PR-5 owns Atlas-on-Atlas calibration; PR-4 only emits the `AgentEvent::AuditVerdict` + `AgentEvent::AuditFire` events PR-5 will aggregate.
- **PR-4 does NOT touch MCP framing.** PR-A landed `rmcp` + `serve_client.rs`. The subprocess MCP / auditor interaction is **not exercised by PR-4** — HTTP backends are the sprint's live path (memory `project_phase7_agent_runtime_default_ratified`).
- **PR-4 does NOT add the `--disallowedTools` probe.** PR-B landed.
- **PR-4 does NOT change the PR-3 producer output shape.** The audit prompt consumes `producer_result` as-rendered; if rendering changes mid-PR, surface.
- **PR-4 does NOT add new workspace dependencies** beyond what Phase 7 + PR-1/PR-2/PR-3/PR-A already pulled in.

If asked to do anything from this list, **stop and surface** — that's not PR-4 scope.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 4 (lines 1977–2535) — your scope. Steps 4.1 → 4.10 are the implementation steps. Also skim §0–§3 (reading order, deliverable restated, non-negotiables, dependency graph) and §2.1's 15 decision rows. PR-4 implements decision rows 5 (cross-provider routing via `Provider::cross()` + `for_provider`), **6 (audit prompt input shape — producer output + transcript tuples)**, **8 (audit verdict failure modes — `Accept | RequestRevision | HardFail | Skipped`; no auditor grade)**, and **15 (on-disk verdict at `.atlas/audit/<stage>/<target>.yaml`)** in their entirety. Row 7 (Opus + GPT-5-Codex pairing) is locked from PR-1 — PR-4 honours it.

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-1's, PR-2's, PR-3's, and PR-A's per-PR notes carry forward-pointers PR-4 must honour. The most load-bearing surface changes from PR-3 (read items 3, 6, 7, 8, 10 in full):
   - **(a) `evidence_pointers` ordering convention** (PR-3 note item 3): index 0 is the primary manifest path; index 1 is the entrypoint. PR-4's audit prompt can reference this convention when rendering the producer's output for the auditor to evaluate evidence-trail soundness.
   - **(b) `expected_classify_tool_id` derives from `kind`** (PR-3 note item 4): the same prefix-match table is the right derivation for the auditor too — do NOT assume the LLM declared its expected tool.
   - **(c) Lane B regime under PR-3** (PR-3 note item 6): `lane_b_wired_into_call_agent_fires_when_evidence_floor_clamps_below_strong` is the post-PR-3 test name. **PR-4's revision-round-trip tests should use the same "evidence-clamped grade" assumption** — synthetic producers with empty transcript / no evidence_pointers → classify_evidence = 0.0 → ceiling = Declines → Lane B fires deterministically.
   - **(d) Test-backend canned-response key pattern** (PR-3 note item 7): PR-3 keys test backends on `"<stage> agent"` (the PR-3 prompts open with `"You are Atlas's <stage> agent"`). **PR-4's audit prompt tests should key on `"auditor"`** (audit prompt opens with `"You are an auditor for an Atlas agent's output"`).
   - **(e) `run_iteration` rollups are local** (PR-3 note item 8): if the auditor needs access to the producer's predecessor-stage rollups (e.g., classify output rollup when auditing a reduce output), **plumb through `AgentRequest` or a runtime-side per-call context map**. PR-3 explicitly flagged this as a PR-4 decision. The brainstorm §7.1 audit prompt template threads only `producer_output_rendered` + `transcript_tuples` — if you discover the auditor needs more, surface before extending.
   - **(f) CLI-layer composition pattern for on-disk artifacts** (PR-3 note item 10): PR-3 wired the canonical-schema shim at `pipeline.rs` not `run_workspace`. **PR-4's `audit_dir` should follow the same pattern** — `pipeline.rs::run_index_agent_runtime` owns the filesystem layout (`<workspace_root>/.atlas/audit/`) and passes it into `AgentRuntime` construction; the runtime stays decoupled from the CLI's filesystem-layout concerns.

   Forward-pointers from PR-2 that PR-4 inherits:
   - **`lane_a_validate` returns `Grade`** (PR-2 note item 2): the now-real grade flow into `resolve_audit_verdict` is what gates Lane B firing. PR-4's recursive `call_agent` invocation for revision picks up the same grade flow.
   - **`Grade` variant order is load-bearing** (`Declines < Weak < Moderate < Strong`; PR-2 note item 3): if PR-4 introduces any grade comparison (e.g., comparing claimed vs. evidence-clamped grades for the audit prompt's producer-grade rendering), use the natural order — do not alphabetise.
   - **`AgentOutput.text` field + YAML-fence-first `parse_final_output`** (PR-2 note items 4 + 9): PR-4's audit prompt advertises a fenced ```yaml verdict; `parse_final_output` will catch it automatically. Audit verdict parsers should read from `output.text` symmetrically with dispatch / classify / reduce / project.

   Forward-pointers from PR-1 that PR-4 builds on:
   - **`Provider::cross()` lives on the enum** (PR-1 note item 5; `crates/atlas-llm/src/lib.rs`): one-line match `Anthropic↔OpenAi`. **PR-4's auditor closure calls `Provider::cross()` to look up the sibling**, then `for_provider(provider)` (the closure plumbed via `Arc<ForProviderFn>` in PR-1 note item 4) to materialise the auditor backend.
   - **`provider_from_config_key` extends cross-provider routing to the canonical subprocess pair** (PR-1 note item 4): cross-provider audit works for `claude_code + codex` configs too, not just HTTP. PR-4 inherits this for free — no special-casing needed.

   Forward-pointer from PR-A (mostly informational; PR-4 doesn't exercise subprocess transports):
   - **`AgentRuntime` already has `mcp_server: Option<Arc<McpServer>>` field** (PR-A note item 5). PR-4's new `audit_dir: PathBuf` field follows the same cascade pattern — update all seven construction sites.

   After your work lands, append PR-4's per-PR note with commit SHAs + deviations + forward-pointers to PR-5 (especially any new event-shape additions PR-5's intrinsic-metrics aggregator needs to know about, plus whether the `lane_b_revisions` counter ships as a sibling field on `ToolLoopOutcome` or repurposes `lane_a_retries`).

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** — read §7 in full (lines 702–861; §7.1 audit prompt template, §7.2 transcript rendering, §7.3 revision prompt path, §7.4 on-disk verdict YAML shape, §7.5 Lane B closure, §7.6 tests, §7.7 acceptance). Also relevant:
   - **§2.2 decision rows 5, 6, 7, 8, 15** — the locked auditor-related decisions.
   - **§12.4 (line 1177): "Recast spec §11.2 'reference-output comparison harness' conflict"** — the recast spec floated a "compare LLM-spine output to a recorded reference output" idea for regression detection. PR-4's audit verdict is NOT that harness; verdicts are intrinsic semantic-soundness signals, not reference-comparison signals. Memory `feedback_no_deterministic_engine_comparison` reinforces.
   - **§12.6 (line 1209): "Per-stage iteration cap × concurrency math"** — relevant if your revision-prompt implementation surfaces unexpected interaction with the per-stage iteration cap (currently no concurrency in classify/reduce/project but the cap math compounds with revision attempts).
   - **§12.8 (line 1233): "YAML-specific risks (Norway problem + indentation drift)"** — the audit verdict YAML shape must survive the same Norway-problem hazards PR-2 hardened against. The `deserialize_string_strict` adapter from `runtime::yaml_strict` is the existing remedy; apply to every identity-shaped string field in `AuditVerdictOnDisk`.

   **If plan and brainstorm disagree, brainstorm wins.**

4. **Sprint framing memories** (durable; condition every decision):
   - `.claude/memory/feedback_cross_provider_llm_audit.md` — **the canonical recipe behind PR-4.** Same-model audit is tautological; different providers (Anthropic↔OpenAI) is the entire point. If you find yourself making the auditor closure work on a single-provider config without exercising `AuditDegraded`, you've drifted.
   - `.claude/memory/feedback_yaml_canonical_interchange.md` — audit verdict on-disk is YAML (per brainstorm §7.4). No JSON sneaks in.
   - `.claude/memory/project_atlas_common_backend_config.md` — the typical Atlas config pairs `claude_code + codex` (subscription-subsidized subprocess pair); HTTP backends are signal-gathering opt-ins. PR-4's auditor closure must work for both config shapes — `provider_from_config_key` handles the routing.
   - `.claude/memory/project_atlas_purpose_llm_consumers.md` — audit verdicts are LLM-consumed; downstream tools read `.atlas/audit/<stage>/<target>.yaml`. Verdict format quality matters as much as code quality.
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` — audit verdicts are intrinsic LLM-spine signals; never positioned as "compare with deterministic-engine output".
   - `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent.
   - `.claude/memory/feedback_prefer_existing_crates.md` — reuse Phase 7 PR-2's `atomic_write_pair` + Phase 7 PR-4's `Stage::as_str` + Phase 7 PR-7's `now_iso` + PR-2's `runtime::yaml_strict::deserialize_string_strict` + PR-2's `runtime::prompt_examples::extract_yaml_fence`. Do not hand-roll YAML extraction, atomic-write pair semantics, or strict-string deserializers.

5. **Operational memories:**
   - `.claude/memory/project_phase7_agent_runtime_default_ratified.md` — `--agent-runtime` opt-in default; HTTP backends are the sprint's live path.
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints.
   - `.claude/memory/feedback_no_iterator_stubs_for_singletons.md` — if you find yourself adding a singleton iterator over `lane_a_retries` etc., simplify.
   - `.claude/memory/feedback_worktree_base_verification.md` — verify any subagent worktree base matches current main.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 4** and follow Steps 4.1 → 4.10 in order. Mark `[x]` as you complete each.

3. **Pre-step grep protocol** before Step 4.3 (replacement at `mod.rs`):
   ```bash
   grep -nE "PR-7-WIRES-REAL-AUDITOR|fn resolve_audit_verdict|pub\(super\) fn now_iso|pub async fn call_agent" \
       /Users/antony/Development/Atlas/crates/atlas-agents/src/runtime/mod.rs
   ```
   The plan's line numbers (`:665`, `:700`) have shifted with PR-2/PR-3 work. As of the most recent main commit before your dispatch the comment was at `:746`, `resolve_audit_verdict` at `:973`, `now_iso` at `:1502`, `call_agent` at `:590`. Re-pin from the grep; if these have drifted further, surface in your PR-4 note for PR-5's reference.

4. **Pre-step grep protocol** before Step 4.3 closure construction:
   ```bash
   grep -nE "type AuditClosure|Option<Arc<AuditClosure>>" /Users/antony/Development/Atlas/crates/atlas-agents/src/runtime/mod.rs
   ```
   The closure signature must match Phase 7 PR-5's existing wiring at `call_agent`. If `AuditClosure` is defined with a different signature than the brainstorm §7.5 sketch (e.g., a sync vs. async return type), adapt — **the brainstorm is design intent, not a literal copy template**.

5. **Pre-step grep protocol** before Step 4.2 (verdict atomic-write):
   ```bash
   grep -nE "pub fn atomic_write_pair|pub fn atomic_write" \
       /Users/antony/Development/Atlas/crates/atlas-engine/src/atomic_write.rs
   ```
   Re-use `atomic_write_pair` verbatim; do not hand-roll pair-write semantics. Phase 7 PR-2 owns this primitive.

6. **`audit_dir` cascade**: after adding `audit_dir: PathBuf` to `AgentRuntime`, run:
   ```bash
   grep -rnE "AgentRuntime\s*\{|AgentRuntime::new|fn build_test_runtime|fn build_agent_runtime" \
       /Users/antony/Development/Atlas/crates/atlas-agents/ /Users/antony/Development/Atlas/crates/atlas-cli/
   ```
   Expect seven construction sites (mirror PR-A note item 5's `mcp_server: None` cascade). Tests pass `tempfile::TempDir`'s path; production pipeline passes `<workspace_root>/.atlas/audit`.

7. **Verify after each non-trivial step:**
   - After Step 4.1 (audit_prompt.rs): `cargo build -p atlas-agents` clean; `cargo test -p atlas-agents --lib audit::audit_prompt` clean.
   - After Step 4.2 (verdict.rs): `cargo build -p atlas-agents` clean; `cargo test -p atlas-agents --lib audit::verdict` clean.
   - After Step 4.3 (stub replacement + `audit_dir` cascade): `cargo build -p atlas-agents` clean; `cargo build -p atlas-cli` clean; `cargo test -p atlas-agents` clean.
   - After Step 4.4 (revision-prompt path): `cargo test -p atlas-agents --test audit_revision_round_trip` clean.
   - After Step 4.5 (prompt-shape test): `cargo test -p atlas-agents --test audit_prompt_shape` clean.
   - After Step 4.6 (revision-round-trip test): see Step 4.4 above.
   - After Step 4.7 (atomic-write test): `cargo test -p atlas-agents --test audit_verdict_atomic_write` clean.
   - After Step 4.8 (cross-provider routing strengthening): `cargo test -p atlas-agents --test cross_provider_audit_routing` clean.

8. **Step 4.9 is the cumulative-regression gate** — run all six:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count in `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`** (memory `feedback_no_tail_pipe_for_long_tests`). **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run** (memory `feedback_atlas_test_subprocess_concurrency`). Use `--skip polyglot_phase3` substring (memory `feedback_cargo_skip_polyglot_pattern`).

   PR-4's surface is in the runtime / audit subtree + the new audit_dir cascade; the polyglot fixture's full override coverage keeps it unreachable from PR-4's edit surface, so the smoke remains unaffected by construction.

9. **Step 4.10 is two commits + status flip** (the sprint's canonical two-commit pattern):
   - **Commit 1:** `audit_prompt.rs` + `verdict.rs` + `mod.rs:746` stub replacement + revision-addendum builder + `resolve_audit_verdict` recursive-call extension + `audit_dir` field + seven construction-site updates + the four new tests + the strengthened `cross_provider_audit_routing.rs`. Message: `sprint: PR-4 cross-provider auditor + audit prompt + on-disk verdict`.
   - **Commit 2:** status flip (PR-4 row `[ ]` → `[x]`; "Last updated" header refresh; PR-4 per-PR note with commit SHAs + deviations + forward-pointers to PR-5). Message: `sprint: PR-4 status flip + per-PR note`.

   If commit 1 grows past 700 LOC and the gates are still clean, consider splitting into two code-commits (e.g., **C1a:** `audit_prompt.rs` + `verdict.rs` + their unit tests; **C1b:** stub replacement + revision path + `audit_dir` cascade + integration tests). The status flip stays as a third commit. Match PR-A's three-commit precedent (verification note + code + flip) if the split is structurally clean. Commit message convention: `sprint: PR-4 <short title>` — never `phase7: PR-4` (collides forensically).

10. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your final implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 4 acceptance gate (plan §5 row PR-4). Particularly:
   - Is the `PR-7-WIRES-REAL-AUDITOR` stub gone from `mod.rs`?
   - Does the audit prompt template embed the verdict rubric for all three kinds (`accept`, `request_revision`, `hard_fail`)?
   - Does the on-disk verdict YAML shape contain every brainstorm-§7.4 key verbatim (`agent_id`, `stage`, `producer.{provider,model,output_sha}`, `auditor.{provider,model,verdict,reason}`, `audit_tokens.{in,out}`, `audited_at`)?
   - Does the revision-prompt path actually re-invoke the producer with the auditor's reason in the system-prompt addendum, and does the cumulative-budget rule escalate to `HardFail` after the cap?
   - Does `cross_provider_audit_routing.rs` exercise the **real** audit code path (auditor.call_count() = 1 on a Weak-graded producer), not the stub-returns-Accept short-circuit?

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues (correctness, security, broken invariants). Specific concerns for PR-4:
   - Does the verdict cache replay path correctly compare `producer.output_sha` (so a changed producer output re-audits, not replay-stale)?
   - Does the half-pair detector treat both orderings (verdict-without-transcript, transcript-without-verdict) as `Ok(None)` → re-audit?
   - Does the auditor closure handle the `AuditDegraded` fallback without panicking when `for_provider(producer_provider)` returns None (defensive: PR-1's invariant says producer's backend always exists, but the `.expect` should panic loudly with a useful message)?
   - Does the strict-string adapter actually apply to every identity-shaped string field, including via the byte-budgeted truncation hint format (no Norway-problem-like coercion of `"NO"` / `"yes"` / `"on"` to bool in the `reason` text)?
   - Does the recursive `call_agent` for revision correctly increment `lane_a_retries` (or its sibling counter) so the cap fires on the next round?
   - Does `Provider::cross()` round-trip cleanly (Anthropic→OpenAi→Anthropic) — already unit-tested by PR-1 (`provider_cross_returns_opposite_vendor`); PR-4's call-site should not re-test the algebra but should re-test the routing.
   - Are the byte-budgeted truncators (`summarise_args` 200; `summarise_result` 400) honouring the budget exactly (no off-by-one; truncation hint format-stable for the auditor's interpretation)?

   HIGHs fixed before status flip; MEDIUMs recorded in PR-4's per-PR note for later sweeps.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**. Don't ship broken code to flip the checkbox.

## Coordination with PR-5 (downstream, sequential)

PR-5 is **sequential after PR-4** — it consumes PR-4's emitted events (`AgentEvent::AuditFire`, `AgentEvent::AuditVerdict`) plus the on-disk verdicts at `.atlas/audit/<stage>/<target>.yaml` as data sources for Atlas-on-Atlas calibration. PR-4 should:

- Emit `AuditFire { agent_id, stage }` at the *start* of every audit invocation (whether cache-replayed or fresh; if cache-replayed, the variant or a sibling event should distinguish — surface to user if you find a cleaner shape than the brainstorm sketch).
- Emit `AuditVerdict { agent_id, verdict, tokens_in, tokens_out }` at the *end* of every fresh audit (replays do not re-emit AuditVerdict; PR-5 reads the on-disk file for cache-hit metrics).
- Ensure `audit_tokens.in` / `audit_tokens.out` are populated from `auditor_backend.call_async(...)` response metadata, not estimated. PR-5's "cold token total (auditor-OpenAI)" metric depends on these numbers being authoritative.
- Append PR-4's per-PR note with a "PR-5 data sources" subsection documenting where the auditor-side cold-token totals live (event-stream vs. on-disk verdicts) and whether there's any pre-computation PR-5 must perform.

## Coordination with PR-A / PR-B (already landed)

PR-A landed the `rmcp` migration + `serve_client.rs` subprocess driver; PR-B landed the `--disallowedTools` probe. **Both are upstream of PR-4 in the dependency graph but do not directly intersect PR-4's edit surface.** The subprocess-transport branch in `runtime/mod.rs::run_tool_loop_with_lane_a` (PR-A note item 5) still passes `mcp_server: None` from all seven construction sites; **PR-4 must do the same for its new `audit_dir` field** (every construction site passes a path; tests pass `TempDir::path().to_path_buf()`).

If PR-4's stub replacement closure intersects PR-A's edit surface in `mod.rs` (likely — both touch the auditor closure construction site), the disjoint-files claim that held PR-3 + PR-B does not hold here. PR-A is already merged on main, so a rebase against current main is the standard recovery; if `git rebase` surfaces semantic conflicts beyond mechanical line-shuffling, **stop and surface**.

## Scope-creep guard

PR-4's budget is 400–700 LOC; brainstorm §12 risk #1 "stop-and-surface at 2×" applies (stop at **1400 LOC**). Indicators you may be heading there:

- **The `audit_dir` cascade balloons beyond seven sites.** If your grep finds more `AgentRuntime` construction sites than PR-A's seven, surface — `audit_dir` semantics may be wrong (e.g., maybe it belongs on `AgentRequest` not `AgentRuntime`).
- **The `resolve_audit_verdict` recursive-call refactor is structurally larger than expected.** If the recursion requires significant signature changes (e.g., async-ifying a sync helper, threading new mutable counters through multiple layers), surface with a "PR-4a: real audit closure without revision; PR-4b: revision path" split proposal.
- **The audit prompt's "producer_output_rendered" requires plumbing the predecessor-stage rollups** (PR-3 note item 8). If you find the auditor genuinely needs reduce's predecessor classify rollup or project's predecessor reduce rollup to render a useful audit prompt, surface — this is a brainstorm §7.1 amendment, not a quiet implementation choice.
- **The cross-provider routing test needs fundamentally new test infrastructure.** Phase 7 PR-5's `cross_provider_audit_routing.rs` already exists; if you find it must be rewritten rather than extended, surface.

Any of those: pause, surface to the user with a "PR-4 is bigger than scoped because X; here's the split proposal" note.

## Begin at Step 4.1

Begin at **Step 4.1: Author the audit prompt template at `crates/atlas-agents/src/runtime/audit/audit_prompt.rs`** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 4.

Open the plan, locate the step, run the pre-step grep at `mod.rs` to re-pin the auditor-stub line + nearby anchors, run the `atomic_write_pair` grep to confirm the helper's signature is unchanged, and proceed.
