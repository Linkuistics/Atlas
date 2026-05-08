---
name: Verify worktree base before parallel dispatch
description: When dispatching parallel agents with isolation:"worktree", verify each worktree's branch base matches current main before letting subagents proceed
type: feedback
originSessionId: 4b55c5ce-f718-4f12-8e29-7393f81148f7
---
When dispatching parallel agents via the Agent tool with `isolation: "worktree"`, do **not** assume the new worktree is branched off current `main`. In a 2026-05-08 Phase 3 wave-3 dispatch (4 agents, parallel), 3 of 4 worktrees were created off a stale ref (`c6ddb67`, 10 commits behind main) while only the 4th was correct. Subagents whose worktree lacked the prerequisite PR's helpers (PR-1's `atlas_engine::atomic_write` + `ensure_atlas_gitignore`) either reported BLOCKED or began silently duplicating those helpers — review-fail in both directions.

**Why:** The harness's worktree-creation mechanism appears to use a session-cached ref rather than resolving HEAD live. Cause is opaque from inside Claude Code; the symptom is reproducible enough to defend against.

**How to apply:**
- After dispatching parallel worktree agents, immediately run `git worktree list` and confirm each new worktree's commit matches the current main HEAD.
- If any worktree is mis-based, the available remediation in this session is **redispatch** (no SendMessage, no in-place reset is reliable mid-flight). Force-removing a running agent's worktree corrupts its process tree; in-place `git reset --hard main` is racy if the agent is mid-write.
- For waves where any prereq PR has landed in the same session, prefer **sequential dispatch** unless you've verified the worktree-creation path is reliable for your harness version. Sequential dispatch lets you fix per-worktree base before the agent does any work.
- If you must go parallel: pre-create the worktrees yourself via `git worktree add -b <branch> <path> main`, then dispatch agents with explicit `cwd` pointing at those paths (NOT `isolation: "worktree"`).
