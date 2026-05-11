# Atlas LLM-spine recast — architectural inversion (design spec)

Status: brainstormed and approved 2026-05-11. Companion plan +
status file land in PR-0 of Phase 7 itself (the first phase
implementing this recast). Phase 6 ships *before* Phase 7 begins as
the final deterministic-spine release; see §12.

Phase 5 shipped on 2026-05-10 (folding `atlas-contracts` in-tree
and retiring the multi-root seam; final commit
`a302ce525bebd2df546472542f798f3c129426ba` on Atlas main). On
2026-05-10 the Phase 6 brainstorm paused mid-design after surfacing
that Atlas had drifted from its original *prompts-as-application*
intent — the canonical roadmap positioned LLM-driven analyses at
Phase 9 (second-to-last), implying LLM is polish rather than spine.
This document captures the agreed architectural inversion and the
phasing that delivers it.

---

## 0. Reading order

§1 (one-paragraph summary) → §2 (non-negotiable invariants) →
§3 (architectural inversion) → §4 (map-reduce primitive) →
§5 (toolbox) → §6 (transcript cache) → §7 (failure-mode envelope) →
§8 (budget invariants) → §9 (progress UX) → §10 (execution
discipline) → §11 (migration shape) → §12 (Phase 6 disposition) →
§13 (canonical §10 retext). §14 lists open questions and deferred
work; §15 is the glossary; §16 is references.

---

## 1. Summary

Atlas's analytical work is performed by an **LLM agent runtime** over
a tree of per-stage tasks. Deterministic Rust code is reserved for
tasks that are *genuinely* deterministic — parsing structured
manifests, walking filesystem trees, computing content shas,
validating schemas, replaying cached transcripts, and supporting the
agent runtime itself. This inverts the §4.3 principle (*Determinism
over fuzziness*) of the canonical system-model design: deterministic
code is now the *scaffolding*; LLM is the *spine*.

The map-reduce primitive is a tree of LLM agents, hybrid-by-stage
in its unit-of-work (L3/L5 per-component, L6 per-dep-pair at LCA,
L8 per-subsystem, L9 deterministic aggregation). Interior dispatch
is itself LLM-decided and lands as reviewable disk artefacts.
A fixed-point iteration loop runs the tree until two consecutive
iterations produce the same L9 projection sha. An audit lane
combines deterministic structural validation (always) with LLM
audit on low confidence (Q3 = E).

The agent toolbox is **text-biased** (LLMs deal poorly with AST
representations; structured input → structured output is fine when
the source already is). Manifest parsers and filesystem primitives
are universal; text-scoping helpers (`find_pub_items`,
`find_imports`, `find_function_defs`) ship only for languages
with mature tooling (Rust, TS/JS). Weak-tooling languages
(Dart, Racket, Elixir, LispKit, Compose, Dockerfile) get no
per-language helpers; agents read whole files. All 10 hand-coded
classifier modules retire; surface analyser modules collapse to
text-scoping tool implementations or retire entirely.

The transcript cache extends the existing `call_llm_cached_with_fp`
single-shot primitive to multi-shot agent runs, keyed on
`(stage || agent_id || agent_version || prompt_template_sha ||
tool_catalog_sha || model_id || backend_version || target_input_shas
|| iteration_number || prior_model_sha)`. Warm=0 (no-op re-run does
zero LLM calls) is the load-bearing correctness invariant.
*Cost-be-damned* in design: budget is observed and asserted in
tests as a regression detector, never enforced as a runtime cap.

**Failure-mode envelope:** user-visible output is complete-or-fail
(`AgentResult = Confident(output) | Failed(error)`; no `Partial`
variant). Cache state is durable and incremental — successful
per-component entries persist across runs even if the overall run
hard-fails. Re-runs resume from cache; only failed entries
re-attempt.

The execution discipline is **small atomic PRs**, each in a **fresh
session** to minimise context rot, with **subagent-aggressive**
parallelism on independent work and quality-first design choices
throughout.

Migration is **engine-first / language-incremental** (Approach 1):
Phase 7 ships the agent runtime and tools without retiring any
language analyser (agents call into existing deterministic
classifiers via thin tool wrappers); Phase 8 retires Cargo as the
calibration moment; Phase 9 retires the remaining languages in
sequenced waves; Phase 10 adds LLM-driven analyses (pattern
detection, fuzzy contract matching) — moved *earlier* than today's
§10.9 placement, since the runtime makes them natural; Phase 11
is server mode plus the web-app progress subscriber.

Phase 6 (user-facing schema cleanups) ships *before* Phase 7 as the
final deterministic-spine release. Its four items survive the
recast untouched and strengthen the user-authoring override
discipline the recast will depend on. The canonical §10 retext
lands as Phase 6's closeout PR.

---

## 2. Non-negotiable invariants

These survive the recast and constrain every design choice.

### 2.1 Plain text is canonical (§4.1 of canonical design)

YAML files on disk are the source of truth. Every in-memory model,
every projected database, every cache is derivable from the YAMLs.
LLM-discovered partitions (subsystem boundaries, component
boundaries) land as reviewable disk artefacts; **user-authored
overrides take precedence over LLM-discovered partitions** at every
level of the agent tree.

### 2.2 Content-addressed cache (§4.5)

Every cacheable computation is keyed by a fingerprint of its complete
inputs. The cache primitive extends from single-shot to multi-shot
(§6). The cache is persistent across processes; warm=0 on no-op
re-run is load-bearing (§8).

