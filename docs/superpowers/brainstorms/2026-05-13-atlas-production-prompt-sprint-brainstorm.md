# Atlas vNext — Production-prompt sprint (brainstorm)

Status: brainstormed 2026-05-13. This is a *brainstorm artifact*, not a plan.
The canonical sprint plan, status file, and continuation prompt are PR-0 work
downstream of this brainstorm. The sprint is the empirical follow-up to Phase 7,
which shipped 2026-05-12 (final commit `bd2bb74` on Atlas main); it replaces
the three `PR-7-WIRES-REAL-*` stubs with production prompts, wires
cross-provider audit, ships a canonical-schema shim, and calibrates
Atlas-on-Atlas. Phase 8 (Cargo retirement) brainstorming is gated on items 1–4
of this sprint landing.

---

## 0. Reading order

§1 (one-paragraph summary) → §2 (architectural framings + PR-0 decision
table, 15 rows) → §3 (wave structure: 5 sequential + 2 parallel + PR-0 plan) →
§4 (Wave 1: PR-1 foundation) → §5 (Wave 2: PR-2 dispatch prompts) →
§6 (Wave 3: PR-3 producer prompts + canonical-schema shim) →
§7 (Wave 4: PR-4 cross-provider auditor) →
§8 (Wave 5: PR-5 Atlas-on-Atlas calibration + closeout) →
§9 (Parallel track: PR-A subprocess MCP + PR-B disallowed-tools probe) →
§10 (testing strategy) → §11 (acceptance criteria) → §12 (open risks) →
§13 (references).

---

## 1. Summary

The sprint replaces the three `PR-7-WIRES-REAL-*` stubs that PR-7 left in
place (`runtime/dispatch.rs:203`, `runtime/dispatch.rs:254`,
`runtime/mod.rs:665`) plus the classify/reduce/project prompts at
`runtime/mod.rs:~910/~920` with **production prompts**. It wires the
**cross-provider auditor** (Anthropic↔OpenAI per memory
`feedback_cross_provider_llm_audit`) by populating PR-7's deferred
`for_provider: None` with a real closure backed by `BackendRouter`. It
extends **Lane A** beyond schema validation to a two-layer validator that
also computes a **per-stage deterministic evidence score** against the
producer's transcript and clamps the LLM's self-graded confidence
(`Strong | Moderate | Weak | Declines`) to what the evidence supports. It
ships a **canonical-schema shim** that maps the runtime's `L9Projection`
into the canonical `components.yaml` / `subsystems.yaml` /
`related-components.yaml` artifacts that downstream Atlas consumers (other
LLM tools) read. It migrates PR-1's hand-rolled MCP JSON-RPC framing to the
`rmcp` crate (per memory `feedback_prefer_existing_crates`) and ships the
subprocess MCP `serve_client` driver so `atlas index --agent-runtime` works
against the canonical `claude_code + codex` config (today it hard-errors).
It calibrates Atlas-on-Atlas against intrinsic properties (schema validity,
evidence-score distribution, convergence behavior, cold-token total, audit
verdict distribution) — not against the deterministic engine, which is
being retired (memory `feedback_no_deterministic_engine_comparison`).
Execution: **5 sequential PRs + 2 parallel + PR-0 plan = 8 PRs total**,
matching Phase 7's cadence; PRs 1–4 are the gating set that unblocks
Phase 8 brainstorming; PR-5 plus the parallel PR-A/PR-B can ship in any
order after PR-1 and may overlap Phase 8 work.

---

## 2. Architectural framings and PR-0 decision table

### 2.1 Architectural framings (drive the entire design)

These are the cross-cutting position statements that gate individual
decisions. They are *not* PR-0 decisions in the row-by-row sense — they
condition every row below.

1. **LLM-spine runtime is the path; the deterministic engine is being
   retired.** Atlas's production-prompt design, calibration targets, and
   downstream success criteria all anchor on intrinsic properties of the
   LLM-spine runtime: Lane A schema validity, evidence-score
   distributions, fixed-point convergence, audit verdict distributions,
   cold-token regression detection. Per-component diff against the
   deterministic engine is **not** a success criterion or rationale.
   Source: user statement 2026-05-13 + memory
   `feedback_no_deterministic_engine_comparison`. The recast spec §11.2's
   "reference-output comparison harness" language conflicts with this
   position; §12 risk #4 documents the resolution path.

2. **Atlas's outputs feed other LLM tools.** The canonical schema
   (`components.yaml`, `subsystems.yaml`, `related-components.yaml`,
   derived contracts/edges/surfaces) is consumed by downstream LLM agents
   for (a) in-codebase work, (b) refactoring cues, (c) documentation
   generation. The quality bar for production-prompt outputs is *"useful
   as LLM context"* — concise, signal-rich, structurally clean — not just
   "schema-valid." Source: user statement 2026-05-13 + memory
   `project_atlas_purpose_llm_consumers`. This sharpens what the project
   prompt's output must contain (§6.4).

3. **Prefer existing maintained crates.** Hand-rolled protocol /
   schema / async / CLI code is a maintenance liability the project
   carries forever; crates absorb upstream protocol changes, security
   fixes, and ecosystem improvements. PR-1's hand-rolled MCP framing
   is a legitimate refactor candidate, not grandfathered. Source: user
   statement 2026-05-13 + memory `feedback_prefer_existing_crates`. PR-A
   §9 enacts this for MCP.

