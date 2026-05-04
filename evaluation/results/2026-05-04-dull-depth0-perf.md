# dull depth-0 perf smoke — 2026-05-04

Backlog task: `confirm-l8-perf-win-on-dull-at-max-depth-0`.

## Outcome

**Blocked, not measurable on dull.** The L8 fix
(`skip-l8-structural-signal-computation-when-policy-will-stop`) is verified in
code, but the perf measurement this task specifies cannot be obtained from
`atlas index ~/Development/dull/ --max-depth 0` because dull's pin coverage is
not total — Atlas's enumeration finds candidates that do not appear in
`components.overrides.yaml`, so L3 Classify fires LLM calls that dominate
wall time and obscure the L8 measurement.

## What the fix is

`subcarve_policy::decide_kind_only` (`crates/atlas-engine/src/subcarve_policy.rs:58`)
returns `Some(Stop)` for leaf-like kinds (CLI, app, service, proc-macro,
docker, installer, scripts, spec/docs/config repos, external, non-component)
and `Some(Recurse)` for Workspace, allowing
`l8_recurse::compute_decision` (`crates/atlas-engine/src/l8_recurse.rs:121`)
to skip `modularity_hint` computation entirely for those kinds.
`modularity_hint` transitively pulls L5 `surface_of`, so eliminating the
hint call eliminates a chain of expensive work whenever the verdict is
already known from kind alone.

## Verification status

The fix is verified at unit-test grain by two regression tests in
`crates/atlas-engine/src/l8_recurse.rs`:

- `max_depth_zero_skips_all_l5_surface_calls_on_library_kind` (line 621) —
  asserts a library kind at `--max-depth 0` makes zero L5 surface calls.
- `cli_kind_skips_all_llm_calls_under_normal_max_depth` (line 653) —
  asserts a CLI-kind candidate produces zero LLM calls of any kind at
  default depth.

Both tests use `RecordingBackend` to record every `PromptId` received and
assert the recorded list is empty — a structural check, not a wall-time
proxy. The fix is in place and has its own regression coverage.

## Why dull does not work as the integration-level smoke test

dull's `components.overrides.yaml` declares 83 pinned ids. A cold-cache
run (cache file present, `entries: []`) of `atlas index ~/Development/dull/
--max-depth 0 --no-budget` was attempted; within minutes, parallel
`claude -p` subprocesses appeared with rendered Classify prompts for at
least these candidates, **neither of which is keyed in the pins file**:

- `session-recording-server/policy_api`
- `shared-backend/Dull.SqlcDal/sql_sanitizer`

The run was killed at ~12 min wall to avoid spending budget on a path
that does not measure what the task is trying to confirm. Cleanup of
orphaned `claude -p` children was required (PIDs reparented to launchd
after the parent atlas process was killed).

**Correction to memory.** `dull-depth-1-skips-llm-path-due-to-dense-pin-coverage`
asserts pin coverage is dense enough that all classify/subcarve decisions
short-circuit deterministically. That is now known to be inaccurate on at
least two paths and should be updated. Whether the gap is wider than the
two paths observed in this smoke is open — a full audit would compare the
keys of `components.overrides.yaml::pins` against Atlas's enumerated
candidates from the L1/L2 walk.

The previously-recorded "0 LLM calls, 7min wall" datapoint was almost
certainly captured with a warm `llm-cache.json` (cache hits on every
candidate); it is not a true cold-cache measurement.

## Recommended follow-up

Two viable paths to a clean integration-level baseline:

1. **Complete dull's pin coverage.** Audit
   `components.overrides.yaml::pins` against Atlas's enumeration; pin every
   uncovered candidate. After full coverage, repeat the smoke run.
2. **Author a tiny synthetic fixture.** A 5–10-component fixture composed
   entirely of leaf-Stop kinds (CLIs, services, installers) tests the fix
   without depending on a real workspace's pin coverage. Existing unit
   tests already cover this at finer grain; an integration-level fixture
   adds value only if it also exercises L0/L1/L2 enumeration end-to-end.

The cheapest unblock is option 2 — the unit tests already cover the
core invariant, so an additional integration test gives diminishing returns.

## Cost of this run

Atlas was killed before any L3 Classify call returned a response from
`claude-code`. Token usage on this attempt is presumed minimal (each
spawned `claude -p` was killed mid-prompt). The host OS reaped the
orphaned children cleanly.
