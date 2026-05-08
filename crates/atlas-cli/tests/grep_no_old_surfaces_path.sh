#!/usr/bin/env bash
# grep_no_old_surfaces_path.sh — Phase 3 PR-2 grep-audit script.
#
# Exits 1 if any git-tracked file contains a reference to
# `.atlas/surfaces.yaml` outside of the `cache/` subdirectory.
#
# Usage (from the repository root):
#   bash crates/atlas-cli/tests/grep_no_old_surfaces_path.sh
#
# The pattern matches:
#   .atlas/surfaces.yaml          ← old (pre-PR-2) path: FORBIDDEN
# but NOT:
#   .atlas/cache/surfaces.yaml    ← new (post-PR-2) path: allowed
#
# Implementation note: we use `git grep` so only tracked files are searched
# (untracked workspace artifacts are ignored). The negative-lookahead pattern
# `\.atlas/(?!cache/)surfaces\.yaml` is matched via Perl-compatible
# regex (`-P`).
#
# Expected exit codes:
#   0 — no forbidden references found (tree is clean)
#   1 — at least one forbidden reference found (must be fixed)
set -euo pipefail

# Move to the repository root regardless of where the script is invoked from.
REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

echo "Auditing tracked files for old surfaces.yaml path..."

# Search for the old path pattern. Exclude:
#   - This script itself (it mentions the pattern in comments).
#   - Documentation/plan files (allowed to document the migration).
#   - LLM_STATE files (LLM session memory, not code).
#   - evaluation/results (golden artefacts, not code).
if git grep -lP '\.atlas/(?!cache/)surfaces\.yaml' \
    -- \
    ':!crates/atlas-cli/tests/grep_no_old_surfaces_path.sh' \
    ':!docs/' \
    ':!LLM_STATE/' \
    ':!evaluation/results/' \
    2>/dev/null | grep -q .; then
    echo "FAIL: found references to old .atlas/surfaces.yaml path in tracked files:" >&2
    git grep -nP '\.atlas/(?!cache/)surfaces\.yaml' \
        -- \
        ':!crates/atlas-cli/tests/grep_no_old_surfaces_path.sh' \
        ':!docs/' \
        ':!LLM_STATE/' \
        ':!evaluation/results/' \
        2>/dev/null >&2
    exit 1
fi

echo "OK: no references to old .atlas/surfaces.yaml found."
exit 0
