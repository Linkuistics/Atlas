---
name: No version on workspace path-deps
description: In-tree path-dep declarations (workspace-root [workspace.dependencies] and per-crate Cargo.toml) carry path only, no version field. Publish-time mechanism injects version externally to source.
type: feedback
originSessionId: 8802eb79-4650-489c-9915-fa2f9372e79e
---
When declaring a workspace-internal path-dep (one crate referencing another that lives in the same workspace), use **path only — never `path + version`**. Applies in two places:

- Workspace-root `Cargo.toml` `[workspace.dependencies]` block
- Per-crate `Cargo.toml` `[dependencies]` blocks that reference sibling workspace members directly via `path = "..."`

Correct: `component-ontology = { path = "../component-ontology" }`
Wrong: `component-ontology = { path = "../component-ontology", version = "0.1.0" }`

**Why:** When `version` is present alongside `path`, cargo strips the path on `cargo publish` and resolves the dependency via crates.io. That triggers the well-known two-crate publishing chicken-and-egg: a downstream crate's `--dry-run` fails with "no matching package found / location searched: crates.io index" until the upstream is actually uploaded. With path-only, `--dry-run` instead fails earlier with "dependency does not specify a version" — which is the correct, **convention-honouring pre-publish state**. The publish-time mechanism (cargo-release rewriting, just-in-time `version` injection, or equivalent) handles the version externally to source so the tree never carries an out-of-band anchor that can rot relative to the actual `[package].version`. Matches atlas-contracts upstream convention pre-fold.

**How to apply:** Surface this rule whenever a PR brief authorises edits to workspace `Cargo.toml` files (e.g., adding a new workspace member, rewriting `[workspace.dependencies]`, adding a sibling-crate dep declaration in a per-crate `Cargo.toml`). If a `cargo publish --dry-run` failure surfaces "does not specify a version" on a workspace path-dep, do **not** add the version — that's the expected pre-publish state, not a regression. The fix lives in the publish workflow, not in source. Discovered at Phase 5 PR-1 (commit `c784050` reverted a `version = "0.1.0"` add on `crates/atlas-index/Cargo.toml:13`).
