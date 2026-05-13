# PR-B continuation prompt — Atlas vNext production-prompt sprint

You are executing **PR-B** of the Atlas vNext production-prompt sprint in a fresh session with no prior context. PR-B is on the **parallel track** — independent of PR-3 / PR-4 / PR-5, unblocked once PR-A shipped (which it has, commit `c07c5d5`). This prompt is self-contained.

## What PR-B ships

PR-B is **small** (LOC budget 100–200; single new test file). It is the live-subprocess regression detector for sprint decision row 15: the assertion that `claude-code --disallowedTools=Read,Grep,Glob,Bash,Write,Edit` (per `mcp/restrictions.md`) actually disables the named built-in tools, end-to-end. Upstream-version sensitivity is localised to one test file so a future upstream change shows up as exactly one failing test.

Deliverables:

1. **`crates/atlas-agents/tests/mcp_disallowed_tools.rs`** — `#[ignore]`-gated `tokio::test` that:
   - spawns a live `claude-code` subprocess via PR-A's `serve_client` driver,
   - sends a probe prompt that explicitly asks the LLM to call the `Read` tool,
   - asserts that the **server-side per-client transcript contains zero `Read` tool calls**.
2. **A sibling codex test** (same shape) iff `mcp/restrictions.md` records a verified codex flag set; if not, leave a `// TODO(PR-B-followup):` stub naming the upstream version that would unblock it.
3. **Optional helper hoist**: if `build_test_mcp_server_with_default_tools()` and `write_temp_mcp_config()` live only in `tests/mcp_serve_client.rs` (PR-A) and aren't visible from `mcp_disallowed_tools.rs`, hoist them to a `crates/atlas-agents/tests/common/mcp_test_helpers.rs` module.

Two valid response shapes from the upstream subprocess satisfy the assertion (the assertion is **on the server-side transcript, not the subprocess exit code**):
- (a) Subprocess succeeds and emits text saying it can't use Read.
- (b) Subprocess fails with an upstream-version-specific error about disabled tools.

Either way, zero `Read` calls in the transcript = pass.

## Scope exclusions — PR-B does NOT do these

- **PR-B does NOT touch the MCP framing.** **That's PR-A.** (Already landed.)
- **PR-B does NOT touch `serve_client.rs`.** PR-B *consumes* PR-A's driver. If `serve_client` lacks a hook PR-B needs (e.g., transcript drain for the per-client recorder), surface to the user — do not extend `serve_client.rs` in PR-B.
- **PR-B does NOT touch production prompts** (classify / reduce / project / dispatch / audit). Those are PR-2 / PR-3 / PR-4.
- **PR-B does NOT run Atlas-on-Atlas.** That's PR-5.
- **PR-B does NOT install or upgrade `claude-code` / `codex`** on the test machine. The `#[ignore]` gate is the contract: when the user runs `cargo test ... -- --ignored`, the user is responsible for the environment.

If asked to do anything from this list, **stop and surface** — that's not PR-B scope.

## Environment prerequisites for the `--ignored` run

Before the `cargo test ... -- --ignored` execution will produce meaningful PASS/FAIL:
- `claude-code` is on `$PATH`. Sanity check: `claude-code --version` returns a version string.
- `ANTHROPIC_API_KEY` is set in the environment.
- (For the codex sibling test, if shipped) `codex` on `$PATH` + `OPENAI_API_KEY` set.

The test itself guards on these via an early-return + `eprintln!` skip message, so absence of any prerequisite means "test is skipped" not "test fails." A failed test means **claude-code upstream actually invoked `Read` despite `--disallowedTools`** — that's a real regression in the unified-envelope invariant.

## Reading order

1. **`docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md`** §4 Task 7 (lines 3045–3165) — your scope. Steps B.1 → B.3 are short. Also skim §0–§3 (reading order, deliverable restated, non-negotiables, dependency graph) and §2.1 row 15 (the `--disallowedTools` probe decision).

2. **`docs/superpowers/plans/2026-05-13-production-prompt-sprint-status.md`** — PR-A's per-PR note has 11 load-bearing call-outs forwarded to PR-B. In particular:
   - Item 5 (`AgentRuntime.mcp_server: Option<Arc<McpServer>>`): PR-B's test builds a real `McpServer` (no `None`) because the probe needs server-side transcript recording.
   - Item 6 (`AgentError::Subprocess*` variants): PR-B may want to pattern-match on these if the subprocess exits non-zero.
   - Item 7 (`serve_client.rs` is structural skeleton; subprocess transport wire-shape pinned by PR-B): **PR-B is the empirical validation that pins down the exact wire shape per upstream.** When PR-B runs against the live binary, **update `mcp/restrictions.md` with the verified upstream version and observed response shape**.
   - Item 8 (`restrictions.md` codex state): PR-B can refine the codex section if the codex sibling test runs.

   After your work lands, append PR-B's per-PR note with commit SHAs + the upstream `claude-code` (and `codex`) versions you tested against + the observed response shape (refusal text vs upstream error).

3. **`docs/superpowers/brainstorms/2026-05-13-atlas-production-prompt-sprint-brainstorm.md`** §10 (subprocess `serve_client` driver design — design context PR-A built on) and §12.7 (upstream-version sensitivity of subprocess restrictions; PR-B is the gate for this risk). If plan and brainstorm disagree, **brainstorm wins**.

4. **Sprint framing memories:**
   - `.claude/memory/project_atlas_common_backend_config.md` — **load-bearing for PR-B**. The canonical user backend pairing is `claude_code + codex`; both halves need disallowed-tool enforcement, hence the codex sibling test (if upstream supports it).
   - `.claude/memory/feedback_prefer_existing_crates.md` — for the `which`-or-equivalent path-detection helper. If `which` isn't already in the workspace, the test can fall back to `Command::new("claude-code").arg("--version").output()` — both are fine; don't pull in a new crate just for the existence check.

