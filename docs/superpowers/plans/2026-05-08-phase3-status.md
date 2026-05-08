# Atlas vNext Phase 3 — Status

Companion to `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`.
This file tracks per-PR completion state across sessions. The
continuation prompt at
`docs/superpowers/prompts/2026-05-08-vnext-continue.md` (Phase-3-shaped)
reads this file (via the `*phase3-plan*` wildcard match) to find the
next PR to dispatch.

**Last updated:** 2026-05-08 (PR-0b landed: design-doc touch-ups in
canonical system-model spec). Wave 2 (PR-1 + PR-7) is now dispatchable
concurrently — independent surfaces, no shared files.

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0a — Plan + status (docs only)
- [x] PR-0b — Design-doc touch-ups in canonical system-model spec (docs only)
- [ ] PR-1  — Gitignore mechanism for `<scope>/.atlas/cache/` + atomic_write helper
- [ ] PR-2  — Phase 1 retrofit: per-component `surfaces.yaml` → cache
- [ ] PR-3  — Phase 1 retrofit: per-component `component.yaml` → cache
- [ ] PR-4  — Phase 1 retrofit: top-level `components.yaml` → cache
- [ ] PR-5  — Phase 1 retrofit: top-level `related-components.yaml` → cache
- [ ] PR-6  — Overrides schema extension: `edges_add` / `edges_suppress` + per-component field overrides
- [ ] PR-7  — `atlas-reports` crate scaffold + CLI subcommand framework
- [ ] PR-8  — Drift report + `atlas drift` CLI subcommand
- [ ] PR-9  — Impact query + `atlas impact <id>` CLI subcommand
- [ ] PR-10 — Modularity report + `atlas modularity` CLI subcommand
- [ ] PR-11 — Composition divergence + `atlas divergence` CLI subcommand
- [ ] PR-12 — Atomic-write fixture suite for stateful files
- [ ] PR-13 — Acceptance: Phase 3 polyglot smoke test

When every box is `[x]`, Phase 3 is complete and the continuation
prompt should report success and stop.

## Dependency graph (informational; canonical in plan §4 + plan §9)

```
PR-0a (plan + status)
  │
  ▼
PR-0b (design-doc touch-ups in canonical system-model spec)
  │
  ├──> PR-1 (gitignore mechanism + atomic_write helper)
  │     │
  │     ├──> PR-2 (retrofit surfaces.yaml)        ──┐
  │     ├──> PR-3 (retrofit per-component component.yaml)  ──┤
  │     ├──> PR-4 (retrofit top-level components.yaml)      ──┤
  │     └──> PR-5 (retrofit related-components.yaml)        ──┤
  │                                                          │
  │                                                          ▼
  │                                                  PR-6 (overrides ext)
  │                                                          │
  └──> PR-7 (atlas-reports scaffold + CLI framework)         │
            │                                                │
            │   ┌─── (PR-2..PR-5 must have landed)  ◀────────┘
            │   │
            ▼   ▼
        PR-8  (drift)         ──┐
        PR-9  (impact)         ─┤
        PR-10 (modularity)    ──┤───> PR-12 (atomic-write fixture suite)
        PR-11 (divergence)    ──┘            │
                                              ▼
                                      PR-13 (Phase 3 polyglot smoke test)
```

**Parallel-safe waves:**

- **Wave 0:** PR-0a (plan + status; this commit).
- **Wave 1 (after PR-0a):** PR-0b (design-doc touch-ups). First task of the next execution session.
- **Wave 2 (after PR-0b):** PR-1, PR-7 — independent surfaces, dispatch concurrently.
- **Wave 3 (after PR-1):** PR-2, PR-3, PR-4, PR-5 — four cache-path retrofits in parallel.
- **Wave 4 (after PR-5):** PR-6 (overrides extension).
- **Wave 5 (after PR-7 + PR-2..PR-5):** PR-8, PR-9, PR-10, PR-11 — four reports concurrently. PR-6 is helpful but not strictly required for the reports to function — they observe whatever edges the engine produces.
- **Wave 6 (after PR-8 + PR-10):** PR-12 (atomic-write fixture suite).
- **Wave 7 (after Wave 6):** PR-13 (Phase 3 polyglot smoke test).

The widest parallel wave is Wave 2 (4 PRs simultaneously). Wave 4 is
also 4 PRs wide. Both waves benefit from
`superpowers:dispatching-parallel-agents` (one Agent tool call per PR,
all in a single message).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of
what's worth recording: deviations from the plan that the next session
needs to know; surprising fixture quirks; manual verification steps that
succeeded; follow-up cleanup deferred; cache-path or schema-mutation
trail (which PR added which field, which PR moved which file).

### PR-0a
2026-05-08 — Landed: the Phase 3 plan
(`docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-plan.md`) and
this status file
(`docs/superpowers/plans/2026-05-08-phase3-status.md`). Companion to
the Phase 3 design spec
(`docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md`,
already on main from commit `02f0914`).

A new Phase-3-shaped continuation prompt at
`docs/superpowers/prompts/2026-05-08-vnext-continue.md` lands in the
same commit and replaces / deprecates the Phase-2-shaped prompt at
`docs/superpowers/prompts/2026-05-07-vnext-continue.md` (which is
moved aside or marked obsolete).

