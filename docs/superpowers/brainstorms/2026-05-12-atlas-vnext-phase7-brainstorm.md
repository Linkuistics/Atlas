# Atlas vNext Phase 7 — LLM-spine runtime (brainstorm)

Status: brainstormed 2026-05-12. This is a *brainstorm artifact*, not a plan.
The canonical Phase 7 plan (`docs/superpowers/specs/2026-05-NN-atlas-vnext-phase7-plan.md`),
status file (`docs/superpowers/plans/2026-05-NN-phase7-status.md`), and
continuation prompt (`docs/superpowers/prompts/2026-05-NN-vnext-continue.md`) are
PR-0 work downstream of this brainstorm.

Phase 7 begins the LLM-spine inversion per the recast spec
(`docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`). Phase 6
shipped 2026-05-11 as the final deterministic-spine release (Atlas main HEAD
9350735).

---

## 0. Reading order

§1 (one-paragraph summary) → §2 (architectural pivots and PR-0 decision table) →
§3 (Wave structure: 8 PRs across 5 waves) → §4 (Wave 1 foundation: PR-1 + PR-2) →
§5 (Wave 2 tool wrappers: PR-3 three parallel subagents) →
§6 (Wave 3 orchestration: PR-4 + PR-5) →
§7 (Wave 4 UX: PR-6) → §8 (Wave 5 closeout: PR-7) →
§9 (testing strategy) → §10 (acceptance criteria) →
§11 (deferred-to-PR-0 plan-time decision list) → §12 (open risks).

---

## 1. Summary

Phase 7 builds the **LLM-spine agent runtime** (recast spec §11.1) on top of
today's deterministic engine without retiring any language analyser. The
runtime is **async Tokio** with per-transport + per-stage semaphores; it
**dual-transports** both subprocess (`claude_code`, `codex`) and HTTP
(`http_anthropic`, `http_openai`) backends; it exposes a **unified `Tool`
trait** to LLM agents via an **in-process MCP stdio server** (subprocess
backends connect with built-in tools disabled; HTTP backends call
`Tool::invoke()` directly).

Dispatch is **LLM-decided** at the workspace and subsystem levels with
**override-file shortcircuit**: when `subsystems.overrides.yaml` /
`components.overrides.yaml` is present and Lane-A-valid, the dispatch agent
emits a synthetic cache-hit transcript without invoking the LLM. The polyglot
smoke fixture has full override coverage → cold dispatch cost = 0 on that
fixture → cold token total matches today's reference. Atlas-on-Atlas typically
lacks `subsystems.overrides.yaml` → dispatch fires → Phase 7 calibrates a new
Atlas-on-Atlas cold baseline.

A **transcript cache primitive** extends today's two-tier write-through single-
shot cache (`crates/atlas-engine/src/llm_cache.rs`) to multi-shot agent runs,
keyed on `(stage_id || agent_id || agent_version || prompt_template_sha ||
tool_catalog_sha || model_id || backend_version || transport_flavour ||
target_input_shas || iteration_number || prior_model_sha)`. The new
`transport_flavour` discriminator (added by dual-transport) prevents
cache-pollution across `claude_code` / `codex` / `http_anthropic` /
`http_openai`. Warm=0 holds per-transport.

An **event bus** (Tokio `broadcast::channel`, capacity ~1024,
lagged-receiver = error-and-log) carries `AgentEvent`s to multiple subscribers:
the **TUI subscriber** (`ratatui`, live tree view, iteration counter, token
cost in real time, stuck detection, replay mode), the **JSON-Lines logger**
(`--no-tui` and `--log-events` flags), and the **transcript-cache writer**
(materialises cache entries from `AgentComplete` events). A drain handshake
guarantees all subscribers flush before `AgentRuntime::run()` returns.

A **fixed-point iteration loop** extends the
`crates/atlas-engine/src/fixedpoint.rs` pattern (monotonic-growth model-sha
equality) to the analysis content itself: two consecutive iterations
producing the same L9 projection sha = converged. `iteration_number +
prior_model_sha` enter every per-agent transcript-cache fingerprint, so
within-run cache replay across iterations is automatic for agents whose
inputs are stable. Default `K = 5`; hard-fail at `K+1` with diagnostic
listing which agents shifted between iterations.

A **two-lane audit surface** validates every agent's output: **Lane A**
(always) is deterministic schema validation; **Lane B** (on `Weak |
Declines` confidence grade) is **cross-provider LLM audit** — an
Anthropic-flavoured producer is reviewed by an OpenAI-flavoured auditor, and
vice versa. Cross-provider audit empirically beats same-model self-audit (the
producer's blind spots become the auditor's blind spots otherwise). Single-
provider configs fall back to same-model audit with an explicit
`AuditDegraded` event-bus warning.

