---
name: Heavy-subprocess Atlas tests don't compose in parallel
description: Running dev-mode `cargo test --workspace` concurrent with release-mode polyglot smoke stalls one of them for 20+ minutes even though cargo locks are independent — shared system process table + subprocess fan-out is the real contention surface.
type: feedback
originSessionId: d751358e-935b-4fea-9dc4-648b3643c0b1
---
When orchestrating Atlas test gates, do NOT run heavy-subprocess test workloads in parallel even when their cargo locks are independent (e.g. dev `cargo test --workspace --no-fail-fast` and release `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast`). Run them serially.

**Why:** Empirically observed in Phase 7 PR-2 (2026-05-12). I launched both concurrently expecting cargo's target-dir lock to be the only contention surface — they hit `target/debug` and `target/release` respectively, no lock contention. But Atlas tests spawn analyzer subprocesses heavily via `process_pool`, and the dev-mode `phase3_polyglot_fixture` stalled for 20+ minutes (16:35 of accumulated CPU before I killed it) while the release-mode polyglot finished in 104s. The release test finishing did NOT unstick the dev one — the degraded state persisted. Re-running the dev workspace test in isolation completed cleanly. Root cause: shared system process table + subprocess fan-out + filesystem fsync pressure, not cargo's lock.

**How to apply:** When the plan §4 Task N Step N.9 lists both `cargo test --workspace --no-fail-fast` and `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` as gates, run them sequentially. Quick gates (`cargo build`, `cargo fmt --check`, `cargo clippy`) can safely overlap with each other or with one polyglot run; the rule is specifically about heavy-subprocess test binaries. For Wave 2 (PR-3, parallel subagents in worktrees) the same caution applies: each subagent's worktree has its own target dir, but if all three subagents run polyglot tests concurrently on the host, they will contend the same way.