PR-0b (design-doc touch-ups in
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`) is
the first task of the next execution session. The touch-ups are
enumerated in plan §4 PR-0b and in the Phase 3 design spec §10.

Load-bearing context for Wave 2 reviewers (after PR-0b lands):

- **Greenfield carries forward across phases.** No on-disk format
  compatibility with Phase 1 / Phase 2; no migration. A user upgrading
  deletes `.atlas/` and re-runs.
- **No schema_version bump in Phase 3.** All on-disk schemas (drift
  snapshot, drift report, impact, modularity per-component +
  rollup, divergence) ship as `schema_version: 1`. The integer is
  reserved for a future breaking-change phase if it ever happens.
- **Phase 3 introduces zero new LLM call sites.** Every report is a
  deterministic projection of L4–L8 engine outputs. The cold and warm
  LLM call budgets in PR-13's polyglot smoke test must match Phase 2's
  PR-14 baseline (~26 cold, 0 warm). A subagent that introduces a new
  LLM call must surface this immediately for design review.
- **Editorial tier is fixed at six file types.** Top-level
  `overrides`, `external-components`, `subsystems`, `analyzers`,
  `config` + per-component `overrides`. PR-2..PR-5 retrofit moves
  Phase 1's `surfaces.yaml`, `component.yaml`, `components.yaml`,
  `related-components.yaml` to the cache (gitignored) tier. Anything
  not in the six editorial files is derived.
- **All cache writes are atomic** (temp+fsync+rename via PR-1's
  helper). Stateful-file writes (drift snapshot in PR-8; modularity
  history in PR-10) are particularly load-bearing — corruption from
  non-atomic writes was an explicit design-spec §6.3 concern.
- **PR-1 must land before PR-2..PR-5 dispatch.** PR-2..PR-5 each
  call `ensure_atlas_gitignore` from PR-1. Wave 2's parallel
  dispatch is conditional on PR-1 having merged.
- **PR-7 is independent of PR-1..PR-6.** It scaffolds the
  `atlas-reports` crate and CLI subcommand framework with stubbed
  `Err(NotImplemented)` handlers. Dispatching PR-7 concurrent with
  PR-1 in Wave 1 is safe.
- **The `atlas-reports` crate is intentionally pure-function**
  (design §3.5 / plan §4 PR-7). Reviewers must reject any I/O or
  Salsa-mutation introduced inside the crate; CLI handlers do all
  I/O. This keeps the Phase 5 conversion path mechanical.
- **Each retrofit PR ships a committed grep-audit script** that
  fails CI on any tracked file referencing the old (non-cache) path.
  These scripts are the canonical guard against missed readers.
- **Per-component `modularity.yaml` is hard-capped at 5 history
  entries** (FIFO). The cap is in plan §4 PR-10 and design §4.3.
  Any subagent attempting to make this configurable must surface
  the question — "modularity history depth >5" is a deferred-
  indefinitely scope item per plan §7.3.

### PR-0b
2026-05-08 — Landed: nine design-doc touch-ups to
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`.
Commit `986b63e` (+70 / −8 lines, single file). Spec-reviewed against
plan §4 PR-0b enumeration; all nine touch-ups verbatim, all five
acceptance criteria satisfied, byte-stability outside touched
sections confirmed.

Two interpretive calls, both sound and consistent with existing spec
conventions:

- §11.2.9 was added as item `9.` in the existing Markdown ordered
  list rather than as a `### 11.2.9` heading. §11.2 has always used
  ordered-list items (1.–8.); the implementer correctly matched the
  convention. Inbound `§11.2.9` cross-references in §6 stanzas
  resolve positionally.
- §6 sub-sections (§6.1–§6.4) did not previously have per-file
  tables, so the Git-status touch-up landed as a one-line
  `**Path:** … **Git status:** …` stanza per the touch-up's explicit
  fallback guidance. Each stanza records the post-retrofit cache
  path (e.g. `<scope>/.atlas/cache/components.yaml`).

Out-of-scope coherence drift surfaced by the spec reviewer (NOT
fixed in PR-0b — outside the nine-touch-up enumeration): four prose
references where "Phase 4" still implicitly means "server mode"
remain in the canonical spec at:

- §5.6 "Server mode (eventual)" header — references "Phase 4".
- §9 introduction — "Server mode is the Phase 4 target".
- §11.4 "Once Phase 4 ships and the server has concrete polyglot
  consumers".
- Glossary line ~1436 — "deferred to Phase 4 and beyond".

These are prose, not §10.X cross-references, so they don't break the
PR-0b acceptance criterion "no broken cross-references." But they're
semantically stale post-renumbering (server mode is now Phase 5 per
new §10.5). **Follow-up: a small docs-only retext PR could land
these later — not blocking on Phase 3, since none of the Phase 3
code reads §5.6 / §9 / §11.4 prose.** Recording here so the next
phase's continuation prompt can decide whether to bundle the retext.

Wave 2 ready: PR-1 (gitignore + atomic_write helper) and PR-7
(`atlas-reports` crate scaffold + CLI subcommand framework) are
independent surfaces and dispatchable concurrently. Use
`superpowers:dispatching-parallel-agents` (one Agent tool call per
PR, both in a single message).