4. **Cross-provider audit is load-bearing.** Lane B fires on
   `Weak | Declines` confidence grades and routes producer (Anthropic) to
   auditor (OpenAI) or reciprocally per memory
   `feedback_cross_provider_llm_audit`. With this sprint's Q5-C
   deterministic evidence floor, the auditor's role narrows to *semantic
   critique given the evidence trail* — not coverage check (which is
   Lane A's job). The audit prompt receives the producer's output + the
   producer's transcript rendered as ordered tool-call tuples.

### 2.2 Decision table (15 rows)

| # | Dimension | Resolution | Locked in / by |
|---|---|---|---|
| 1 | Final-output envelope for production prompts | **JSON-in-text.** Prompts emit one fenced ```json block whose body deserializes to the target struct via existing `serde_json::from_value` scaffolding at `runtime/dispatch.rs:306, :327`. Lane A retries on `LlmOutputMalformed`. Symmetric across HTTP and (future) subprocess transports. | PR-2 + PR-3 prompt template body |
| 2 | Schema advertisement inside prompt | **Schema-in-prompt for all four stages.** Each prompt embeds a Rust-style type definition or JSON schema fragment of the target struct (`SubsystemsOverrideFile`, `ComponentsOverrideFile`, plus new typed shapes for classify / reduce / project). Unit test asserts each `build_*_prompt` site's embedded schema matches the live `schema_for!(TargetStruct)` (drift catcher). | PR-2 + PR-3 prompt templates + drift tests |
| 3 | Tool catalog scope per agent call | **Per-stage catalog.** Dispatch agents see `query_l1_index`, `list_dir`, `query_existing_overrides`, `read_file`. Classify agents see `read_file`, `parse_<all-manifests>`, `classify_<all-languages>`. Surface agents see `read_file` + `surface_<all-languages>` + (where applicable) `find_pub_items`, `find_imports`. Reduce/project agents see `lookup_neighbour_surface`, `query_l1_index`. Per-stage catalog sha is the per-stage `tool_catalog_sha` discriminator in the transcript-cache fingerprint (recast §6.1). | PR-2 + PR-3 catalog construction; one-line "applicable when:" docstring discipline on every `Tool::json_schema().description` |
| 4 | Per-agent iteration budget | **Per-stage hard caps + soft guidance in prompts.** Initial values (calibrated upward in PR-5): dispatch=30, classify=12, surface=25, reduce/project=8. Soft caps in prompts ≈ half of hard. `MaxStepsExceeded` is hard fail (not retry). The `build_*_prompt` functions accept the cap as a parameter so prompt text and `AgentRequest::max_steps` cannot drift. | PR-2 + PR-3 prompt construction; PR-5 calibration |
| 5 | Confidence rubric + Lane A evidence-score floor | **Outcome-driven rubric with deterministic floor.** Each stage's prompt embeds an evidence rubric (§5.4, §6.4 specifics). Lane A computes a per-stage evidence score from `transcript.tool_calls[]` (e.g., classify: `1.0 if read_file(primary_manifest) else 0.5 if classify_tool_called else 0.0`; surface: `items_inspected / items_declared`). The deterministic floor *clamps* the LLM's self-grade: claimed `Strong` with evidence score `<0.9` downgrades to `Moderate`, etc. The LLM may grade *lower* than the deterministic max (legitimately uncertain despite full evidence), but never higher. Threshold ladder: ≥0.9 max Strong; ≥0.5 max Moderate; ≥0.1 max Weak; <0.1 max Declines. | PR-2 (dispatch scoring) + PR-3 (classify/reduce/project scoring) |
| 6 | Audit prompt input shape | **Producer output + producer transcript** (rendered as ordered `(tool_name, args_summary, result_summary)` tuples — not raw JSON-RPC frames). Auditor verifies *semantic soundness given the evidence trail*. With Q5's deterministic floor, Lane B fires correlate with small transcripts (evidence thin by construction), so worst-case audit token cost is bounded. | PR-4 auditor prompt template + transcript renderer |
| 7 | Producer + auditor model pairing | **Opus 4.7 (Anthropic) primary; GPT-5-Codex (OpenAI) cross-provider auditor.** No model downgrade tier (recast §8.4); sprint commits to Opus from day 1 including during prompt-engineering iteration. HTTP transports `http_anthropic` + `http_openai`. | PR-1 `--config <path>` example file; PR-4 auditor wiring |
| 8 | Audit verdict failure modes | **`{Accept, RequestRevision, HardFail, Skipped}`.** `RequestRevision` threads the auditor's textual reason back as a system-prompt addendum on the producer's retry. Cumulative retry cap = 2 per agent (Lane A + Lane B combined; recast §4.3). Auditor emits verdict only — no auditor-side confidence grade (avoids auditor-of-auditor regress). | PR-4 auditor prompt + revision-prompt path |
| 9 | `for_provider` plumbing | **Sibling method.** Add `BackendRouter::backend_for_provider(provider: Provider) -> Option<&Arc<dyn LlmBackend>>`; leave `from_dispatch_table` as `#[cfg(test)]`. PR-1 also constructs the `Arc<ForProviderFn>` closure (`runtime/mod.rs:356` type alias) inside `atlas-cli` from a built `BackendRouter` reference. | PR-1 `BackendRouter` extension + atlas-cli wiring |
| 10 | HTTP-backend config infrastructure | **`--config <path>` flag** + checked-in `.atlas/config.sprint.example.yaml` (no keys; `${ANTHROPIC_API_KEY}` / `${OPENAI_API_KEY}` env-var substitution). Developers `cp` to `.atlas/config.sprint.yaml` (gitignored) and supply their keys. Avoids overwriting canonical `.atlas/config.yaml` (claude_code + codex). | PR-1 atlas-cli flag + config-loading + gitignore extension |
| 11 | Atlas-on-Atlas baseline numbers | **Calibrated empirically by PR-5.** PR-5 records: cold token total per provider, iteration count to convergence, wall time, audit-verdict distribution, evidence-score distribution per stage, Lane A retry counts. These numbers are the regression detector for future Phase 7+ changes; informational, never enforced as runtime caps (recast §2.4 / §8.4). | PR-5 calibration + sprint closeout note |
| 12 | Backend transport during sprint | **HTTP only.** `claude_code + codex` subprocess support is item 5 (PR-A, parallel). HTTP `http_anthropic` + `http_openai` is the live path during the sprint's empirical work (memory `project_phase7_agent_runtime_default_ratified`). | PR-1 example config + PR-5 calibration uses HTTP |
| 13 | MCP `serve_client` task design | **`rmcp`-first migration + `serve_client` on top of it.** PR-A migrates PR-1's hand-rolled framing in `crates/atlas-agents/src/mcp/{mod.rs, server.rs, descriptors.rs}` to `rmcp` (Rust MCP SDK). Plan-time gate: confirm `rmcp` is actively maintained, supports multi-client server, has acceptable transitive-dep footprint. Fallback: `jsonrpsee` + thin MCP-protocol shim (§12 risk #2). Per-agent subprocess spawn via `tokio::process::Command`; restriction set `--disallowedTools=Read,Grep,Glob,Bash,Write,Edit` for claude-code (codex equivalent recorded in PR-A). Drain handshake on subprocess exit before agent result returns. | PR-A scope |
| 14 | Projection-to-ontology shim | **Full canonical-schema shim.** `crates/atlas-agents/src/runtime/projection_to_canonical.rs` maps `L9Projection` → `components.yaml` + `subsystems.yaml` + `related-components.yaml`. Hard-fail (not silent gap) when L9 lacks info to populate canonical fields — shim's hard-fail error doubles as a prompt-correctness oracle during PR-5 calibration. | PR-3 shim module + tests |
| 15 | `--disallowedTools` probe shape | **Dedicated `crates/atlas-agents/tests/mcp_disallowed_tools.rs`.** Spawns a live `claude-code` subprocess via PR-A's `serve_client`, provokes a `Read` tool call, asserts upstream's "tool not available" error shape. Upstream-version sensitivity (PR-7 closeout risk #5) localised to one test file. | PR-B test |

---

## 3. Wave structure: 8 PRs across 5 waves + parallel track

| Wave | PR | Scope | Dependency / parallelism | Gating? |
|---|---|---|---|---|
| 0 | PR-0 | Plan + status + continuation prompt + this brainstorm reference; close items 1–15 above | n/a | n/a |
| 1 (foundation) | PR-1 | `BackendRouter::backend_for_provider` + `Arc<ForProviderFn>` construction in atlas-cli + `--config <path>` flag + `.atlas/config.sprint.example.yaml` checked in + gitignore update + HTTP-backend smoke against synthetic prompts | Sequential | **Yes — gates PR-2/3/4/A** |
| 2 (dispatch) | PR-2 | Production prompts for `build_dispatch_subsystems_prompt` + `build_dispatch_components_prompt` (replace stubs at `dispatch.rs:203, :254`) + dispatch-stage Lane A evidence scoring | Sequential after PR-1 | **Yes — gates Phase 8** |
| 3 (producer prompts + shim) | PR-3 | Production prompts for `build_classify_prompt` + `build_reduce_prompt` + new `build_project_prompt` + their Lane A evidence scoring + `projection_to_canonical.rs` shim + tests | Sequential after PR-2 | **Yes — gates Phase 8** |
| 4 (cross-provider audit) | PR-4 | Replace `PR-7-WIRES-REAL-AUDITOR` stub at `runtime/mod.rs:665` with real audit-prompt round-trip + revision-prompt path + on-disk verdict at `.atlas/audit/<stage>/<target>.yaml` | Sequential after PR-3 (needs prompts producing real outputs) and PR-1 (`for_provider` populated) | **Yes — gates Phase 8** |
| 5 (calibration + closeout) | PR-5 | Atlas-on-Atlas calibration run; record intrinsic baseline metrics; sprint closeout note; memory updates; PR-7 closeout note's "Atlas-on-Atlas baseline" line backfilled | Sequential after PR-4 | No (post-gate) |
| Parallel | PR-A | `rmcp` migration + subprocess MCP `serve_client` driver + restriction-set encoding | After PR-1; parallel with PR-2/3/4/5 | No (post-gate) |
| Parallel | PR-B | `tests/mcp_disallowed_tools.rs` | After PR-A | No (post-gate) |

PR count: 8 (matching Phase 7's cadence). Gating set: PR-1 → PR-2 → PR-3 → PR-4
unblocks Phase 8 brainstorming per the Phase 7 → Phase 8 handoff. PR-5 +
PR-A + PR-B can land in any order after PR-1 + PR-A's predecessor; if Phase 8
work begins in parallel before PR-5 ships, the brainstorm should be aware
the Atlas-on-Atlas baseline isn't recorded yet.

---

## 4. Wave 1 — Foundation (PR-1)

PR-1 is small, structural, and unblocks every downstream PR in the sprint.

### 4.1 `BackendRouter::backend_for_provider` (kickoff dim 4 / item 4)

In `crates/atlas-llm/src/router.rs`, alongside the existing `#[cfg(test)]`
`from_dispatch_table`, add:

```rust
impl BackendRouter {
    /// Returns the first backend whose `TransportFlavour` belongs to the
    /// requested provider. Production code path for Lane B cross-provider
    /// audit (Phase 7 production-prompt sprint).
    pub fn backend_for_provider(&self, provider: Provider) -> Option<&Arc<dyn LlmBackend>> {
        self.entries.iter()
            .find(|entry| entry.transport.provider() == provider)
            .map(|entry| &entry.backend)
    }
}
```

This is a 5–10 LOC addition that targets exactly the lookup Lane B needs;
leaves `from_dispatch_table`'s test-fixture assumptions encapsulated.

### 4.2 `Arc<ForProviderFn>` construction in atlas-cli

In `crates/atlas-cli/src/pipeline.rs`'s `run_index_agent_runtime` (introduced
by PR-7 commit `88cbad7`), replace `for_provider: None` with:

```rust
let router_for_closure = Arc::clone(&backend_router);
let for_provider: Arc<ForProviderFn> = Arc::new(move |provider| {
    router_for_closure.backend_for_provider(provider).cloned()
});
let agent_runtime = AgentRuntime {
    backend_router,
    tools,
    cache,
    event_bus,
    semaphores: Semaphores::defaults(),
    max_iterations: config.max_iterations.unwrap_or(5),
    for_provider: Some(for_provider),
};
```

Lane B routes cross-provider out-of-box; `AuditDegraded` event fires only
when the requested provider isn't configured (e.g., user runs with
HTTP-Anthropic-only).

### 4.3 `--config <path>` flag + example file

In `crates/atlas-cli/src/main.rs` (or the `index` subcommand definition),
add a clap-level `--config <PATH>` argument that overrides the default
`<workspace_root>/.atlas/config.yaml` resolution. The argument is universal
(applies to all subcommands), not just `index`.

Checked-in file `.atlas/config.sprint.example.yaml`:

```yaml
schema_version: 1
backends:
  - id: producer
    transport:
      kind: http_anthropic
      api_key: ${ANTHROPIC_API_KEY}
      model: claude-opus-4-7
  - id: auditor
    transport:
      kind: http_openai
      api_key: ${OPENAI_API_KEY}
      model: gpt-5-codex
default_transport: http_anthropic
```

Env-var substitution at config-load time (deny missing vars with a clear
error, not silent empty string). Gitignore: add `.atlas/config.sprint.yaml`
explicitly (the `.example.yaml` is checked in; the working file is not).

### 4.4 HTTP-backend smoke test

`crates/atlas-cli/tests/agent_runtime_http_smoke.rs` — exercises
`atlas index --agent-runtime --config <path-to-sprint-example>` against a
synthetic minimal workspace with a `subsystems.overrides.yaml` so dispatch
short-circuits and never fires the LLM, but every downstream agent call
*does* fire (against `test_backend` canned responses). Verifies the wiring
from `for_provider` through Lane B's auditor lookup is sound. Synthetic;
no real API keys required.

### 4.5 PR-1 acceptance

- `BackendRouter::backend_for_provider` exists + tested in `router.rs#mod tests`.
- `for_provider: Some(_)` populated in `run_index_agent_runtime`; PR-7's
  `AuditDegraded`-on-single-provider behavior unchanged when running with
  one-provider config.
- `--config <path>` flag works; example file checked in; gitignore extended.
- HTTP-backend smoke test green.
- All cargo gates clean; polyglot smoke unchanged.

PR-1 LOC budget: 200–350 LOC across `router.rs`, `pipeline.rs`,
`main.rs`/`config.rs`, the example yaml, gitignore, and one new test file.

---

## 5. Wave 2 — Dispatch prompts (PR-2)

PR-2 replaces the two `PR-7-WIRES-REAL-PROMPT` stubs in
`crates/atlas-agents/src/runtime/dispatch.rs` with production prompts, and
introduces the dispatch-stage half of Lane A evidence scoring.

### 5.1 Workspace → subsystems prompt (`build_dispatch_subsystems_prompt`)

The dispatch agent reads the workspace, identifies subsystem partitions,
and emits a JSON object deserializable to `SubsystemsOverrideFile`
(`dispatch.rs:103`). The producer's job is to discover natural
subsystem boundaries — typically aligned to top-level directory structure,
crate / package boundaries, or domain coherence — and partition all L1
candidates into them.

Prompt template skeleton (concrete text is plan-time work; this captures
the shape):

```
You are a workspace dispatcher for Atlas, a tool that produces
LLM-consumable analyses of monorepos. Your task: partition this
workspace's components into subsystems whose members share coherent
purpose.

# Workspace context
{workspace_root_listing}
{l1_candidate_components_summary}
{existing_overlay_signals — Phase 6 PR-3 subsystem field overlays}

# Available tools
{per-stage_tool_catalog — read_file, list_dir, query_l1_index,
 query_existing_overrides; with one-line "applicable when:" descriptions}

# Output shape (emit one ```json block containing exactly this struct)
{schema for SubsystemsOverrideFile, embedded inline}

