---
name: Atlas memory lives in-tree at .claude/memory/
description: Atlas tracks .claude/memory/ in git via a .gitignore exception; canonical and in-repo memory paths are equivalent (likely symlinked). Treat as the canonical commit target.
type: feedback
originSessionId: 78784dc8-e71a-406c-8d83-3d94e3baf10f
---
In Atlas, the canonical project memory location IS in the repo at
`.claude/memory/` (committed via a `.gitignore` exception). The
on-disk relationship between `~/.claude/projects/-Users-antony-
Development-Atlas/memory/` and `Atlas/.claude/memory/` is a
symlink set up by `scripts/setup-claude-memory.sh`; both paths
expose the same files.

**Why:** `.gitignore` has `.claude/* / !.claude/memory/` —
explicitly un-ignores memory after ignoring everything else under
`.claude/`. The comment in `.gitignore` says: "the project's
persistent memory is tracked so it travels via git across
machines." Several past commits (`f80e179`, `579a809`, `c30aae1`,
`ffc3a05`) commit memory edits to the repo as `phase{N}: memory —
…` commits. This contradicts the default Claude Code guidance
(memory lives outside the repo) — Atlas is the exception.

**How to apply:** When updating Atlas memory, edit either path
(they're the same files via symlink) and `git add .claude/memory/...`
+ commit on a `phase{N}: memory — …` message. Don't attempt to
keep the in-repo and canonical paths "in sync" — they're already
the same file system object. Phase plans that say "memory edits
do NOT commit to the repo" are mistaken about Atlas's specific
convention; treat the codified `.gitignore` rule as authoritative.
