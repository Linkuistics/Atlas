---
name: Phase 6 brainstorm paused for LLM-spine recast
description: Phase 6 brainstorm (started 2026-05-10) suspended mid-design. Next strategic brainstorm is the LLM-spine architecture recast; §10 roadmap retext lands at the end of that brainstorm; Phase 6 (or its replacement) resumes after.
type: project
originSessionId: e8e4bd69-2188-46c3-adaf-550f21a65b07
---
**SUPERSEDED 2026-05-11.** The four candidate items shipped as Phase 6 per `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md` (PR-2 + PR-3 + PR-4; PR-1 deferred to Phase 9c on polyglot-fixture pre-flight). The LLM-spine recast begins in Phase 7 per `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`. This memory retained for forensic context.

Phase 6 brainstorm reached PR-2's design section (Contract rename-match owner-follows) before the user surfaced the load-bearing concern: Atlas's trajectory has drifted from its original prompts-as-application intent. The roadmap had LLM-driven analyses at Phase 9 (second-to-last); the user wants LLM as the spine, with deterministic code reserved for genuinely deterministic helpers (parsing, schema, cache). See `feedback_atlas_llm_spine_intent.md` for the strategic preference.

**Decision (2026-05-10):** Option B — pause Phase 6, brainstorm the LLM-spine recast first, retext canonical §10 as part of that brainstorm, then return to Phase 6 (or its replacement) as a normal design task.

**Phase 6 candidate items reached before pivot (not committed; may survive a recast):**

1. **`is_manifest_file` Makefile/shell extension** — add `Makefile`/`makefile`/`GNUmakefile`/`*.mk`/`*.sh` to the recognition table. No paired classifier (B1 purity). Smallest item.
2. **Contract rename-match owner-follows** — when component rename-match maps `prior_id A → new_id B`, contracts owned by `A` follow to `B`. Owner-follows only; independent fuzzy contract matching deferred to Phase 7+ (blocked on §11.2.2 content-sha canonicalisation). α/β implementation choice (id-embeds-owner vs content-sha-stable) picked at plan-time.
3. **`subsystem` field overlay** — wire the parsed-but-ignored per-component `subsystem:` override as an overlay on `subsystems.overrides.yaml`. Co-located authoring; central file canonical with overlay-aware resolution. Adds a new "override names non-existent component" warning class.
4. **`--strict-overrides` + closed enumeration + dual-mode contract test** — escalates a *closed* enumerated list of override warnings (scope-violation, edges_suppress no-match, plus #3's new warning) to errors with non-zero exit. The deferred Phase 3 PR-10 stderr-capture test for `edges_suppress` becomes the strict-mode contract test (item #5 from canonical §10.6 folded into this PR).

**Items struck from Phase 6 scope during brainstorm:**

- Worktree commit-sha annotations (§11.2.8) — *dropped* (motivation evaporated with Phase 5's multi-root collapse).
- Cache compression (§11.2.7) — *deferred to its own cache-architecture phase*; orthogonal to user-authoring concerns.
- Make/shell classifier — *deferred to Phase 7+* (per-language work).
- Independent fuzzy matching for contracts — *deferred to Phase 7+* (blocked on §11.2.2).

**Approach 3 (dependency-ordered, parallel-safe wave) had been picked for PR sequencing.** Wave-1: PR-1 (manifest extension) + PR-2 (rename-match owner-follows) parallel. Wave-2: PR-3 (subsystem overlay) sequential. Wave-3: PR-4 (`--strict-overrides`, depends on PR-3's full warning surface). Wave-4: PR-5 (acceptance + closeout). Plus PR-0 (plan + status + continuation prompt) at the head.

**Why:** Capturing what was reached pre-pivot avoids losing analysis already done. These items are mostly orthogonal to the LLM/code balance (editorial-tier plumbing on user-authored YAML), so they may survive the recast — but they should be reviewed in that light before plan-writing, not after.

**How to apply:**

- When starting the LLM-spine brainstorm, treat the four items as candidate work that's been pre-analysed but not committed. After designing the LLM-spine recast, ask: do these items ship as a Phase 6 *pre-pivot* release? Do they fold into a recasted Phase 6? Or do they defer?
- When retexting canonical §10 in `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`, note this transition explicitly (Phase 6 paused mid-design 2026-05-10; resumed under LLM-spine premise).
- The LLM-spine brainstorm itself needs to answer: (i) what's the map-reduce primitive's unit-of-work shape (per-component? per-file? per-subsystem?); (ii) what stays deterministic vs becomes LLM-driven; (iii) how byte-identical no-op re-runs are preserved (deterministic prompt → cached response keyed on content-sha); (iv) the LLM-call budget calibrated against workspace size; (v) how Phase 7's per-language refinements, Phase 8's subprocess convergence, and Phase 9's pattern detection retext under the new premise.
- No design spec was written for Phase 6 — the brainstorm stopped before the `docs/superpowers/specs/2026-05-10-phase6-design.md` write step. The decisions above live only in memory + this session's transcript.