# Soft budget
You should normally complete in {soft_cap} tool calls; if you need more,
prefer fewer-larger reads over many-small reads. Hard cap {hard_cap}.

# Confidence rubric
- Strong: every top-level candidate's manifest was read; partitions reflect
  observed manifest + naming + path structure
- Moderate: most candidates' manifests read; some partitions inferred from
  path conventions
- Weak: mostly inferred from paths; few manifests actually read
- Declines: couldn't enumerate workspace
```

Initial values: `hard_cap = 30`, `soft_cap = 15`.

### 5.2 Subsystem → components prompt (`build_dispatch_components_prompt`)

Mirrors §5.1's shape but scoped to a single subsystem's component
candidates. Emits `ComponentsOverrideFile` (`dispatch.rs:131`). Rubric
mirrors §5.1 with substituted nouns ("subsystem manifests" → "component
manifests within this subsystem").

### 5.3 Dispatch-stage Lane A evidence scoring

In `crates/atlas-agents/src/runtime/audit/lane_a.rs` (PR-7 ships
`lane_a_validate`), extend to a two-layer validator:

```rust
pub async fn lane_a_validate(
    output: &AgentOutput,
    transcript: &Transcript,
    stage: Stage,
) -> Result<Grade, AgentError> {
    // Layer 1: schema validation (existing PR-4 behavior)
    let schema = stage_response_schema(stage);
    schema.validate(&output.value).map_err(AgentError::LaneAFail)?;

    // Layer 2: deterministic evidence score
    let evidence_score = compute_evidence_score(stage, transcript, output);
    let evidence_max = grade_ceiling(evidence_score);

    // Clamp the LLM's self-grade
    let llm_claim = output.confidence_grade;
    Ok(llm_claim.min(evidence_max))
}

