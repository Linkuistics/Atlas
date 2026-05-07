# Atlas vNext Phase 2 — Status

Companion to `docs/superpowers/specs/2026-05-07-atlas-vnext-phase2-plan.md`.
This file tracks per-PR completion state across sessions. The continuation
prompt at `docs/superpowers/prompts/2026-05-07-vnext-continue.md` reads
this file (via the `*phase2-plan*` wildcard match) to find the next PR
to dispatch.

**Last updated:** 2026-05-07 (PR-0 plan + status file landed; Phase 2 in flight).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]`
when the PR is reviewed and committed. Append a one-line note (date +
commit sha + anything load-bearing the next session needs to know).

- [x] PR-0  — Plan + status file (docs only)
- [ ] PR-1  — TypeScript / JavaScript surface analyser (in-process)
- [ ] PR-2  — Subprocess analyser transport (stdio JSON)
- [ ] PR-3  — Python surface analyser (first subprocess analyser)
- [ ] PR-4  — Per-analyser `analyser_id` / `analyser_version` plumbing through L3 dispatch
- [ ] PR-5  — Rust binding extractor: regex → `syn`
- [ ] PR-6  — C# surface analyser (subprocess)
- [ ] PR-7  — Dart / Flutter surface analyser (subprocess)
- [ ] PR-8  — Elixir surface analyser (subprocess)
- [ ] PR-9  — Racket surface analyser (subprocess)
- [ ] PR-10 — LispKit surface analyser (subprocess)
- [ ] PR-11 — Compose composition-edge analyser (deterministic, in-process)
- [ ] PR-12 — Shell-script LLM-fallback analyser (in-process)
- [ ] PR-13 — Phase 1 hangover bundle (L8 phantoms + PR-12-of-Phase-1 polish)
- [ ] PR-14 — Acceptance: polyglot dull-shaped fixture (smoke test)

When every box is `[x]`, Phase 2 is complete and the continuation prompt
should report success and stop.

## Dependency graph (informational; canonical in plan §4 + plan §9)

```
PR-0 ──┬──> PR-1  (TS/JS in-process)            ──┐
       │                                            │
       ├──> PR-2  (subprocess transport) ──> PR-3   ├──> Wave 2 (parallel):
       │                                            │      PR-6  (C#)
       ├──> PR-4  (id/ver plumbing)        ─────────┤      PR-7  (Dart/Flutter)
       │                                            │      PR-8  (Elixir)
       ├──> PR-5  (rust binding → syn)              │      PR-9  (Racket)
       │                                            │      PR-10 (LispKit)
       └──> PR-13 (L8 + polish bundle)              │      PR-11 (Compose)
                                                    │      PR-12 (shell-script)
                                                    │
                                                    ▼
                                      PR-14 (acceptance smoke test)
```

**Parallel-safe waves:**
- Wave 0: PR-0 (this commit).
- Wave 1 (after PR-0): PR-1, PR-2, PR-4, PR-5, PR-13 — all five concurrently. (PR-13 is independent of analyser work.)
- Wave 2 (after PR-2 + PR-4): PR-3 (must precede Wave 3 to settle the `Visibility::Conventional` / `module_path` / `attributes` schema mutations on `surfaces.rs`).
- Wave 3 (after PR-3): PR-6, PR-7, PR-8, PR-9, PR-10, PR-11, PR-12 — all seven concurrently.
- Wave 4 (after Wave 3): PR-14 (smoke test).

## Per-PR notes

Sessions append session-relevant context here as PRs land. Examples of
what's worth recording: deviations from the plan that the next session
needs to know, surprising fixture quirks, manual verification steps that
succeeded, follow-up cleanup deferred, schema-mutation trail (which PR
added which field to `surfaces.rs`).

### PR-0
2026-05-07 — Landed in same commit as the plan. The Phase 2 plan
(`2026-05-07-atlas-vnext-phase2-plan.md`) and this status file are the
two artefacts. The continuation prompt
(`docs/superpowers/prompts/2026-05-07-vnext-continue.md`) is unchanged —
its Step 1 wildcard `*phase2-plan*` matches the new plan filename and
auto-routes future sessions into Step 3 (execution).

Load-bearing context for Wave 1 reviewers:

- **Greenfield carries forward across phases.** No on-disk format
  compatibility with Phase 1; no migration. A user upgrading deletes
  `.atlas/` and re-runs.
- **No schema_version bump in Phase 2.** `SurfacesFile.schema_version`
  stays integer `1`. Each language analyser PR mutates the v1 *shape*
  freely (PR-3 adds `Visibility::Conventional`, `module_path`,
  `attributes`; PR-8 adds `ContractKind::Behaviour`; etc.). Append the
  schema-mutation contribution to the per-PR note when the PR lands so
  the trail is auditable.
- **Subprocess analysers are deterministic-only in Phase 2.** No LLM
  access from subprocess. The shell-script LLM-fallback (PR-12) stays
  in-process. Phase 3+ may add a bidirectional callback channel if
  needed.
- **Per-analyser parser library choice is per-PR and overridable by the
  subagent.** Plan §4 names a default (e.g. `swc_ecma_parser` for
  PR-1, `rustpython-parser` for PR-3, `tree-sitter-c-sharp` for PR-6,
  `syn` for PR-5); a subagent that finds the named library inadequate
  during implementation may swap to a different mature pure-Rust
  alternative, recording the swap and its rationale in the per-PR
  status note.
- **Wave 1 first-dispatch order matters slightly:** PR-2 (subprocess
  transport) and PR-4 (id/ver plumbing) are both needed by PR-3
  (Python). PR-1 and PR-5 are independent. PR-13 is fully independent
  and can be parallel-dispatched with any of Wave 1.

### PR-1
(awaiting subagent dispatch)

### PR-2
(awaiting subagent dispatch)

### PR-3
(awaiting subagent dispatch)

### PR-4
(awaiting subagent dispatch)

### PR-5
(awaiting subagent dispatch)

### PR-6
(awaiting subagent dispatch)

### PR-7
(awaiting subagent dispatch)

### PR-8
(awaiting subagent dispatch)

### PR-9
(awaiting subagent dispatch)

### PR-10
(awaiting subagent dispatch)

### PR-11
(awaiting subagent dispatch)

### PR-12
(awaiting subagent dispatch)

### PR-13
(awaiting subagent dispatch)

### PR-14
(awaiting subagent dispatch)
