# PR-5 continuation prompt — Atlas vNext production-prompt sprint (sprint closeout)

You are executing **PR-5** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. This prompt is self-contained — read this, then read the canon in the order below, then drive the implementation via `superpowers:executing-plans`.

PR-5 is **Wave 5 — the final PR**. When PR-5 ships, the sprint is complete and Phase 8 (Cargo retirement) unblocks.

## What PR-5 ships

PR-5 is **measurement-heavy, small code surface** — Wave 5 of the sprint dependency graph. It runs the full agent runtime against Atlas's own workspace, records intrinsic baseline metrics, runs a within-LLM-spine cross-transport parity check (decision row 1 framing — *not* deterministic-engine-vs-runtime parity), and closes out the sprint. Bulk of PR-5 is **measurement + analysis**, not code.

Deliverables:

1. **`crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs`** — new test, gated `#[ignore = "requires ANTHROPIC_API_KEY and OPENAI_API_KEY"]`. Runs a synthetic workspace through `(http_anthropic primary + http_openai auditor)` and `(http_openai primary + http_anthropic auditor)`. Asserts: same component-id set, same subsystem-id set, same edge multiset `(from, to, kind) -> count`. Structural disagreements are **signal worth investigating**, not failure — document any tolerated refinements in the closeout note.

2. **Atlas-on-Atlas calibration run** — run `atlas-cli index --workspace-root . --agent-runtime --config .atlas/config.sprint.yaml --log-events /tmp/atlas-on-atlas-events.jsonl` against the Atlas repo (no `subsystems.overrides.yaml` present → dispatch agent fires). Wall-time, token usage, convergence behaviour all recorded via the JSON-Lines event subscriber (PR-7-era mechanism, still active).

