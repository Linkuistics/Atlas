---
name: cargo build --release --workspace before release-mode polyglot tests
description: Release-mode polyglot smoke tests require standalone analyzer binaries that `cargo test --workspace --release` does NOT build. Always prepend `cargo build --release --workspace`.
type: feedback
---

`cargo test --workspace --release --no-fail-fast` is **not sufficient** to run the Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs::polyglot_phase3_acceptance`) in release mode. The polyglot fixture discovers the standalone analyzer binaries (atlas-{python,csharp,dart,elixir,racket,lispkit}-analyzer) at runtime via path lookup at `target/release/<bin>`, `target/release/deps/<bin>`, `target/<bin>`. Those paths only get populated by `cargo build --release --workspace`, which builds every workspace member's `[[bin]]` targets — `cargo test --workspace --release` only builds *test binaries*, not standalone bins.

**Symptom when binaries are missing:** the test fails with `Other(related-components.yaml contains 1 unresolved contract participant(s): consumes-contract edge: component <X> → unresolved contract <Y>)`. Surfaces from the missing-binary languages are empty; cross-language `consumes-contract` edges then have no contract to resolve to. Look for `warning: <lang>-analyzer binary not found in any of: ...` lines in the test output for the diagnostic signal.

**Why:** Carl Phase 3 introduced the standalone subprocess analyzers; the polyglot test exercises cross-language contract resolution. Future Phase N+ verification chains for code-changing PRs need both the workspace build (for analyzer bins) AND the test invocation. PR-1 of Phase 4 hit this implicitly (the implementer pre-built the workspace as part of release-build verification); PR-4 surfaced it explicitly when the verification chain ran the focused polyglot test alone; PR-2 + PR-3 of Phase 4 hit it again when a v2 verification chain dropped the explicit `cargo build --release --workspace` step.

**How to apply:** for any verification chain that needs to run the polyglot smoke test in release mode, the canonical pattern is:

```
cargo build --release --workspace && cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Or, for a broader release-mode test pass:

```
cargo build --release --workspace && cargo test -p atlas-cli --release --no-fail-fast
```

(The second form runs all atlas-cli tests in release including the polyglot fixture; the test binary's incremental cache is reused if the build was already performed.)

The release polyglot run takes ~88-92s on this machine vs >9 min for the debug-mode polyglot inside `cargo test --workspace`. **Do NOT use** `cargo test --workspace --no-fail-fast` for routine verification — it runs the polyglot in debug mode (unusably slow) AND, in release, doesn't build the analyzer bins.

The continuation-prompt verification protocol for Phase 5+ should spell out this prerequisite explicitly. Pairs with the `feedback_no_tail_pipe_for_long_tests` memory (the polyglot test's runtime is long enough that any tail-piping hides progress).
