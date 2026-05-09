---
name: Atlas vNext Phase 4+ roadmap (post-Phase-3)
description: Validated phase ordering after Phase 3 ships. Phase 4 = cleanup; consolidation early (Phase 5); polish + capability waves before subprocess + LLM frontier; server mode last.
type: project
originSessionId: c700e6f1-bd91-4b5d-b7e9-b6d2304c687c
---
User validated 2026-05-09 the post-Phase-3 phase ordering. Phase 4 = cleanup release (internal code-quality + docs only). Subsequent phases sequenced "consolidation early" so future analyser / LLM work doesn't carry the multi-root tax.

**Why:** Phase 3 design §9.1 swept ~11 deferred candidates into "Phase 4" but only some are cleanups. Splitting them into focused phases keeps each phase tractable (Phase 3 was 14 PRs; bigger gets unwieldy). Consolidation at Phase 5 means six subsequent phases of analyser / LLM / subprocess work all benefit from the simpler single-root structure.

**How to apply:**

- **Phase 4** — Cleanup release (~9 PRs). Code-quality refactors (LenientBackend extraction, decoder consolidation, L8 phantom-subcomponent fix, atomic_write helper convergence, build_engine_database/build_database_for_reports convergence, sweep-test boilerplate, orphan re-export removal) + §10 retext. NO new capability, NO schema change, NO LLM call sites. Spec at `docs/superpowers/specs/2026-05-09-atlas-vnext-phase4-design.md`.
- **Phase 5** — Monorepo consolidation. Fold atlas-contracts + Ravel + Ravel-Lite into Atlas; delete multi-root machinery.
- **Phase 6** — User-facing schema cleanups. Contract rename-match, `--strict-overrides`, cache compression, worktree commit-sha annotations, `is_manifest_file` extension, `subsystem` field wiring (deferred from Phase 3 PR-9), `edges_suppress` stderr-capture test (deferred from Phase 3 PR-10).
- **Phase 7** — Per-language refinements (Dart / Racket / Elixir / LispKit depth).
- **Phase 8** — Subprocess convergence + bidirectional LLM callback + rust-analyzer integration (stretch).
- **Phase 9** — LLM-driven analyses: pattern detection + threshold calibration.
- **Phase 10** — Server mode (file watcher, gRPC + GraphQL API, subscription primitives, reactive recomputation).
- **Deferred indefinitely** — gate/strict exit-code flags, modularity-score thresholds, upstream / subsystem-input impact variants, modularity history depth >5, per-language coupling normalisation, multi-tenant SaaS hosting.

Phase 4 PR-8 retexts the canonical system-model spec's §10 with this ordering and fixes the four stale "Phase 4 = server mode" prose references in §5.6, §9, §11.4, and the glossary.