### 2.3 Complete-or-fail user output (§7)

`atlas index` either writes complete top-level projections or writes
none and exits non-zero. No `Partial(output, gaps)` variant, no
graceful-degradation tier, no best-effort envelope. Cache state is
durable and incremental — incremental progress at the cache layer
is fine and required; partial *output* is forbidden.

### 2.4 No runtime budget gates (§8)

Budget is observed and asserted in tests as a regression detector,
never enforced as a runtime cap. A real run with cache miss runs to
completion or hard-fails. Atlas does not trade correctness for
cost.

### 2.5 Quality over cost in design choices (§10)

Design choices are evaluated against "does this make the analysis
better or worse?" — not against "does this make the analysis
cheaper?" Trade-offs that compromise quality for speed, simplicity,
or cost are flagged explicitly in plans, never chosen by default.

---

## 3. Architectural inversion

### 3.1 The §4.3 retext

Canonical design §4.3 today reads:

> Atlas prefers deterministic analysers (manifest parsing, AST
> analysis, Dockerfile parsing, k8s manifest parsing, JSON Schema
> introspection) over LLM analysers. LLMs are used for: …

The Phase 7 retext (which lands in §13 along with the §10 retext) is:

> **§4.3 (recast): LLM is the spine; deterministic code is the
> scaffolding.** Atlas's analytical work is performed by an LLM
> agent runtime over a tree of per-stage tasks. Deterministic Rust
> code is reserved for tasks that are *genuinely* deterministic —
> parsing structured manifests, walking filesystem trees, computing
> content shas, validating schemas, replaying cached transcripts,
> and supporting the agent runtime itself. Each deterministic
> component must justify *why it is deterministic*; "easier to code
> than to prompt" is not sufficient justification.

### 3.2 What survives unchanged in §4

- §4.1 (plain-text-is-canonical) — unchanged.
- §4.2 (multi-repo-equals-monorepo) — unchanged.
- §4.4 (pluggable analysers) — reframed as *pluggable tools*; the
  `Analyzer` trait retires in favour of a `Tool` trait (§5.1).
- §4.5 (content-addressed cache) — unchanged in principle; the cache
  primitive extends (§6).
- §4.6 (data co-locates with source) — unchanged.
- §4.7 (Salsa as engine) — unchanged.

### 3.3 What retires

- §7.3 cost-class ordering (`deterministic-cheap < deterministic-expensive
  < llm-cheap < llm-expensive`) — retires entirely. Dispatch is
  LLM-agent-decided (§4.2), not cost-class-table-decided.
- The dispatcher (`crates/atlas-analyzers/src/dispatcher.rs`,
  `registry.rs`) — retires.
- All 10 hand-coded classifier modules (`cargo_classifier.rs`,
  `ts_js_classifier.rs`, `python_classifier.rs`,
  `csharp_classifier.rs`, `dart_classifier.rs`,
  `elixir_classifier.rs`, `racket_classifier.rs`,
  `lispkit_classifier.rs`, `compose_classifier.rs`,
  `dockerfile_classifier.rs`) — retire (Phase 8 → Phase 9).
- `llm_classify.rs`, `shell_script_llm_analyzer.rs` — retire under
  uniform agent dispatch.
- Surface analyser modules — Rust and TS/JS collapse to text-scoping
  tool implementations; other languages' surface analysers retire
  entirely (Phase 9c).

---

## 4. Map-reduce primitive: tree of LLM agents

### 4.1 Hybrid-by-stage unit-of-work

The map-reduce unit-of-work is a property of the *stage*, not of the
primitive:

| Stage | Unit | Notes |
|---|---|---|
| L3 classify | Per-component agent | One agent per L2 candidate. |
| L5 surface | Per-component agent | Sub-carve for very large components is the agent's own responsibility via text-scoping tools. |
| L6 edges | Per-dep-pair agent | Hosted at the lowest common ancestor (LCA) of the two participants in the tree. |
| L8 recurse / pattern | Per-subsystem agent | Operates over reduced child outputs. |
| L9 projection | **Deterministic** | No LLM. Pure aggregation; also the convergence judge (§4.4). |

The fingerprint table (§8.1 of canonical design) survives with the
same per-stage discriminators, extended with `iteration_number` and
`prior_model_sha` for fixed-point support (§4.4).

### 4.2 Tree shape & LLM-decided dispatch

The tree is **fully LLM-decided** (Q2 = D):

```
workspace agent (root)
   ├── dispatches → subsystem agents
   │      ├── dispatches → component agents
   │      │      ├── invokes tools (read_file, parse_cargo_toml,
   │      │      │                 find_pub_items, …)
   │      │      └── emits L3 classify + L5 surface
   │      └── reduces children → subsystem-level facts
   └── reduces children → workspace projection (L9, deterministic)
```

Every interior dispatch is itself an LLM call. Dispatch outputs are
**first-class disk artefacts** in `.atlas/discovery/<level>-plan.yaml`
(e.g. `subsystem-plan.yaml`, `component-plan.yaml`), reviewable
and overridable per §4.1. User-authored overrides at any level take
precedence over LLM-discovered partitions:

- `subsystems.yaml` + `subsystems.overrides.yaml` (existing) take
  precedence over the workspace agent's `subsystem-plan.yaml`.
- `components.overrides.yaml` (existing) takes precedence over
  subsystem agents' `component-plan.yaml`.