fn compute_evidence_score(
    stage: Stage,
    transcript: &Transcript,
    output: &AgentOutput,
) -> f32 {
    match stage {
        Stage::DispatchSubsystems => dispatch_subsystems_evidence(transcript, output),
        Stage::DispatchComponents => dispatch_components_evidence(transcript, output),
        Stage::Classify           => classify_evidence(transcript, output),
        Stage::Surface            => surface_evidence(transcript, output),
        Stage::Reduce             => reduce_evidence(transcript, output),
        Stage::Project            => project_evidence(transcript, output),
    }
}

fn grade_ceiling(score: f32) -> Grade {
    if score >= 0.9 { Grade::Strong }
    else if score >= 0.5 { Grade::Moderate }
    else if score >= 0.1 { Grade::Weak }
    else { Grade::Declines }
}
```

Dispatch-stage scoring:

```rust
fn dispatch_subsystems_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    // Count L1 candidates whose primary manifest was read.
    let candidates = output.l1_candidates_referenced();
    let reads = transcript.read_file_paths();
    let manifests_read = candidates.iter()
        .filter(|c| reads.contains(&c.primary_manifest_path))
        .count();
    manifests_read as f32 / candidates.len().max(1) as f32
}
```

Similar shape for `dispatch_components_evidence` scoped to the subsystem.

### 5.4 Tests

- `tests/dispatch_prompt_shape.rs` — assert each `build_dispatch_*_prompt`
  emits a string containing the schema definition string for the target
  struct (drift catcher from row 2 of §2.2).
- `tests/lane_a_dispatch_evidence_floor.rs` — assert that an agent claiming
  `Strong` with an empty transcript gets clamped to `Declines`; a `Strong`
  claim with all manifests read stays `Strong`.

### 5.5 PR-2 acceptance

- Both dispatch stub markers removed from `dispatch.rs`.
- Schema-drift test green for both dispatch prompts.
- Evidence-floor test green for both dispatch stages.
- Cargo gates clean.
- Polyglot smoke cold ≈ today's reference (dispatch agents short-circuit on
  the polyglot fixture's full override coverage; cold count stays in the
  loose bound `0 < cold < 100`).
- `--agent-runtime` smoke against a synthetic workspace *without* override
  files now actually emits dispatch decisions through the LLM (previously
  hard-failed at the stub).

PR-2 LOC budget: 600–900 LOC across prompt templates, lane_a extension,
two new test files.

---

## 6. Wave 3 — Producer prompts + canonical-schema shim (PR-3)

PR-3 is the largest single PR in the sprint. It produces real outputs for
the four non-dispatch stages and emits canonical artifacts. The brainstorm
§12 risk #1 "stop and surface at >2× LOC budget" carve-out from Phase 7's
PR-5 applies here.

### 6.1 Classify prompt (`build_classify_prompt`)

Per-component agent. Input: one component's id, primary manifest, and the
per-stage classify tool catalog. Output: a `ClassifyAgentOutput` typed
struct (new; lives next to `ComponentsOverrideFile` in `dispatch.rs` or a
sibling `outputs.rs`) containing:

```rust
#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ClassifyAgentOutput {
    pub component_id: String,
    pub kind: ComponentKind,         // library | binary | service | etc.
    pub language: Language,
    pub lifecycle: Lifecycle,        // active | deprecated | etc.
    pub subsystem_hint: Option<String>,
    pub evidence_pointers: Vec<EvidencePointer>,  // path + line range refs
    pub confidence_grade: Grade,
}
```

The `evidence_pointers` field is load-bearing for §2.1 framing 2 (LLM
consumers can verify analyses by re-reading the cited evidence) and for
§7's cross-provider auditor (semantic critique threads back to specific
file regions). It's not optional.

Soft cap 6; hard cap 12. Evidence rubric:

- Strong: primary manifest read AND at least one source entrypoint
  (`lib.rs` / `main.rs` / `index.ts` etc.) read; classifier tool called;
  declared `kind` consistent with the entrypoint's structure.
- Moderate: manifest read; entrypoint inferred from manifest's declared
  paths but not directly read; classifier tool called.
- Weak: manifest read but no source inspected; classification inferred
  from manifest alone.
- Declines: manifest unreadable or absent.

Evidence score: `1.0 if manifest_read && entrypoint_read else 0.6 if
manifest_read else 0.0`.

### 6.2 Reduce prompt (`build_reduce_prompt`)

Per-subsystem agent. Input: a subsystem partition + all child component
agents' outputs (their `ClassifyAgentOutput` + `SurfaceAgentOutput`). Output:
a `ReduceAgentOutput` containing the subsystem's purpose, key contracts
shared among its components, declared cross-component edges within the
subsystem, and (importantly for §2.1 framing 2 use case b)
*refactoring-cue signals*: notable patterns, redundancies, abstraction
opportunities the reducer identifies.

```rust
#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ReduceAgentOutput {
    pub subsystem_id: String,
    pub purpose: String,              // 1-3 sentences, LLM-consumable
    pub component_ids: Vec<String>,
    pub key_contracts: Vec<ContractRef>,
    pub internal_edges: Vec<EdgeRef>,
    pub refactoring_cues: Vec<RefactoringCue>,  // §2.1 framing 2 use case b
    pub evidence_pointers: Vec<EvidencePointer>,
    pub confidence_grade: Grade,
}

#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct RefactoringCue {
    pub kind: RefactoringCueKind,    // duplication | mis-modularised | etc.
    pub component_ids: Vec<String>,
    pub rationale: String,           // 1 sentence
    pub evidence_pointers: Vec<EvidencePointer>,
}
```

Soft cap 4; hard cap 8.

### 6.3 Project prompt (`build_project_prompt`) — *new* (no PR-7 stub)

Workspace-level agent. Input: all subsystem reducers' outputs. Output: the
`L9Projection` data structure (canonical) — workspace-level purpose +
subsystem catalog + cross-subsystem edges + workspace-wide refactoring
cues + documentation-scaffold structure (§2.1 framing 2 use case c).

The project prompt's output is the **primary LLM-consumable artifact** —
downstream tools that want a high-level architecture summary read this
first, then drill into per-subsystem reduces and per-component classify
outputs as needed.

```rust
#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct ProjectAgentOutput {
    pub workspace_purpose: String,    // 2-5 sentences
    pub subsystem_catalog: Vec<SubsystemSummary>,
    pub cross_subsystem_edges: Vec<EdgeRef>,
    pub workspace_refactoring_cues: Vec<RefactoringCue>,
    pub doc_scaffold: DocScaffoldOutline,  // §2.1 framing 2 use case c
    pub confidence_grade: Grade,
}

#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocScaffoldOutline {
    pub sections: Vec<DocSection>,    // top-level: overview, architecture,
                                       //   subsystems, contracts, etc.
}

#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocSection {
    pub heading: String,
    pub source_references: Vec<EvidencePointer>,  // files / regions doc-gen
                                                   //   should pull from
    pub child_sections: Vec<DocSection>,
}
```

Soft cap 4; hard cap 8.

### 6.4 Per-stage Lane A evidence scoring (continued from §5.3)

```rust
fn classify_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let reads = transcript.read_file_paths();
    let manifest_path = output.primary_manifest_path();
    let entrypoint_path = output.declared_entrypoint_path();
    let manifest_read = reads.contains(&manifest_path);
    let entrypoint_read = entrypoint_path.map_or(false, |p| reads.contains(&p));
    let classify_tool_called = transcript.tool_called(output.expected_classify_tool_id());
    if manifest_read && entrypoint_read && classify_tool_called { 1.0 }
    else if manifest_read && classify_tool_called { 0.6 }
    else if manifest_read { 0.4 }
    else { 0.0 }
}

