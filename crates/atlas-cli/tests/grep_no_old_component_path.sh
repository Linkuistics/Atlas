#!/usr/bin/env bash
# grep_no_old_component_path.sh — Phase 3 PR-3 grep-audit script.
#
# Exits 1 if any git-tracked file contains a reference to
# `.atlas/component.yaml` (singular) outside of the `cache/` subdirectory.
# Does NOT match the plural-form `components.yaml` — that is PR-4's
# territory. The negative-lookahead `(?!s)` anchors on the singular form.
#
# Usage (from the repository root):
#   bash crates/atlas-cli/tests/grep_no_old_component_path.sh
#
# The pattern matches:
#   .atlas/component.yaml          ← old (pre-PR-3) path: FORBIDDEN
# but NOT:
#   .atlas/cache/component.yaml    ← new (post-PR-3) path: allowed
#   plural-form components.yaml    ← different file, not matched
#
# Implementation note: we use `git grep` so only tracked files are searched
# (untracked workspace artifacts are ignored). The negative-lookahead pattern
# `\.atlas/(?!cache/)component\.yaml(?!s)` is matched via Perl-compatible
# regex (`-P`).
#
# Expected exit codes:
#   0 — no forbidden references found (tree is clean)
#   1 — at least one forbidden reference found (must be fixed)
set -euo pipefail

# Move to the repository root regardless of where the script is invoked from.
REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

echo "Auditing tracked files for old per-component component.yaml path..."

# Search for the old path pattern. Exclude:
#   - This script itself (it mentions the pattern in comments).
#   - Documentation/plan files (allowed to document the migration).
#   - LLM_STATE files (LLM session memory, not code).
#   - evaluation/results (golden artefacts, not code).
if git grep -lP '\.atlas/(?!cache/)component\.yaml(?!s)' \
    -- \
    ':!crates/atlas-cli/tests/grep_no_old_component_path.sh' \
    ':!docs/' \
    ':!LLM_STATE/' \
    ':!evaluation/results/' \
    2>/dev/null | grep -q .; then
    echo "FAIL: found references to old per-component component.yaml path in tracked files:" >&2
    git grep -nP '\.atlas/(?!cache/)component\.yaml(?!s)' \
        -- \
        ':!crates/atlas-cli/tests/grep_no_old_component_path.sh' \
        ':!docs/' \
        ':!LLM_STATE/' \
        ':!evaluation/results/' \
        2>/dev/null >&2
    exit 1
fi

echo "OK: no references to old per-component component.yaml path found."
exit 0
