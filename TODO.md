# Atlas — TODO

Next steps as Claude Code prompts.

## 1. Choose a language and set up the project scaffold

```
Atlas is a design recovery tool that will analyse large codebases to extract architectural patterns. Choose an implementation language (Rust is a strong candidate given the ecosystem's preferences, but evaluate alternatives like Swift or a hybrid approach). Set up the project scaffold: build system, directory structure, CI, and a basic CLI entry point that accepts a path to a target codebase. No analysis logic yet — just the skeleton.
```

## 2. Implement source file discovery and parsing

```
Atlas needs to ingest source code from a target codebase. Implement a source file discovery phase: walk a directory tree, identify source files by extension/shebang, and parse them into ASTs or a language-neutral intermediate representation. Start with one or two languages (e.g., Rust and Python). Use existing parser libraries (tree-sitter is a good candidate for multi-language support). Output a summary of what was found: file count by language, top-level declarations, basic statistics.
```

## 3. Build a dependency graph extractor

```
Given parsed source files from step 2, extract a dependency graph: imports, module references, function calls across files, type references. Represent this as a directed graph data structure. Implement a simple text or DOT-format output so the graph can be visualised externally. Start with intra-project dependencies only (not third-party).
```

## 4. Implement bottom-up pattern detection

```
Using the dependency graph and parsed ASTs, implement bottom-up pattern detection. Start with structural patterns: identify clusters of types that form common design patterns (factory, strategy, observer, builder). Use heuristics based on naming conventions, structural relationships (interface + multiple implementations), and call patterns. Output detected patterns with confidence scores and source locations.
```

## 5. Implement top-down decomposition

```
Implement top-down architectural decomposition. Given a codebase, identify major subsystem boundaries using: directory structure, module/package declarations, dependency density (tightly coupled clusters vs loosely coupled boundaries), and namespace conventions. Produce a hierarchical system map: system -> subsystems -> modules -> components. Output as structured JSON.
```

## 6. Design the output format and reporting

```
Design Atlas's output format. It should be both machine-readable (JSON/structured) for consumption by tools like Uplift, and human-readable for direct use. Include: system map (hierarchical decomposition), dependency graph, detected patterns with locations, metrics (coupling, cohesion, complexity), and anti-pattern warnings. Implement a report generator that produces this output from the analysis results of steps 3-5.
```