fn surface_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let declared = output.declared_public_items_count();
    let inspected = transcript.tool_calls_for("find_pub_items").count()
        + transcript.read_file_paths()
            .into_iter()
            .filter(|p| output.declared_public_item_paths().contains(p))
            .count();
    inspected as f32 / declared.max(1) as f32
}

fn reduce_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let expected_children = output.declared_child_component_ids().len();
    let observed_children_consumed = output.component_ids.len();
    observed_children_consumed as f32 / expected_children.max(1) as f32
}

fn project_evidence(transcript: &Transcript, output: &AgentOutput) -> f32 {
    let expected_subsystems = output.declared_subsystem_ids().len();
    let observed = output.subsystem_catalog.len();
    observed as f32 / expected_subsystems.max(1) as f32
}
```

### 6.5 Projection-to-canonical shim (`projection_to_canonical.rs`)

```rust
// crates/atlas-agents/src/runtime/projection_to_canonical.rs

pub fn project_l9_to_canonical(
    l9: &L9Projection,
    output_dir: &Path,
) -> Result<CanonicalArtifactSet, ShimError> {
    let components_yaml = build_components_yaml(l9)?;
    let subsystems_yaml = build_subsystems_yaml(l9)?;
    let related_yaml    = build_related_components_yaml(l9)?;

    atomic_write_pair(
        output_dir.join("components.yaml"),
        serde_yaml::to_string(&components_yaml)?,
        output_dir.join("subsystems.yaml"),
        serde_yaml::to_string(&subsystems_yaml)?,
    )?;
    atomic_write(
        output_dir.join("related-components.yaml"),
        serde_yaml::to_string(&related_yaml)?,
    )?;

    Ok(CanonicalArtifactSet { components: components_yaml, subsystems: subsystems_yaml,
                              related: related_yaml })
}

fn build_components_yaml(l9: &L9Projection) -> Result<ComponentsYaml, ShimError> {
    // Walk l9.subsystem_catalog → per-subsystem components.
    // For each component, populate canonical fields. Any field whose source
    // is missing from L9 → ShimError::MissingProjectionField with the
    // specific field name; do NOT emit a partial canonical artifact.
    todo!()
}
```

`ShimError::MissingProjectionField { field, path }` errors are
*intentionally noisy* — they're the prompt-correctness oracle (§2.1
framing 2): if the project prompt didn't produce enough info to populate
canonical fields, the prompt is wrong, not the shim.

### 6.6 Tests

- `tests/classify_prompt_shape.rs`, `tests/reduce_prompt_shape.rs`,
  `tests/project_prompt_shape.rs` — schema-drift catchers (row 2 of §2.2).
- `tests/lane_a_classify_evidence_floor.rs` + sibling files per stage —
  evidence-floor clamping tests.
- `tests/projection_to_canonical_shim.rs` — synthetic `L9Projection` →
  canonical YAML round-trip; assert that the emitted YAMLs deserialize
  back into the canonical structs that downstream Atlas tooling consumes.
- `tests/projection_to_canonical_shim_missing_field.rs` — synthetic L9
  missing a required field → `ShimError::MissingProjectionField`, no
  partial-write residue on disk.

### 6.7 PR-3 acceptance

- All three producer-prompt stubs replaced (classify / reduce / project).
- Schema-drift tests green for all three.
- Evidence-floor tests green for all four non-dispatch stages.
- `projection_to_canonical.rs` exists; round-trip test green; missing-field
  test green.
- `--agent-runtime` against a synthetic workspace now runs end-to-end
  through all stages and emits canonical YAMLs.
- Cargo gates clean; polyglot smoke unchanged.

PR-3 LOC budget: 1500–2200 LOC across four prompt templates, four
evidence-scoring functions, four new typed output structs, the shim, and
six+ new test files. This is the brainstorm §12 risk #1 "watch for
scope creep" PR; if it exceeds 2× this budget, the implementer should
stop and surface.

---

## 7. Wave 4 — Cross-provider auditor (PR-4)

PR-4 replaces the `PR-7-WIRES-REAL-AUDITOR` stub at `runtime/mod.rs:665`
with a real audit-prompt round-trip + revision-prompt path.

### 7.1 Audit prompt template

```
You are an auditor for an Atlas agent's output. The producer is a
{producer_provider} model; you are a {auditor_provider} model. Your role
is to evaluate the producer's *semantic soundness given the evidence
trail*, not its coverage (coverage is verified separately).

# Producer's stage
{stage}

# Producer's output
{producer_output_rendered}

# Producer's evidence trail (ordered tool calls + their results)
{transcript_rendered_as_tuples}

# Verdict shape (emit one ```json block)
{
  "verdict": "accept" | "request_revision" | "hard_fail",
  "reason": "<one-paragraph rationale>"
}

# Verdict rubric
- accept: output is consistent with the evidence; reasoning is sound
- request_revision: output has correctable issues — provide the reason
  in plain language; the producer will retry with your reason as
  additional context
- hard_fail: output is unsalvageable given the evidence; the stage cannot
  produce useful output on this target
```

### 7.2 Transcript-to-tuples rendering

```rust
pub fn render_transcript_for_audit(transcript: &Transcript) -> String {
    let mut out = String::new();
    for (idx, call) in transcript.tool_calls().iter().enumerate() {
        writeln!(out, "{}. tool: {}", idx + 1, call.tool_name).unwrap();
        writeln!(out, "   args: {}", summarise_args(&call.args, 200)).unwrap();
        writeln!(out, "   result: {}", summarise_result(&call.result, 400)).unwrap();
    }
    out
}
```

`summarise_*` functions truncate large values to the indicated byte budget
with a "[N bytes truncated]" suffix. Large transcripts → bounded audit
prompt size.

### 7.3 Revision prompt path

When the auditor emits `request_revision`, the producer is re-invoked with
its original prompt + a system-prompt addendum:

```
PRIOR ATTEMPT:
{producer_previous_output}

AUDITOR'S CRITIQUE:
{auditor_reason}

Revise your output to address the auditor's critique. You may invoke
tools again if additional evidence is needed. Cumulative retry budget
remaining: {retries_remaining}.
```

Cumulative cap = 2 retries per agent (Lane A retry + Lane B revision
combined). Recast §4.3 enforces this; PR-4 wires the counter.

### 7.4 On-disk verdict artefact

```yaml
# .atlas/audit/<stage>/<target_id>.yaml
agent_id: classify_atlas-engine
stage: classify
producer:
  provider: anthropic
  model: claude-opus-4-7
  output_sha: 2b91...
auditor:
  provider: openai
  model: gpt-5-codex
  verdict: accept   # accept | request_revision | hard_fail
  reason: "Producer's component_kind 'library' is consistent with the
           Cargo.toml's [lib] section and absent [[bin]] sections. Evidence
           pointers reference real file regions."
audit_tokens:
  in: 1240
  out: 320
audited_at: 2026-05-13T14:32:11Z
```

Atomic-write via Phase 4's `atomic_write` helper. On agent re-run, the
existing verdict is read from disk (cheap) and either accepted as still
valid (if producer output sha matches) or re-audited (if producer output
changed).

### 7.5 Lane B closure in `call_agent`

```rust
// crates/atlas-agents/src/runtime/mod.rs::call_agent (replaces stub at :665)

