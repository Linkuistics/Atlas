---
name: Atlas vNext Phase 4+ roadmap (post-Phase-3)
description: Validated phase ordering after Phase 3 ships. Phase 4 SHIPPED 2026-05-09 (cleanup release; -1014 net LOC). Consolidation early (Phase 5, next-up); polish + capability waves before subprocess + LLM frontier; server mode last.
type: project
originSessionId: c700e6f1-bd91-4b5d-b7e9-b6d2304c687c
---
User validated 2026-05-09 the post-Phase-3 phase ordering. Phase 4 SHIPPED 2026-05-09 as a cleanup release (internal code-quality + docs only). Subsequent phases sequenced "consolidation early" so future analyser / LLM work doesn't carry the multi-root tax.

**Why:** Phase 3 design §9.1 swept ~11 deferred candidates into "Phase 4" but only some are cleanups. Splitting them into focused phases keeps each phase tractable (Phase 3 was 14 PRs; bigger gets unwieldy). Consolidation at Phase 5 means six subsequent phases of analyser / LLM / subprocess work all benefit from the simpler single-root structure.

**How to apply:**

- **Phase 4** — *SHIPPED 2026-05-09.* 7 code/docs PRs (PR-1..PR-6 + PR-8) plus PR-0 (plan); PR-7 dropped (alias not orphan after Phase 3 PR-9 added two callers). Cumulative −1014 net LOC. Commits on main: `5e781d9` + `abb7f44` (PR-1), `d1a4378` (PR-2), `5bff442` (PR-3), `02d608d` (PR-4), `e89c55f` (PR-5), `2892a82` (PR-6), `009d7e5` (PR-8) + status commits. Status file: `docs/superpowers/plans/2026-05-09-phase4-status.md` (per-PR notes + closeout). Design spec: `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`. Phase 3 polyglot smoke test passed across every PR; LLM-call budget unchanged (cold≈40, warm/reports=0); zero new LLM call sites; six-file editorial tier preserved; `atlas-reports` stays pure-function.
- **Phase 5** — *SHIPPED 2026-05-10.* Monorepo consolidation part 1: atlas-contracts folded in-tree; multi-root machinery deleted. 7 PRs (PR-0..PR-6). Final commit: `a302ce5` (§5.X sweep) + `448d166` (closeout). Status: `docs/superpowers/plans/2026-05-10-phase5-status.md`. Design: `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md`. Ravel + Ravel-Lite fold deferred to a later phase (may include Bazel migration).
- **Phase 6** — *Next up.* User-facing schema cleanups. Contract rename-match, `--strict-overrides`, cache compression, worktree commit-sha annotations, `is_manifest_file` extension, `subsystem` field wiring (deferred from Phase 3 PR-9), `edges_suppress` stderr-capture test (deferred from Phase 3 PR-10).
- **Phase 7** — Per-language refinements (Dart / Racket / Elixir / LispKit depth).
- **Phase 8** — Subprocess convergence + bidirectional LLM callback + rust-analyzer integration (stretch).
- **Phase 9** — LLM-driven analyses: pattern detection + threshold calibration.
- **Phase 10** — Server mode (file watcher, gRPC + GraphQL API, subscription primitives, reactive recomputation).
- **Deferred indefinitely** — gate/strict exit-code flags, modularity-score thresholds, upstream / subsystem-input impact variants, modularity history depth >5, per-language coupling normalisation, multi-tenant SaaS hosting.

Phase 4 PR-8 retexts the canonical system-model spec's §10 with this ordering and fixes the four stale "Phase 4 = server mode" prose references in §5.6, §9, §11.4, and the glossary.
