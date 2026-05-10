---
name: Atlas long-term monorepo consolidation
description: User intends to fold atlas-contracts (currently sibling-repo path-dep), Ravel, and Ravel-Lite into the Atlas repo, eventually deleting multi-repo / multi-root tooling support.
type: project
originSessionId: 326b77b4-5af3-410f-aa61-c3f18f3444e8
---
User stated 2026-05-08: long-term goal is to consolidate Atlas-related work
into a single repo:

1. Pull `~/Development/atlas-contracts` (currently a sibling-repo path-dep
   schema crate) back into the Atlas repo. The repo's public-artefact role is
   a *publishing* concern (publish to crates.io), not a structural reason to
   keep it separate.
2. Fold Ravel and Ravel-Lite into the same repo, making Atlas a monorepo.
3. **Drop multi-repo / multi-root tooling support entirely** as a consequence
   of the above.

**Why:** Multi-root was always a complexity tax (design.md §5.3 + §10.1
"Architectural seam"). Once everything Atlas analyses lives in one repo, the
seam stops earning its keep.

**Status:**

- **atlas-contracts in-tree: COMPLETE (Phase 5, shipped 2026-05-10).** `crates/component-ontology` and `crates/atlas-index` now live in the Atlas workspace. Multi-root machinery deleted. Final commits: `a302ce5` + `448d166`. The `~/Development/atlas-contracts` sibling repo is now redundant (see manual checklist in `docs/superpowers/plans/2026-05-10-phase5-status.md`).
- **Ravel + Ravel-Lite fold: DEFERRED.** Deferred to a later phase (post-Phase-5, name TBD). May include a Bazel build-system migration for the polyglot tree. Ravel-Lite currently path-deps `../Atlas/crates/{component-ontology,atlas-index}` (updated in PR-1 to point at Atlas rather than atlas-contracts).
- Phase 5 deleted `§5.3 Multi-root workspace` from the canonical design and the `[retired Phase 5]` note was added to `§10.1`.
