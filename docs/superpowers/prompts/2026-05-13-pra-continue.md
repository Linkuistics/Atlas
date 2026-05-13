# PR-A continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-A** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. PR-A is the **parallel track** — independent of PR-2 / PR-3 / PR-4 / PR-5, unblocked once PR-1 ships (which it has, commit `a064f63`). This prompt is self-contained.

## What PR-A ships

PR-A is **parallel, medium-sized**. It migrates PR-1's hand-rolled MCP JSON-RPC framing at `crates/atlas-agents/src/mcp/{mod.rs, server.rs, descriptors.rs}` to the `rmcp` crate (Rust MCP SDK), and ships the subprocess MCP `serve_client` driver so `atlas index --agent-runtime` works against the canonical `claude_code + codex` config.

**PR-A is gated by a four-criterion maturity verification on `rmcp`.** Step A.1 runs the verification and authors a verification note as the **first commit** of PR-A. If `rmcp` fails any of the four criteria, PR-A pivots to `jsonrpsee` + a thin Atlas-specific MCP shim. Both paths are pre-specified in the plan; the prompt below leaves both live.

Deliverables (assuming `rmcp` passes — see Step A.1 for the fallback branch):

1. **`crates/atlas-agents/src/mcp/rmcp_verification.md`** — the maturity-verification note (cadence + activity + multi-client support + dep footprint as-of the verification date). First commit of PR-A. Documents the PASS/FAIL decision.
2. **Workspace + crate `Cargo.toml`** additions for `rmcp` (or `jsonrpsee` on fallback). Pinned version per memory `feedback_no_version_on_workspace_path_deps` (this is an external crate, so it DOES carry a version).
3. **`crates/atlas-agents/src/mcp/mod.rs`** — hand-rolled JSON-RPC framing types replaced with `rmcp`'s equivalents (or shimmed if `rmcp`'s API differs significantly).
4. **`crates/atlas-agents/src/mcp/server.rs`** — multi-client multiplexing reimplemented on top of `rmcp`'s server abstractions. The `Arc<McpServer>` + per-client `serve_client`-per-task pattern carries forward; framing delegates to `rmcp`.
5. **`crates/atlas-agents/src/mcp/descriptors.rs`** — `Tool::json_schema()` → MCP tool-descriptor conversion adapts to `rmcp`'s descriptor type.
6. **`crates/atlas-agents/src/mcp/serve_client.rs`** — per-subprocess driver. Spawns `claude-code` or `codex` via `tokio::process::Command`; attaches stdio to the MCP server's per-client `serve_client` task; sends the initial prompt; drains the client transcript on subprocess exit.
7. **`crates/atlas-agents/src/runtime/tool_loop_http.rs`** — replaces the existing "PR-4 runtime does not drive subprocess transports directly" error branch with a call to `serve_client` for `TransportFlavour::ClaudeCode | Codex`.
8. **`crates/atlas-agents/tests/mcp_multiplex.rs`** — *no test logic changes*; this test is the regression detector. Post-migration, it must pass with the same observable multi-client behaviour.
9. **`crates/atlas-agents/tests/mcp_serve_client.rs`** — `serve_client` exercised against a stub subprocess (e.g., `tokio::process::Command::new("cat")` as a no-op subprocess that echoes stdin to stdout); verifies stdio wiring + drain handshake.
10. **`crates/atlas-agents/src/mcp/restrictions.md`** — updated with the codex `--disallowedTools` flag documentation (the live-subprocess probe is PR-B; this PR documents the flag).

## Scope exclusions — PR-A does NOT do these

- **PR-A does NOT touch dispatch prompts** at `dispatch.rs:203, :254`. **That's PR-2.**
- **PR-A does NOT migrate Lane A's deserializer** at `dispatch.rs:306, :327`. **That's PR-2.**
- **PR-A does NOT touch classify/reduce/project prompts.** **That's PR-3.**
- **PR-A does NOT touch the auditor stub** at `mod.rs:665`. **That's PR-4.**
- **PR-A does NOT add the `--disallowedTools` live-subprocess probe.** **That's PR-B.**
- **PR-A does NOT run Atlas-on-Atlas.** **That's PR-5.**

If asked to do anything from this list, **stop and surface** — that's not PR-A scope.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 6 (lines 2724–3045) — your scope. Both `rmcp` and `jsonrpsee` fallback paths are documented. Also read §0–§3 and §2.1's 15 decision rows. PR-A implements row 14 (PR-1 MCP framing → maintained crate) and prepares row 15 (`--disallowedTools` probe, which is PR-B).

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-1's per-PR note documents the hand-rolled MCP framing PR-A replaces. Item 9 mentions the cascade of `Provider` enum hoist files; some of those (`tool_loop_http.rs`) intersect PR-A's surface. **Re-read item 9 before touching `tool_loop_http.rs`** to avoid stepping on PR-1's import-path edits.

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** §12.2 — the rmcp maturity verification gate origin. If plan and brainstorm disagree on the gate criteria, **brainstorm wins** — but spec deviations should be raised before you re-interpret the gate. §10 documents the subprocess `serve_client` driver design.

