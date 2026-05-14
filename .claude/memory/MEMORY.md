<!--
This directory holds the project's persistent Claude memory. It is
symlinked from ~/.claude/projects/<encoded-workspace-path>/memory/ so the
Claude Code harness reads from here transparently while the files live
under git in the repo, traveling across machines via `git pull`.

Per-machine setup: run `scripts/setup-claude-memory.sh` once after first
checkout. The script is idempotent. On a machine that has accumulated
local memories before setup, the script migrates them in (skipping
filename conflicts and asking for manual resolution).

Privacy: anything written here lands in git history. Project-scoped
memories (decisions, deferrals, codebase conventions) are appropriate;
user-personal memories ("dislikes verbose output") should stay machine-
local — keep those out of this directory.

This file is the index. Each pointer below is one line:
`- [Title](file.md) — one-line hook`. Memory files are siblings.
Keep this index ≤200 lines (the harness truncates beyond that).
-->

- [Atlas long-term monorepo consolidation](project_monorepo_consolidation.md) — user wants to fold atlas-contracts + Ravel + Ravel-Lite into Atlas and delete multi-root support.
- [Verify worktree base before parallel dispatch](feedback_worktree_base_verification.md) — `isolation:"worktree"` may create worktrees off a stale ref; verify each base matches current main before subagents proceed.
- [Phase 3 PR-6 overrides edges + field overrides](project_phase3_overrides_edges.md) — `edges_add`/`edges_suppress` (top-level) and per-component `overrides:` block (language/kind/lifecycle/subsystem) are canonical user-authoring seams; future Phase 9 LLM analysers should emit candidate edges as `edges_add` suggestions.
- [Atlas vNext Phase 4+ roadmap](project_phase4_plus_roadmap.md) — Phases 4 + 5 + 6 + 7 SHIPPED + production-prompt sprint SHIPPED 2026-05-14 (logically Phase-7-completion); Phase 8 (Cargo retirement) formally unblocked, with new agent-runtime/HTTP-backend wiring-gap prerequisite surfaced by PR-5 calibration.
- [Don't `tail`-pipe long cargo tests](feedback_no_tail_pipe_for_long_tests.md) — buffered tail makes a working 99%-CPU process look stuck. Brief implementers to let stdout pass through.
- [`cargo build --release --workspace` before release polyglot](feedback_release_workspace_build_for_polyglot.md) — release polyglot test discovers analyzer bins via runtime path lookup; `cargo test --workspace --release` does NOT build standalone `[[bin]]` targets, only test binaries.
- [Don't run heavy-subprocess Atlas tests in parallel](feedback_atlas_test_subprocess_concurrency.md) — dev `cargo test --workspace` + release polyglot run concurrently stalls one for 20+ min; shared process table + subprocess fan-out, not cargo lock.
- [cargo --skip pattern for Atlas polyglot fixture](feedback_cargo_skip_polyglot_pattern.md) — the correct substring to skip the dev-mode polyglot is `polyglot_phase3`, NOT `phase3_polyglot`; `--skip` is a literal substring filter on test function names.
- [Phase 5 split + Ravel/Ravel-Lite Bazel intent](project_phase5_split_and_ravel_bazel.md) — Phase 5 scoped to atlas-contracts fold + multi-root delete only; Ravel/Ravel-Lite fold deferred to a later phase that may migrate the build system to Bazel.
- [User rarely values git-history preservation](feedback_user_low_git_history_value.md) — for repo moves/consolidations, default to plain snapshot copy with a single import commit; don't propose subtree-merge or filter-repo unless asked.
- [No iterator stubs for known singletons](feedback_no_iterator_stubs_for_singletons.md) — when removing plurality, simplify the API shape end-to-end; don't leave iterator/slice accessors over length-1 collections.
- [No version on workspace path-deps](feedback_no_version_on_workspace_path_deps.md) — workspace-internal path-deps carry path only, no `version` field; publish-time mechanism injects version externally to source.
- [Atlas memory lives in-tree at .claude/memory/](feedback_atlas_memory_in_repo.md) — Atlas tracks memory in-repo via a `.gitignore` exception; canonical and in-repo paths are symlinked. Treat in-repo path as the commit target.
- [Atlas as prompts-as-application — LLM is the spine](feedback_atlas_llm_spine_intent.md) — LLM-driven analysis should be Atlas's spine via map-reduce over per-component tasks; deterministic code reserved for parsing/schema/cache. Roadmap's Phase 9 positioning of LLM is upside-down.
- [Atlas's purpose: LLM-consumed analyses of monorepos](project_atlas_purpose_llm_consumers.md) — Atlas exists to feed other LLM tools with monorepo context for (a) in-codebase agents, (b) refactoring cues, (c) documentation generation. Output quality bar = "useful as LLM context."
- [Cross-provider LLM audit beats same-model self-audit](feedback_cross_provider_llm_audit.md) — for any "LLM grades LLM" pattern, use different providers (Anthropic↔OpenAI); same-model audit is tautological. Empirically a major improvement.
- [Atlas common backend config — claude_code + codex](project_atlas_common_backend_config.md) — typical Atlas runtime pairs both subprocess backends (subscription-subsidized); HTTP backends are signal-gathering opt-ins; MCP server must multiplex 2 concurrent subprocess clients.
- [Don't frame LLM runtime against deterministic-engine](feedback_no_deterministic_engine_comparison.md) — deterministic path was an error; don't propose "compare with deterministic output" as success criteria or rationale for LLM-spine work.
- [Prefer existing crates over hand-rolled code](feedback_prefer_existing_crates.md) — "almost always use existing crates rather than writing our own code"; applies to protocol framing, schema, CLI, async primitives; existing hand-rolled code (e.g., PR-1's MCP framing) is a legitimate refactor candidate.
- [YAML is canonical interchange for Atlas](feedback_yaml_canonical_interchange.md) — LLM envelopes, internal artifacts, user-authored files all default to YAML; JSON reserved for LLM tool-use APIs (wire format) and JSONL event streams (streaming).
- [--agent-runtime flag default-false ratified](project_phase7_agent_runtime_default_ratified.md) — Phase 7 PR-7 shipped `atlas index --agent-runtime` as opt-in (default false); user ratified 2026-05-13; HTTP backends are the live path during the production-prompt sprint.
- [Phase 6 brainstorm paused for LLM-spine recast](project_phase6_paused_for_llm_spine.md) — SUPERSEDED 2026-05-11: the four pre-pivot candidate items shipped as Phase 6 (PR-1 deferred to Phase 9c); retained for forensic context.
