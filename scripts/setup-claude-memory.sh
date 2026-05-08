#!/usr/bin/env bash
# setup-claude-memory.sh — symlink ~/.claude/projects/<workspace>/memory/ to
# the in-repo .claude/memory directory so memories travel via git across
# machines. Idempotent. Safe to run on any machine, any number of times.
#
# Usage: run once per machine after the first checkout of the repo:
#     ./scripts/setup-claude-memory.sh
#
# Behaviour:
#   - If the harness memory dir doesn't exist yet on this machine: create the
#     symlink. Done.
#   - If the harness memory dir is already a symlink to the right target:
#     no-op. Done.
#   - If the harness memory dir exists with files (memories accumulated on
#     this machine before setup): for each file, move it into the repo's
#     .claude/memory/ unless a file with the same name already exists in the
#     repo. Same-name conflicts are reported and left in the harness dir for
#     manual resolution. Then replace the harness dir with the symlink.
#
# After running on a second machine that has its own accumulated memories,
# inspect any conflicts reported, resolve them by hand (typically a textual
# merge of MEMORY.md entries and a rename of any new memory files), then
# `git add .claude/memory/` and commit.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_MEMORY_DIR="$REPO_ROOT/.claude/memory"

# Encode the workspace path the way Claude Code does: /a/b/c -> -a-b-c
ENCODED_PATH="$(echo "$REPO_ROOT" | sed 's|/|-|g')"
HARNESS_DIR="$HOME/.claude/projects/${ENCODED_PATH}"
HARNESS_MEMORY_DIR="$HARNESS_DIR/memory"

mkdir -p "$REPO_MEMORY_DIR"

# Already correctly symlinked?
if [[ -L "$HARNESS_MEMORY_DIR" ]]; then
    target="$(readlink "$HARNESS_MEMORY_DIR")"
    if [[ "$target" == "$REPO_MEMORY_DIR" ]]; then
        echo "OK: $HARNESS_MEMORY_DIR -> $REPO_MEMORY_DIR (already linked)"
        exit 0
    else
        echo "ERROR: $HARNESS_MEMORY_DIR is a symlink to a different target:"
        echo "    actual:   $target"
        echo "    expected: $REPO_MEMORY_DIR"
        echo "Resolve manually (rm the symlink and re-run, or fix the target)."
        exit 1
    fi
fi

# Harness dir exists as a real directory — migrate its contents.
conflicts=()
if [[ -d "$HARNESS_MEMORY_DIR" ]]; then
    shopt -s nullglob dotglob
    for f in "$HARNESS_MEMORY_DIR"/*; do
        base="$(basename "$f")"
        if [[ -e "$REPO_MEMORY_DIR/$base" ]]; then
            # Conflict: leave in harness dir; user resolves.
            conflicts+=("$base")
        else
            mv "$f" "$REPO_MEMORY_DIR/$base"
            echo "moved: $base"
        fi
    done
    shopt -u nullglob dotglob

    if [[ ${#conflicts[@]} -gt 0 ]]; then
        echo
        echo "WARNING: ${#conflicts[@]} file(s) already present in repo memory:"
        for c in "${conflicts[@]}"; do
            echo "    $c"
        done
        echo
        echo "Compared paths:"
        for c in "${conflicts[@]}"; do
            echo "    diff $HARNESS_MEMORY_DIR/$c $REPO_MEMORY_DIR/$c"
        done
        echo
        echo "Resolve by hand: typically MEMORY.md needs a line-union merge,"
        echo "and same-name memory files need either a content merge or a rename."
        echo "After resolution, mv the file(s) out of $HARNESS_MEMORY_DIR and re-run."
        exit 1
    fi

    # Empty now — remove and replace with symlink.
    rmdir "$HARNESS_MEMORY_DIR"
fi

# Ensure parent dir exists.
mkdir -p "$HARNESS_DIR"

ln -s "$REPO_MEMORY_DIR" "$HARNESS_MEMORY_DIR"
echo "OK: $HARNESS_MEMORY_DIR -> $REPO_MEMORY_DIR (linked)"