let auditor_closure = self.for_provider.as_ref().map(|fp| {
    let fp = Arc::clone(fp);
    let producer_transport = transport_flavour;
    Arc::new(move |producer_result: &AgentResult| {
        let producer_provider = producer_transport.provider();
        let auditor_provider = producer_provider.cross();
        let auditor_backend = fp(auditor_provider).unwrap_or_else(|| {
            event_bus.emit(AgentEvent::AuditDegraded {
                reason: format!("provider {:?} not configured", auditor_provider).into(),
            });
            fp(producer_provider).expect("producer's backend must exist")
        });
        run_audit_round_trip(auditor_backend, producer_result, &transcript)
    })
}) as Option<Arc<AuditClosure>>;
```

`Provider::cross()` returns the opposite provider (Anthropic→OpenAI;
OpenAI→Anthropic).

### 7.6 Tests

- `tests/audit_prompt_shape.rs` — audit prompt embeds verdict-rubric +
  transcript rendering format.
- `tests/audit_revision_round_trip.rs` — synthetic producer + auditor,
  auditor emits `request_revision`, producer's retry call receives the
  reason in the system prompt addendum.
- `tests/audit_verdict_atomic_write.rs` — on-disk verdict written
  atomically; concurrent reads during write don't see partial files.
- `tests/cross_provider_audit_routing.rs` — Anthropic producer → OpenAI
  auditor lookup via `for_provider`; OpenAI producer → Anthropic auditor;
  single-provider config → `AuditDegraded` + same-model fallback (PR-7's
  existing test, now exercising the real prompt code path).

### 7.7 PR-4 acceptance

- `PR-7-WIRES-REAL-AUDITOR` stub at `mod.rs:665` removed.
- All four new tests green.
- PR-7's existing Lane B tests still green (cross-provider routing
  fallback behavior preserved).
- `--agent-runtime` against a synthetic workspace with a forced-Weak
  producer agent triggers real cross-provider audit; verdict written to
  `.atlas/audit/<stage>/<target>.yaml`.
- Cargo gates clean; polyglot smoke unchanged.

PR-4 LOC budget: 400–700 LOC across auditor closure, audit prompt template,
transcript renderer, on-disk verdict writer, four new test files.

---

## 8. Wave 5 — Atlas-on-Atlas calibration + closeout (PR-5)

PR-5 runs the full agent runtime against Atlas's own workspace, records
intrinsic baseline metrics, and closes out the sprint.

### 8.1 Calibration invocation

```bash
ANTHROPIC_API_KEY=... OPENAI_API_KEY=... \
    cargo run --release --package atlas-cli -- index \
        --workspace-root . \
        --agent-runtime \
        --config .atlas/config.sprint.yaml \
        --log-events /tmp/atlas-on-atlas-events.jsonl
```

### 8.2 Intrinsic metrics recorded (§2.1 framing 1 — not vs deterministic)

In the PR-5 closeout note, record:

| Metric | Value |
|---|---|
| Cold token total (producer-Anthropic) | TBD |
| Cold token total (auditor-OpenAI) | TBD |
| Iteration count to convergence | TBD |
| Wall time | TBD |
| Number of components classified | TBD |
| Number of subsystems partitioned | TBD |
| Evidence-score distribution per stage (p25 / p50 / p90) | TBD |
| Lane A retry count (per stage) | TBD |
| Audit verdict distribution (Accept / RequestRevision / HardFail / Skipped) | TBD |
| Audit revision rounds (cumulative) | TBD |
| Hard-fail count + per-agent diagnostics | TBD |
| `ShimError::MissingProjectionField` count + field names (prompt-quality signal) | TBD |

These numbers become the **regression detector for future Phase 7+ changes**
(recast §8.4 "observed and asserted in tests, never enforced as a runtime
cap"). PR-5 doesn't assert thresholds; it records the empirical baseline
that future PRs assert against.

### 8.3 Cross-transport parity (within LLM-spine, not vs deterministic)

Run the same workspace through both `http_anthropic` and `http_openai` as
*primary producer* (with the opposite as auditor). Compare the structural
shape of emitted canonical artifacts (component set equality, subsystem
set equality, edge multiset equality — modulo justifiable provider-side
refinements). This is the within-LLM-spine cross-provider parity check
(§2.1 framing 1) — replaces the deterministic-vs-runtime parity that PR-7's
`polyglot_smoke_cross_transport_parity_*` shipped.

### 8.4 Memory + status updates

- `.claude/memory/project_phase4_plus_roadmap.md` — Atlas-on-Atlas baseline
  numbers recorded; sprint marked SHIPPED; Phase 8 (Cargo retirement)
  unblocked.
- `docs/superpowers/plans/2026-05-12-phase7-status.md` — "Atlas-on-Atlas
  cold token total baseline: DEFERRED" line at :462 replaced with the
  recorded baseline.

### 8.5 PR-5 acceptance

- Atlas-on-Atlas invocation completes (or hard-fails with specific
  diagnostic; the brainstorm §12 risk #5 captures the latter case).
- All intrinsic metrics recorded in the PR-5 closeout note section.
- Cross-transport parity check passes (or surfaces a structural
  disagreement worth investigating — that's *signal*, not failure).
- Memory updates land; status file Atlas-on-Atlas line backfilled.
- Sprint marked SHIPPED across memory + status file.

PR-5 LOC budget: 150–300 LOC (mostly invocation script, harness for
cross-transport parity comparison, closeout-note text) — bulk of PR-5 is
**measurement + analysis**, not code.

---

## 9. Parallel track — Subprocess MCP + probe (PR-A, PR-B)

PR-A and PR-B unblock `atlas index --agent-runtime` against the canonical
`claude_code + codex` config (today's hard-error per PR-7 closeout's
"User-visible note"). They can ship anytime after PR-1 lands and may
overlap with PR-2/3/4/5 or with early Phase 8 work.

### 9.1 PR-A — `rmcp` migration + `serve_client` driver

Two coupled changes:

**(i) Migrate PR-1's hand-rolled framing to `rmcp`.**
`crates/atlas-agents/src/mcp/{mod.rs, server.rs, descriptors.rs}` lose
their hand-rolled JSON-RPC framing; the multi-client multiplexing logic
is reimplemented on top of `rmcp`'s server abstractions. Plan-time gate:

- Confirm `rmcp` is actively maintained (commits in last 6 months;
  no unresolved CRITICAL-severity issues; documented MCP-protocol
  version compatibility).
- Confirm multi-client server support (PR-1's defining feature — 2
  concurrent subprocess clients each isolated, each with their own
  recording buffer).
- Verify transitive-dep footprint is acceptable (e.g., not pulling in
  `tokio-tungstenite` if Atlas doesn't otherwise need WebSocket).
- If unsuitable: fall back to `jsonrpsee` + thin Atlas-specific
  MCP-protocol shim handling only tool descriptors + capability
  negotiation (the MCP-specific bits not covered by generic JSON-RPC).

PR-1's `mcp_multiplex.rs` integration test is the regression detector: it
must pass post-migration with the same observable behavior.

**(ii) `serve_client` per-subprocess driver.**

```rust
// crates/atlas-agents/src/mcp/serve_client.rs

pub async fn serve_client(
    server: Arc<McpServer>,    // post-migration: rmcp-backed
    backend_id: BackendId,
    initial_prompt: String,
    config: SubprocessConfig,
) -> Result<AgentOutput, AgentError> {
    let mut child = tokio::process::Command::new(&config.executable_path)
        .args(&config.subprocess_args)  // includes --mcp-config, --disallowedTools
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let client_id = server.register_client(child.stdin.take().unwrap(),
                                            child.stdout.take().unwrap()).await;

    // Send the initial prompt over stdin (subprocess reads it as the user prompt)
    // ... transport-specific framing

    let exit_status = child.wait().await?;
    let transcript = server.drain_client_transcript(client_id).await;

    if exit_status.success() {
        let output = parse_subprocess_final_output(&transcript)?;
        Ok(output)
    } else {
        Err(AgentError::SubprocessFailed(exit_status))
    }
}