LLM dispatch fills gaps; user-authored YAML pins them. The "LLM
proposes, user disposes" dynamic is the user-authoring discipline
the recast depends on (and that Phase 6's items 3 + 4 strengthen).

### 4.3 Audit lane (Q3 = E)

Two-lane validation of each agent's output:

- **Lane A — deterministic structural validation (always).** Schema
  validates the agent's output (does it list at least one surface?
  do declared edges resolve to known components? does it satisfy
  §6 ontology constraints?). Failures cause one retry; second
  failure = hard fail.
- **Lane B — LLM audit (on low confidence).** Each agent emits
  `{ output, confidence_grade, evidence }` per §7.4. If
  `confidence_grade ∈ {weak, declines}`, an LLM auditor reviews
  and either accepts, requests revision with broader context (one
  retry), or escalates to hard fail.

Audit verdicts land on disk alongside production output
(`.atlas/audit/<stage>/<component>.yaml`) — reviewable per §4.1.
Audit transcripts (raw LLM trajectories) stay in cache. The on-disk
artefact lets the user override an audit verdict; the transcript is
replay material.

Audit fires *only* on uncertainty; a confident workspace incurs no
audit cost. Audit token spend counts in the same budget bucket as
production calls (§8); audit re-prompt retries are capped at one
per audit firing (cumulative cap: two retries per agent, after
which hard fail).

### 4.4 Fixed-point iteration

Real codebases aren't hierarchically partitioned. A sub-tree's
discovery ("this 'utility' is actually two coupled subsystems")
can require backing out earlier dispatch decisions. The map-reduce
is therefore **iterative**, not single-pass:

- Iteration 1: agent tree runs from cold dispatch decisions; produces
  a complete model with L9 projection sha `P1`.
- Iteration 2: agent tree runs again, with iteration 1's model
  available as prompt context for dispatch agents (so sub-tree
  discoveries propagate); produces model with L9 projection sha
  `P2`.
- **Convergence**: `P2 == P1` → converged; emit final output.
- Otherwise: continue iterating until convergence or hit iteration
  cap (initial `K = 5`, calibrated empirically).

Each iteration's per-agent transcripts are cached separately, keyed
on `iteration_number + prior_model_sha`. Cache hits within a single
run replay across iterations where inputs haven't shifted.

The convergence judge is **deterministic** — content-sha equality
on the L9 projection. No LLM oracle for "are we done." The pattern
extends the existing `crates/atlas-engine/src/fixedpoint.rs`
discipline (multi-root path-dep expansion) to the analysis content
itself.

Iteration cap is hard fail: if iteration `K+1`'s projection sha
still differs from iteration `K`'s, `atlas index` exits non-zero
with diagnostic output identifying which agents' outputs shifted
between iterations. (Most likely cause: a poorly-bounded prompt
producing non-deterministic outputs; the diagnostic guides
prompt-engineering fixes.)

---

## 5. Toolbox

### 5.1 The `Tool` trait

Replaces today's `Analyzer` trait (§7.1 of canonical design). Each
tool exposes:

```rust
pub trait Tool {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn json_schema(&self) -> &ToolSchema;  // for LLM tool-use API
    fn invoke(&self, args: ToolArgs, ctx: &ToolContext)
              -> Result<ToolResult, ToolError>;
    fn fingerprint_inputs(&self, args: &ToolArgs)
                          -> Vec<FingerprintInput>;
}
```

Tools are **deterministic functions of their args + filesystem
content**. The `fingerprint_inputs` hook lets the transcript cache
record what the tool actually read (file content shas, override-file
shas, etc.) so cache replay can verify staleness.

The `ToolContext` carries cache handles, filesystem readers, and an
event-bus emitter for the progress UX (§9).

### 5.2 Toolbox contents

| Category | Tools | Languages |
|---|---|---|
| **Manifest parsers** | `parse_cargo_toml`, `parse_package_json`, `parse_pyproject`, `parse_csproj`, `parse_dockerfile`, `parse_compose`, `parse_k8s_manifest`, `parse_helm_chart`, `parse_release_toml` | Universal — structured input → structured output. |
| **Text-scoping** | `find_pub_items(file)`, `find_imports(file)`, `find_function_defs(file)` | **Mature langs only** — Rust, TS/JS. Returns *line spans*, not AST nodes. Tree-sitter is the implementation; agents never see it. |
| **Filesystem & content** | `read_file(path, range?)`, `list_dir(path, glob?)`, `file_sha(path)`, `file_size(path)` | Universal. |
| **Index & cache queries** | `lookup_neighbour_surface(component_id)`, `query_l1_index()`, `query_existing_overrides(scope)` | Universal. |

Weak-tooling languages (Dart, Racket, Elixir, LispKit,
Compose-for-surfaces, Dockerfile-for-surfaces) get **no text-scoping
helpers**. Agents read whole files via `read_file` and reason over
raw text. Lower maintenance burden; consistent with the empirical
finding that LLMs work better with text than with AST representations.

### 5.3 What survives, what retires

- **Manifest parser implementations** (today living inside
  classifier modules) survive as their own thin crates / modules,
  exposed via the `Tool` trait. The TOML/JSON/YAML parsing logic
  itself is unchanged.
- **Tree-sitter walkers for Rust + TS/JS** survive as internal
  implementations of `find_pub_items` / `find_imports` /
  `find_function_defs`. Their public API becomes text-shaped.
