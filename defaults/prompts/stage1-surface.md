# Discovery — Stage 1: Extract Interaction Surface

You are analysing the component whose catalog id is **`{{COMPONENT_ID}}`**.
Its source lives at these workspace-relative paths:

`{{COMPONENT_PATHS}}`

Limit your analysis to files inside those paths. Each element of the
array above is one of the component's path segments; a component may
span more than one directory.

Your task is to read the component thoroughly and emit a structured
interaction-surface record describing how this component interacts with
the outside world — *not* what it does internally.

You have Read / Grep / Glob / Bash tools available. For large components,
you may dispatch sub-subagents to analyse specific subdirectories in
parallel, then merge their findings into your final output. Use your
judgement.

## What to extract

For each field below, include evidence from the code — do not speculate.
If a field does not apply, emit an empty list or empty string.

**List-item rule:** each list item is a bare name, path, or URL. Put
descriptive context in the `notes` field, not inline with list items.

- `purpose` — one paragraph describing what this component does, written
  from evidence in the README, main entry points, and top-level modules.
- `consumes_files` — file paths or glob patterns this component *reads*
  from the filesystem (config files, data files, plan-state files, etc.).
  Include both absolute paths and well-known relative patterns.
- `produces_files` — file paths or glob patterns this component *writes*.
- `network_endpoints` — protocols and addresses it serves or consumes.
  Use the format `<protocol>://<address-or-description>`. Examples:
  `grpc://task-service:50051`, `http://localhost/api/tasks`,
  `mcp://stdio (tool server)`.
- `data_formats` — named message types, schema IDs, struct names that
  define the data this component emits or consumes (e.g., `BacklogFile`,
  `TaskCounts`, `MyProtoMessage`).
- `external_tools_spawned` — binaries this component shells out to
  (`git`, `claude`, `cargo`, etc.).
- `explicit_cross_component_mentions` — names of *other components from the
  catalog list below* that this component directly references in its
  README, memory files, or code comments.

  STRICT RULES for this field:
  - Only include names that appear EXACTLY in the catalog list below.
    Do not paraphrase, abbreviate, or expand names.
  - Do NOT include third-party libraries, frameworks, or vendor
    applications (e.g. Raycast, swift-lispkit, React, ffmpeg) — those
    are dependencies, not catalog components.
  - Do NOT include this component's own name.
  - If no catalog components are mentioned, emit an empty list.
- `interaction_role_hints` — *optional, closed vocabulary*. Advisory
  labels this component's own prose (README, top-level doc comments,
  package description) declares about itself. Hints are priors for
  Stage 2 — they are not edges. Pick each hint only when the component
  *explicitly* presents itself in that role; leave the list empty
  rather than guess.
- `notes` — anything else relationally relevant that did not fit above.

## Role hints (optional)

The `interaction_role_hints` field takes zero or more values from this
closed vocabulary. Unknown values are rejected at parse time, so spell
them exactly:

- `generator` — emits committed source artifacts (code, schemas, config)
  that another component consumes as source.
- `orchestrator` — manages another component's lifecycle, state, or
  multi-step workflow (stronger than mere invocation).
- `test-harness` — exists primarily to exercise another component's
  behaviour end-to-end.
- `spec-document` — is itself a specification (an RFC, a protocol doc,
  an architectural design note) that other components implement.
- `spawner` — shells out to other binaries as a routine part of its
  operation (but does not necessarily own their lifecycle).
- `documented-by` — is primarily user-facing documentation for another
  component.
- `client` — calls endpoints another component serves.
- `server` — serves endpoints clients consume.
- `library` — is consumed by other components as a library dependency
  (importable API, not a standalone program).
- `tool` — is a standalone CLI / GUI / interactive utility invoked by a
  human or by other tooling.

A component may carry multiple hints when its prose legitimately
declares multiple roles. When prose is ambiguous, emit no hint — Stage 2
can still pick edges from cross-referenced surface-field evidence.

## Other catalog components

These are the names of the user's other catalogued components. Only these
names are valid values for `explicit_cross_component_mentions`:

{{CATALOG_COMPONENTS}}

## Output

Return **only** a single JSON object on stdout. No markdown fences, no
prose preamble, no trailing commentary — the calling process parses your
entire stdout as one JSON document and will error on any extra bytes.

The object must match this schema (every field is required; use an
empty list or empty string where the field does not apply):

    {
      "purpose": "<one paragraph>",
      "consumes_files": ["<glob or path>", "..."],
      "produces_files": ["<glob or path>", "..."],
      "network_endpoints": ["<protocol>://<address>", "..."],
      "data_formats": ["<name>", "..."],
      "external_tools_spawned": ["<binary-name>", "..."],
      "explicit_cross_component_mentions": ["<catalog-component-name>", "..."],
      "interaction_role_hints": ["<role from the vocabulary above>", "..."],
      "notes": "<free-form prose>"
    }

Do NOT emit `schema_version`, `component`, `tree_sha`, or `analysed_at`
— those fields are injected by the caller. Do NOT write this record to
a file on disk: stdout is the only channel consumed.
