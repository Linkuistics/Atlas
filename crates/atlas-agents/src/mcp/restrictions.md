# Subprocess built-in-tool restrictions

When Atlas's MCP server hosts a subprocess backend, the subprocess must
NOT use its own built-in tools (Read/Grep/Glob/Bash/Write/Edit and
provider-equivalents). Atlas's `Tool` impls are the only tools available;
the unified envelope (recast §5.4) requires single-trait sourcing.

The presets in `crates/atlas-agents/src/mcp/serve_client.rs`
(`claude_code_config`, `codex_config`) encode the flag set per upstream.
PR-B's live-subprocess probe at `tests/mcp_disallowed_tools.rs`
verifies that each upstream actually honours the flag (i.e., the
declared restrictions correspond to tools that are demonstrably
disabled when the LLM tries to call them).

## claude-code

`--disallowedTools=Read,Grep,Glob,Bash,Write,Edit`

Targeted upstream versions: claude-code ≥ 2.0 (current daily-driver).

Source of truth for the flag name + accepted value shape:
[`claude-code` CLI reference](https://docs.claude.com/en/docs/claude-code).
The accepted value is a comma-separated tool-name list. The named tools
match `claude-code`'s built-in tool registry IDs (`Read`, `Grep`,
`Glob`, `Bash`, `Write`, `Edit`).

## codex

`--mcp-config <path>` is the active flag for pointing codex at an
external MCP server. As of PR-A's verification date (2026-05-13), the
upstream codex CLI does not expose a dedicated `--disallowedTools`-
equivalent flag; tool-availability is controlled implicitly by what
the MCP servers in the config file advertise. Atlas's MCP server
exposes ONLY the Atlas tool catalog (no Read/Grep/Glob/Bash/Write/Edit
equivalents). Therefore the restriction is enforced by *omission* on
the codex side rather than by an explicit disallow flag.

If a future codex upstream version introduces an explicit disallow
flag, update `codex_config` in `serve_client.rs` to set it AND extend
PR-B's probe (`tests/mcp_disallowed_tools.rs`) with a codex-targeted
case to verify the flag is honoured.

Targeted upstream version: codex 0.x (current daily-driver).

## Forward compatibility

If a future upstream version of either backend adds a new built-in
tool, this file must be updated AND PR-B's probe will catch the gap —
the probe asserts the named tool is *not* invokable from inside the
subprocess agent loop.