5. **Operational memories:**
   - `.claude/memory/feedback_release_workspace_build_for_polyglot.md` + `.claude/memory/feedback_no_tail_pipe_for_long_tests.md` + `.claude/memory/feedback_atlas_test_subprocess_concurrency.md` + `.claude/memory/feedback_cargo_skip_polyglot_pattern.md` — execution-discipline constraints. PR-B's test spawns a real subprocess; the subprocess-concurrency memory is particularly relevant if you're tempted to run PR-B's test in parallel with the polyglot fixture (don't — `#[ignore]` gating already ensures it doesn't run by default in `cargo test --workspace`).

## How to execute

1. **Invoke `superpowers:executing-plans`** to load the plan-execution discipline.

2. **Open the plan at §4 Task 7** and follow Steps B.1 → B.3 in order. Mark `[x]` as you complete each.

3. **Step B.1 is the test file authoring.** Plan §4 Task 7 includes the test scaffold inline; follow it. Key points:
   - `#[ignore = "requires claude-code on PATH and ANTHROPIC_API_KEY configured"]` attribute is mandatory.
   - Assertion is on **server-side transcript Read-call count == 0**, not on subprocess exit code.
   - On test failure, the assertion message should explicitly say *"claude-code upstream regressed restriction enforcement (refresh mcp/restrictions.md with current upstream version)"* so the failure is actionable.
   - `eprintln!` the upstream version + response shape for forensic traceability.

4. **Verify after authoring** — the test compiles and is skipped by default:
   ```bash
   cargo build -p atlas-agents --tests
   cargo test -p atlas-agents --test mcp_disallowed_tools
   # ^ should report "1 test, 0 passed, 1 ignored" without spawning a subprocess
   ```

5. **Run the `--ignored` test against your local environment** (if you have claude-code + an API key):
   ```bash
   cargo test -p atlas-agents --test mcp_disallowed_tools --release -- --ignored
   ```
   Expected: passes (zero Read calls in the transcript). Record the upstream `claude-code --version` output in your commit message + PR-B per-PR note.

6. **Step B.2 is the cumulative-regression gate** — run all six:
   ```bash
   cargo build --workspace
   cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   cargo build --release --workspace
   cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
   ```
   The new `mcp_disallowed_tools.rs` test is `#[ignore]` by default; it does NOT run in `cargo test --workspace`. Polyglot smoke must hold at cold count `0 < cold < 100`; warm + reports = 0. **Do NOT pipe through `tail`**. **Do NOT run dev-mode `phase3_polyglot_fixture` concurrently with the release-mode run.** Use `--skip polyglot_phase3` substring.

7. **Step B.3 is the two-commit pattern.** First commit: the test file + any helper hoist (commit message: `sprint: PR-B subprocess --disallowedTools probe`). Second commit: status file — flip PR-B row from `[ ]` to `[x]`, update "Last updated" header, fill in PR-B's per-PR note with the commit SHA + upstream versions tested + observed response shape.

8. **Do not push.** The user pushes when ready.

## Two-stage review (recommended)

After your implementation commit but before the status-flip commit, run a two-stage review via `superpowers:subagent-driven-development`:

1. **Spec compliance review** — `feature-dev:code-reviewer` against plan §4 Task 7 acceptance gate (plan §5 row PR-B). Particularly: is the test correctly `#[ignore]`-gated? Does the assertion target the server-side transcript (not the subprocess exit code)? Is the failure message actionable (points to `mcp/restrictions.md` for refresh)?

2. **Code quality review** — `feature-dev:code-reviewer` for HIGH issues. Specific concerns for PR-B:
   - Does the test leak the subprocess on assertion failure (orphaned tokio task)? Standard answer: use `tokio::test` + ensure `serve_client`'s drop-impl reaps the subprocess.
   - Does the prompt actually provoke a `Read` attempt? A prompt like *"Read the file /etc/hosts using the Read tool. Do not invoke any other tool — only Read."* is the load-bearing input; if it's too weak, the LLM may just refuse without trying.
   - Are the environment-prerequisite early-returns differentiating "skipped" from "failed"? `eprintln!` + `return` is correct; `panic!` or `assert!` would mis-signal.

   HIGHs fixed before status flip; MEDIUMs recorded in PR-B's per-PR note for later sweeps.

If a flagged issue can't be resolved in one fix-cycle, **stop and surface**. Don't ship broken code to flip the checkbox.

## Coordination with PR-3 (parallel)

PR-3 (classify/reduce/project prompts + outputs.rs + canonical-schema shim + projection JSON→YAML) is **parallel-safe** with PR-B and may be running in another session. File sets are disjoint:
- **PR-3:** `crates/atlas-agents/src/runtime/{outputs.rs, projection_to_canonical.rs, audit/evidence.rs, audit/lane_a.rs, mod.rs}` + `crates/atlas-cli/src/pipeline.rs` + per-stage prompt-shape + evidence-floor + shim tests.
- **PR-B:** `crates/atlas-agents/tests/mcp_disallowed_tools.rs` only (single new file) + the optional `tests/common/mcp_test_helpers.rs` hoist if needed.

If a rebase conflict surfaces, **stop and surface** — the disjoint-files claim was load-bearing.

## Begin at Step B.1

Begin at **Step B.1: Author the probe test** in `docs/superpowers/specs/2026-05-13-atlas-production-prompt-sprint-plan.md` §4 Task 7.

Open the plan, locate the step, and proceed. Run the workspace build first (`cargo build -p atlas-agents --tests`) to confirm the worktree is clean before authoring.
