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