3. **Intrinsic-metrics extraction from `/tmp/atlas-on-atlas-events.jsonl`** — the closeout note records every row from plan §4 Task 5 Step 5.4's table. Cold token totals must be **summed from the on-disk verdict files** at `<output_dir>/audit/<stage>/<target>.yaml`'s `audit_tokens.{in,out}` (PR-4 note item 6 — `AuditVerdict` event payload doesn't carry token counts; the on-disk record is authoritative). Lane A vs Lane B retries are bucketed separately (PR-4 note item 2 — `lane_a_retries` + `lane_b_revisions` on `AgentRequest`).

4. **Status-file closeout** — append PR-5 per-PR note to `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` with every recorded metric. Append a **"Sprint — complete"** closeout section following the Phase 7 status precedent (lines 447+ of `docs/superpowers/plans/2026-05-12-phase7-status.md` are the template): SHIPPED date, commit lineage per PR (PR-1 through PR-5 + PR-A + PR-B), polyglot smoke cumulative regression guard summary, Atlas-on-Atlas baseline section, cross-transport parity outcome, sprint → Phase 8 handoff section. Flip PR-5's row from `[ ]` to `[x]`; refresh the "Last updated" header.

5. **Memory updates** — `.claude/memory/project_phase4_plus_roadmap.md` gets a "Sprint — SHIPPED 2026-05-NN" entry under Phase 7 (the sprint is logically Phase-7-completion work); mark Phase 8 (Cargo retirement) unblocked. Refresh `.claude/memory/MEMORY.md`'s roadmap-memory hook line *only* if the description text drifts. **No other memory writes** — the five framing memories (`feedback_no_deterministic_engine_comparison`, `project_atlas_purpose_llm_consumers`, `feedback_prefer_existing_crates`, `feedback_yaml_canonical_interchange`, `feedback_cross_provider_llm_audit`) are durable framings authored at the brainstorm; the sprint operated *within* them and does not need to amend them.

6. **Phase 7 status backfill** — `docs/superpowers/plans/2026-05-12-phase7-status.md` at line 462 currently reads `**Atlas-on-Atlas cold token total baseline:** DEFERRED (Step 7.4). The Atlas-on-Atlas baseline number is the regression detector for future Phase 7+ changes; it is RECORDED IN A FOLLOW-UP SPRINT once production prompt templates ship.` Backfill this line with the actual recorded baseline (cold tokens producer + auditor; or, if Atlas-on-Atlas hard-fails, with the specific diagnostic — both are valid signal per brainstorm §8.5 acceptance).

7. **(Optional)** `crates/atlas-agents/tests/common/atlas_on_atlas_harness.rs` — only if the cross-provider parity test plus any future Atlas-on-Atlas test genuinely shares non-trivial setup. The plan flags this as optional; don't extract a one-test helper.

8. **(Forensic-only, plan §4 Task 5 file-list item 5)** — `crates/atlas-cli/tests/phase3_polyglot_fixture.rs::polyglot_smoke_cross_transport_parity_claude_code_vs_codex` is the *polyglot-side* cross-transport parity test (subprocess label parity). Only edit if it needs a comment update to clarify its forensic-not-load-bearing status post-sprint. The within-LLM-spine parity check in deliverable 1 above is the load-bearing one going forward.

LOC budget: **150–300** (mostly the cross-provider-parity test + closeout-note text + memory updates). Bulk is measurement + analysis. If approaching **600 LOC** (2× budget), **stop and surface** — brainstorm §12 risk framing applies to any PR in this sprint.

## Scope exclusions — PR-5 does NOT do these

- **PR-5 does NOT alter any production code beyond the new test.** The new cross-provider-parity test is the only Rust file PR-5 lands. If the calibration run surfaces a bug, **stop and surface** — it's a finding to record, not a fix to ship inside PR-5 (post-sprint Phase 8 work picks it up).
- **PR-5 does NOT change any prompt template.** PR-2/PR-3/PR-4 own those. Prompt-quality signals are *recorded as metrics*, not silently patched.
- **PR-5 does NOT assert thresholds.** The brainstorm §8.2 framing locks: "observed and asserted in tests, never enforced as a runtime cap." PR-5 records the empirical baseline that future PRs assert against — it does not itself assert.
- **PR-5 does NOT alter the on-disk verdict shape.** PR-4 locked `.atlas/audit/<stage>/<target>.yaml`; PR-5 *reads* this file for token totals (PR-4 note item 6 — authoritative source). If the shape needs change, that's a post-sprint follow-up.
- **PR-5 does NOT brainstorm Phase 8.** Phase 8 (Cargo retirement) unblocks *because* PR-5 ships; brainstorming Phase 8 happens in a follow-on session via `superpowers:brainstorming`. Memory `project_phase4_plus_roadmap.md` records the unblock event; Phase 8 design is not in scope here.
- **PR-5 does NOT touch the `#[ignore]`'d HTTP smoke test** (PR-4 note item 9). The proposed fix paths (wiremock-rs vs. lifting `BackendRouter::from_dispatch_table` out of `#[cfg(test)]`) are explicit *PR-5 follow-up* per the PR-4 note, but **only if** Atlas-on-Atlas calibration depends on them. The cross-provider parity test uses real HTTP backends (gated on the two env vars) and does not need that smoke; surface if you find the smoke is blocking calibration.
- **PR-5 does NOT add new workspace dependencies.** A small extraction helper (Python or `jq`-based bash) for events-jsonl analysis is fine but **do not commit it unless it's clean and useful for re-runs** (plan §4 Task 5 Step 5.4 says explicitly: "Do *not* commit the helper script unless it's clean and useful for re-runs").
- **PR-5 does NOT touch `crates/atlas-cli/tests/phase3_polyglot_fixture.rs` beyond a possible comment update.** That test's parity claim (subprocess labels) is decoupled from the within-LLM-spine parity PR-5 introduces.

If asked to do anything from this list, **stop and surface** — that's not PR-5 scope.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 5 (lines 2536–2722) — your scope. Steps 5.1 → 5.9 are the implementation steps. Also skim §0–§3 (reading order, deliverable restated, non-negotiables, dependency graph) and §2.1's 15 decision rows. PR-5 implements decision row 1 (within-LLM-spine cross-transport parity, **not** deterministic-engine parity — memory `feedback_no_deterministic_engine_comparison`) and decision row 11 (Atlas-on-Atlas baseline numbers — informational, never enforced as runtime caps; recast §2.4 / §8.4) in their entirety. Row 7 (Opus + GPT-5-Codex pairing) carries through from PR-1 — the calibration baseline uses this pair.

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-1's, PR-2's, PR-3's, PR-4's, PR-A's, PR-B's per-PR notes carry forward-pointers PR-5 must honour. **Read PR-4's note in full** (lines 202–240) — items 1, 2, 5, 6, 7, 9, 12 are load-bearing for the calibration:

   - **(item 1) Auditor's `auditor.provider` field is authoritative** (anthropic / openai). The `Degraded` wrapper on the `AuditVerdict` enum is the in-memory signal for "same-model fallback fired" — orthogonal to the on-disk provider field. PR-5's per-provider token-total breakdown reads `auditor.provider` from the on-disk verdict.
   - **(item 2) Cumulative-retries cap policy = `lane_a_retries + lane_b_revisions >= 1 → HardFail`.** Both contribute to the cap but are semantically distinct. **Bucket them separately in the retry-count metric** — Lane A retries vs Lane B revisions are different prompt-quality signals.
   - **(item 5) Cache replay path: when producer output_sha changes, fresh audit fires.** Auditor-call-count metric must account for cache replays — a producer emitting identical output across iterations sees auditor.call_count drop to 1, subsequent calls served from cache. **This is correct behavior, not a bug.** The PR-5 metric should split "fresh audits" from "cache replays" using `audited_at` timestamps vs. run start time.
   - **(item 6) Token extraction tries Anthropic shape first (`usage.input_tokens` / `usage.output_tokens`), then OpenAI shape (`usage.prompt_tokens` / `usage.completion_tokens`), falls back to `(0, 0)`.** Same-process test backends contribute (0, 0) — correct since they're not "cold tokens." **PR-5 sums tokens from on-disk verdicts**, not from `AuditVerdict` event payloads.
   - **(item 7) `AuditFire` fires before closure runs (cache + fresh both emit); `AuditVerdict` fires after closure resolves (fresh only, with wire-form label `"accept"` / `"degraded:accept"` / `"request_revision"` / `"hard_fail"`).** PR-5's verdict-distribution metric reads both — `AuditFire` gives total invocations including cache hits; `AuditVerdict` gives fresh-audit distribution.
   - **(item 9) HTTP smoke `agent_runtime_http_smoke_completes_with_config_loaded_from_env` is `#[ignore]`'d** with a PR-5 follow-up note. The proposed fix paths are documented (wiremock-rs or lifting `BackendRouter::from_dispatch_table`); leave them for post-sprint Phase 8 follow-up *unless* Atlas-on-Atlas calibration is blocked by them.
   - **(item 12) `AuditorEmittedVerdict` (LLM emission) is distinct from `AuditVerdictOnDisk` (full record with producer/auditor metadata).** For the verdict-distribution metric, consume `AuditVerdictOnDisk.auditor.verdict` from the on-disk records — same value as `AuditorEmittedVerdict::verdict`, but the on-disk path includes cache replays consistently.

   Forward-pointers from prior PRs (lighter weight for PR-5):
   - **PR-3 note item 8: `run_iteration` rollups are local.** The cross-provider parity test runs the full pipeline end-to-end, so this doesn't affect PR-5 directly — but if you write any helper that re-walks the pipeline, it inherits the local-rollup constraint.
   - **PR-3 note item 10: CLI-layer composition pattern for on-disk artifacts.** PR-4 followed this for `audit_dir`. PR-5's cross-provider parity test will call `run_workspace_via_agent_runtime` or equivalent — that helper materialises an `AgentRuntime` and needs the same `output_dir`-derived `audit_dir`. Tests pass `tempfile::TempDir`.
   - **PR-2 note items 4 + 9: `parse_final_output` is YAML-fence-first.** The cross-provider parity test reads the full `CanonicalArtifactSet` (PR-3 shim output), not raw LLM responses, so YAML-fence handling is transparent at this level.
   - **PR-1 note items 4 + 5: `Provider::cross()` + `provider_from_config_key`.** PR-5's calibration runs the canonical Anthropic+OpenAI pair; cross-provider audit routing is already locked by PR-1 and exercised by PR-4. PR-5 reads the outcome.

   After your work lands, append PR-5's per-PR note with: recorded baseline metrics; cross-provider parity outcome (pass / structural-disagreement-with-diff); any deviations from plan §4 Task 5; the sprint-closeout section (per Phase 7 status §"Phase 7 — complete" precedent, lines 447+).

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** — read §8 in full (lines 865–936; §8.1 calibration invocation, §8.2 intrinsic metrics, §8.3 cross-transport parity, §8.4 memory + status updates, §8.5 acceptance). Also relevant:
   - **§2.1 framing 1** — within-LLM-spine cross-provider parity (the parity that matters); NOT deterministic-engine-vs-runtime parity. The decision row table at line 130-160 also restates this.
   - **§2.2 decision rows 1 and 11** — locked.
   - **§12.4 (line 1177) "Recast spec §11.2 reference-output comparison harness conflict"** — restated: PR-5's cross-provider parity is **not** that harness. Verdicts and parity are intrinsic semantic-soundness signals, not reference-comparison signals. Memory `feedback_no_deterministic_engine_comparison` reinforces.
   - **§12.5 (line 1195) `L9Projection` shape may lack canonical-schema fields** — shim hard-fails are **prompt-correctness signals** (§2.1 framing 2 — "they're a feature"). If Atlas-on-Atlas hard-fails with `ShimError::MissingProjectionField`, *record the diagnostic in the closeout note* and proceed — that's signal, not failure to ship PR-5.
   - **§12.6 (line 1209) Per-stage iteration cap × concurrency math** — the HTTP transport semaphore (default 8) is the real backstop. Atlas-on-Atlas surfaces peak; the recorded metric tells future tightening efforts.

   **If plan and brainstorm disagree, brainstorm wins.**

4. **Sprint framing memories** (durable; condition every decision):
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` — **the load-bearing framing for PR-5.** The within-LLM-spine parity check is the *replacement* for the deterministic-engine comparison; if you find yourself writing "compare LLM output to the deterministic engine's output," you've drifted.
   - `.claude/memory/feedback_cross_provider_llm_audit.md` — PR-5's calibration runs the canonical Anthropic + OpenAI pairing. Same-model audit is tautological.
   - `.claude/memory/feedback_yaml_canonical_interchange.md` — closeout note and metrics tables use YAML where structured; no JSON sneaks in unless it's the event-stream JSON-Lines format (PR-7 era; preserved).
   - `.claude/memory/project_atlas_purpose_llm_consumers.md` — the on-disk verdicts at `<output_dir>/audit/<stage>/<target>.yaml` are LLM-consumed downstream. PR-5 reads them; doesn't reshape them.
   - `.claude/memory/feedback_prefer_existing_crates.md` — re-use PR-7-era event-subscriber, PR-4's `read_verdict_if_complete`, PR-3's `CanonicalArtifactSet`. Do not hand-roll a YAML parser for the verdict files.
   - `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent. The sprint-closeout note is the durable artefact this strategic framing produces.

5. **Operational memories:**
   - `.claude/memory/project_phase7_agent_runtime_default_ratified.md` — HTTP backends are the sprint's live path; `--agent-runtime` opt-in default; calibration uses HTTP.
   - `.claude/memory/project_atlas_common_backend_config.md` — the canonical config pairs `claude_code + codex` (subprocess) for general use; PR-5 calibration uses HTTP backends per memory `project_phase7_agent_runtime_default_ratified`.
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints. Step 5.6's cargo-gates run honours all four.
   - `.claude/memory/feedback_worktree_base_verification.md` — verify any subagent worktree base matches current main.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 5** and follow Steps 5.1 → 5.9 in order. Mark `[x]` as you complete each.

3. **Step 5.1 preflight grep protocol:**
   ```bash
   git log --oneline -10                    # PR-1..PR-4 SHAs present
   ls .atlas/config.sprint.example.yaml     # PR-1 deliverable
   test -f crates/atlas-agents/src/runtime/projection_to_canonical.rs && echo OK  # PR-3
   grep -rnE "PR-7-WIRES-REAL" crates/atlas-agents/src/  # zero hits expected
   ls crates/atlas-agents/src/runtime/audit/  # PR-4: audit_prompt.rs + verdict.rs present
   ```
   As of the current main commit (PR-4 merged via `3bf1aab` on 2026-05-14), all five preflight checks pass. Re-verify; if any check fails, **stop and surface**.

4. **Step 5.2 (cross-provider parity test) — implementation notes**:
   - The plan's test sketch (lines 2563-2620) names helper functions `build_synthetic_workspace_with_three_subsystems()` and `run_workspace_via_agent_runtime(workspace_root, primary_flavour, auditor_flavour)`. **These don't exist yet** — author them inside the test file (or in `crates/atlas-agents/tests/common/atlas_on_atlas_harness.rs` if you also need them elsewhere, per plan §4 Task 5 file-list item "Optional create"). Don't extract a single-use harness module.
   - The synthetic workspace should be small (3 subsystems, 5-10 components, single-language Rust to keep classify deterministic) and assembled in a `tempfile::TempDir`. Goal: exercise dispatch + classify + reduce + project on a non-trivial-but-bounded input; don't over-engineer the fixture.
   - The test result type should be `Result<CanonicalArtifactSet, AgentError>` (or the equivalent at the head of the pipeline). Use the existing `run_workspace` entry point if it threads through cleanly; otherwise, plumb the necessary args.
   - The three equivalence assertions (component set, subsystem set, edge multiset) are exactly per the plan. Structural disagreements should produce *informative* diff output (use `pretty_assertions` if it's already a workspace dev-dep, otherwise plain `assert_eq` is fine).
   - **The test is `#[ignore]`-gated** — `cargo test` doesn't run it; only `cargo test -- --ignored` with the two env vars set. CI does not exercise it; PR-5's measurement step does.

5. **Step 5.3 (Atlas-on-Atlas calibration) — operational hazards:**
   - **Wall-time budget**: a full Atlas-on-Atlas run with Opus 4.7 + GPT-5-Codex on the Atlas workspace is likely 10-60 minutes depending on iteration count. Run in `run_in_background: true` mode (the Bash tool's option) and stream the JSON-Lines events via tail to monitor progress.
   - **Token cost**: brainstorm §12.3 flags Opus 4.7 token cost during prompt iteration; the production-prompt sprint locked the producer-Anthropic + auditor-OpenAI pair for the baseline run. **Do not iterate prompts during PR-5** — the baseline is recorded against the as-shipped prompts. If a prompt obviously needs fixing, surface as a finding; don't quietly patch.
   - **API key surfacing**: ensure `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` are in env (or sourced from the user's `.envrc` / `.zshenv` / equivalent). The CLI reads them via `provider_from_config_key`'s env-substitution path (PR-1).
   - **Output dir**: `--output-dir` defaults; verdicts land at `<output_dir>/audit/<stage>/<target>.yaml` per PR-4. Note the path for Step 5.4's token extraction.
   - **No overrides.yaml**: confirm `ls .atlas/` does not show `subsystems.overrides.yaml` or `components.overrides.yaml` — their presence would short-circuit the dispatch agent (defeating the calibration). The Atlas repo as of main does not have these.

6. **Step 5.4 (intrinsic metrics) — extraction approach:**
   - Cold token totals: read **every `.atlas/audit/<stage>/<target>.yaml` file** under the calibration run's output_dir; sum `audit_tokens.in` + `audit_tokens.out` per `auditor.provider`. Producer-Anthropic token total comes from event-stream `AgentComplete { provider: Anthropic, tokens_in, tokens_out }` (NOT from the on-disk verdict, which is the auditor's token usage).
   - Iteration count + wall time: read from the JSON-Lines event stream (`IterationBoundary` count, `RuntimeComplete` timestamp − first `AgentStart` timestamp).
   - Evidence-score distribution: per-stage `AgentComplete.evidence_score` if emitted (PR-2/PR-3 may or may not include it on every event); fall back to parsing transcript-cache `.transcript` files which carry the grade-evidence pair (PR-2-era).
   - Lane A vs Lane B retry counts: read `lane_a_retries` and `lane_b_revisions` from event payloads or the transcript-cache files. Bucket separately per PR-4 note item 2.
   - Audit verdict distribution: bucket `AuditVerdict` events by wire-form label (`"accept"` / `"degraded:accept"` / `"request_revision"` / `"hard_fail"`). For cache-vs-fresh disambiguation, cross-reference with `audited_at` field on the on-disk verdict (older than run start = replay).
   - Shim missing-field count: surface via CLI stderr error or the canonical-schema shim's own emitted-error path (PR-3). If it never fires, record `0`.
   - **A small `jq` / Python helper for the extraction is fine** but per plan §4 Task 5 Step 5.4: "Do *not* commit the helper script unless it's clean and useful for re-runs." Default to hand-analysis for a single run.

7. **Step 5.5 (cross-provider parity test):**
   ```bash
   ANTHROPIC_API_KEY=... OPENAI_API_KEY=... \
       cargo test -p atlas-agents --test agent_runtime_cross_provider_parity \
           --release --no-fail-fast -- --ignored
   ```
   This is another long-running, expensive call. Run in background; record pass/fail and any structural-disagreement diff for the closeout note.

8. **Step 5.6 cargo gates** — the standard six-gate cumulative regression run:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count in `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`** (memory `feedback_no_tail_pipe_for_long_tests`). **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run** (memory `feedback_atlas_test_subprocess_concurrency`). Use `--skip polyglot_phase3` substring (memory `feedback_cargo_skip_polyglot_pattern`).

   PR-5's edit surface is in the test subtree + the docs/memory paths; the polyglot fixture's full override coverage keeps it unreachable from PR-5's edit surface by construction.

9. **Step 5.7 status-file closeout** — append the PR-5 per-PR note in the existing PR-5 placeholder at `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` (line ~245 — the section currently reads `*(Empty — to be filled by PR-5's session...)*`). Then append a **"Sprint — complete"** section at the bottom of the file. Use Phase 7 status file's §"Phase 7 — complete" (line 447+) as the structural precedent: SHIPPED date, commit lineage per PR (PR-0/PR-1/PR-2/PR-3/PR-4/PR-A/PR-B/PR-5 SHAs), polyglot smoke cumulative regression guard summary, Atlas-on-Atlas baseline section, cross-transport parity outcome, sprint → Phase 8 handoff.

   Also: backfill the line in `docs/superpowers/plans/2026-05-12-phase7-status.md:462` (`Atlas-on-Atlas cold token total baseline: DEFERRED`) with the recorded baseline + a pointer to this sprint's closeout for full detail.

10. **Step 5.8 memory** — `.claude/memory/project_phase4_plus_roadmap.md` gets the Phase-7-completion entry; refresh `.claude/memory/MEMORY.md`'s roadmap-memory hook line only if its description text drifts. **No other memory writes** — the durable framings stand.

11. **Step 5.9 commit shape** — plan-time recommendation **shape (a)** (two-commit):
    - **Commit 1:** the new `agent_runtime_cross_provider_parity.rs` test + status-file PR-5 note + status-file "Sprint — complete" section + Phase 7 status backfill + memory updates. Message: `sprint: PR-5 Atlas-on-Atlas calibration + cross-transport parity + closeout`.
    - **Commit 2:** PR-5 row `[ ]` → `[x]`; "Last updated" header refresh. Message: `sprint: PR-5 status flip + sprint SHIPPED`.

    Single-commit (shape (b)) is acceptable only if no Rust code lands (e.g., the optional harness module isn't extracted). Default to shape (a).

12. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your final implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 5 acceptance gate (plan §5 row PR-5). Particularly:
   - Does the cross-provider parity test cover all three equivalence rules (component-id set, subsystem-id set, edge multiset)?
   - Are *all* intrinsic metric rows from Step 5.4's table populated in the closeout note (or marked with the specific reason they're unavailable — e.g., "0 audit verdicts emitted because every producer was Accepted on first try")?
   - Does the closeout note record the cross-provider parity outcome (pass / structural-disagreement-with-diff)?
   - Is Phase 7 status §line 462 backfilled?
   - Are PR-1 through PR-5 + PR-A + PR-B commit SHAs all present in the closeout commit lineage?

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues (correctness, security, broken invariants). Specific concerns for PR-5:
   - The new test's edge-multiset comparison: does it correctly key on `(from, to, kind)` and tolerate multiplicity differences only where the plan explicitly admits "modulo justifiable provider-side refinements"? Or is it a strict multiset equality? The plan's pseudocode is strict equality (`assert_eq!(edges_a, edges_o)`); if you find provider-side refinement is structurally unavoidable, surface it as a deviation in the PR-5 note rather than weakening the assertion.
   - Are env-var probes in the test correctly returning *early-with-message* (per the plan's `eprintln!("skipping: ...")` pattern) rather than panicking? `#[ignore]` already gates execution, but the early-return prevents partial setup work when one key is missing.
   - Token-extraction logic in the closeout: does the code (or `jq` helper, if committed) correctly read on-disk verdicts rather than event-stream payloads? PR-4 note item 6 is load-bearing here.
   - Lane A vs Lane B retry bucketing: does it read both counters? PR-4 note item 2.
   - Cache-vs-fresh audit disambiguation: does it cross-reference `audited_at` with run start? PR-4 note items 5 + 7.

   HIGHs fixed before status flip; MEDIUMs recorded in PR-5's per-PR note for future Phase 8 reference.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**. Don't ship a broken closeout to flip the checkbox.

## Coordination with already-landed PRs

PR-1, PR-2, PR-3, PR-4, PR-A, PR-B have all merged on main. PR-5's edit surface is **the test subtree + the docs/memory paths only**:

- New file: `crates/atlas-agents/tests/agent_runtime_cross_provider_parity.rs`.
- Modified: `docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md` (PR-5 note + "Sprint — complete" + status flip), `docs/superpowers/plans/2026-05-12-phase7-status.md` (line 462 backfill), `.claude/memory/project_phase4_plus_roadmap.md`, `.claude/memory/MEMORY.md` (only if hook text drifts).
- Read-only: the on-disk verdicts at `<output_dir>/audit/<stage>/<target>.yaml`; the event-stream JSON-Lines log at `/tmp/atlas-on-atlas-events.jsonl`; PR-4's `read_verdict_if_complete` helper if you choose to use it programmatically.

No code/runtime surface conflict with earlier PRs by construction. **The cross-provider parity test does call into the production runtime** (PR-1 through PR-4) — that's its purpose. If the runtime fails inside the test in a way that suggests a real bug, surface as a finding; don't fix inside PR-5.

## Scope-creep guard

PR-5's LOC budget is **150–300**; the 2× stop-and-surface threshold is **600 LOC**. Indicators you may be heading there:

- **The synthetic workspace fixture balloons.** Three subsystems, 5-10 components, single-language Rust. If you find yourself adding language variety, polyglot scaffolding, or fine-grained per-component overrides, surface — the fixture is for parity-equivalence, not coverage breadth.
- **The cross-provider parity test grows past ~200 LOC.** If the harness needs the optional `tests/common/atlas_on_atlas_harness.rs` module to fit cleanly, fine — but only if a second test would also use it. One test → in-file helpers.
- **The Atlas-on-Atlas calibration hard-fails and you start patching the runtime.** Hard-fails are *signal*; record them. Patching them is **Phase 8 follow-up work**, not PR-5 scope.
- **The closeout note grows past ~500 lines of markdown.** That suggests over-analysis. The Phase 7 status closeout (lines 447+) is the structural precedent — match its terseness.
- **The Phase 7 status backfill becomes a partial rewrite.** Line 462 is one line; the backfill is one line plus a pointer. If you find yourself rewriting nearby paragraphs, surface — that's drift from the targeted backfill.

Any of those: pause, surface to the user with a "PR-5 is bigger than scoped because X; here's the split proposal" note.

## After PR-5 lands — sprint complete

When PR-5's status-flip commit lands, the sprint is **SHIPPED** and Phase 8 (Cargo retirement) unblocks. Final cleanup pass at sprint close:

- The expired PR-5 continuation prompt (`docs/superpowers/prompts/2026-05-14-pr5-continue.md` — this file) is dropped after the sprint flip, per the precedent at commit `7d6f6f3`.
- Brainstorm Phase 8 in a follow-on session via `superpowers:brainstorming`.
- Sprint docs trio (brainstorm + plan + status) are retained as forensic record of the sprint that gated Phase 8.

## Begin at Step 5.1

Begin at **Step 5.1: Pre-flight — verify PRs 1–4 landed and the sprint config exists** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 5.

Run the five preflight greps, confirm clean, and proceed to Step 5.2.
