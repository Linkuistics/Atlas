#!/usr/bin/env bash
# Phase 3 PR-4 grep-audit: ensure no tracked source file references the
# old top-level `.atlas/components.yaml` path outside the cache
# sub-directory.
#
# Pattern matched: `.atlas/components.yaml` (without `cache/` between
# the directory separator and the filename).
#
# Exits 0 when no matches are found (audit passes).
# Exits 1 when at least one match is found (audit fails).
#
# Files excluded by git grep:
# - docs/  — specs and design docs intentionally reference the old path
#             for historical context.
# - evaluation/results/ — historical run artefacts; not source code.
# - .sh files in crates/atlas-cli/tests/ — this script itself.
#
# Usage (from workspace root):
#   bash crates/atlas-cli/tests/grep_no_old_components_path.sh
#
# The script uses `git grep` so untracked files are not scanned.

set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$WORKSPACE_ROOT"

# Pattern: `.atlas/` followed immediately (no intervening `cache/`) by
# `components.yaml`.  We use Perl regex (`-P`) for the negative-lookahead
# form `(?!cache/)`.
HITS=$(git grep -Pn '\.atlas/(?!cache/)components\.yaml' -- \
    '*.rs' '*.sh' '*.toml' '*.json' \
    2>/dev/null \
  | grep -v '^docs/' \
  | grep -v '^evaluation/results/' \
  | grep -v 'grep_no_old_components_path.sh' \
  || true)

if [ -n "$HITS" ]; then
    echo "ERROR: found reference(s) to .atlas/components.yaml outside cache/:" >&2
    echo "$HITS" >&2
    echo "" >&2
    echo "All top-level components.yaml files must live at" >&2
    echo ".atlas/cache/components.yaml (Phase 3 PR-4 retrofit)." >&2
    exit 1
fi

echo "grep-audit: OK — no tracked files reference .atlas/components.yaml outside cache/"
exit 0