pub struct SubprocessConfig {
    pub executable_path: PathBuf,
    pub subprocess_args: Vec<String>,
}

pub fn claude_code_config(mcp_config_path: &Path) -> SubprocessConfig {
    SubprocessConfig {
        executable_path: "claude-code".into(),
        subprocess_args: vec![
            "--mcp-config".into(), mcp_config_path.to_string_lossy().into(),
            "--disallowedTools".into(),
            "Read,Grep,Glob,Bash,Write,Edit".into(),
        ],
    }
}

pub fn codex_config(mcp_config_path: &Path) -> SubprocessConfig {
    // Codex's equivalent restriction flags — survey upstream at plan-time.
    todo!()
}
```

### 9.2 PR-A wiring into `tool_loop_http.rs`

The `tool_loop_http.rs::run_tool_loop_with_lane_a` function currently
errors out on `TransportFlavour::ClaudeCode | Codex` with the PR-7
diagnostic. PR-A replaces that branch with a call to `serve_client` for
subprocess transports.

### 9.3 PR-A acceptance

- Post-migration, `mcp_multiplex.rs` passes with the same observable
  multi-client behavior.
- `serve_client` exercised by a new unit test against a stub subprocess
  (Atlas spawns `cat` or similar as a no-op subprocess; verifies stdio
  wiring + drain handshake).
- `--agent-runtime` against the canonical `claude_code + codex` config
  no longer hard-errors at the first `call_agent`; subprocess transports
  drive real agent calls via the MCP server.
- Cargo gates clean.

PR-A LOC budget: 600–1200 LOC (migration is the bulk; the new
`serve_client` is ~150 LOC). The migration risk is the wider unknown;
PR-0 should commit to the crate choice at plan-time after the maturity
verification.

### 9.4 PR-B — `--disallowedTools` probe (kickoff dim 12 / item 7)

```rust
// crates/atlas-agents/tests/mcp_disallowed_tools.rs

#[tokio::test]
async fn claude_code_subprocess_cannot_invoke_disallowed_read_tool() {
    let server = build_test_mcp_server_with_default_tools().await;
    let mcp_config = write_temp_mcp_config(&server);
    let probe_prompt = "Read the file /etc/hosts using the Read tool.";

    let result = serve_client(
        server.clone(),
        BackendId::ClaudeCode,
        probe_prompt.into(),
        claude_code_config(&mcp_config),
    ).await;

    // Expect either: (a) subprocess succeeds but emits text saying it can't
    // use Read; or (b) subprocess fails with an upstream-version-specific
    // error about disabled tools. Both are valid; the assertion is that
    // *the Read tool was not actually invoked* (server's per-client
    // transcript contains zero Read tool calls).

    let transcript = server.drain_client_transcript(...).await;
    assert!(transcript.tool_calls().iter().all(|c| c.tool_name != "Read"),
            "Read was invoked despite --disallowedTools");
}
```

Upstream-version sensitivity: this test depends on `claude-code` honouring
the `--disallowedTools` flag. The test's failure mode is "upstream
regressed restriction enforcement," which is a CI-visible signal worth
having.

PR-B LOC budget: 100–200 LOC (single test file).

---

## 10. Testing strategy

| Layer | What | Where |
|---|---|---|
| Schema-drift tests | Each `build_*_prompt` site's embedded schema matches `schema_for!(TargetStruct)` | `crates/atlas-agents/tests/{dispatch_prompt_shape,classify_prompt_shape,reduce_prompt_shape,project_prompt_shape,audit_prompt_shape}.rs` |
| Evidence-floor tests | Per-stage: claimed Strong with empty transcript → clamped to Declines; claimed Strong with full evidence → stays Strong | `crates/atlas-agents/tests/lane_a_{dispatch,classify,surface,reduce,project}_evidence_floor.rs` |
| Audit round-trip tests | Auditor emits `request_revision` → producer retry sees reason in system prompt | `crates/atlas-agents/tests/audit_revision_round_trip.rs` |
| On-disk audit verdict tests | Atomic write; deserializes correctly; re-run replay logic | `crates/atlas-agents/tests/audit_verdict_atomic_write.rs` |
| Cross-provider routing tests | Provider mapping; `AuditDegraded` fallback; same-model fallback exercises real audit code path | `crates/atlas-agents/tests/cross_provider_audit_routing.rs` |
| Shim tests | `L9Projection` → canonical YAMLs round-trip; `MissingProjectionField` errors | `crates/atlas-agents/tests/projection_to_canonical_shim.rs` + sibling |
| HTTP-backend smoke | `--agent-runtime --config <sprint-example>` against synthetic workspace; verifies wiring end-to-end | `crates/atlas-cli/tests/agent_runtime_http_smoke.rs` |
| MCP `serve_client` test | Stub subprocess + drain handshake | `crates/atlas-agents/tests/mcp_serve_client.rs` |
| `--disallowedTools` probe | Live `claude-code` subprocess with disabled Read | `crates/atlas-agents/tests/mcp_disallowed_tools.rs` |
| Atlas-on-Atlas (manual) | Real workspace, real API keys, intrinsic metrics recorded | Manual + PR-5 closeout note |

Polyglot smoke (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is
unchanged — its full override coverage means dispatch agents continue to
short-circuit, cold call count stays in the loose bound `0 < cold < 100`,
and the cross-transport parity test (PR-7) continues to exercise the
deterministic engine path. *Note:* per §2.1 framing 1, that parity test
is forensic, not load-bearing for future work; PR-5's within-LLM-spine
cross-transport parity replaces it for new-work regression detection.

---

## 11. Acceptance criteria

- [ ] PR-1: `BackendRouter::backend_for_provider` shipped; `for_provider: Some(_)` populated in atlas-cli; `--config <path>` flag works; example config checked in; gitignore extended; HTTP-backend smoke green.
- [ ] PR-2: Both dispatch stubs replaced; schema-drift + evidence-floor tests green for both dispatch stages.
- [ ] PR-3: All three producer prompt stubs replaced (classify + reduce + project); schema-drift + evidence-floor tests green for all four non-dispatch stages; canonical-schema shim exists with round-trip + missing-field tests; `--agent-runtime` against synthetic workspace emits canonical YAMLs.
- [ ] PR-4: Auditor stub replaced; revision round-trip test green; on-disk verdict tests green; cross-provider routing tests green.
- [ ] PR-5: Atlas-on-Atlas calibration ran; all intrinsic metrics recorded in closeout note (cold tokens per provider, iteration count, wall time, evidence-score distribution per stage, Lane A retry counts, audit verdict distribution, shim missing-field count); within-LLM-spine cross-transport parity check ran.
- [ ] PR-A: `rmcp` migration complete (or `jsonrpsee` fallback if `rmcp` unsuitable at plan-time); `mcp_multiplex.rs` regression-green; `serve_client` driver shipped; `--agent-runtime` against canonical claude_code + codex no longer hard-errors at first `call_agent`.
- [ ] PR-B: `tests/mcp_disallowed_tools.rs` shipped; passes against current `claude-code` upstream version.
- [ ] All cargo gates clean across all PRs (build / fmt / clippy / test workspace / release build / polyglot release).
- [ ] Sprint closeout note appended to `phase7-status.md` (or sibling sprint-status file); Phase 8 (Cargo retirement) marked unblocked in memory `project_phase4_plus_roadmap`.

End-of-sprint acceptance: PRs 1–4 land in main; Phase 8 brainstorming
unblocked. PR-5, PR-A, PR-B may land afterward (and may overlap with
Phase 8 plan-writing).

---

## 12. Open risks

Brainstorm-level open questions; not resolved until the PR that owns the
mitigation lands.

### 12.1 PR-3 size

**Risk:** PR-3 (four prompt templates + four evidence-scoring functions +
four typed output structs + canonical-schema shim + six+ test files) is
the largest single PR in the sprint. Brainstorm §12 risk #1 from Phase 7
(>2× LOC budget → stop and surface) applies.

**Mitigation:** Plan-writing splits PR-3 internally into well-bounded
commits (one commit per prompt + its scoring + tests; final commit for
the shim). If subagent implementation exceeds 4400 LOC (2× the budget),
the implementer surfaces rather than continues; the brainstorm reopens
with a split-PR-3 proposal.

### 12.2 `rmcp` maturity verification at plan-time

**Risk:** the brainstorm commits to `rmcp` (or equivalent maintained MCP
SDK) without independent verification of the crate's current health.

**Mitigation:** PR-0 plan-writing includes an explicit `rmcp` verification
step: check crates.io publishing cadence; check repo activity; check
multi-client server support; check transitive deps. If the verification
fails any criterion, fall back to `jsonrpsee` + thin MCP-protocol shim
(noted as the documented contingency in §2.2 row 13).

### 12.3 Opus 4.7 token cost during prompt iteration

**Risk:** The sprint's prompt-engineering iteration is calibrated against
Opus 4.7 per row 7 (no model downgrade tier; recast §8.4). Iteration
costs accumulate quickly during prompt-debugging.

**Mitigation:** PR-1's `--config <path>` infrastructure makes it trivial
to swap providers for *non-Opus* iteration during prompt-engineering work
(e.g., `claude-haiku-4-5` for fast feedback) **provided the final
calibration in PR-5 uses Opus 4.7**. This is not a "downgrade tier" — the
sprint commits to Opus for the recorded baseline; cheaper iteration during
dev is a sprint-internal choice.

### 12.4 Recast spec §11.2 "reference-output comparison harness" conflict

**Risk:** the recast spec §11.2 names a Phase 8 "reference-output
comparison harness" (Cargo agent output vs deterministic-classifier
output). This sprint's framing (§2.1 framing 1) rejects deterministic
comparison.

**Mitigation:** Two paths, picked at Phase 8 brainstorm time, not now:
- (a) Phase 8 brainstorm proposes a spec-text amendment to §11.2 (replace
  "vs deterministic" with "intrinsic + cross-provider parity within
  LLM-spine"); the amendment lands as a Phase 8 PR.
- (b) Phase 8 brainstorm preserves §11.2's text but redefines "reference"
  as the canonical-schema shim's output (the canonical YAMLs are now
  Atlas's reference output, not a deterministic classifier's).

Either way, this sprint does **not** ship the comparison harness in its
recast-§11.2 sense.

### 12.5 `L9Projection` shape may lack canonical-schema fields

**Risk:** PR-3's canonical-schema shim hard-fails on missing fields. If
the project prompt's output (PR-3's new `ProjectAgentOutput`) doesn't
contain all info the canonical YAMLs require, PR-3 surfaces this as a
shim error during testing.

**Mitigation:** PR-3 ships the shim and the project prompt together so
the loop "prompt produces output → shim consumes → discovers missing
field → prompt updated" is local to PR-3, not a cross-PR ping-pong.
Synthetic-workspace tests catch most cases; Atlas-on-Atlas in PR-5
catches the rest. The brainstorm framing (§2.1 framing 2) treats shim
hard-fails as **prompt-correctness signals** — they're a feature.

### 12.6 Per-stage iteration cap × concurrency math

**Risk:** classify hard cap 12 × per-stage semaphore default 8 = 96
in-flight tool calls peak. Multiplied across stages on a large workspace,
total fan-out could be large.

**Mitigation:** The HTTP transport semaphore (default 8) is the real
backstop — it caps the actual outbound API call rate regardless of
per-stage fan-out. PR-1's `agent_runtime_http_smoke.rs` exercises this
under synthetic load. Atlas-on-Atlas in PR-5 will surface real-world
peak.

### 12.7 Upstream-version sensitivity of subprocess restrictions

**Risk:** `--disallowedTools=Read,Grep,Glob,Bash,Write,Edit` for
claude-code depends on upstream honouring the flag. A future upstream
version that changes flag semantics or adds new built-in tools breaks
the restriction guarantee.

**Mitigation:** PR-B's `mcp_disallowed_tools.rs` is the CI-visible
regression detector. PR-A records the exact `claude-code` and `codex`
upstream versions targeted, in code comments next to the
`subprocess_args` constants. Memory note as Phase 8+ ongoing concern.

### 12.8 Schema-drift test framing

**Risk:** the schema-drift test asserts that a *string* (the prompt
template's embedded schema text) matches `schema_for!(T).to_string()`.
Whitespace + ordering differences in serde_json's output could break
this even when the schemas are semantically identical.

**Mitigation:** Use `serde_json::Value` equality (semantic equality), not
string equality, in the assertion. Parse both the embedded prompt
fragment and the live `schema_for!()` output as `Value` and compare. Plan
PR-2 to ship this assertion shape; subsequent prompt PRs reuse it.

---

## 13. References

- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` —
  design anchor for the LLM-spine runtime; this sprint completes §11.1's
  acceptance text.
