# Claude memory — git-tracked, cross-machine

This directory is the project's persistent Claude memory. It is symlinked
from `~/.claude/projects/<encoded-workspace-path>/memory/` on each machine
so Claude Code's harness reads it transparently while the files live under
git in the repo. `git pull` is the cross-machine sync mechanism.

## Why this exists

`~/.claude/` is per-machine local state. By default, anything Claude writes
to memory during a session lives only on the machine that wrote it. When
you move machines, those memories are stranded on the original machine,
not lost but not accessible either. Symlinking the harness path into a
git-tracked directory in this repo makes memories travel with the code.

## First-time setup on a new machine

After the first `git clone` (or `git pull` that brings down this directory)
on a new machine, run **once**:

```bash
./scripts/setup-claude-memory.sh
```

The script:

- Computes the encoded workspace path the harness uses.
- Creates the parent directory under `~/.claude/projects/` if needed.
- Symlinks the harness memory path to `.claude/memory/` in this repo.
- Is idempotent — safe to re-run; reports `already linked` if so.

After this, future Claude sessions on this machine read and write through
the symlink to the in-repo directory. Files appear in `git status` when
Claude writes them; commit and push to share with other machines.

## Merging memories from another machine

If you have an older machine that accumulated memories before this
infrastructure existed (i.e., its memories are still in
`~/.claude/projects/<encoded-workspace>/memory/` as a real directory, not
a symlink), they need a one-time merge into the repo:

### Steps

1. **On the other machine**, pull the latest:

   ```bash
   cd <repo-root>
   git pull
   ```

2. Run the setup script:

   ```bash
   ./scripts/setup-claude-memory.sh
   ```

   The script sees the harness dir has accumulated memories. For each file:

   - **No filename collision with the repo's `.claude/memory/`**: file is
     moved into the repo. Reported as `moved: <name>`.
   - **Filename collision** (same name exists in both): the file is left in
     the harness dir, added to the conflict report, and the script exits
     with status 1.

3. **Resolve any conflicts.** Almost always the only conflict is
   `MEMORY.md` (both machines maintain their own index). Other memory
   files conflict only if the same memory name was independently created
   on both machines — rare but possible.

   For each conflict:

   ```bash
   # Inspect
   diff ~/.claude/projects/<encoded>/memory/<name>.md \
        <repo-root>/.claude/memory/<name>.md

   # For MEMORY.md: open the repo's version, paste in any new lines from
   # the harness version, preserving the preamble. Save.
   # For body files: pick the merged content, edit the repo version, save.

   # Remove the now-resolved file from the harness dir:
   rm ~/.claude/projects/<encoded>/memory/<name>.md
   ```

4. Re-run the setup script. With no remaining files in the harness dir,
   it creates the symlink and reports OK.

5. Stage, commit, push:

   ```bash
   git add .claude/memory/
   git commit -m "infra: import accumulated memories from <machine-name>"
   git push
   ```

6. **On the first machine**, pull:

   ```bash
   git pull
   ```

   The other machine's memories are now visible everywhere.

## Steady state

Once both machines are set up:

- Claude writes a memory during a session → file appears unstaged in
  `git status`.
- Commit and push promptly (ideally as part of the same commit that
  captured the work which produced the memory — keeps `git log` legible).
- On the other machine: `git pull` brings in any new memories.
- True merge conflicts (rare) resolve via standard 3-way text merge, since
  memory files are line-oriented YAML/Markdown.

## What belongs here vs not

**Belongs here** (project-scoped memory, appropriate for git history):

- Project decisions and deferrals (`project_monorepo_consolidation`,
  `project_phase2_closeout_known_issues`).
- Codebase conventions (`feedback_toml_parsing`,
  `feedback_use_serde_yaml`).
- References to external systems used by the project.

**Does not belong here** (user-personal, machine-local):

- Personal preferences about communication style ("user dislikes verbose
  explanations").
- Anything you wouldn't say in a public PR description.
- Anything that would be embarrassing if rediscovered six months later.

For user-personal memory, either accept it's lost on machine moves
(usually low cost — Claude infers it again quickly) or maintain a separate
personal `claude-state` git repo synced via your own remote.

## Troubleshooting

**Symlink is wrong target** (e.g., pointing at a stale path after the repo
moved on disk): remove the symlink and re-run the script.

```bash
rm ~/.claude/projects/<encoded>/memory
./scripts/setup-claude-memory.sh
```

**Harness writes feel slow** (e.g., shared filesystem performance issues):
not a known problem with local filesystems. If the repo is on a network
share, expect some latency on memory reads.

**`MEMORY.md` got truncated** (Claude's harness truncates beyond ~200
lines): split memory entries into themed index sections, or move the
oldest entries into a sibling `ARCHIVE.md` and update pointers.

## Files in this directory

- `MEMORY.md` — the index. Auto-loaded by Claude Code at session start.
- `<name>.md` — individual memory entries with frontmatter (`name`,
  `description`, `type`, then body).
- `README.md` — this file.