4. **Sprint framing memories:**
   - `.claude/memory/feedback_prefer_existing_crates.md` — **the load-bearing memory for PR-A**. PR-A's existence is a direct consequence of this framing. The four-criterion gate is the operationalisation: prefer the maintained crate *if it meets the bar*; fall back to `jsonrpsee` + shim if not.
   - `.claude/memory/feedback_no_deterministic_engine_comparison.md` + `.claude/memory/project_atlas_purpose_llm_consumers.md` + `.claude/memory/feedback_atlas_llm_spine_intent.md` + `.claude/memory/feedback_yaml_canonical_interchange.md` — durable framings.
   - `.claude/memory/project_atlas_common_backend_config.md` — **specifically load-bearing for PR-A**. The canonical user backend pairing is `claude_code + codex`; the MCP server must multiplex two concurrent subprocess clients. The multi-client criterion in Step A.1 reflects this.

5. **Operational memories:**
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints. PR-A's `mcp_serve_client.rs` spawns real subprocesses; the subprocess-concurrency memory is particularly relevant if you're tempted to run PR-A's tests in parallel with the polyglot fixture.

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 6** and follow Steps A.1 → A.9 in order. Mark `[x]` as you complete each.

3. **Step A.1 is the gate.** Run the verification commands, fill in the table in `crates/atlas-agents/src/mcp/rmcp_verification.md`, mark PASS or FAIL. Commit as the first commit of PR-A: `sprint: PR-A rmcp maturity verification`. **On FAIL, the rest of PR-A pivots to the `jsonrpsee` + shim path** — re-read plan §4 Task 6 "Files (fallback path)" before continuing.

4. **Verify after each non-trivial step:**
   - After Step A.3 (mcp framing migration): `cargo build -p atlas-agents` clean; `cargo test -p atlas-agents --test mcp_multiplex` passes with no logic changes to the test itself.
   - After Step A.4 (`serve_client.rs`): `cargo test -p atlas-agents --test mcp_serve_client` clean.
   - After Step A.5 (`tool_loop_http.rs` wiring): `cargo build -p atlas-agents` clean.

5. **Step A.8 is the cumulative-regression gate** — run all six:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   Polyglot smoke must hold at cold count `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`**. **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run.** Use `--skip polyglot_phase3` substring.

6. **Step A.9 is the multi-commit pattern.** PR-A is unusual — Step A.1 is its own commit (verification note), then code commits in logical chunks (framing migration; serve_client + tests; tool_loop_http wiring + restrictions doc), then the status-flip commit. Match the per-step commit-message HEREDOCs in plan §4 Task 6.

7. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your final implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 6 acceptance gate. Particularly: did `mcp_multiplex.rs` pass with no logic changes? Was the four-criterion verification documented? Did the right fallback path activate if `rmcp` failed?

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues. Specific concerns for PR-A: subprocess lifetime management in `serve_client.rs` (orphaned tokio tasks on subprocess exit); stdio drain handshake races; transitive-dep footprint expansion beyond the verification snapshot. HIGHs fixed before status flip; MEDIUMs recorded in PR-A per-PR note.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**.

## Coordination with PR-2

PR-2 (production dispatch prompts + Lane A YAML migration) is **parallel-safe** with PR-A and may be running in another session. File sets are disjoint:
- **PR-A:** `crates/atlas-agents/src/mcp/*` + `crates/atlas-agents/src/runtime/tool_loop_http.rs` (subprocess-branch wiring only) + `crates/atlas-agents/tests/mcp_*.rs`.
- **PR-2:** `crates/atlas-agents/src/runtime/{dispatch.rs, prompt_examples.rs, yaml_strict.rs, audit/}` + `crates/atlas-agents/tests/{dispatch_*, lane_a_*, yaml_envelope_*}.rs`.

The one shared edit surface is `crates/atlas-agents/src/runtime/mod.rs` (PR-2 adds `pub mod prompt_examples; pub mod yaml_strict;` declarations; PR-A doesn't touch mod-level declarations). If a rebase conflict surfaces beyond `mod.rs`, **stop and surface** — the disjoint-files claim was load-bearing.

## Begin at Step A.1

Begin at **Step A.1: Verify `rmcp` against the four maturity criteria + commit the note** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 6.

Open the plan, locate the step, and proceed.
