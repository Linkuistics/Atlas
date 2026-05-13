---
name: --agent-runtime flag default-false ratified
description: Phase 7 PR-7 shipped `atlas index --agent-runtime` as opt-in (default false); user ratified 2026-05-13; HTTP backends are the live path during the production-prompt sprint.
type: project
---

PR-7 wired AgentRuntime into `atlas index` via a single `Handle::block_on`
boundary but gated the runtime path behind a new `--agent-runtime` flag
with **default false**. The spec text at plan §5 line 2027 reads "atlas
index runs end-to-end through AgentRuntime" without specifying default-on;
the implementer's call (preserve deterministic-engine default until
production prompts ship) was ratified by the user on 2026-05-13.

**Why:** Production dispatch + classify + reduce + project prompt templates
are still `PR-7-WIRES-REAL-PROMPT` stubs (Phase 7 → Phase 8 handoff items
1–2). The cross-provider auditor closure is still `PR-7-WIRES-REAL-AUDITOR`
stub (item 3). Subprocess MCP `serve_client` driver isn't wired (item 5).
Flipping the binary default would break `atlas index` for real users on the
canonical claude_code + codex backend config (MEDIUM-3 footgun documented
in Phase 7 closeout). With API keys available, HTTP backends
(`http_anthropic` / `http_openai`) are the live path for the
production-prompt sprint's empirical work.

**How to apply:** Phase 8 brainstorming should NOT assume default-on
AgentRuntime; the flag flip to default-on is a separate decision gated on
items 1–5 of the production-prompt sprint landing first. The
cross-transport parity test in `phase3_polyglot_fixture.rs` exercises the
deterministic engine path with two transport labels — correct for the
default-false world; if/when default flips, that test needs to exercise
AgentRuntime through two real transports instead.