- **All 10 classifier modules** retire (Phase 8 → Phase 9). The
  reasoning they performed (kind classification, role inference)
  becomes the work of an LLM agent driving the toolbox.
- **Surface analyser modules for non-mature langs** retire entirely.
- **`llm_classify.rs`, `shell_script_llm_analyzer.rs`** retire.
- **`dispatcher.rs`, `registry.rs`** retire — replaced by agent
  dispatch.

### 5.4 Tool design principles

- **Text-biased.** No tools that return AST node trees. Where
  structure is genuinely useful (TOML key-value), tools return
  typed structured data; everywhere else, tools return text spans
  or raw bytes.
- **Cost-be-damned.** No tools that exist purely to reduce token
  count (e.g. a tool that summarises a file in N tokens when the
  raw file is 2N tokens). Tools earn their place by enabling
  correctness, not by reducing cost.
- **Deterministic inputs.** Tools never call the LLM internally;
  they never include random or wall-clock state in outputs (timestamps
  are passed as args if needed). A tool invocation with identical
  args + filesystem state is a pure function.

---

## 6. Transcript cache primitive

### 6.1 Extension from single-shot to multi-shot

Today's `call_llm_cached_with_fp(Stage, Fingerprint, &LlmRequest)
→ LlmResponse` is single-shot. The agent runtime extension is:

```rust
pub fn call_agent_cached(
    stage: Stage,
    fingerprint: AgentInputFingerprint,
    request: AgentRequest,
) -> Result<AgentResult, AgentError>;

pub struct AgentRequest {
    pub initial_prompt: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub model: ModelId,
    pub backend: BackendId,
    pub audit_policy: AuditPolicy,  // confidence threshold, max retries
    pub max_steps: u32,             // hard cap on tool-call rounds
}

