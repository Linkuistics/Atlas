---
name: User rarely values git-history preservation
description: For repo moves / consolidations / refactors, default to snapshot mechanics rather than subtree-merge or filter-repo gymnastics — user explicitly does not value preserved blame/log continuity.
type: feedback
originSessionId: 59c1d765-ecdc-4f31-971e-3aa9bb256513
---
User stated 2026-05-10, when picking how to fold atlas-contracts into Atlas: "I rarely care about git history, and in this case not at all. You use it more than I do. We should just move the dirs with no git mechanics."

**Why:** History preservation is a tooling-cost the user does not pay back. They navigate by current code + status docs, not by `git log` / `git blame`. Cross-phase references (e.g., "Phase 3 PR-6 added edges_add") are already captured in spec/status documents, which are the authoritative narrative — git history is redundant.

**How to apply:**

- For repo consolidations or directory moves, default to plain `cp -R` (or `git mv` within one repo) with a single import/move commit. Don't propose `git subtree add` or `git filter-repo` unless explicitly asked.
- Don't add "preserve blame continuity" or "keep git history" to the list of trade-offs when presenting options — for this user, that column is empty.
- This is a default, not a hard rule: if a refactor *specifically* hinges on tracking who-changed-what (rare), still raise it.
