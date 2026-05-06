# Atlas vNext Phase 1 — Status

Companion to `docs/superpowers/specs/2026-05-06-atlas-vnext-phase1-plan.md`.
This file tracks per-PR completion state across sessions. The session
prompt at `docs/superpowers/plans/2026-05-06-phase1-session-prompt.md`
reads this file to find the next PR to dispatch.

**Last updated:** 2026-05-06 (PR-0 landed).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Companion specs (docs only) — *blocks PR-1+*
- [ ] PR-1  — Schema definitions for new types
- [ ] PR-2  — Persistent content-addressed cache (no wiring)
- [ ] PR-3  — Multi-root `Workspace` (Salsa input)
- [ ] PR-4  — Path-dep root expansion to fixed point
- [ ] PR-5  — Plugin protocol + three reference analysers
- [ ] PR-6  — Scattered per-component `.atlas/` writers
- [ ] PR-7  — `surfaces.yaml` emission (Rust binding shape)
- [ ] PR-8  — Contract participants in `related-components.yaml`
- [ ] PR-9  — Composition edges from Dockerfiles
- [ ] PR-10 — Wire persistent cache into L3 / L5 / L6
- [ ] PR-11 — L6 cache key includes participant surface shas
- [ ] PR-12 — Acceptance: atlas-contracts visible in Ravel-Lite

When every box is `[x]`, Phase 1 is complete and the session prompt
should report success and stop.

## Dependency graph (informational; canonical in plan §4)

```
PR-0 ─┬─> PR-1 ─┬─> PR-5 ─┬─> PR-7 ─> PR-8
      │         │         ├─> PR-9
      │         │         └─> PR-10 ──> PR-11
      │         └─> PR-2 ─/                │
      │                                    │
      └─> PR-3 ──> PR-4                    │
                  └─> PR-6 ──> PR-7        │
                                           ▼
                            PR-12 (depends on everything)
```

Parallel-safe waves:
- Wave 1 (after PR-0): PR-1, PR-2, PR-3 concurrently.
- Wave 2 (after PR-1, PR-3): PR-4, PR-5, PR-6 concurrently.
- Wave 3 (after PR-2 + PR-5 + PR-6): PR-7 (then PR-8), PR-9, PR-10.
- Wave 4: PR-11 (after PR-7 + PR-10).
- Wave 5: PR-12 (after all).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples
of what's worth recording: deviations from the plan that the next
session needs to know, surprising fixture quirks, manual verification
steps that succeeded, follow-up cleanup deferred.

### PR-0
2026-05-06 — Landed in same commit as the plan/design/status/session-prompt
docs (all were untracked from the prior planning session). Two new specs:
`2026-05-06-contract-content-sha-canonicalisation.md` (resolves §11.2.2)
and `2026-05-06-override-scoping-scattered-atlas.md` (resolves §11.2.3).
Memory entry `feedback_phase1_open_questions` records the closure.

Load-bearing for downstream review: PR-7 must implement both branches of
the canonicalisation algorithm (§2.1 byte-range for Phase 1 code-derived,
§2.2 canonical YAML for the test-only schema-derived fixture). PR-6 must
emit the per-component-scoping warning in the format spelled out in §6 of
the override-scoping spec.

### PR-1
(none yet)

### PR-2
(none yet)

### PR-3
(none yet)

### PR-4
(none yet)

### PR-5
(none yet)

### PR-6
(none yet)

### PR-7
(none yet)

### PR-8
(none yet)

### PR-9
(none yet)

### PR-10
(none yet)

### PR-11
(none yet)

### PR-12
(none yet)
