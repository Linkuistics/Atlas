# Subprocess built-in-tool restrictions

When Atlas's MCP server hosts a subprocess backend, the subprocess must
NOT use its own built-in tools (Read/Grep/Glob/Bash/Write/Edit and
provider-equivalents). Atlas's `Tool` impls are the only tools available;
the unified envelope (recast §5.4) requires single-trait sourcing.

## claude-code

`--disallowedTools=Read,Grep,Glob,Bash,Write,Edit`

Targeted upstream versions: claude-code ≥ 2.0 (current daily-driver).

## codex

Per upstream documentation: `--disable-tools <set>` flag (placeholder;
exact set TBD by PR-1 implementer at subprocess wiring time). Targeted
upstream versions: codex 0.x (current daily-driver).

If a future upstream version adds a new built-in tool, this file must
be updated AND PR-7's acceptance test ("tool-call-Read-and-fail" probe)
will catch the gap.
