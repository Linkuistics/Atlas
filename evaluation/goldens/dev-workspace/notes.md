# dev-workspace golden notes

Hand-authored on 2026-04-24 as the first golden for Atlas's evaluation
harness. Target: `/Users/antony/Development/`.

## Scope of this golden

Top-level only. Every top-level entry is one golden component; the
granularity deliberately matches what a human glancing at the folder
would describe, not what the Atlas pipeline will ultimately produce
(sub-carving, fixedpoint, per-crate components). That difference is
the metric gap the evaluation is meant to measure.

`content_sha` is `hand-authored-placeholder` for every segment — real
SHAs live in the tool's output. Metrics that depend on content_sha
(rename stability across runs) are defined on tool-output pairs, so
placeholder SHAs in the golden are fine.

## Why each boundary is where it is

- **Atlas**, **Ravel-Lite**, **mdformat-rs**, **APIAnyware-MacOS**,
  **zellij-alt-nav**, **HeavyMentalToolkit** have unambiguous build
  manifests and are obvious top-level boundaries.
- **ravel-lite-config** has no manifest but ships yaml/markdown
  consumed by Ravel-Lite at runtime. Classified as
  `configuration-bundle` — not a build target, not a doc-only spec.
- **Ravel**, **Modaliser**, **Modaliser-Racket**: Scheme/Racket
  projects without `info.rkt` at the top level. Evidence grade `weak`
  for Ravel and Modaliser until a pass of the source confirms the
  language and role.
- **TheGreatExplainer**, **0-docs**, **Roadmap**, **InTheLoop**,
  **TestSubject**: README+TODO shape with no build manifest → kind
  `spec`. Atlas's pipeline would likely classify them the same way.
- **www.linkuistics.com**: static HTML site, no `package.json` → kind
  `static-site` rather than `node-workspace`.
- **Branding**, **MaxLovesBlue**: art/design asset collections, not
  code. `asset-collection` / `art-asset-collection`. v1 has no
  lifecycle role that fits cleanly, so `design`/`runtime` were chosen
  to keep the field non-empty.
- **IDEs**, **Legacy**: collections of sub-projects. v1 treats them as
  single components with kind `monorepo-collection`. v2 should expand
  them into per-sub-project components — that work is left for when
  the tool's sub-carving is good enough that hand-enumeration is
  wasteful.
- **homebrew-taps**: Homebrew tap with a `Formula/` directory; classed
  as `package-distribution` + `ruby`.

## Edges

Only three edges are recorded, covering the relationships I am most
confident about (the Atlas/Ravel-Lite migration axis). Per design
§8.1, exhaustive edges are not required for v1 — the goal is to have
*some* strong-evidence edges to drive precision/recall non-trivially.

## Deliberate disagreements with `.git` structure

- **None recorded.** Every top-level entry here corresponds to its own
  git repo (checked informally), so git structure and component
  structure agree at this granularity. When sub-carving lands, the
  cases to watch for are Atlas and Ravel-Lite's `LLM_STATE/` subtrees,
  which are separate git-managed plan state and should not appear as
  child components.

## Known limitations

1. **No sub-components.** Atlas has 5–6 internal crates;
   HeavyMentalToolkit has multiple packages; Ravel-Lite has its own
   crates. When the tool is compared against this golden, expect many
   "spurious" tool components (the sub-crates) that simply aren't in
   the golden. This is intentional for v1 and should be tightened in a
   later iteration.
2. **Weak-evidence classifications.** Entries graded `weak` or
   `medium` are provisional and may be wrong. Inspect each when the
   metrics report flags a kind mismatch against this golden.
3. **`content_sha` placeholders.** Rename-match metrics are not
   meaningful against this golden. Run the tool twice and diff the
   outputs for rename stability instead.
