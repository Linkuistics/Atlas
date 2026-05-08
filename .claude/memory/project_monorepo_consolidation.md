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

**How to apply:**

- Future phase planning: a dedicated phase will delete multi-root machinery
  and treat Atlas as single-root. Substantial deletion phase. Likely Phase 5+
  given the current §10 numbering (Phase 3 = reports; Phase 4 = convergence;
  Phase 5 = server mode; consolidation slots after that).
- Don't *over-invest* in multi-root-specific features in interim phases.
  Phase 3 (current — reports) operates on engine outputs and is unaffected.
  Phase 4 (subprocess convergence) likewise unaffected.
- atlas-contracts in-tree absorption is its own milestone; doesn't affect
  Phase 3 (contract content shas are content-derived, path-independent).
- Continuation prompt's references to `atlas-contracts` as a sibling repo
  (`/Users/antony/Development/atlas-contracts`) need updating once the
  consolidation lands.
- Phase 1's design.md §5.3 "Multi-root workspace" and §10.1 references will
  need a "deprecated as of Phase X" note when the consolidation phase lands.
