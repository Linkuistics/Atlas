# Component-kind vocabulary — analyst reference

This document is the human/LLM-readable companion to
[`component-kinds.yaml`](component-kinds.yaml). The YAML is the
machine-readable source of truth that the engine and prompts consume;
this Markdown is intended for hand-off to a human analyst or to a
peer LLM doing classification by hand. The two files **must stay in
sync** — when a kind is added, renamed, or has its description
revised, update both.

When using this document as a prompt fragment, copy the sections
below directly. The slugs are stable and the descriptions are
self-contained.

---

## How to use the vocabulary

When classifying a directory as a component, choose **exactly one**
kind from the list below. Output the kebab-case slug verbatim
(e.g. `rust-library`, not `Rust Library`). If multiple kinds
plausibly fit, prefer the more specific one; the language-agnostic
kinds (`service`, `website`, `installer`) are fall-backs for cases
where a more specific `<lang>-*` form does not apply.

The implementation language goes in a separate `language` field —
do **not** encode language information in the kind beyond what the
slug already carries. For example, a Python codegen tool is
`kind: codegen-tool`, `language: python` — *not*
`python-codegen-tool`.

Components are always **directories**, never files. A repository
containing many shell scripts in one directory is a single
`shell-scripts` component, not one component per file.

---

## Rust

- **`rust-library`** — A Rust crate exposing a library interface.
  Recognised by `Cargo.toml` containing a `[lib]` section with no
  `[[bin]]` section, or by `src/lib.rs` alongside a manifest.

- **`rust-cli`** — A Rust crate that produces one or more executable
  binaries. Recognised by `Cargo.toml` containing a `[[bin]]`
  section, or by `src/main.rs` alongside a manifest. Use this for
  any Rust crate whose deliverable is an executable, regardless of
  whether it is interactive or batch.

- **`rust-proc-macro`** — A Rust procedural-macro crate. Recognised
  by `Cargo.toml` with `[lib] proc-macro = true`. Compile-time only;
  the crate ships no runtime artefact.

## Node.js / TypeScript / React

- **`node-library`** — A Node.js or TypeScript package exposing a
  library interface. Recognised by `package.json` with a `main` or
  `exports` field and no `bin` field. Whether the source is JS or TS
  is recorded in the `language` field, not the kind.

- **`node-cli`** — A Node.js or TypeScript package that installs one
  or more CLI executables. Recognised by `package.json` with a
  `bin` field.

- **`react-app`** — A React-based frontend application — typically
  Create React App, Next.js, Remix, or a Vite-based React project.
  Distinguished from `website` by an explicit React-framework
  manifest or a top-level `react` / `react-dom` runtime dependency.
  Use `website` for framework-agnostic frontends.

- **`react-library`** — A library of reusable React components,
  distributed as an npm package. Recognised by `package.json`
  declaring `react` as a `peerDependency` plus a library-style entry
  point.

## Python

- **`python-library`** — A Python library package — recognised by
  `pyproject.toml` or `setup.py` without a runnable entry point.

- **`python-app`** — A runnable Python application — typically
  `pyproject.toml` declaring `[project.scripts]` or
  `[tool.poetry.scripts]`, or a top-level `__main__.py`.
  Distinguished from `python-library` by a clear executable entry
  point.

## Dart / Flutter

- **`dart-library`** — A Dart package exposing a library interface.
  Recognised by a `pubspec.yaml` whose `lib/` tree is the public
  surface, with no top-level `bin/` directory.

- **`dart-app`** — A non-Flutter Dart application — `pubspec.yaml`
  plus a `bin/` directory containing the entry point. Use
  `flutter-app` when the manifest depends on the Flutter SDK.

- **`flutter-app`** — A Flutter application targeting iOS, Android,
  web, or desktop. Recognised by `pubspec.yaml` declaring
  `flutter:` as a top-level key and depending on the `flutter`
  SDK. Distinct from `dart-app` because Flutter has its own
  build/deploy chain.

## .NET

- **`dotnet-library`** — A .NET class library. Recognised by a
  `.csproj` / `.fsproj` / `.vbproj` whose `<OutputType>` is
  `Library` (the default) and which has no `Main` entry point.

- **`dotnet-service`** — A .NET long-running service. Recognised by
  a project file whose `<OutputType>` is `Exe` and which uses a
  service host (e.g., `Microsoft.AspNetCore.App` framework
  reference, or a `Program.cs` calling `IHostBuilder`).

