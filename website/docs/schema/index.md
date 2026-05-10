---
title: atlas-contracts
---

`atlas-contracts` contains the public data-format, vocabulary, and schema crates for [Atlas](https://github.com/linkuistics/Atlas) — the contracts Atlas produces and that downstream tools consume. They are extracted into this public repository so that OSS consumers such as [Ravel-Lite](https://github.com/Linkuistics/Ravel-Lite) can depend on them without pulling in Atlas's private implementation.

Two crates are published here. **`component-ontology`** provides a host-agnostic ontology of component-relationship edges — kinds, lifecycles, and evidence grades — along with the `related-components.yaml` schema. **`atlas-index`** provides the schema and reader for the four Atlas YAML files, including rename-matching and merge-with-overrides logic.

Consume them as git dependencies pinned to a commit SHA for reproducible builds:

```toml
[dependencies]
component-ontology = { git = "https://github.com/linkuistics/atlas-contracts", rev = "<commit-sha>" }
atlas-index        = { git = "https://github.com/linkuistics/atlas-contracts", rev = "<commit-sha>" }
```

The computational parts of Atlas — the Salsa pipeline, CLI, prompt templates, and LLM wiring — remain in the private Atlas repository. This repo holds only the public vocabulary and file-format contracts.