- `docs/superpowers/specs/2026-05-12-atlas-vnext-phase7-plan.md` — Phase 7
  plan; §4 Task 4 (Lane A) and §4 Task 5 (LLM-decided dispatch + Lane B)
  are the structural foundation this sprint builds on.
- `docs/superpowers/plans/2026-05-12-phase7-status.md` — Phase 7 status;
  PR-7 closeout note + Phase 7 → Phase 8 handoff (lines 375–477) name
  the 7 sprint items this brainstorm resolves.
- `docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md` —
  Phase 7 brainstorm; §6 PR-5 (lines 357–462) is the architectural intent
  the production prompts realise.
- `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent
  (LLM-spine).
- `.claude/memory/feedback_cross_provider_llm_audit.md` — Lane B design
  rationale.
- `.claude/memory/project_atlas_common_backend_config.md` — canonical
  user backend config (`claude_code + codex`); subprocess MCP
  multiplexing requirement.
- `.claude/memory/project_phase7_agent_runtime_default_ratified.md` —
  `--agent-runtime` flag default-false; HTTP backends are the live path
  during this sprint.
- `.claude/memory/feedback_no_deterministic_engine_comparison.md` —
  framing #1: deterministic engine is being retired; no comparison-based
  success criteria.
- `.claude/memory/project_atlas_purpose_llm_consumers.md` — framing #2:
  Atlas outputs feed other LLM tools; quality bar = "useful as LLM
  context."
- `.claude/memory/feedback_prefer_existing_crates.md` — framing #3:
  prefer maintained crates; PR-A migrates PR-1's hand-rolled MCP framing
  to `rmcp` (or fallback).
- `.claude/memory/project_phase4_plus_roadmap.md` — phase-ordering state;
  Phase 8 (Cargo retirement) unblocked by this sprint's items 1–4.
- `crates/atlas-agents/src/runtime/dispatch.rs` — `SubsystemsOverrideFile`
  / `ComponentsOverrideFile` shapes the dispatch prompts must satisfy
  (lines 103, 131); current stub markers at lines 203, 254.
- `crates/atlas-agents/src/runtime/mod.rs` — `ForProviderFn` type alias
  (line 356); `for_provider` field on `AgentRuntime` (line 350); auditor
  closure stub site (line 665); classify + reduce prompt sites
  (~lines 910 / 920).
- `crates/atlas-llm/src/router.rs` — `from_dispatch_table` (line 142,
  `#[cfg(test)]`); `backend_for_provider` is added here by PR-1.