## Web frontends and services (language-agnostic)

- **`website`** — A framework-agnostic static or server-rendered
  web frontend (e.g., a docs site, a Hugo / Jekyll / Astro project)
  that does not declare itself as a React app. Use `react-app` for
  React frontends.

- **`service`** — A long-running server process whose internal
  language is either unknown or not yet captured by a more specific
  `<lang>-service` kind — typically a Dockerised opaque binary or a
  polyglot server.

## Infrastructure, packaging, and scripts

- **`docker-image`** — A directory rooted at a single `Dockerfile`
  that builds one container image. The image's contents may
  themselves be a separate component (e.g., a `node-cli` packaged
  into the image); this kind describes the packaging artefact, not
  its payload.

- **`docker-compose-bundle`** — A directory whose primary artefact
  is a `docker-compose.yml` (or `compose.yaml`) declaring a
  multi-image deployment. Used for grouping a set of related
  services into one component.

- **`installer`** — A directory whose purpose is producing an
  end-user installer package — e.g., a WiX MSI definition, a `.dmg`
  build script, a Debian `.deb` packaging tree, or an RPM spec.
  Distinct from `docker-image` because installers target host
  installation rather than container runtime.

- **`shell-scripts`** — A directory of standalone shell scripts
  (bash, zsh, sh) treated as one component. Plural by convention
  because a single shell file is not itself a component.

- **`sql-scripts`** — A directory of standalone SQL scripts —
  typically migrations or seed data — treated as one component.

- **`codegen-tool`** — A code-generation tool whose output is
  consumed by other components — e.g., a schema compiler, a
  scaffolding generator, or a template processor. The tool's
  implementation language lives in the `language` field.

## Repository-shape and special kinds

- **`workspace`** — A multi-crate or multi-package workspace root.
  Recognised by `Cargo.toml` with `[workspace]`, a
  `pnpm-workspace.yaml`, or similar workspace manifests. The root
  is a component in its own right; its members are child
  components.

- **`config-repo`** — A repository that exists primarily to hold
  configuration (dotfiles, infrastructure-as-code, declarative
  deployment specs) rather than code.

- **`docs-repo`** — A repository whose purpose is documentation —
  prose, reference material, or a specification authored in
  Markdown or similar.

- **`spec`** — A specification document or collection of documents
  describing behaviour that other components implement.
  Distinguished from a `docs-repo` by its role as a contract rather
  than reference prose.

- **`external`** — A third-party component referenced from a
  manifest (e.g., a published crate or npm package). Externals are
  recorded separately from internal components.

- **`non-component`** — A directory that looks like a candidate on
  paper but is confirmed not to be a component — for example, a
  `.git`-rooted directory whose contents are just metadata, or an
  empty placeholder.

---

## Mappings to avoid

Do not invent these slugs — they belong to the canonical form on
the right. The list captures the most common LLM-invented variants
seen in the wild.

| Tempting label | Use instead |
|---|---|
| `rust-binary` | `rust-cli` |
| `node-package` | `node-library` |
| `python-package` | `python-library` |
| `dart-package` | `dart-library` |
| `typescript-library` | `node-library` *(record `typescript` in `language`)* |
| `wix-installer`, `msi-installer`, `deb-package`, `dmg` | `installer` |
| `shell-script` *(singular)* | `shell-scripts` *(components are directories)* |
| `sql-script` *(singular)* | `sql-scripts` |
| `python-codegen-tool`, `rust-codegen-tool` | `codegen-tool` *(language goes in `language`)* |
| `python-cli` | `python-app` *(or `codegen-tool` if it generates code)* |
| `python-service`, `node-service` | `service` *(language-prefixed services exist only for `dotnet`)* |

---

## Keeping this document in sync

The canonical vocabulary lives in
[`component-kinds.yaml`](component-kinds.yaml). Three places must
move together when a kind is added, renamed, or removed:

1. `defaults/component-kinds.yaml` — the YAML data form. Drives the
   `{{COMPONENT_KINDS}}` token in the classification prompt.
2. `crates/atlas-engine/src/types.rs` — the `ComponentKind` enum
   and its `as_str` / `parse` / `all` mappings. A drift test in
   `crates/atlas-engine/src/defaults.rs` asserts bijection with the
   YAML and fails the build on divergence.
3. This document.

Steps 1 and 2 are enforced by the build; step 3 is by convention —
revise this file in the same change.
