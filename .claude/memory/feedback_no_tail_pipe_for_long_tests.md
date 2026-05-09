---
name: Don't pipe long-running cargo tests through `tail`
description: When dispatching subagents to run long verification tests (phase3_polyglot_fixture etc.), instruct them to let stdout pass through unbuffered, not `... 2>&1 | tail -N`
type: feedback
originSessionId: 1c1635ff-e870-47f6-a275-5d01e54515bb
---
When an Atlas implementer subagent runs a long verification test like
`cargo test -p atlas-cli --test phase3_polyglot_fixture --no-fail-fast`,
do NOT have them pipe it through `tail -N`. Buffered tail output makes
the test look stuck to anyone watching the worktree (process is at
99% CPU but no stdout visible until EOF). Let stdout stream
unbuffered so the orchestrator and the user see progress live.

**Why:** During Phase 4 PR-1 (2026-05-09), the implementer ran
`cargo test ... 2>&1 | tail -10` as its final regression guard. The
polyglot fixture test ran for 8+ minutes at full CPU. From outside,
this looked like a hang — the user (correctly) flagged it as stuck.
Inspection showed the test was working normally; only the stream
buffering hid that fact.

**How to apply:** When briefing implementers for Phase 4+ PRs that
include the cumulative polyglot regression guard, instruct them
explicitly: "Run verification tests with stdout passthrough — do not
pipe through `tail`, `head`, or similar buffered filters. Use the
plain `cargo test ... --no-fail-fast` form so progress is visible."
The polyglot fixture test naturally takes 5–15 minutes on a clean
build; that's normal, not a regression.
