---
name: cargo --skip pattern for Atlas polyglot fixture
description: To elide the dev-mode polyglot test (which is redundant with the release-mode regression guard), the correct cargo --skip substring is `polyglot_phase3`, NOT `phase3_polyglot`.
type: feedback
originSessionId: 6f813009-d364-4b96-beb0-a0255935e6a4
---
When running `cargo test --workspace` against Atlas, the dev-mode `polyglot_phase3_acceptance` integration test (in `crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) takes 13+ minutes and is REDUNDANT with the release-mode polyglot regression guard that every PR-N+ runs separately (per PR-3 closeout). The intended skip pattern was `--skip phase3_polyglot` (per PR-3 closeout recommendation), but this is a LITERAL SUBSTRING filter on test FUNCTION names — and `phase3_polyglot` doesn't appear as a contiguous substring of `polyglot_phase3_acceptance`.

**Correct skip command:**

```bash
cargo test --workspace --no-fail-fast -- --skip polyglot_phase3
```

**Why:** the test function is named `polyglot_phase3_acceptance` (with `polyglot` before `phase3`). The substring `polyglot_phase3` matches; the substring `phase3_polyglot` does not.

**How to apply:** brief any subagent running `cargo test --workspace` to use `--skip polyglot_phase3`. Pair this with the existing rule from `feedback_no_tail_pipe_for_long_tests` (no `| tail` on long tests; use `tee` instead for visible streaming output).

**Why this matters operationally:** three implementer subagent sessions during Phase 7 PR-4 / PR-6 verification burned 30+ minutes each running the dev-mode polyglot because the `--skip phase3_polyglot` recommendation in the PR-3 closeout didn't actually elide the test. The orchestrator (me) eventually had to kill processes and re-run with the correct pattern.

**Related memories:**
- `feedback_no_tail_pipe_for_long_tests.md` — pair with `tee` for visibility.
- `feedback_atlas_test_subprocess_concurrency.md` — don't run dev-mode workspace tests concurrently with release polyglot.
- `feedback_release_workspace_build_for_polyglot.md` — `cargo build --release --workspace` is the prerequisite for release polyglot (test runtime path-resolves analyzer bins).