Phase 7 ships **no language retirements**. Each of today's 10 classifier
modules and 7 surface analyser modules is wrapped as a thin `Tool`
implementation (pass-through invocation, no behaviour change). Per-component
agents call these wrappers via the toolbox. Today's `dispatcher.rs` /
`registry.rs` / `llm_classify.rs` / `shell_script_llm_analyzer.rs` stay
intact; they retire in Phase 8 onward.

Execution discipline (per recast §10): **8 PRs across 5 waves**; each PR is
small + atomic + ships a working state; PR-3 (Wave 2 tool wrappers)
dispatches three parallel subagent worktrees by language tooling maturity.

---

## 2. Architectural pivots and PR-0 decision table

| # | Pivot / decision | Resolution | Rationale |
|---|---|---|---|
| 1 | PR slicing axis | Wave-first, 5 waves (foundation / tool-wrappers / orchestration / UX / closeout), 8 PRs total | Highest subagent-parallelism leverage on Wave 2 (26 independent wrappers); cadence matches Phase 5/6 |
| 2 | Dispatch semantics in Phase 7 | LLM-decided dispatch *on*, with override-file shortcircuit. Polyglot smoke (full overrides) → cold=reference; Atlas-on-Atlas (no override) → calibrate new baseline | Honours both §11.1 (cold-matches-reference) and §4.2 (fully-LLM-decided dispatch); aligns with Phase 6 PR-3 subsystem overlay's user-authoring discipline |
| 3 | Backend transport | Dual: subprocess (`claude_code`, `codex`) + HTTP (`http_anthropic`, `http_openai`); all four drive the runtime | Subprocess is daily-driver (subscription-subsidized; see `project_atlas_common_backend_config`); HTTP is signal-gathering for "is HTTP-tool-use-loop better?" |
| 4 | `Tool` trait bridge | Unified envelope. In-process stdio MCP server in `crates/atlas-agents/` re-exposes `Tool` impls; subprocess backends connect with `--mcp-config` + disabled built-in tools; HTTP backends call `Tool::invoke()` directly | Single trait, single fingerprint hook, uniform transcripts; §5.4 invariant ("tools never call LLM internally") enforceable across transports |
| 5 | Concurrency | Async Tokio task pool. Per-transport semaphores (initial defaults: `HTTP=8`, `subprocess=2`); per-stage semaphores cap in-flight siblings; engine→agents boundary is a single `Handle::block_on(...)` | Natural fit for §9.1 Tokio broadcast event bus; subprocess-vs-HTTP latency asymmetry needs separate caps; engine stays sync until Phase 11 forces inversion |
| 6 | TUI library | `ratatui` | Default in spec §9.2 / §14.1; async-render-loop compatible with Tokio runtime |
| 7 | Iteration cap default | `K = 5`; calibrate against Atlas-on-Atlas in PR-7 | Spec §4.4 default; Phase 7 captures empirical data for PR-0 recalibration |
| 8 | Audit-lane confidence grade enum | `enum Grade { Strong, Moderate, Weak, Declines }`; Lane B fires on `Weak | Declines` | Mapping in prompt templates (LLM grades own output); Lane A schema validation enforces parse-time |
| 9 | Transcript-cache key shape | `(stage_id || agent_id || agent_version || prompt_template_sha || tool_catalog_sha || model_id || backend_version || transport_flavour || target_input_shas || iteration_number || prior_model_sha)`; atomic-write via Phase 4 `atomic_write` helper (extended to two-file atomic-pair primitive — see §12 risk #2) | Recast §6.1 + transport-flavour discriminator from dual-transport; prevents cross-transport cache pollution |
| 10 | Event bus format | Tokio `broadcast::channel<AgentEvent>` capacity ~1024; lagged-receiver = error-and-log (not silent-drop); drain handshake before `AgentRuntime::run()` returns | Recast §9.1 + (D) concurrency follow-on |
| 11 | Lane B auditor | Cross-provider: Anthropic-flavoured producer → OpenAI-flavoured auditor, and vice versa. Single-provider config: same-model fallback with `AuditDegraded` warning | Cross-provider audit empirically beats same-model self-audit (see `feedback_cross_provider_llm_audit`); producer/auditor blind-spot asymmetry is the mechanism |
| 12 | Async surface on `LlmBackend` | Add `async fn call_async(...) -> Result<...>` alongside today's sync `fn call(...)`; sync wrapper preserved for non-agent callers | Required for (D); avoids forcing non-agent paths through `block_on` |
| 13 | Default `BackendRouter` config | `claude_code` + `codex` paired (cross-provider audit out-of-box, no HTTP backend needed) | Common Atlas deployment (see `project_atlas_common_backend_config`); subscription-subsidized; cross-provider Lane B works on first run |
| 14 | Budget posture | Coarse: single cold token total assertion in polyglot smoke (regression detector); TUI cost display informational only; no per-provider buckets; no runtime gates | Recast §2.4 / §8.4 ("Budget is observed and asserted in tests as a regression detector, never enforced as a runtime cap"); fine-grained tracking is over-engineered |

---

## 3. Wave structure: 8 PRs across 5 waves

| Wave | PR | Scope | Parallel? |
|---|---|---|---|
| 0 | PR-0 | Plan + status + continuation prompt; close items 1-14 of §2 | n/a |
| 1 (foundation) | PR-1 | `crates/atlas-agents/` skeleton + `Tool` trait + MCP server + async surface on `LlmBackend` | Sequential |
| 1 (foundation) | PR-2 | Transcript cache primitive + event bus + JSON-Lines logger subscriber | Sequential (depends on PR-1) |
| 2 (tool wrappers) | PR-3 | 26 tool wrappers (9 manifest + 10 classifier + 7 surface analyser) | **Three parallel subagents** by language tooling maturity |
| 3 (orchestration) | PR-4 | Agent runtime (single-iteration; deterministic-only dispatch) + Lane A schema validation | Sequential (depends on PR-2, PR-3) |
| 3 (orchestration) | PR-5 | Fixed-point iteration loop + LLM-decided dispatch with override-shortcircuit + Lane B cross-provider auditor | Sequential (depends on PR-4) |
| 4 (UX) | PR-6 | TUI subscriber (`ratatui`) + `--no-tui` JSON-Lines fallback + `--replay-from-cache` mode | Sequential (depends on PR-2) |
| 5 (closeout) | PR-7 | End-to-end wiring + polyglot smoke extension + Atlas-on-Atlas calibration + acceptance + closeout | Sequential (depends on PR-5, PR-6) |

PR count matches Phase 5 cadence (7 PRs). Each PR ships a working state with
tests passing; no "this PR depends on the next." PR-3 is the only
multi-subagent PR.

---

## 4. Wave 1 — Foundation (PR-1 + PR-2)

### PR-1 — `atlas-agents` crate + `Tool` trait + MCP server + async surface

New crate layout under `crates/atlas-agents/`:

```
src/
├── lib.rs              — public re-exports
├── tool.rs             — Tool trait, ToolArgs, ToolResult, ToolError, ToolContext, FingerprintInput
├── mcp/
│   ├── mod.rs          — Atlas-side in-process MCP stdio server
│   ├── server.rs       — JSON-RPC framing + tool-dispatch loop + multi-client multiplexing
│   └── descriptors.rs  — Tool::json_schema() → MCP tool-descriptor conversion
└── runtime/            — empty stub; populated by PR-4
```

`Tool` trait (async-ised for concurrency model (D)):

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn json_schema(&self) -> &ToolSchema;   // both MCP descriptors and HTTP tool-use API
    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext)
                    -> Result<ToolResult, ToolError>;
    fn fingerprint_inputs(&self, args: &ToolArgs)
                          -> Vec<FingerprintInput>;
}
```

Async surface added to `LlmBackend` in `crates/atlas-llm/src/lib.rs`:

```rust
pub trait LlmBackend: Send + Sync {
    fn call(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;      // existing
    async fn call_async(&self, req: LlmRequest)                             // new
        -> Result<LlmResponse, LlmError>;
    fn fingerprint(&self) -> LlmFingerprint;                                // unchanged
}
```

Subprocess backends (`claude_code`, `codex`) get native `call_async` via
`tokio::process::Command`. HTTP backends (`http_anthropic`, `http_openai`)
get native `call_async` via `reqwest::Client` (replacing today's blocking
adapter). Existing `BackendRouter::call(...)` (sync) is preserved for
non-agent prompts (`llm_classify` fallback, `shell_script_llm_analyzer`); a
parallel `BackendRouter::call_async(...)` is added for agent prompts.

The MCP server runs as a Tokio task inside Atlas's process. **Multi-client
multiplexing** is required: with the default `BackendRouter` config
(`claude_code` + `codex`), two subprocess clients connect concurrently, each
over its own stdio pipe, each with disabled built-in tools
(`--disallowedTools=Read,Grep,Glob,Bash,Write,Edit` for claude-code; each
provider's equivalent restriction set for codex). PR-1 documents the
restriction sets.

### PR-2 — Transcript cache + event bus + JSON-Lines subscriber

`crates/atlas-engine/src/llm_cache.rs` gains a multi-shot extension:

```rust
pub fn call_agent_cached(
    stage: Stage,
    fingerprint: AgentInputFingerprint,
    request: AgentRequest,
) -> Result<AgentResult, AgentError>;
```

Same two-tier L1 (in-memory) + L2 (persistent) write-through pattern as
today's single-shot cache. Persistent layout:
`.atlas/cache/agents/<stage>/<sha>.transcript` +
`.atlas/cache/agents/<stage>/<sha>.output` (separate from single-shot cache
layout, no key collision). Atomic writes via Phase 4's `atomic_write` helper
(extended to two-file atomic-pair primitive — see §12 risk #2). Cache-hit path
spot-checks recorded `fingerprint_inputs` against current `file_sha(path)`
and evicts on mismatch (per recast §6.3).

Event bus in `crates/atlas-agents/src/events.rs`:

```rust
pub enum AgentEvent {
    IterationBoundary { iter: u32, prior_model_sha: Option<ContentSha> },
    AgentStart    { agent_id, parent_id, stage, target, fingerprint, started_at },
    ToolCall      { agent_id, tool_name, args_summary },
    ToolResult    { agent_id, tool_name, result_summary, ms, bytes },
    AgentComplete { agent_id, output_sha, confidence_grade, tokens_in, tokens_out, ms, provider },
    AuditFire     { agent_id, audit_reason, auditor_provider },
    AuditVerdict  { agent_id, verdict },
    AuditDegraded { reason: &'static str },     // single-provider fallback
    HardFail      { agent_id, error_kind, error_summary, retry_count },
    CacheHit      { agent_id, fingerprint, replayed_at, source: CacheHitSource },
    RuntimeComplete,    // drain-handshake sentinel
}

pub struct EventBus { tx: tokio::sync::broadcast::Sender<AgentEvent> }

impl EventBus {
    pub fn new(capacity: usize) -> Self;        // capacity = 1024
    pub fn subscribe(&self) -> Subscriber;
    pub fn emit(&self, event: AgentEvent);
}
```

Subscribers (per recast §9.1):

- **JSON-Lines subscriber** (in `crates/atlas-cli/src/jsonl_subscriber.rs`):
  one event per line to stdout when `--no-tui` set, or to a file when
  `--log-events events.jsonl` set.
- **Transcript-cache writer subscriber** (in
  `crates/atlas-engine/src/agent_cache_writer.rs`): materialises cache
  entries from `AgentComplete` events. Async-fire-and-forget — but the
  drain-handshake on `RuntimeComplete` guarantees all writes flush before
  `AgentRuntime::run()` returns, honouring §6.4's "on success, write
  transcript + output atomically" requirement.
- **TUI subscriber** lands in PR-6.

Tokio `broadcast::channel` is the transport. Lagged receivers (slow
subscribers) get an explicit `RecvError::Lagged(n)` returned from
`recv().await` — PR-2 logs this as an error and emits a `HardFail` event;
silent drop is forbidden because dropping `AgentComplete` events would
corrupt the transcript cache.

---

## 5. Wave 2 — Tool wrappers (PR-3, three parallel subagents)

PR-3 is the only multi-subagent PR. 26 wrappers split by language tooling
maturity:

| Subagent | Owns | Modules wrapped |
|---|---|---|
| **PR-3a Mature** | Rust + TS/JS surface analysers + classifiers + universal manifest-parser set | `rust_surface_analyzer.rs`, `ts_js_surface_analyzer.rs`, `cargo_classifier.rs`, `ts_js_classifier.rs`, plus 9 manifest parsers (`parse_cargo_toml`, `parse_package_json`, `parse_pyproject`, `parse_csproj`, `parse_dockerfile`, `parse_compose`, `parse_k8s_manifest`, `parse_helm_chart`, `parse_release_toml`) |
| **PR-3b Mid-tier** | Python + C# + Dart | `python_classifier.rs`, `python_surface_analyzer.rs`, `csharp_classifier.rs`, `csharp_surface_analyzer.rs`, `dart_classifier.rs`, `dart_surface_analyzer.rs` |
| **PR-3c Weak-tooling** | Elixir + Racket + LispKit + Compose + Dockerfile | `elixir_classifier.rs`, `elixir_surface_analyzer.rs`, `racket_classifier.rs`, `racket_surface_analyzer.rs`, `lispkit_classifier.rs`, `lispkit_surface_analyzer.rs`, `compose_classifier.rs`, `dockerfile_classifier.rs` |

Each wrapper follows the same pass-through pattern:

```rust
pub struct CargoClassifyTool;

#[async_trait]
impl Tool for CargoClassifyTool {
    fn id(&self) -> &'static str { "classify_cargo_component" }
    fn version(&self) -> &'static str { "v1" }
    fn json_schema(&self) -> &ToolSchema { /* args: component_id; result: Kind + evidence */ }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext)
                    -> Result<ToolResult, ToolError> {
        // Reads filesystem via ctx.fs; calls into atlas_analyzers::cargo_classifier
        // (sync — wrapped in spawn_blocking). Returns existing classifier
        // output verbatim. No LLM, no reasoning of its own.
    }

