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

**Phase 5 shipped 2026-05-10.** Scope A (atlas-contracts in-tree) + C (multi-root delete) complete. Final commits: `a302ce5` + `448d166` on main. Status file: `docs/superpowers/plans/2026-05-10-phase5-status.md`.

**Remaining deferred work:**

- Ravel + Ravel-Lite fold is still the only remaining monorepo consolidation work. No Bazel code written in Phase 5 (scoped out by design). Ravel-Lite was updated in Phase 5 PR-1 (`820c083` on Ravel-Lite main) to path-dep `../Atlas/crates/{component-ontology,atlas-index}` instead of `../atlas-contracts/...`. The `~/Development/atlas-contracts` sibling repo is now an archive candidate (see manual checklist).
- Phase 5 design spec (`docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md`) canonical §10.5 records "Folding Ravel + Ravel-Lite into Atlas is deferred to a later phase (post-Phase-5, slot TBD), possibly tied to a Bazel build-system migration for the polyglot tree."