pub struct AgentResult {
    pub output: AgentOutput,        // typed per stage
    pub confidence_grade: Grade,    // Strong | Moderate | Weak | Declines
    pub evidence: Vec<EvidenceField>,
    pub transcript: TranscriptHandle, // cache key, not inline bytes
}
```

`AgentInputFingerprint` is keyed on:

- `stage_id || agent_id || agent_version`
- `prompt_template_sha || tool_catalog_sha || model_id || backend_version`
- `target_input_shas` (component_id, file content shas, parent
  dispatch decision sha, neighbour surface shas)
- `iteration_number || prior_model_sha` (new; pinned to 0 / empty
  on iteration 1)

### 6.2 Cache layout

Filesystem-native, per §8.3 unchanged:

- `.atlas/cache/<stage>/<sha>.transcript` — full record of
  `(initial_prompt, tool_calls[], tool_results[], completions[],
  final_output)` plus per-call timings and token counts.
- `.atlas/cache/<stage>/<sha>.output` — decoded typed output.

Both written **atomically** (write-tempfile-then-rename, the
`atomic_write` helper from Phase 4) so a hard-fail mid-write doesn't
leave a half-cached entry that the next run treats as valid.

### 6.3 Cache hit semantics

On a cache hit, the recorded `output` is returned directly. The
recorded transcript's `tool_calls[].fingerprint_inputs` are
spot-checked against current filesystem state — concretely, every
file path the transcript recorded via `read_file` / `file_sha` /
`parse_*` has its current `file_sha(path)` recomputed and compared
to the recorded value; if any differ, the entry **evicts** and the
agent re-runs. This preserves warm=0 in the face of partial edits:
an edit invalidates only the cache entries whose recorded tool
inputs shifted, not the whole cache.

### 6.4 Cache miss semantics

The agent runs from initial prompt; tool calls dispatch through
the runtime (which records each call and result in the transcript);
the LLM eventually emits a final output; the audit lane fires per
§4.3; on success, the transcript + output are written atomically to
cache; on hard fail, no cache write happens for the failing entry,
but any successful sibling entries remain cached.

---

## 7. Failure-mode envelope

| State | Behaviour |
|---|---|
| Cache hit (fingerprint match, tool inputs verified) | Replay output. Zero LLM calls. Zero tool calls. |
| Cache miss + agent succeeds (`Confident` after Lane A pass) | Write transcript + output atomically; return output. |
| Cache miss + agent succeeds (`Confident` after Lane A + Lane B audit accept) | Write transcript + output + audit verdict atomically; return output. |
| Cache miss + agent low confidence + audit requests revision | One retry with broader context. If revision succeeds, write & return. If revision still low-confidence → hard fail. |
| Cache miss + agent transient failure (rate limit, timeout, transient parse error) | One retry. Second failure = hard fail. |
| Cache miss + context-window overflow even after agent's own decomposition attempts | Hard fail. (Decomposition via text-scoping tools is the agent's responsibility; if even decomposition can't fit, the stage is unsupportable on the current model.) |
| Hard fail at any agent | `atlas index` exits non-zero. Top-level projections (`components.yaml`, etc.) are **not** written. Cache state is durable; on re-run, only failed entries re-attempt. |

`AgentResult = Confident(output) | Failed(error)`. No `Partial`
variant. The CLI surfaces hard-fail diagnostics including: agent id,
stage, target, fingerprint, retry history, last transcript snippet,
suggested investigation.

**Distinction (load-bearing):** *user-visible output is
complete-or-fail; cache state is incremental and durable*. A failed
run that completed 450 of 500 component agents leaves 450 entries
cached on disk; the next re-run replays those 450 and re-attempts
the failing 50. This is the same posture compilers take: a failed
build can leave `.o` files; `make` resumes incrementally.

---

## 8. Budget invariants

### 8.1 Unit

**Tokens** (input + output, summed). Audit calls count in the same
bucket as production calls. Token totals are emitted on the event
bus (§9) and tallied in real time for the TUI.

### 8.2 Warm-state taxonomy

- **Warm=0**: no-op re-run after a converged successful run on an
  unchanged workspace. Zero LLM calls. **Load-bearing correctness
  invariant.**
- **Warm=N (recovery)**: re-run after a *failed* prior run on an
  unchanged workspace. N = number of entries that hadn't cached
  before the failure. Falls to zero on the next successful run.
- **Warm=K (edit)**: re-run after the workspace changed. K = number
  of entries whose fingerprint shifted.

The actually-load-bearing rule is *no redundant compute*; "0" is
the special case where nothing changed.

### 8.3 Cold budget

Cold token totals are CI regression detectors. The Phase 3 polyglot
smoke test (today asserting cold LLM-call count) is extended to
assert cold token total ≤ threshold-empirical-per-language.

**Numbers are calibrated empirically**, not guessed upfront:

- Phase 7 (runtime alone, no retirements) inherits today's polyglot
  budget unchanged — the agent runtime wraps existing deterministic
  classifiers, so cold calls match today's reference.
- Phase 8 (Cargo retirement) is the **first calibration moment**.
  Cold token totals for Cargo-language fixtures are measured on
  Atlas-on-Atlas + the polyglot smoke; the empirical numbers become
  the budget assertion.
- Phase 9 waves each set their own language-specific cold budgets
  at retirement time.

### 8.4 No runtime budget gates

Budget is **observed and asserted in tests**, never enforced as a
runtime cap. A real run with cache miss runs to completion or
hard-fails. There is no "budget exhausted, return partial" path.
There is no graceful degradation. There is no model downgrade tier.
Atlas does not trade correctness for cost.

---

## 9. Progress UX

### 9.1 Event bus

A single in-process channel (Tokio `broadcast` or equivalent) carries
typed events from the agent runtime:

```rust
pub enum AgentEvent {
    IterationBoundary { iter: u32, prior_model_sha: ContentSha },
    AgentStart    { agent_id, parent_id, stage, target,
                    fingerprint, started_at },
    ToolCall      { agent_id, tool_name, args_summary },
    ToolResult    { agent_id, tool_name, result_summary, ms, bytes },
    AgentComplete { agent_id, output_sha, confidence_grade,
                    tokens_in, tokens_out, ms },
    AuditFire     { agent_id, audit_reason },
    AuditVerdict  { agent_id, verdict },
    HardFail      { agent_id, error_kind, error_summary, retry_count },
    CacheHit      { agent_id, fingerprint, replayed_at },
}
```

Fields are summary-shaped (no raw transcript content on the bus —
that lives in the cache). Subscribers compose:

- **TUI subscriber** (Phase 7): live tree view.
- **JSON-Lines logger** (Phase 7): `--log-events events.jsonl` for
  post-hoc analysis or external dashboards.
- **Transcript-cache writer** (Phase 7): materialises cache entries
  from `AgentComplete` events.
- **Web-app subscriber** (Phase 11): cross-process via the server
  mode's existing transport.

### 9.2 TUI subscriber (Phase 7)

Principles, not pixel-spec:

- **Live tree view.** Workspace at root; subsystem agents indented
  below; component agents below those; pair agents at LCA. Each node
  shows state (running / waiting / done / failed / cache-hit),
  elapsed time, current tool call (if running), confidence grade
  (if done).
- **Iteration counter + convergence indicator.** "Iteration 2 of
  K_max=5; previous projection sha 7a3f…; current sha 2b91… (changed
  → another iteration likely)."
- **Token cost in real time.** Sums `tokens_in + tokens_out` across
  the bus. Observability, not a gate.
- **Stuck detection.** Heuristic: if any agent's last event is older
  than 90 seconds and it's not in a known-long-running state (large
  file read, audit-lane LLM call), highlight as *possibly stuck*.
  Not an automatic fail — a visual cue for user investigation.
- **Replay mode.** `atlas index --replay-from-cache` renders the same
  TUI as a live run, sourced from cached transcripts. Useful for
  "what did Atlas decide last time?" without re-running.

Implementation library choice (`ratatui` is the obvious default for
Rust) is a plan-time decision.

The CLI defaults to TUI when stdout is a terminal; falls back to a
JSON-Lines event stream when stdout is piped or `--no-tui` is set.
Headless CI runs use the JSON-Lines fallback.

### 9.3 Web-app subscriber (Phase 11)

Lands with server mode. Same event bus, exposed over WebSocket /
SSE. The server already runs the bus across process boundaries; the
web app is a subscriber that needn't exist before the server does.

---

## 10. Execution discipline

This section codifies the meta-instructions for plan-writing and PR
execution. The writing-plans phase (§11) honours these as
non-negotiable.

### 10.1 Small atomic PRs

Each PR ships a working state with tests passing. No "this PR depends
on the next one." Mega-PRs are anti-pattern. Per-stage retirements
(Phase 8 / 9) are individual PRs even when several languages retire
in the same phase.

### 10.2 Minimise context rot

LLM performance degrades as irrelevant context accumulates — earlier
turns dilute attention, instructions drift, framing weakens. Each
PR is executed in a **fresh session** with a **lean prompt** that
includes only material the task actually needs (relevant code
regions, prior decisions, continuation pointers to the plan).

The **plan document carries cross-session continuity**; conversation
history does not. Context-rot resistance is the *reason* fresh
sessions are non-negotiable; the small-PR / minimal-scope disciplines
exist to keep individual fresh-session prompts lean.

### 10.3 Deep focus, one problem at a time

A PR addresses one problem completely; it doesn't half-address
several. Open questions get explicit deferral notes, not silent
leftover scope.

### 10.4 Subagent-aggressive parallelism

Where independent work exists (independent file regions, independent
test suites, independent code reviews), dispatch parallel subagents.
Worktree-isolated subagents per the existing
`feedback_worktree_base_verification.md` discipline (verify each
worktree's base sha matches current main before subagent dispatch).

### 10.5 Cost-be-damned in design

The agent runtime, tool layer, and test suite must not include
"cheap shortcuts" whose only purpose is token reduction. Tools earn
their place by enabling correctness; audit fires whenever it would
help; iterations run to convergence.

### 10.6 Quality bar

Best-in-class on a hard problem is the stated standard. Trade-offs
that compromise quality for any other axis (speed, simplicity, cost)
are flagged explicitly in plans, never chosen by default.

---

## 11. Migration shape (Approach 1)

### 11.1 Stream 1 — Phase 7 (LLM-spine runtime)

**Goal:** Build the agent runtime, tool layer, transcript cache
primitive, event bus, TUI subscriber, fixed-point iteration loop,
and audit lane. **No language retirements.** Existing deterministic
classifiers and surface analysers stay in place; the agent runtime
drives them via thin `Tool` wrappers.

**Deliverables:**

- `crates/atlas-agents/` (new) — runtime, `Tool` trait, agent
  dispatch, transcript cache, event bus, fixed-point loop, audit
  lane.
- Thin `Tool` wrappers around existing manifest parsers and
  classifier modules in `crates/atlas-analyzers/` (no behaviour
  change).
- TUI subscriber in `crates/atlas-cli/` (new module).
- `--no-tui` flag + JSON-Lines event stream fallback.
- Transcript cache reaches feature-parity with single-shot cache
  for the wrapped-classifier path (warm=0 verified).
- Phase 3 polyglot smoke test extended to assert cold token total
  equal to today's reference (since no retirements yet, cold should
  be unchanged).

**Acceptance:** `atlas index` runs through the agent runtime end-to-end
on Atlas-on-Atlas and the polyglot smoke fixture; cold token total
matches reference; warm=0 holds; TUI renders live progress; hard
fail diagnostics surface correctly.

### 11.2 Stream 2 — Phase 8 (Cargo retirement)

**Goal:** Retire `cargo_classifier.rs` and the Cargo-specific bits
of surface analysis in favour of an LLM agent driving the toolbox.
First **real** LLM agent for L3 + L5 on a mature language. Calibrate
empirical cold-token budget.

**Deliverables:**

- L3 classify prompt template for Cargo-language components
  (`crates/atlas-agents/prompts/l3_classify_cargo.md`).
- L5 surface prompt template for Cargo-language components.
- Retirement of `cargo_classifier.rs` and Cargo-specific surface
  analyser code (or collapse to text-scoping tool implementations
  where appropriate).
- Updated Phase 3 polyglot smoke test budget assertion: Cargo-language
  fixtures shift from today's reference to the empirical Phase-8
  number.
- Reference-output comparison harness. Structural-equivalence
  (not byte-equality) is the bar: same set of components, same
  set of contracts, same set of edges (modulo justifiable
  refinements). Differences are reviewed and either
  accepted-as-improvement (LLM agent found a real signal the
  deterministic classifier missed) or flagged-as-regression
  (LLM agent dropped a signal the deterministic classifier had).
  Specific equivalence rules are a plan-time decision.

**Acceptance:** Cargo-language outputs match (or improve on)
reference outputs; cold token budget is locked in; warm=0 still
holds.

### 11.3 Stream 2 — Phase 9 (remaining language retirements, in waves)

**Phase 9a — TS/JS + Python.** Next-most-mature deterministic
analysers. Surface analyser code collapses to text-scoping helper
implementations.

**Phase 9b — C# + Dart.** Mid-tier. Mature manifest parsing; weaker
surface analysers (which retire entirely).

**Phase 9c — Elixir + Racket + LispKit + Compose + Dockerfile.**
Weak-tooling languages. No text-scoping helpers; agents
read whole files. Make/shell classifier (deferred from Phase 6
pre-pivot brainstorm) folds in here.

Each wave is its own phase with its own PRs, budget assertions, and
reference-output comparisons. Wave ordering is fixed (a → b → c);
within a wave, language order is flexible.

### 11.4 Stream 3 — Phase 10 (LLM-driven analyses)

**Moved earlier than today's §10.9 placement.** Once the agent
runtime exists and all languages are LLM-driven, pattern detection
and fuzzy contract matching become natural.

**Deliverables:**

- Pattern detection (recurring component / edge shapes; anti-patterns)
  as a new L8 agent stage.
- Fuzzy contract matching (deferred from Phase 6 pre-pivot brainstorm)
  as an extension of contract rename-match using semantic similarity
  rather than only owner-follows or content-sha-stability.
- Modularity reports (already in Phase 3) get qualitative LLM-driven
  augmentation alongside the quantitative metrics.
- LLM confidence threshold calibration (today's §11.2.6) — empirical
  thresholds set against the now-richer agent landscape.

### 11.5 Stream 4 — Phase 11 (server mode)

**Unchanged target end-state.** Long-running service with reactive
recomputation, query API, file watcher, Salsa input updates,
gRPC / HTTP+GraphQL, subscriptions, lifecycle, GC.

**New for the recast:** the web-app subscriber to the event bus.
The server already runs the bus across process boundaries; the web
app subscribes via WebSocket / SSE.

---

## 12. Phase 6 disposition

Phase 6 ships *before* Phase 7 begins, as the final
**deterministic-spine release**. The four pre-pivot brainstorm
items survive the recast untouched and strengthen the user-authoring
override discipline that the recast depends on.

| # | Item | Recast-relevance |
|---|---|---|
| 1 | `is_manifest_file` Makefile/shell extension | Pure deterministic L1/L2 plumbing. Manifest recognition is the boundary signal *before* any agent runs. Survives untouched. |
| 2 | Contract rename-match owner-follows | Pure deterministic identity tracking. Owner-follows survives; independent fuzzy-contract-matching folds into Phase 10. |
| 3 | `subsystem` field overlay | Wires user-authored override surface. Under the recast, LLM-discovered subsystem partitions sit *underneath* user-authored overlays. Survives — and gets *more* important under the recast, since LLM discovery makes user override more frequently exercised. |
| 4 | `--strict-overrides` + closed enumeration + dual-mode contract test | Warning-machinery plumbing. Independent of LLM/deterministic boundary. Survives untouched. |

PR enumeration (from pre-pivot brainstorm, preserved):

- **PR-0**: plan + status + continuation prompt.
- **PR-1 + PR-2 (Wave 1, parallel-safe)**: items #1 + #2.
- **PR-3 (Wave 2)**: item #3.
- **PR-4 (Wave 3, depends on PR-3's warning surface)**: item #4.
- **PR-5 (Wave 4)**: acceptance + closeout + canonical §10 retext
  (this spec's §13 lands here).

Items struck during the pre-pivot brainstorm stay struck:

- Worktree commit-sha annotations — *dropped* (Phase 5 collapsed
  the motivating multi-root case).
- Cache compression — *deferred to its own cache-architecture phase*
  (post-Phase-11, slot TBD).
- Make/shell classifier — *folded into Phase 9c*.
- Independent fuzzy contract matching — *folded into Phase 10*.

---

## 13. Canonical §10 retext

This retext lands in Phase 6 PR-5 (acceptance + closeout), updating
`docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`
§10 (Phasing and migration) and §4.3 (Architectural principles).

### 13.1 §10 retext (full table)

| § | Phase | Status | Scope |
|---|---|---|---|
| 10.1 | Phase 1 | Shipped | Architectural seam (multi-root retired Phase 5). |
| 10.2 | Phase 2 | Shipped | Pluggability and polyglot. |
| 10.3 | Phase 3 | Shipped | Drift, impact, modularity reports. |
| 10.4 | Phase 4 | Shipped | Cleanup release. |
| 10.5 | Phase 5 | Shipped | Monorepo consolidation, part 1. |
| 10.6 | Phase 6 | Now-next | User-facing schema cleanups (final det-spine release). |
| 10.7 | **Phase 7 (new)** | After Phase 6 | **LLM-spine runtime** — agent runtime, toolbox, transcript cache, event bus, TUI, fixed-point loop, audit lane. No language retirements. |
| 10.8 | **Phase 8 (new)** | After Phase 7 | **Cargo retirement** — first language LLM-driven; cold-token budget calibration. |
| 10.9 | **Phase 9 (new, in waves)** | After Phase 8 | **Remaining language retirements** — wave a: TS/JS + Python; wave b: C# + Dart; wave c: Elixir + Racket + LispKit + Compose + Dockerfile. |
| 10.10 | **Phase 10 (new)** | After Phase 9 | **LLM-driven analyses** — pattern detection, fuzzy contract matching, qualitative modularity. Moved *earlier* than today's §10.9. |
| 10.11 | **Phase 11** | Last | Server mode + web-app subscriber to event bus. |

The §10.11 "Migration from v1" subsection remains marked OBSOLETE
(already so today; unchanged).

### 13.2 §4.3 retext

Replaces today's §4.3 ("Determinism over fuzziness") with the
§3.1 text above. Adds a one-line forward-pointer:

> The Phase 6 → Phase 7 boundary is the inversion moment in the
> codebase. Phase 6 ships as the final deterministic-spine release;
> Phase 7 ships the LLM-spine runtime; subsequent phases retire
> language-specific deterministic analysers in waves.

### 13.3 §7 reframe

§7.1 (Analyser interface) is replaced by §5.1 of this spec (the
`Tool` trait). §7.3 (Cost classes and dispatch) retires entirely.
§7.2 (Subprocess analyser protocol) survives in concept but
generalises to subprocess **tools** — the protocol shape (stdio
JSON, handshake, lifecycle) is unchanged.

### 13.4 §8 extension

§8.1 (fingerprint discipline) extends per §6.1 of this spec
(`iteration_number` + `prior_model_sha` added to the L3/L5/L6/L8
discriminators). §8.2 (cross-component invalidation), §8.3 (cache
durability), §8.4 (cache GC) survive unchanged.

---

## 14. Open questions and deferred work

These are explicit *known unknowns* that the spec defers to plan-time
or later phases.

### 14.1 Plan-time decisions (Phase 7 PR-0)

- **TUI library choice.** `ratatui` is the default; plan-time
  confirms.
- **Backend transport choice for agent runtime.** Atlas today
  supports multiple LLM backends (`crates/atlas-llm/src/`:
  `claude_code.rs`, `codex.rs`, `http_anthropic.rs`,
  `http_openai.rs`). The agent runtime needs at least one
  tool-use-capable backend; choice (Anthropic Messages with tools,
  OpenAI function-calling, etc.) is plan-time.
- **Iteration cap initial value.** Spec says `K = 5`; plan-time
  may revise based on initial Phase 7 calibration.

### 14.2 Empirical calibration moments

- **Phase 8**: cold token budget for Cargo-language fixtures.
- **Phase 9 waves**: cold token budgets for each retiring language.
- **Phase 10**: pattern-detection prompt thresholds; fuzzy-match
  similarity thresholds.

### 14.3 Deferred to post-Phase-11

- **Cache compression** (struck from Phase 6 pre-pivot brainstorm).
  Becomes a cache-architecture phase post-Phase-11 if cache size
  pressure motivates it.
- **SQLite-backed cache** (today's §8.3 mention as a future
  admissible alternative). Same trigger.

### 14.4 Out of scope for this spec entirely

- **LLM model selection / routing strategy** beyond "pick a tool-use-capable
  backend." Atlas does not become a model-routing platform; it uses
  whatever backend is configured.
- **Cost tracking / billing surfaces.** Token totals are observable
  for UX (TUI cost-to-date) but not aggregated into reports.
- **Multi-tenant server mode.** Phase 11 is single-user server mode
  per today's §9; multi-tenant is post-Phase-11.

---

## 15. Glossary

- **Agent.** An LLM-driven unit of analytical work. Takes an input
  fingerprint and a prompt template; invokes tools; emits an output
  with confidence grade and evidence. Runs through the transcript
  cache.
- **Agent runtime.** The Phase 7 deliverable that orchestrates agent
  dispatch, tool invocation, transcript cache replay, audit lane,
  fixed-point iteration, and event bus.
- **Audit lane.** The validation surface around each agent's output.
  Lane A is deterministic structural validation (always); Lane B is
  LLM audit on low confidence.
- **Convergence (fixed-point).** Two consecutive iterations of the
  agent tree produce the same L9 projection sha. Convergence is
  deterministic; no LLM oracle.
- **Context rot.** LLM performance degradation as irrelevant context
  accumulates — earlier turns dilute attention, instructions drift,
  framing weakens. Mitigated by fresh sessions per PR plus lean
  prompts.
- **Dispatch agent.** An interior tree node whose role is to decide
  the partition of its scope (workspace → subsystems, subsystem →
  components). Dispatch decisions land as reviewable disk artefacts.
- **Event bus.** In-process channel carrying typed `AgentEvent`s
  from the runtime to subscribers (TUI, JSON-Lines logger,
  transcript-cache writer, web-app).
- **Fixed-point iteration.** The map-reduce primitive runs the agent
  tree multiple times until two consecutive iterations produce the
  same L9 projection sha. Sub-tree discoveries propagate as parent
  reduce-step outputs that become input context for the next
  iteration.
- **Hard fail.** A stage that cannot produce complete output exits
  the CLI non-zero. No partial output. Cache state remains durable.
- **Map-reduce primitive.** The architectural pattern of the
  analytical work — a tree of LLM agents, hybrid-by-stage in its
  unit-of-work, with deterministic L9 aggregation as the reducer at
  the root.
- **Pair agent.** An L6 edge-derivation agent for a single
  dependency-pair, hosted at the LCA of the two participants in
  the tree.
- **Tool.** A deterministic Rust function exposed to LLM agents via
  the `Tool` trait. Tools parse manifests, scope text, read files,
  query cache state. Tools never call the LLM.
- **Toolbox.** The set of `Tool` implementations available to
  agents. Text-biased; language-calibrated.
- **Transcript.** The full record of a single agent invocation
  (initial prompt, tool calls and results, completions, final
  output). Stored in cache; not replayed against the LLM on cache
  hit.
- **Transcript cache.** The Phase 7 extension of today's single-shot
  LLM cache to multi-shot agent runs.
- **Warm=0.** No-op re-run after a converged successful run does
  zero LLM calls. Load-bearing correctness invariant.

---

## 16. References

- `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` —
  canonical system-model design (the document this recast amends).
- `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-design.md` —
  Phase 5 design (shipped 2026-05-10; the immediately preceding
  phase).
- `.claude/memory/feedback_atlas_llm_spine_intent.md` — the load-bearing
  strategic preference that motivated this recast.
- `.claude/memory/project_phase6_paused_for_llm_spine.md` — captures
  the Phase 6 pre-pivot brainstorm state and the four candidate items.
- `.claude/memory/feedback_worktree_base_verification.md` — subagent
  dispatch discipline (worktree base-sha verification) honoured by
  §10.4.
- `crates/atlas-engine/src/llm_cache.rs` — today's single-shot LLM
  cache implementation; the transcript cache extends this.
- `crates/atlas-engine/src/fixedpoint.rs` — today's multi-root
  fixed-point iteration; the pattern the agent-tree fixed-point
  extends.
- `crates/atlas-llm/src/` — today's backend abstractions; the agent
  runtime will sit alongside.
