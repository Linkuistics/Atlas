---
name: Atlas common backend config — claude_code + codex (subscription-pairing)
description: User's typical Atlas runtime config pairs claude_code (Anthropic, subprocess) with codex (OpenAI, subprocess) because both are subscription-plan-subsidized. This is the right cross-provider audit default for Phase 7+ and shapes the MCP server's client-multiplexing requirements.
type: project
originSessionId: e4e7343b-07b1-45ba-b660-fdbccef4a564
---
The common Atlas runtime configuration pairs **`claude_code`** (Anthropic, subprocess) with **`codex`** (OpenAI, subprocess) as the two configured backends. Both are subscription-plan-subsidized in their respective providers' offerings, making them the natural daily-driver pair. HTTP backends (`http_anthropic`, `http_openai`) are pay-per-token and reserved for experimental/signal-gathering runs.

**Why:** During Phase 7 brainstorm (2026-05-12), user noted: "A common configuration is likely to be claude code + codex, because both are available on subscription (subsidized) planes." HTTP backends incur per-token cost; subprocess backends use the user's existing subscription quota. The two-subprocess pairing happens to also cover both major providers (Anthropic + OpenAI), which makes cross-provider audit (per `feedback_cross_provider_llm_audit.md`) work out-of-the-box without any HTTP backend configured.

**How to apply:**

- **Phase 7 PR-0 default config.** The default `BackendRouter` pairs `claude_code` + `codex` so cross-provider Lane B audit fires out-of-the-box on first run with no additional configuration. Producer-to-auditor mapping: `claude_code` (Anthropic) → `codex` (OpenAI); `codex` (OpenAI) → `claude_code` (Anthropic).
- **MCP server client multiplexing.** Atlas's in-process MCP server (recast §5.1 + Phase 7 §B design) must handle *two* concurrent subprocess clients (claude_code + codex simultaneously), each with disabled built-in tools (`--disallowedTools=Read,Grep,Glob,Bash,Write,Edit` or each provider's equivalent), each connecting over its own stdio pipe. MCP supports this natively; PR-0 names the rule explicitly.
- **HTTP backends are opt-in.** `http_anthropic` / `http_openai` are not default-active. They are configured explicitly for signal-gathering runs comparing HTTP-tool-use-loop quality against subprocess-observed-tool-call quality.
- **Budget posture stays coarse.** No per-provider per-bucket budget assertion in CI. The polyglot smoke test asserts one cold token total (regression detector, not cost target — per recast spec §2.4 / §8.4). TUI cost-to-date display can show per-provider breakdown for user awareness, but invariants don't depend on the split.
- **Wider applicability beyond Atlas.** When designing other LLM-driven CLIs for daily use: subprocess backends on subscription plans are the cost-efficient daily driver; HTTP backends are signal-gathering tools, not the production path.
