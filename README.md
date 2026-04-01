# Atlas

> Design recovery and architectural pattern extraction for large codebases.

A [Linkuistics](https://github.com/linkuistics) project.

---

## Vision

Atlas will extract a hierarchic, pattern-based description of large software systems by working both top-down and bottom-up, finding common architectural and code patterns at multiple levels of abstraction. The result will be a complete map of a codebase — hence the name.

## Why Atlas Exists

Large codebases are hard to understand. Documentation drifts, architecture diagrams go stale, and tribal knowledge leaves with departing team members. Atlas aims to recover the *actual* architecture — not what someone wrote on a whiteboard two years ago, but what the code actually does today.

Atlas is designed as a standalone discovery tool: sometimes understanding the system is the entire goal. When paired with [Uplift](https://github.com/linkuistics/Uplift), the extracted patterns could become the foundation for architecture-level refactoring.

## Planned Capabilities

The following capabilities are planned but not yet implemented.

### Multiple Levels of Abstraction

- **System level** — major subsystems, their boundaries, and communication patterns
- **Module level** — packages/crates/namespaces, their responsibilities, and dependency structure
- **Class/Type level** — type hierarchies, trait implementations, protocol conformances
- **Function level** — call graphs, data flow, algorithmic patterns

### Bidirectional Analysis

- **Top-down** — decompose the system into subsystems, each subsystem into modules, each module into components
- **Bottom-up** — identify recurring patterns in the code (factories, strategies, observers), group related code into conceptual units, compose units into architectural descriptions

### Cross-Cutting Patterns

- Architectural patterns (MVC, pipeline, event-driven, microservice boundaries)
- Design patterns (strategy, observer, factory, builder, visitor)
- Anti-patterns and code smells (god classes, feature envy, circular dependencies)
- Idiom patterns (language-specific conventions and their usage)

## Use Cases

- **Onboarding** — new team members get a navigable map of the codebase on day one
- **Due diligence** — evaluate a codebase's architecture before acquisition or major investment
- **Modernization planning** — understand what exists before deciding what to change (see [Redeveloper](https://github.com/linkuistics/Redeveloper))
- **Documentation recovery** — generate accurate architecture documentation from code that has none

## Status

**Pre-development.** No implementation exists yet. This repository currently contains only project vision and planning.

## Related Projects

- **[Uplift](https://github.com/linkuistics/Uplift)** — uses Atlas's extracted patterns to guide abstraction-level refactoring
- **[Redeveloper](https://github.com/linkuistics/Redeveloper)** — packages Atlas + Uplift for the legacy enterprise modernization market
- **[PolyModalCoder](https://github.com/linkuistics/PolyModalCoder)** — visualizes code in multiple representations (FSMs, sequence diagrams, BPMN)

## License

Apache-2.0 — see [LICENSE](LICENSE).
