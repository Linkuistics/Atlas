---
name: Phase 5 split + Ravel/Ravel-Lite Bazel intent
description: Phase 5 = atlas-contracts fold + multi-root delete only. Folding Ravel + Ravel-Lite is deferred to a later phase that may also migrate the build system to Bazel.
type: project
originSessionId: 59c1d765-ecdc-4f31-971e-3aa9bb256513
---
User decided 2026-05-10 to split the original Phase 5 scope (atlas-contracts in-tree + Ravel/Ravel-Lite in-tree + multi-root delete) into two phases:

- **Phase 5** — atlas-contracts in-tree + multi-root delete only. Tight causal chain (atlas-contracts moving in-tree is the prerequisite that makes deleting multi-root low-risk).
- **Later phase (post-Phase-5, name TBD)** — fold Ravel (Elixir) + Ravel-Lite (Rust) into the Atlas repo. *This phase may include a build-system migration to Bazel* so the polyglot (Rust + Elixir + vendored Lua) tree builds under a single tool.

**Why:** Ravel/Ravel-Lite consolidation is independent from the multi-root deletion (Ravel-Lite is the *consumer* of multi-root via path-dep, not the engine). Bundling would inflate the phase and couple unrelated risk surfaces. Bazel-curiosity for the polyglot tree is the user's signal — not a commitment, but a likely direction.

**How to apply:**

- Phase 5 design spec must scope to A + C only; explicitly call out that Ravel/Ravel-Lite folding is deferred and may involve Bazel migration.
- After Phase 5, Ravel-Lite still path-deps `../atlas-contracts/crates/{component-ontology,atlas-index}`. That path no longer exists post-fold, so Phase 5 must define a transition story for external consumers (Ravel-Lite, plus crates.io publishing).
- Update `project_monorepo_consolidation` and `project_phase4_plus_roadmap` references to reflect this split when the Phase 5 design spec is approved.
- Don't write or recommend Bazel-related code in Phase 5 itself; only in the deferred polyglot-fold phase.