    fn fingerprint_inputs(&self, args: &ToolArgs) -> Vec<FingerprintInput> {
        // Cargo.toml sha, Cargo.lock sha, src/lib.rs / src/main.rs presence
    }
}
```

Today's `dispatcher.rs` / `registry.rs` / `llm_classify.rs` /
`shell_script_llm_analyzer.rs` are **not** touched by Wave 2. They retire in
Phase 8 onward.

Subagent base verification: per `feedback_worktree_base_verification.md`,
the PR-3 driver verifies `git rev-parse HEAD` matches current main HEAD
and that each worktree base is the verified sha before dispatching the
three parallel subagents. Each subagent commits to its worktree branch; PR-3
lands as a single merged PR pulling all three branches.

---

## 6. Wave 3 — Orchestration (PR-4 + PR-5)

### PR-4 — Agent runtime (single-iteration) + Lane A

Tree under `crates/atlas-agents/src/runtime/`:

```
runtime/
├── mod.rs              — AgentRuntime struct + run() entry point + state machine
├── agent.rs            — Agent (one instance per stage-target-iteration tuple)
├── dispatch.rs         — Workspace→subsystem→component partitioning. PR-4: deterministic-only.
├── tool_loop_http.rs   — HTTP-side tool-use loop (Atlas owns dispatch, records transcript byte-for-byte)
├── tool_loop_mcp.rs    — MCP-side observation (subprocess owns dispatch, Atlas MCP server records each call)
└── audit/
    └── lane_a.rs       — Schema validation (always fires; one retry on fail; second fail = hard fail)
```

`AgentRuntime`:

```rust
pub struct AgentRuntime {
    backend_router: Arc<BackendRouter>,
    tools: Arc<ToolCatalog>,
    cache: Arc<AgentCache>,
    event_bus: Arc<EventBus>,
    semaphores: Semaphores,    // per-transport + per-stage caps
}

impl AgentRuntime {
    pub async fn run_workspace(&self, workspace: &Workspace)
        -> Result<L9Projection, AgentError>;

