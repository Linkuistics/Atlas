---
name: Atlas as prompts-as-application — LLM is the spine, code is the scaffolding
description: User reaffirmed Atlas's original design intent during Phase 6 brainstorm — LLM-driven analysis should be the spine, with deterministic code reserved for genuinely deterministic helpers (parsing, schema, cache infra). The roadmap's positioning of LLM-driven analyses at Phase 9 is upside-down.
type: feedback
originSessionId: e8e4bd69-2188-46c3-adaf-550f21a65b07
---
Atlas should be a "prompts-as-application" system: LLMs analyse components (read files, understand build systems, map dependencies, derive edges, classify kinds), with deterministic code reserved for tasks that are *genuinely* deterministic (parsing, file I/O, schema validation, cache infrastructure, content-sha, Salsa wiring). The primary architectural goal is a map-reduce-style mechanism that breaks large systems into small per-component LLM tasks that each fit comfortably under context limits.

**Why:** Phases 1–5 invested heavily in hand-coded per-language analysers (Rust/TS/Python/C#/Dart/Racket/Elixir/LispKit classifiers, surface extractors, contract derivers, edge emitters). By Phase 6 brainstorming the user observed the trajectory had drifted from the original intent — the roadmap had "LLM-driven analyses" at Phase 9 (second-to-last), implying LLM is polish rather than spine. The user explicitly stated: "I was really thinking of Atlas as a 'prompts-as-application' kind of system, with coded helpers where needed for easily deterministic tasks such as parsing. The primary goal of Atlas is to find a mechanism, like a map-reduce, that allows us to use LLMs to analyse large systems by breaking them into small tasks that don't consume much context."

**How to apply:**

- **When designing future analysers** (per-language work, edge derivation, surface extraction, kind classification, pattern detection): default to *LLM map-reduce task* shape, not hand-coded heuristics. The unit-of-LLM-work is per-component or per-small-region; the reducer merges into the canonical schema.
- **When proposing deterministic code**: justify *why this is genuinely deterministic* (parsing, schema, cache infra, ID derivation) and not just "easier to code than to prompt right now."
- **Roadmap items most affected**: Phase 7 (per-language refinements — full tree-sitter-dart, raco-driven Racket, Phoenix sub-kinds, Mix umbrella, LispKit symbolic resolution), Phase 8 (subprocess convergence — reframes as LLM-task subprocess management), Phase 9 (LLM-driven analyses — already named, but should arguably be earlier and broader).
- **Phase 6 itself is mostly orthogonal** to this pivot — its items are editorial-tier plumbing on user-authored YAML. The pivot bites Phase 7+, not Phase 6's content.
- **Preserve LLM-call-budget invariants**: Atlas today has strict cold/warm-call budget assertions (Phase 3 polyglot smoke test). An LLM-spine Atlas needs a calibrated budget per workspace size and a cache strategy that hits hard on no-op re-runs (warm = 0).
- **Determinism-where-it-matters**: byte-identical no-op re-runs are a load-bearing test invariant; cache-keyed LLM responses (deterministic given the same prompt + same content-shas) preserve this even when LLM is the spine.
