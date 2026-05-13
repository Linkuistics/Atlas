---
name: LLM-spine runtime is the path; deterministic engine was an error
description: User explicitly rejects "compare with deterministic-engine output" as a success-criterion frame; deterministic path is being retired, not preserved as a reference.
type: feedback
---

When designing for the LLM-spine runtime (Phase 7+), do not frame success criteria, calibration targets, or canonical-artifact rationale in terms of "comparable to deterministic-engine output" or "reference-output comparison vs deterministic baseline."

**Why:** User stated explicitly on 2026-05-13 during the production-prompt sprint brainstorm: *"I'm not interested in comparisons with our previous deterministic path. That path was an error — I'm only interested in the tree-of-agents with LLM-driven tooling support."* The Phase 7 recast (`docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`, 2026-05-11) was the architectural correction; downstream of that, deterministic-engine output is legacy artifact awaiting retirement, not a quality baseline.

**How to apply:**

- **Success criteria / calibration.** Anchor on intrinsic properties of the LLM-runtime: Lane A schema validity, evidence-score distributions, fixed-point convergence behavior, cold-token-total regression detection, audit-verdict distributions, semantic correctness of outputs against the source workspace. NOT on diff-vs-deterministic-engine.
- **Canonical schema shims** (projection-to-ontology, `L9Projection` → `components.yaml`, etc.). Justify them by "downstream Atlas tools and human reviewers consume canonical schema." NOT by "enables comparison with deterministic output."
- **Phase 8 (Cargo retirement) and later language retirements.** The deterministic classifier being retired is **not** a reference for the LLM agent that replaces it. The LLM agent is the new reference; the deterministic classifier is the thing being deleted.
- **Cross-transport parity tests.** The interesting parity is *within* the LLM-spine runtime — e.g., `http_anthropic` vs `http_openai` running the *same* production prompts and producing structurally-equivalent outputs. NOT deterministic-engine vs runtime.
- **Recast spec §11.2 conflict.** The recast spec's "reference-output comparison harness" language (Phase 8 acceptance) conflicts with this position. Treat this memory as superseding that wording until the spec is updated, and propose a spec-text amendment if/when Phase 8 brainstorming reopens that section.
- **Wider applicability.** Any future "is the LLM agent doing the right thing?" check should compare against the *workspace source of truth* (the files themselves), not against a legacy deterministic implementation.