    async fn call_agent(&self, request: AgentRequest)
        -> Result<AgentResult, AgentError>;
}
```

PR-4's dispatch is **deterministic-only**: workspace dispatch reads
`subsystems.overrides.yaml` (mandatory in PR-4; PR-5 makes it optional) +
Phase 6 PR-3's subsystem field overlays; component dispatch reads
`components.overrides.yaml` (mandatory in PR-4) + L1 candidate emission.
Lane A schema validation enforces ontology constraints per recast §4.3.

Per-component agents invoke Wave 2 wrappers via the toolbox; each agent's
prompt template is essentially "call `classify_$language` and
`surface_$language`; emit their outputs verbatim with `Grade::Strong`."
This ensures PR-4's polyglot smoke cold token total matches today's reference.

### PR-5 — Fixed-point iteration + LLM-decided dispatch + Lane B

Three subsystems added:

**(i) LLM-decided dispatch with override-shortcircuit.** Workspace +
subsystem dispatch agents invoke the LLM *when no override file is present*:

```rust
async fn dispatch_subsystems(workspace: &Workspace) -> Vec<SubsystemPartition> {
    let override_path = workspace.root().join("subsystems.overrides.yaml");
    if override_path.exists() {
        let content = read_and_lane_a_validate(&override_path).await?;
        event_bus.emit(CacheHit { source: CacheHitSource::DispatchedFromOverride,
                                  sha: content_sha });
        cache.write_synthetic_transcript(...);
        return parse_partitions(content);
    }
    let agent_result = self.call_agent(workspace_dispatch_request(workspace)).await?;
    parse_partitions(agent_result.output)
}
```

Polyglot smoke (overrides fully populated) → zero dispatch LLM calls → cold
matches today's reference. Atlas-on-Atlas (no override) → dispatch fires →
calibrated baseline. Dispatch outputs land as
`.atlas/discovery/subsystem-plan.yaml` / `component-plan.yaml`, reviewable
and override-able per recast §4.1.

Cache-invariant rule: the dispatch agent's fingerprint includes
`override_file_content_sha` (or sentinel `None` if absent). Adding an
override invalidates the LLM-dispatch transcript; removing an override
invalidates the synthetic-from-override transcript. PR-0 names this rule
explicitly.

**(ii) Fixed-point iteration loop.** New file
`crates/atlas-agents/src/runtime/fixedpoint_loop.rs`:

```rust
pub async fn run_fixedpoint(runtime: &AgentRuntime, workspace: &Workspace, max_iter: u32)
    -> Result<L9Projection, AgentError> {
    let mut prior_model_sha: Option<ContentSha> = None;
    for iter in 1..=max_iter {
        event_bus.emit(IterationBoundary { iter, prior_model_sha });
        let l9 = runtime.run_workspace_at_iteration(workspace, iter, prior_model_sha).await?;
        let l9_sha = content_sha(&l9);
        if Some(l9_sha) == prior_model_sha { return Ok(l9); }
        prior_model_sha = Some(l9_sha);
    }
    Err(AgentError::FixedpointDiverged { iterations: max_iter, last_changed_agents: ... })
}
```

`iteration_number + prior_model_sha` enter every per-agent transcript-cache
fingerprint. Within-run cache replay across iterations is automatic for
agents whose inputs are stable. Convergence judge is deterministic content-
sha equality. Default `K = 5`; hard-fail at `K+1` with diagnostic listing
which agents shifted between iterations.

**(iii) Lane B cross-provider auditor.**
`crates/atlas-agents/src/runtime/audit/lane_b.rs`:

```rust
async fn lane_b_audit(producer_result: &AgentResult, producer_provider: Provider)
    -> Result<AuditVerdict, AgentError> {
    if !matches!(producer_result.confidence_grade, Grade::Weak | Grade::Declines) {
        return Ok(AuditVerdict::Skipped);
    }
    let auditor_provider = match producer_provider {
        Provider::Anthropic => Provider::OpenAI,
        Provider::OpenAI => Provider::Anthropic,
    };
    let auditor = match backend_router.for_provider(auditor_provider) {
        Some(b) => b,
        None => {
            event_bus.emit(AuditDegraded { reason: "single-provider config" });
            backend_router.for_provider(producer_provider).unwrap()
        }
    };
    let verdict = auditor.review_agent_output(producer_result).await?;
    Ok(verdict)
}
```

Audit verdict on-disk artefact at `.atlas/audit/<stage>/<target>.yaml`:

```yaml
agent_id: classify_atlas-engine
producer:
  provider: anthropic
  model: claude-sonnet-4-6
  output_sha: 2b91...
auditor:
  provider: openai
  model: gpt-5-codex
  verdict: accept            # accept | request_revision | hard_fail
  evidence: "Producer's edge_kinds field is consistent with §6.3 ontology;
             evidence chain references actual file lines."
audit_token_cost: { in: 1240, out: 320 }
```

Per recast §4.3: max one Lane B retry per audit firing; cumulative max two
retries per agent (Lane A + Lane B); hard-fail beyond. Audit transcripts
land in `.atlas/cache/audit/<stage>/<sha>.transcript`, separate from
production transcripts, same key structure + `audit_provider`.

---

## 7. Wave 4 — UX (PR-6)

Layout under `crates/atlas-cli/src/`:

```
cli/src/
├── tui/
│   ├── mod.rs           — TUI runtime; subscribes to EventBus; owns ratatui terminal
│   ├── state.rs         — Arc<Mutex<TuiState>> mutated from events
│   ├── tree_view.rs     — workspace→subsystem→component live tree widget
│   ├── token_panel.rs   — running token-cost display (optional per-provider breakdown)
│   ├── iteration_bar.rs — iteration counter + convergence indicator
│   └── stuck_detect.rs  — 90s heuristic
└── jsonl_subscriber.rs   — `--no-tui` fallback (already added in PR-2)
```

CLI defaults per recast §9.2:

- stdout is terminal AND `--no-tui` not set → TUI subscriber active.
- stdout piped OR `--no-tui` set → JSON-Lines subscriber active.
- `--log-events events.jsonl` always adds JSON-Lines file logger as a
  *parallel* subscriber, regardless of TUI mode.

Token-cost display: live sum across the bus; optional
`--tui-show-providers` flag splits `tokens_in + tokens_out` per backend
provider for user awareness. Since budget is coarse (no per-provider
invariants), this is purely informational.

Replay mode: `atlas index --replay-from-cache` runs the TUI against cached
transcripts without invoking any backend. Single-transport: you replay the
transport you originally ran (transcript-cache `transport_flavour`
discriminator prevents cross-transport replay).

---

## 8. Wave 5 — Closeout (PR-7)

Five things:

**(i) Engine-to-runtime wiring.** `atlas index`'s existing entry point
in `crates/atlas-cli/src/` constructs an `AgentRuntime`, wires
subscribers, and calls
`tokio::runtime::Handle::block_on(runtime.run_workspace(workspace))`. This
is the **single** sync→async boundary in Phase 7. Pre-existing engine code
paths (`l4_tree`, deterministic `fixedpoint.rs`) stay synchronous.

**(ii) Polyglot smoke test extension** in `crates/atlas-engine/tests/`:

- Single coarse cold token total assertion against today's reference (~40
  LLM calls / reference token count) (regression detector per recast §8.4).
- Warm=0 unchanged (load-bearing invariant).
- Cross-transport parity check: run polyglot smoke through both
  `claude_code` and `codex` transports; assert structural equivalence of
  outputs (same component set, same contract set, same edge set — modulo
  refinements). Specific equivalence rules are part of this PR per recast
  §11.2's reference-output comparison harness shape.

**(iii) Atlas-on-Atlas calibration.** Full agent runtime run against
Atlas's own workspace, where `subsystems.overrides.yaml` is absent so
dispatch agents fire. Calibrates the dispatch-overhead baseline and locks
it as the Atlas-on-Atlas cold token total (recorded in the closeout note
for future regression detection).

**(iv) Acceptance checklist** (§10 below); ticked one-by-one in
`docs/superpowers/plans/2026-05-NN-phase7-status.md`.

**(v) Closeout discipline.** Status file: per-PR notes for PR-0..PR-7;
final closeout note with cumulative LOC change, Atlas-on-Atlas baseline
numbers, list of PR-0 plan-time decisions taken vs deferred.
Commit-sha backfill for the closeout commit per Phase 4/5/6 discipline.

**Canonical-spec retext: not needed in Phase 7.** Phase 6 PR-5 already
shipped the §4.3 + §7 + §8 + §10 retext per recast §13. PR-7 only patches
the canonical spec *if* Phase 7's shipped scope deviates from the headline;
current understanding says it matches, so this is contingent work, not
planned work.

---

## 9. Testing strategy

| Layer | What | Where |
|---|---|---|
| `test_backend.rs` extensions | Multi-turn tool-use loop, audit Lane A retry, audit Lane B verdict shapes, fixed-point iteration with deterministic synthetic outputs | `crates/atlas-llm/src/test_backend.rs` extensions; `crates/atlas-agents/tests/` |
| Override-shortcircuit unit tests | With/without override file; valid/invalid override (Lane A fail); cache-key invariants when override sha changes | `crates/atlas-agents/tests/dispatch_shortcircuit.rs` |
| Cross-provider audit unit tests | Mock backends for both providers; verify producer→auditor mapping; verify `AuditDegraded` fires on single-provider config | `crates/atlas-agents/tests/audit_lane_b.rs` |
| MCP multi-client unit tests | Two concurrent subprocess clients connecting to the same in-process MCP server; verify isolation, fingerprint integrity | `crates/atlas-agents/tests/mcp_multiplex.rs` |
| Drain-handshake unit tests | `AgentRuntime::run()` returns only after all subscribers process `RuntimeComplete`; transcript-cache writer flushes before return | `crates/atlas-agents/tests/drain_handshake.rs` |
| Polyglot smoke (production fixture) | Cold token total (regression detector), warm=0, cross-transport parity | `crates/atlas-engine/tests/polyglot_smoke.rs` extension |
| Atlas-on-Atlas (real workload) | Calibrate dispatch-overhead baseline; TUI renders correctly on a real run | Manual + recorded baseline in closeout note |
| Replay-from-cache | TUI renders identically on `atlas index --replay-from-cache` after a real run | `crates/atlas-cli/tests/replay.rs` |

---

## 10. Phase 7 acceptance criteria

- [ ] `atlas index` runs end-to-end through the agent runtime on `claude_code` transport.
- [ ] `atlas index` runs end-to-end through the agent runtime on `codex` transport.
- [ ] `atlas index` runs end-to-end through the agent runtime on `http_anthropic` and `http_openai` transports (HTTP signal path).
- [ ] Polyglot smoke cold token total matches today's reference within calibrated tolerance (regression detector).
- [ ] Polyglot smoke warm=0 holds (zero LLM calls on no-op re-run).
- [ ] Atlas-on-Atlas cold token total baseline (including dispatch overhead) recorded and locked.
- [ ] Cross-transport parity check on polyglot smoke: `claude_code` and `codex` outputs structurally equivalent.
- [ ] TUI renders live tree progress correctly during a real run.
- [ ] `--no-tui` JSON-Lines event stream fallback works on piped stdout.
- [ ] `--log-events events.jsonl` parallel file subscriber works alongside both TUI and JSON-Lines modes.
- [ ] `--replay-from-cache` renders identical TUI from cached transcripts.
- [ ] Hard-fail diagnostics surface correctly (agent id, stage, target, fingerprint, retry history, last transcript snippet, suggested investigation).
- [ ] Cross-provider Lane B audit fires on `Weak`/`Declines`; accepts / requests revision / hard-fails per recast §4.3.
- [ ] Single-provider config emits `AuditDegraded` warning; falls back to same-model audit cleanly.
- [ ] Fixed-point loop converges on polyglot fixture (single iteration sufficient); `K = 5` cap validated on Atlas-on-Atlas; hard-fail diagnostic on divergence.
- [ ] MCP server multiplexes two concurrent subprocess clients (`claude_code` + `codex`) with disabled built-in tools.
- [ ] Drain handshake guarantees all subscribers flush before `AgentRuntime::run()` returns.
- [ ] Cumulative LOC change reported in closeout note.
- [ ] Status file updated with per-PR notes and closeout commit sha.

---

## 11. Deferred-to-PR-0 (plan-time decisions list)

Brainstorm-locked positions ready for PR-0 to formalise:

1. PR slicing axis: wave-first, 5 waves, 8 PRs total.
2. Dispatch semantics: LLM-decided with override-file shortcircuit.
3. Backend transport: dual; subprocess + HTTP both drive the runtime.
4. `Tool` bridge: unified envelope via in-process MCP stdio server.
5. Concurrency: async Tokio; per-transport semaphores (`HTTP=8`, `subprocess=2` initial); per-stage semaphores.
6. TUI library: `ratatui`.
7. Iteration cap: `K = 5`; calibrate against Atlas-on-Atlas in PR-7.
8. Audit-lane confidence grade enum: `{Strong, Moderate, Weak, Declines}`; Lane B fires on `Weak | Declines`.
9. Transcript-cache key: recast §6.1 + `transport_flavour` discriminator; atomic-write via Phase 4 helper (extended to two-file atomic-pair primitive).
10. Event-bus format: Tokio `broadcast::channel` capacity ~1024; lagged-receiver = error-and-log; drain handshake before `run()` returns.
11. Lane B auditor: cross-provider (Anthropic↔OpenAI); fallback to same-model with `AuditDegraded` warning on single-provider config.
12. Async surface on `LlmBackend`: add `async fn call_async(...)` alongside sync `fn call(...)`.
13. Default `BackendRouter` config: `claude_code` + `codex` paired (cross-provider out-of-box).
14. Budget posture: coarse single cold token total assertion; observability via TUI; no per-provider buckets, no runtime gates.

Specific MCP wire-format-version targets (Claude Code MCP version, codex MCP
version) and ratatui-specific layout details are PR-1 / PR-6 work, not PR-0
work.

---

## 12. Open risks

1. **Engine→agents sync→async boundary discipline.** The single
   `Handle::block_on(runtime.run_workspace(...))` call is the only legal
   sync→async crossover. If engine code later tries to call back into agents
   transitively, nested `block_on` will deadlock. PR-0 names the rule:
   **engine code is sync; agent code is async; the sync→async boundary is
   the engine's call-out to the agent runtime; no nested block_on.**

2. **`atomic_write` two-file atomic-pair primitive.** Today's `atomic_write`
   is per-file. Transcript-cache writes two files per agent
   (`<sha>.transcript` + `<sha>.output`). A crash between writes leaves a
   `.transcript` without its `.output`. PR-2 adds a two-file atomic-pair
   primitive: write both to `.tmp` paths, fsync both, rename both. Forensic
   value (transcripts debuggable side-of-output even when output corrupt) is
   why I prefer the two-file primitive over an envelope-wrapper.

3. **Phase 7's "no language retirements" rule double-paths classifier code.**
   Today's `l3_classify.rs` calls `cargo_classifier::classify(...)` directly
   (sync). In Phase 7, the per-component agent invokes
   `CargoClassifyTool::invoke()` which calls into the same classifier via
   `spawn_blocking`. Both code paths remain reachable until Phase 8 retires
   the direct path. PR-0 names the rule: until Phase 8, the agent-runtime
   path is the **only** caller exercised in production from `atlas index`;
   the direct-call path stays compiled and unit-tested but unreachable from
   the CLI.

4. **Replay-from-cache is single-transport.** `transport_flavour` cache
   discriminator means `atlas index --replay-from-cache` can only replay the
   transport you originally ran. PR-6 documents this; the CLI may emit a
   helpful error if the configured transport differs from what's in cache.

5. **Subprocess built-in tool restrictions are upstream-version-sensitive.**
   The `--disallowedTools=Read,Grep,Glob,Bash,Write,Edit` flag for
   claude-code and the codex equivalent depend on upstream agent versions
   honouring the restriction set. If a future upstream version adds a new
   built-in tool, the restriction list must be updated. PR-0 records the
   exact upstream versions targeted; PR-7 acceptance test verifies that
   `disallowed` actually disables (via a "tool-call-Read-and-fail" test
   probe).

---

## 13. References

- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` — the
  design anchor for Phase 7+ (this brainstorm implements §11.1).
- `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` §10.7 —
  canonical Phase 7 entry retexted in Phase 6 PR-5.
- `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic intent.
- `.claude/memory/project_phase4_plus_roadmap.md` — phase-ordering state.
- `.claude/memory/feedback_cross_provider_llm_audit.md` — Lane B design
  rationale.
- `.claude/memory/project_atlas_common_backend_config.md` — default
  `BackendRouter` config + MCP multiplexing requirement.
- `.claude/memory/feedback_worktree_base_verification.md` — Wave 2 subagent
  dispatch discipline.
- `crates/atlas-engine/src/llm_cache.rs` — single-shot cache the transcript
  cache extends.
- `crates/atlas-engine/src/fixedpoint.rs` — monotonic-growth fixed-point
  pattern reused by the iteration loop.
- `crates/atlas-engine/src/progress.rs` — engine-side progress events that
  generalise into the agent runtime's `AgentEvent`.
- `crates/atlas-llm/src/agent_observer.rs` — subprocess single-call
  observation pattern that generalises into the MCP-side tool-call recording
  in `runtime/tool_loop_mcp.rs`.
- `crates/atlas-llm/src/tool_use.rs` — HTTP-side `SandboxedFilesystem` +
  `ToolBudget` that generalises into the §5 toolbox.
