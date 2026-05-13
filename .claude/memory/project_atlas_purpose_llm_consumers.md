---
name: Atlas's purpose — LLM-consumed analyses of large monorepos
description: Atlas exists to produce monorepo analyses consumed by *other LLM tools* — for in-codebase agents, refactoring cues, and documentation scaffolding. The output quality bar is "useful as LLM context," not just "schema-valid."
type: project
---

Atlas's purpose, as stated by the user 2026-05-13: *"analyse large monorepos/codebases in a way that (a) allows other LLM tools to work with them efficiently and effectively, (b) provides refactoring cues to improve modularity, reusability, simplification and composability, (c) provides a structure for LLMs to generate documentation."*

**Why:** Atlas's *ultimate user is other LLMs*, not (primarily) humans reading YAML by hand. The canonical schema (`components.yaml`, `subsystems.yaml`, `related-components.yaml`, derived contracts, edges, surfaces) is consumed by downstream LLM agents (in-codebase coding assistants, refactoring tools, doc generators) that need efficient, structured context about a monorepo. This framing was previously implicit in the LLM-spine recast but not stated explicitly; the 2026-05-13 statement makes it canonical.

**How to apply:**

- **Production-prompt design.** When designing prompts that produce Atlas outputs (classify, surface, edges, reduce, project), the quality bar is "would another LLM, given this output as context, be able to act efficiently and correctly on the codebase?" Prefer concise + signal-rich output over verbose + exhaustive output. LLM-token efficiency is a quality property of the output, not just an internal cost concern.
- **Confidence grading.** Confidence grades on Atlas outputs are *consumer-facing signals* — downstream LLM consumers should be able to read a Strong/Moderate/Weak grade and adjust their behavior (trust fully / cite carefully / re-verify). This sharpens the calibration requirement: Q5-C's deterministic evidence floor isn't internal hygiene, it's a load-bearing input to downstream tools.
- **Refactoring cues** (use case b). The surface analyses + edge derivations + contract derivations should be designed to *enable* modularity / reusability / simplification / composability assessment by downstream LLM tools. Pattern detection (recast Phase 10) is the explicit feature, but even basic classify + surface + reduce outputs should preserve signal that downstream tools can mine for refactoring cues.
- **Documentation scaffolding** (use case c). The project prompt's workspace-level output, plus per-subsystem reduces, should be *structured for doc generation* — i.e., a downstream LLM tool can take Atlas output + the source files and produce coherent architecture documentation. This influences what `L9Projection` carries and what the canonical-schema shim emits.
- **Calibration framing.** Atlas-on-Atlas (and future workspace) calibration evaluates: "if a downstream LLM consumed this output as context, would it have what it needs?" Not just "did the runtime complete?" or "did Lane A pass?" Those are necessary, not sufficient.
- **Schema choices.** When designing future canonical-schema fields (Phase 8+ as classifiers retire), prefer fields and shapes that are *useful to LLM consumers*: explicit cross-references, named contracts, declared edges with kinds, evidence pointers (so consumers can verify). Avoid fields whose only purpose is human-display formatting.
- **Recast spec resonance.** The recast §1 summary implicitly assumes this purpose (map-reduce LLM analysis of large systems) but doesn't state the downstream-LLM-consumer framing explicitly. Future spec revisions should make this canonical — it's the *why* behind the LLM-spine architecture, not just the *what*.
