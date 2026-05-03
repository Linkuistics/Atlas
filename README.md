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

v1 delivers component discovery (see [Status](#status)). The capabilities below build on
that foundation and are still planned.

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

**v1 — component discovery is live.** The Salsa-backed query graph that drives Atlas's
fixedpoint runs end-to-end, from filesystem ingest through layered classification to the
four output YAMLs. `atlas index <root>` is the user-facing entry point.

Implemented:

- **L0 — inputs.** Filesystem seeding with `.gitignore`-aware filtering.
- **L1 — enumeration.** File tree, manifests, doc headings, shebangs, git boundaries.
- **L2 — candidate generation.** Per-directory candidate components.
- **L3 — classification.** LLM-driven `is_component` decisions, deterministic short-circuits,
  pin-based overrides, all routed through a content-addressed LLM response cache.
- **L4 — tree assembly + rename-match.** Stable id allocation (slug → suffix → SHA cascade)
  with content-SHA bipartite matching across runs.
- **L5/L6 — surface and edge proposal.** Hand-rolled JSON Schema validation against four
  prompt templates in `defaults/prompts/`.
- **L7 — graph-structural analysis.** SCCs, cliques, seam density, modularity hint.
- **L8 — sub-carve recursion.** Bounded by `--max-depth`, driven by structural signals.
- **L9 — projections.** Emits `components.yaml`, `external-components.yaml`,
  `related-components.yaml`, plus an `llm-cache.json` for byte-identical no-op re-runs.
- **CLI.** `atlas index <root>` with `--budget`, `--no-budget`, `--max-depth`, `--recarve`,
  `--dry-run`, `--model`, `--no-gitignore`. `BudgetSentinel` gates LLM calls; cache and
  stable timestamps deliver zero-diff re-runs when nothing changed.
- **Distribution.** Homebrew bottles for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`.

Not yet implemented: the higher-level pattern-detection and architectural-decomposition
work outlined in [Planned Capabilities](#planned-capabilities). v1 is the foundation;
those layers compose on top of the discovered component graph.

## Installation

```sh
brew install linkuistics/taps/atlas
```

The tap is `linkuistics/homebrew-taps`. If `brew tap` has not picked it up yet:

```sh
brew tap linkuistics/taps
brew install atlas
```

Verify:

```sh
atlas --version
# atlas 1.0.0 (v1.0.0, built 2026-04-30T01:09:23Z)
```

## Usage

Index a codebase:

```sh
atlas index /path/to/project --budget 200000
```

Outputs are written to `<root>/.atlas/` by default (override with `--output-dir`):

- `components.yaml` — discovered components, ids, classifications, surfaces.
- `external-components.yaml` — external dependencies parsed from manifests.
- `related-components.yaml` — proposed edges between components.
- `llm-cache.json` — content-addressed cache of LLM responses; commit it to make re-runs
  byte-identical and free.

Common flags:

| Flag | Purpose |
|---|---|
| `--budget <N>` | LLM token budget; required unless `--no-budget` is passed. |
| `--no-budget` | Skip the budget check (local development only). |
| `--max-depth <N>` | Cap L8 sub-carve recursion. `0` = top-level only. |
| `--recarve` | Discard prior `components.yaml` so rename-match cannot anchor stale ids. |
| `--dry-run` | Compute outputs but do not write them. |
| `--model <id>` | Override the Claude model id; defaults to `$ATLAS_LLM_MODEL` or built-in. |
| `--no-gitignore` | Disable `.gitignore` filtering; useful for fixtures without `.git`. |

`atlas index --help` prints the full surface.

### LLM providers

Per-operation routing lives in `.atlas/config.yaml` (created by `atlas init`).
Each operation's `model:` is a `<provider>/<model-id>` string. Atlas
ships with these providers:

- `claude-code/<model>` — Claude Code CLI subprocess (full agent, filesystem tools).
- `codex/<model>` — OpenAI Codex CLI subprocess (full agent, filesystem tools).
- `anthropic/<model>` — Anthropic Messages HTTP API. Ephemeral prompt caching is on by default.
- `openai/<model>` — OpenAI Chat Completions HTTP API.
- `openrouter/<provider>/<model>` — [OpenRouter](https://openrouter.ai) aggregated HTTP API
  speaking the OpenAI Chat Completions wire format. Model ids carry an extra
  slash; the rest of the string passes verbatim to OpenRouter (e.g.
  `openrouter/anthropic/claude-sonnet-4-6`).

HTTP providers (`anthropic`, `openai`, `openrouter`) currently service only
`Classify` and `Subcarve`; `Stage1Surface` and `Stage2Edges` need filesystem
access and require a subprocess provider until the tool-use loop lands.

### Pre-seeding subsystems

A *subsystem* is a named group of components with hand-drawn boundaries.
Atlas reads them from `subsystems.overrides.yaml` (alongside `.atlas/`) and
emits the resolved boundaries into `.atlas/subsystems.yaml`.

```yaml
# subsystems.overrides.yaml
schema_version: 1
subsystems:
  - id: auth
    members:
      - services/auth/*           # glob: contains '/' or '*'
      - libs/identity             # glob
      - identity-core             # id: no '/' and no '*'
    role: identity-and-authorisation
    rationale: "owns all session/token surfaces"
    evidence_grade: strong
```

A `members` entry containing `/` or `*` is treated as a path glob and
matched against component path segments; otherwise it is treated as a
component id and looked up directly. Globs that match zero components
produce a warning; id forms that don't resolve are a hard error.

After `atlas index`, the resolved boundaries appear in `.atlas/subsystems.yaml`,
one entry per subsystem with concrete member ids and a `member_evidence`
block recording how each id was matched (glob pattern or direct id).

Subsystem ids share a namespace with component ids — `atlas index` will
halt before saving if a subsystem id collides with a component id.

Validate without running the pipeline:

```sh
atlas validate-overrides
```

## Release Process

Atlas ships as Homebrew bottles built locally and uploaded to GitHub Releases. The flow
has three logical phases: **tag**, **build**, **publish**. Steps 1–4 below run from
the Atlas repo root; step 5 verifies on a fresh shell.

### One-time setup

```sh
cargo install cargo-release cargo-zigbuild
brew install zig gh
gh auth login

rustup target add aarch64-apple-darwin x86_64-apple-darwin \
                  aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu

# Tap repo for formula publication; required on first run.
git clone https://github.com/linkuistics/homebrew-taps ~/Development/homebrew-taps
mkdir -p ~/Development/homebrew-taps/Formula
```

`scripts/release-doctor.sh` checks every prerequisite above and emits remediation hints
for anything missing. It runs automatically as the first step of `release-build.sh`, but
you can run it standalone before committing to a release attempt.

### 1. Tag — `cargo-release`

`cargo release` bumps the workspace version, commits, and creates an annotated tag
matching `v{version}`. It does **not** push, and it does **not** publish to crates.io
(both behaviours are pinned in `release.toml`).

```sh
cargo release patch --execute     # 1.0.0 → 1.0.1, tag v1.0.1
# or: cargo release minor --execute / cargo release major --execute
```

Omit `--execute` for a dry run.

### 2. Push

The release commit and tag are local until you push. This is intentional — inspect
`git log -1` and `git show v<version>` first, then:

```sh
git push origin main --follow-tags
```

### 3. Build — `scripts/release-build.sh`

```sh
./scripts/release-build.sh
```

This script:

1. Re-runs `release-doctor.sh` and aborts on any missing prerequisite.
2. Asserts the working tree is clean and HEAD is on a tagged commit.
3. Builds release binaries for all four targets:
   - macOS targets via `cargo build --release --target <triple>`.
   - Linux targets via `cargo zigbuild --release --target <triple>.2.17` so the bottle
     runs on glibc 2.17+ (RHEL-7-era and newer).
4. Stages each binary alongside `LICENSE` and `README.md` into a per-target directory and
   produces `atlas-v<version>-<target>.tar.xz`.
5. Renders `target/dist/atlas.rb` from `scripts/templates/atlas.rb.tmpl`, substituting
   the per-target SHA-256 sums.

Inspect `target/dist/` before continuing.

### 4. Publish — `scripts/release-publish.sh`

```sh
./scripts/release-publish.sh
```

This script:

1. Verifies `gh` is authenticated and the rendered formula and tarballs are present.
2. Verifies tarball filenames match the current tag.
3. Creates a GitHub Release for `v<version>` on `linkuistics/Atlas` and uploads every
   `*.tar.xz` from `target/dist/`.
4. Copies `atlas.rb` into `$ATLAS_TAP_DIR/Formula/atlas.rb` (default
   `~/Development/homebrew-taps`), commits, and pushes to the tap repo.

### 5. Verify

```sh
brew update
brew install linkuistics/taps/atlas
atlas --version
```

The `--version` output should match the tag you just published (e.g.
`atlas 1.0.1 (v1.0.1, built …)`). A trailing `-dirty` means the build was made from an
uncommitted tree — never publish that.

### Rollback

If publication is botched (bad formula SHA, missing target, etc.):

```sh
gh release delete v<version> --repo linkuistics/Atlas --yes
git -C ~/Development/homebrew-taps revert HEAD && git -C ~/Development/homebrew-taps push
git tag -d v<version> && git push origin :refs/tags/v<version>
```

Then fix the underlying issue and re-run from step 1 with the same level (`patch` etc.)
so the version number advances.

## Related Projects

- **[Uplift](https://github.com/linkuistics/Uplift)** — uses Atlas's extracted patterns to guide abstraction-level refactoring
- **[Redeveloper](https://github.com/linkuistics/Redeveloper)** — packages Atlas + Uplift for the legacy enterprise modernization market
- **[PolyModalCoder](https://github.com/linkuistics/PolyModalCoder)** — visualizes code in multiple representations (FSMs, sequence diagrams, BPMN)

## License

Apache-2.0 — see [LICENSE](LICENSE).
