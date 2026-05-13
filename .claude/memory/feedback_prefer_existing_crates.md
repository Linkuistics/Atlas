---
name: Prefer existing crates over hand-rolled code
description: User wants existing, maintained crates used wherever reasonable, including for protocol framing, schema validation, and infrastructure code that we might otherwise hand-roll.
type: feedback
---

When designing new functionality in Atlas (and any Rust project under user direction), prefer existing maintained crates over hand-rolled implementations. This applies even when the hand-rolled code would be small or straightforward — the maintenance cost, protocol-compliance risk, and missed-optimisation cost of bespoke code typically exceed a sensible crate dependency.

**Why:** User stated 2026-05-13 during production-prompt sprint brainstorm: *"we should almost always use existing crates rather than writing our own code."* The principle generalises beyond any one sprint: hand-rolled code becomes a maintenance liability that the project carries forever, while crates absorb upstream protocol changes, security fixes, and ecosystem-wide improvements.

**How to apply:**

- **New protocol code.** JSON-RPC, MCP, gRPC, WebSocket framing — reach for a maintained crate (`rmcp`, `tonic`, `tungstenite`, etc.) rather than writing framing/parsing by hand. If the crate's API doesn't fit, *adapt to it* rather than hand-rolling.
- **Schema and parsing.** `serde_*`, `schemars`, `jsonschema`, `yaml-rust`, `toml` — these solve well-known problems; don't reimplement.
- **CLI plumbing.** `clap`, `dialoguer`, `console`, `indicatif` — standard surface.
- **Async / concurrency primitives.** `tokio` ecosystem first, then `futures` utilities. Don't hand-roll task lifecycle or channel logic when `tokio::sync` covers it.
- **Don't reimplement standard formats.** YAML, JSON, TOML, JSON Schema, JSON-RPC, MCP — all have mature crate ecosystems.
- **"Almost always"** allows judgment-call exceptions: (i) the candidate crate is unmaintained or abandoned, (ii) it doesn't compile on a needed target, (iii) it pulls in disproportionate transitive dependencies for a trivial use case, (iv) its API materially worse than 50 lines of direct code would be. *Document the exception when you take it* — don't silently prefer hand-rolled.
- **Existing hand-rolled code is not grandfathered.** If a prior PR shipped hand-rolled framing (e.g., PR-1's `mcp/mod.rs`), revisiting it to migrate to a maintained crate is a legitimate refactor — propose it explicitly rather than building further hand-rolled scaffolding on top.
- **Atlas-specific implication.** PR-1's hand-rolled MCP JSON-RPC framing in `crates/atlas-agents/src/mcp/` should be migrated to a maintained MCP/JSON-RPC crate as part of the production-prompt sprint's PR-A (subprocess MCP `serve_client`), provided a sufficiently mature crate (e.g., `rmcp`) exists at the time. Verify crate health at plan-time before committing.
