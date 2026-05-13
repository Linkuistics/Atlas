//! Async agent runtime (plan §4 Task 4 / brainstorm §6).
//!
//! PR-4 lands the single-iteration spine:
//!
//! - `AgentRuntime` — top-level handle holding the backend router,
//!   tool catalog, transcript cache, event bus, and concurrency caps.
//!   Exposes `run_workspace(&Workspace)` as the async entry point the
//!   CLI (PR-7) drives from its top-level Tokio runtime.
//! - `dispatch` — workspace → subsystem → component partitioning.
//!   **Deterministic-only in PR-4** — reads override files; PR-5 adds
//!   LLM dispatch.
//! - `tool_loop_http` — Atlas-owned tool-use loop for HTTP backends.
//! - `tool_loop_mcp` — MCP-side observation harness for subprocess
//!   backends.
//! - `audit` — Lane A schema validation (always on; PR-5 adds Lane B).
//! - `semaphores` — per-transport + per-stage concurrency caps.
//!
//! The runtime is fully async. No `tokio::runtime::Handle::block_on`
//! calls are permitted inside it (the `clippy::disallowed_methods`
//! rule in `crates/atlas-agents/clippy.toml` enforces this). The
//! sync→async boundary lives at the CLI entry point (PR-7).

pub mod agent;
pub mod audit;
pub mod dispatch;
pub mod fixedpoint_loop;
pub mod outputs;
pub mod projection_to_canonical;
pub mod prompt_examples;
pub mod semaphores;
pub mod tool_loop_http;
pub mod tool_loop_mcp;
pub mod yaml_strict;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use atlas_engine::llm_cache::{
    AgentGrade, AgentInputFingerprint, FingerprintInputSpotCheck, LlmResponseCache,
    TRANSCRIPT_FRAME_PREFIX,
};
use atlas_index::Stage as IndexStage;
use atlas_llm::{LlmBackend, LlmError, LlmRequest, Provider};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::events::{AgentEvent, CacheHitSource, EventBus, Grade};
use crate::transport::TransportFlavour;
use crate::ToolHandle;

pub use agent::Agent;
pub use audit::{lane_a_validate, AgentOutput, AuditVerdict, SchemaError, Stage};
pub use dispatch::{
    dispatch_components, dispatch_subsystems, ComponentPartition, SubsystemPartition,
};
pub use semaphores::Semaphores;
pub use tool_loop_http::{
    build_llm_request_with_tools, extract_tool_uses, parse_final_output, run_tool_loop_http,
    ToolUse, Transcript, TranscriptRecord,
};
pub use tool_loop_mcp::run_tool_loop_mcp;

/// Runtime-internal content-addressed sha. Hex form so it round-trips
/// through serde and event-bus payloads without raw-bytes plumbing.
///
/// Atlas's engine carries a `Sha256Hex = String` alias under
/// `atlas_engine::cache`; PR-4 chooses the same shape here under a
/// runtime-owned alias rather than reaching into the engine's cache
/// module for what is morally a private helper. PR-5 may unify if it
/// turns out useful.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentSha(pub String);

impl ContentSha {
    /// Convenience accessor for the hex string. Idiomatic with
    /// `IterationBoundary { prior_model_sha: Option<String> }`.
    pub fn to_hex(&self) -> String {
        self.0.clone()
    }
}

/// PR-4: minimal `L9Projection` shape. The engine's L9 type is
/// today a Salsa-mediated query result; PR-4 surfaces a thin handle
/// that captures the runtime's per-iteration output. PR-7 will widen
/// this to the engine's projection shape when the runtime is wired
/// into `atlas index`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L9Projection {
    /// Per-component projection results indexed by component id.
    pub components: HashMap<String, AgentOutput>,
    /// Per-subsystem reduce results indexed by subsystem id.
    pub subsystems: HashMap<String, AgentOutput>,
    /// Workspace-level projection.
    pub project: Option<AgentOutput>,
}

impl L9Projection {
    /// True iff no components, subsystems, or project entry. Used by
    /// the smoke test.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.subsystems.is_empty() && self.project.is_none()
    }
}

/// Errors surfaced by the agent runtime. Variants intentionally
/// granular so the runtime + CLI can format clean diagnostics; the
/// `LaneBFail` variant is a PR-5 placeholder reserved here so
/// downstream pattern matches don't churn between PR-4 and PR-5.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("override file missing or malformed: {0}")]
    OverrideRequired(String),
    /// PR-7: dispatch agent output failed to deserialise into the
    /// expected envelope shape (`SubsystemsOverrideFile` /
    /// `ComponentsOverrideFile`). Distinct from
    /// [`AgentError::OverrideRequired`], which is reserved for
    /// user-authored override-file load/parse errors; this variant
    /// surfaces *LLM* output that did not conform to the wire envelope.
    /// PR-5 closeout MEDIUM-2 follow-up.
    #[error("LLM dispatch agent emitted malformed output: {0}")]
    LlmOutputMalformed(String),
    #[error("lane A schema validation failed: {0}")]
    LaneAFail(#[from] SchemaError),
    #[error("lane B audit failed: {0}")]
    LaneBFail(String),
    #[error("tool invocation failed: {0}")]
    ToolFailure(String),
    #[error("unknown tool `{0}`")]
    UnknownTool(String),
    #[error("max-steps cap ({0}) exceeded during tool-use loop")]
    MaxStepsExceeded(u32),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("subprocess spawn failed: {0}")]
    SubprocessSpawn(#[source] std::io::Error),
    #[error("subprocess wait failed: {0}")]
    SubprocessWait(#[source] std::io::Error),
    #[error("subprocess exited non-zero: {exit_status:?}")]
    SubprocessFailed {
        exit_status: std::process::ExitStatus,
    },
    #[error("subprocess produced no parseable final output")]
    NoFinalOutput,
    #[error("transcript-cache error: {0}")]
    Cache(String),
    /// PR-5: fixed-point loop ran `iterations` times without
    /// converging. `last_changed_agents` is the diagnostic-only set of
    /// agent ids whose transcript sha changed between iteration K-1
    /// and iteration K. PR-5 ships the variant with an empty
    /// diagnostic vec (the plan's `collect_shifted_agents` is hand-
    /// wavy, see implementer-brief known-unknown #3); PR-7 enriches.
    #[error(
        "fixed-point loop diverged after {iterations} iterations \
         (last_changed_agents={last_changed_agents:?})"
    )]
    FixedpointDiverged {
        iterations: u32,
        last_changed_agents: Vec<String>,
    },
}

impl AgentError {
    /// Lift an `LlmError` to an `AgentError::Backend`. Kept as a
    /// helper rather than a `From` impl so an accidental `?` from
    /// engine code that should fail differently surfaces as a
    /// compile error rather than a silent recategorisation.
    pub fn from_llm_error(err: LlmError) -> Self {
        AgentError::Backend(err.to_string())
    }
}

/// Tool catalog — type-erased map keyed by `Tool::id()`. Built once
/// at runtime construction and shared across every agent call. PR-4
/// populates via `default_tool_catalog()` from the 22 PR-3 wrappers.
pub struct ToolCatalog {
    by_id: HashMap<String, ToolHandle>,
}

impl ToolCatalog {
    /// Construct from any iterator of handles. Duplicate ids overwrite
    /// in iteration order; callers are expected to register from a
    /// single source of truth.
    pub fn new(tools: impl IntoIterator<Item = ToolHandle>) -> Self {
        let by_id: HashMap<String, ToolHandle> =
            tools.into_iter().map(|t| (t.id().to_string(), t)).collect();
        Self { by_id }
    }

    /// Look up a tool by id.
    pub fn get(&self, id: &str) -> Option<&ToolHandle> {
        self.by_id.get(id)
    }

    /// Tool count. Useful for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True iff no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Iterate over the registered tools. Order is unspecified
    /// (`HashMap` iteration); callers that need a deterministic order
    /// should sort by `Tool::id()`.
    pub fn iter(&self) -> impl Iterator<Item = &ToolHandle> {
        self.by_id.values()
    }

    /// SHA-256 over the sorted `id || \x00 || version` byte stream
    /// of every registered tool. Contributes to the agent transcript
    /// cache key (recast §6.1 `tool_catalog_sha`).
    pub fn catalog_sha(&self) -> [u8; 32] {
        let mut entries: Vec<(String, String)> = self
            .by_id
            .values()
            .map(|t| (t.id().to_string(), t.version().to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hasher = Sha256::new();
        for (id, ver) in &entries {
            hasher.update(id.as_bytes());
            hasher.update(b"\x00");
            hasher.update(ver.as_bytes());
            hasher.update(b"\x00");
        }
        hasher.finalize().into()
    }
}

impl Default for ToolCatalog {
    fn default() -> Self {
        default_tool_catalog()
    }
}

/// Build the default tool catalog from the 22 PR-3 wrappers.
///
/// Layout mirrors `crate::tools::{classifiers, manifests, surfaces}`:
///
/// - 10 classifiers
/// - 4 manifest parsers
/// - 8 surface analysers
pub fn default_tool_catalog() -> ToolCatalog {
    use crate::tools::classifiers::{
        CargoClassifyTool, ComposeClassifyTool, CsharpClassifyTool, DartClassifyTool,
        DockerfileClassifyTool, ElixirClassifyTool, LispKitClassifyTool, PythonClassifyTool,
        RacketClassifyTool, TsJsClassifyTool,
    };
    use crate::tools::manifests::{
        ParseCargoTomlTool, ParseComposeTool, ParseDockerfileTool, ParsePackageJsonTool,
    };
    use crate::tools::surfaces::{
        CsharpSurfaceTool, DartSurfaceTool, ElixirSurfaceTool, LispKitSurfaceTool,
        PythonSurfaceTool, RacketSurfaceTool, RustSurfaceTool, TsJsSurfaceTool,
    };
    let handles: Vec<ToolHandle> = vec![
        // Classifiers (10).
        Arc::new(CargoClassifyTool),
        Arc::new(ComposeClassifyTool),
        Arc::new(CsharpClassifyTool),
        Arc::new(DartClassifyTool),
        Arc::new(DockerfileClassifyTool),
        Arc::new(ElixirClassifyTool),
        Arc::new(LispKitClassifyTool),
        Arc::new(PythonClassifyTool),
        Arc::new(RacketClassifyTool),
        Arc::new(TsJsClassifyTool),
        // Manifest parsers (4).
        Arc::new(ParseCargoTomlTool),
        Arc::new(ParseComposeTool),
        Arc::new(ParseDockerfileTool),
        Arc::new(ParsePackageJsonTool),
        // Surfaces (8).
        Arc::new(CsharpSurfaceTool),
        Arc::new(DartSurfaceTool),
        Arc::new(ElixirSurfaceTool),
        Arc::new(LispKitSurfaceTool),
        Arc::new(PythonSurfaceTool),
        Arc::new(RacketSurfaceTool),
        Arc::new(RustSurfaceTool),
        Arc::new(TsJsSurfaceTool),
    ];
    ToolCatalog::new(handles)
}

/// Workspace shape the runtime drives. PR-4 uses a lightweight handle
/// (root path only) rather than the engine's Salsa-input `Workspace`
/// because the runtime never reads through Salsa — it reads the
/// filesystem directly via override files and tool calls. PR-7 will
/// adapt this to the engine's projection shape when wiring the
/// runtime into `atlas index`.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: std::path::PathBuf,
}

impl Workspace {
    /// Construct a workspace handle from a filesystem root. PR-4's
    /// runtime reads `subsystems.overrides.yaml` and
    /// `components.overrides.yaml` from this root.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The workspace root path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Top-level handle for the agent runtime. Held by the CLI; cloned
/// into per-task futures via `Arc`-wrapped fields.
///
/// PR-4 lands the single-iteration entry; PR-5 wraps `run_iteration`
/// in `run_fixedpoint`.
///
/// The `backend_router` field is typed as `Arc<dyn LlmBackend>` rather
/// than `Arc<BackendRouter>` (which the plan §4 Task 4.1 sketch named).
/// Reasoning: the runtime only ever invokes trait methods on the
/// router, and `BackendRouter::from_dispatch_table` is `#[cfg(test)]`
/// on the producer crate so an integration test cannot construct one
/// without an upstream lift of the constructor. PR-7 will wire a real
/// `BackendRouter` instance into the field (it implements
/// `LlmBackend`), so production wiring is unaffected. The PR-4 brief
/// allows for design deviations of this shape; logged in the DONE
/// report.
pub struct AgentRuntime {
    pub backend_router: Arc<dyn LlmBackend>,
    pub tools: Arc<ToolCatalog>,
    pub cache: Arc<LlmResponseCache>,
    pub event_bus: Arc<EventBus>,
    pub semaphores: Semaphores,
    /// Default transport flavour used when the runtime can't infer
    /// one from the request (PR-4: always `HttpAnthropic` because the
    /// only PR-4 production deployment is the test backend, but
    /// callers can override per-call).
    pub default_transport: TransportFlavour,
    /// Default max-steps cap for the HTTP tool-use loop. PR-4 ships
    /// 8; PR-5 may make this per-stage.
    pub default_max_steps: u32,
    /// PR-5: max fixed-point iterations before
    /// [`AgentError::FixedpointDiverged`] surfaces. Default `5` per
    /// brainstorm §2 row 7; tunable. Plumbed through `IndexConfig` in
    /// PR-7.
    pub max_iterations: u32,
    /// PR-5 (known-unknown #1, approach (c)): cross-provider backend
    /// lookup. When `Some`, Lane B (`runtime/audit/lane_b.rs`) calls
    /// this closure with the auditor provider to obtain a sibling
    /// `LlmBackend`. When `None`, Lane B falls back to the same-model
    /// auditor (the `backend_router` field) and emits an
    /// `AuditDegraded` event.
    ///
    /// Approach rationale: a closure carries the same shape as the
    /// existing `current_sha_fn` placeholder pattern and costs no
    /// `LlmBackend`-trait surgery. Production wiring delegates to
    /// `BackendRouter::backend_for_provider`; tests inject simpler
    /// mocks via [`AgentRuntime::with_for_provider`].
    pub for_provider: Option<Arc<ForProviderFn>>,
    /// PR-A: per-runtime MCP server instance, drives subprocess
    /// transports (`TransportFlavour::ClaudeCode | Codex`) via
    /// [`crate::mcp::serve_client::serve_client`]. `None` means the
    /// runtime can only service HTTP transports; selecting a subprocess
    /// transport under that condition returns a clear error.
    pub mcp_server: Option<Arc<crate::mcp::server::McpServer>>,
    /// PR-4: filesystem root for on-disk audit verdicts. The runtime
    /// writes each Lane B verdict to
    /// `<audit_dir>/<stage>/<target_id>.yaml` via
    /// [`crate::runtime::audit::write_verdict_pair`] and replays from
    /// disk on agent re-run. The CLI pipeline constructs this as
    /// `<workspace_root>/.atlas/audit/`; tests pass a `TempDir` path.
    pub audit_dir: std::path::PathBuf,
}

/// Type alias for the Lane B cross-provider lookup closure. Boxed
/// dyn-`Fn` so the trait object is `Send + Sync` (required because
/// `AgentRuntime` is shared across Tokio tasks via `Arc`).
pub type ForProviderFn = dyn Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync + 'static;

/// PR-4 internal: one agent invocation request. Distinct from
/// `atlas_engine::llm_cache::AgentRequest` (which is the cache
/// layer's opaque payload) — this carries the stage / target /
/// transport selector the runtime needs to route. PR-5 may
/// extend additively.
#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub stage: Stage,
    pub target_id: String,
    pub iteration: u32,
    pub transport: TransportFlavour,
    pub initial_prompt: String,
    pub fingerprint_inputs: Vec<crate::FingerprintInput>,
    pub candidate_ids: HashSet<String>,
    pub prior_model_sha: Option<ContentSha>,
    /// PR-4: how many Lane B revisions have already fired against
    /// *this* agent target. Starts at 0 on a fresh `call_agent`; the
    /// revision-prompt path increments this when re-invoking
    /// `call_agent` recursively so the cumulative-budget rule
    /// (`lane_a_retries + lane_b_revisions >= 1` → escalate next
    /// `RequestRevision` to `HardFail`) fires at the right time.
    #[doc(hidden)]
    pub lane_b_revisions: u32,
}

/// PR-4 internal: one agent invocation result.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub output: AgentOutput,
    pub grade: Grade,
    /// Transcript bytes for cache materialisation. Empty on cache
    /// hit (the cache returned the already-framed bytes; the
    /// `agent_cache_writer` subscriber sees these via `AgentComplete`).
    pub transcript_bytes: Vec<u8>,
    /// Output bytes, encoded for cache storage.
    pub output_bytes: Vec<u8>,
}

/// PR-5 follow-up: per-call retry accounting threaded through
/// `run_tool_loop_with_lane_a` so the Lane B retry harness can apply
/// the cumulative-budget rule (recast §4.3: combined max 2 retries per
/// agent across Lane A + Lane B).
#[derive(Debug, Clone)]
struct ToolLoopOutcome {
    result: AgentResult,
    /// Number of Lane A retries that actually fired (0 or 1).
    lane_a_retries: u32,
    /// PR-4: a clone of the producer's `Transcript` made before
    /// `into_bytes` consumed the original. Lane B's audit closure
    /// renders this via
    /// [`crate::runtime::audit::render_transcript_for_audit`] to
    /// supply the auditor with the producer's tool-call trail.
    transcript: Transcript,
}

impl AgentRuntime {
    /// Top-level entry point. PR-5 wraps PR-4's `run_iteration` in
    /// `run_fixedpoint` so the runtime drives the iteration loop to
    /// convergence (recast §4.4).
    ///
    /// `AgentEvent::RuntimeComplete` is the drain-handshake sentinel —
    /// every subscriber waits for it before flushing. Emit
    /// unconditionally on both Ok and Err paths (including
    /// [`AgentError::FixedpointDiverged`]) so subscribers always drain
    /// even when the loop fails; the body is wrapped in a `let result
    /// = ...await;` binding to guarantee a single emit site.
    pub async fn run_workspace(&self, workspace: &Workspace) -> Result<L9Projection, AgentError> {
        let result =
            crate::runtime::fixedpoint_loop::run_fixedpoint(self, workspace, self.max_iterations)
                .await;
        self.event_bus.emit(AgentEvent::RuntimeComplete);
        result
    }

    /// One iteration of the LLM-spine loop. PR-4: deterministic
    /// dispatch via override files; per-component Classify; reduce;
    /// project. PR-5: dispatch may also be LLM-decided when no
    /// override file is present.
    ///
    /// `AgentEvent::IterationBoundary` emission lives in
    /// [`crate::runtime::fixedpoint_loop::run_fixedpoint`] — the loop
    /// owns the iteration counter and the prior-iteration sha — so
    /// this function does not emit it. Calling `run_iteration`
    /// directly (e.g. from a test that bypasses `run_workspace`) is
    /// fine but will not produce an `IterationBoundary` event.
    pub async fn run_iteration(
        &self,
        workspace: &Workspace,
        iter: u32,
        prior_model_sha: Option<ContentSha>,
    ) -> Result<L9Projection, AgentError> {
        let subsystems = dispatch_subsystems(self, workspace).await?;
        let mut projection = L9Projection::default();
        let mut all_component_ids: HashSet<String> = HashSet::new();

        // Pass 1: collect every component id across every subsystem so
        // Lane A's candidate-id check at later stages has the full
        // workspace view.
        let mut subsystem_components: Vec<(SubsystemPartition, Vec<ComponentPartition>)> =
            Vec::with_capacity(subsystems.len());
        for subsystem in subsystems {
            let components = dispatch_components(self, workspace, &subsystem).await?;
            for c in &components {
                all_component_ids.insert(c.id.clone());
            }
            subsystem_components.push((subsystem, components));
        }

        // Pass 2: drive per-component Classify, then per-subsystem
        // Reduce. Per-subsystem reduce outputs accumulate for the
        // workspace-level Project pass below.
        //
        // PR-3: each stage's prompt now embeds caller-supplied
        // soft/hard caps that are also threaded through
        // `AgentRequest::max_steps` so prompt-text and request-budget
        // cannot drift (decision row 4).
        let mut reduce_rollup: Vec<(String, String, u32)> = Vec::new();
        for (subsystem, components) in subsystem_components {
            // Pluck (id, kind, language) tuples for the reducer's
            // context after each component's classify completes. The
            // values come from the parsed YAML body when present; if
            // a synthetic backend emits a degenerate response the
            // fallbacks are empty strings.
            let mut classify_rollup: Vec<(String, String, String)> =
                Vec::with_capacity(components.len());
            for component in &components {
                let classify_req = AgentRequest {
                    stage: Stage::Classify,
                    target_id: component.id.clone(),
                    iteration: iter,
                    transport: self.default_transport,
                    initial_prompt: build_classify_prompt(
                        workspace.root(),
                        component,
                        DEFAULT_CLASSIFY_SOFT_CAP,
                        DEFAULT_CLASSIFY_HARD_CAP,
                    ),
                    fingerprint_inputs: Vec::new(),
                    candidate_ids: all_component_ids.clone(),
                    prior_model_sha: prior_model_sha.clone(),
                    lane_b_revisions: 0,
                };
                let classify_res = self.call_agent(classify_req).await?;
                let kind = classify_res
                    .output
                    .value
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let language = classify_res
                    .output
                    .value
                    .get("language")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                classify_rollup.push((component.id.clone(), kind, language));
                projection
                    .components
                    .insert(component.id.clone(), classify_res.output);
            }

            let reduce_req = AgentRequest {
                stage: Stage::Reduce,
                target_id: subsystem.id.clone(),
                iteration: iter,
                transport: self.default_transport,
                initial_prompt: build_reduce_prompt(
                    workspace.root(),
                    &subsystem,
                    &classify_rollup,
                    DEFAULT_REDUCE_SOFT_CAP,
                    DEFAULT_REDUCE_HARD_CAP,
                ),
                fingerprint_inputs: Vec::new(),
                candidate_ids: all_component_ids.clone(),
                prior_model_sha: prior_model_sha.clone(),
                lane_b_revisions: 0,
            };
            let reduce_res = self.call_agent(reduce_req).await?;
            let subsystem_purpose = reduce_res
                .output
                .value
                .get("purpose")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let component_count = reduce_res
                .output
                .value
                .get("component_ids")
                .and_then(Value::as_array)
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            reduce_rollup.push((subsystem.id.clone(), subsystem_purpose, component_count));
            projection
                .subsystems
                .insert(subsystem.id.clone(), reduce_res.output);
        }

        // Workspace-level project — runs once after every per-subsystem
        // reduce completes. PR-3 ships the real `build_project_prompt`
        // here; PR-7's placeholder text is replaced.
        let project_req = AgentRequest {
            stage: Stage::Project,
            target_id: "_workspace".to_string(),
            iteration: iter,
            transport: self.default_transport,
            initial_prompt: build_project_prompt(
                workspace.root(),
                &reduce_rollup,
                DEFAULT_PROJECT_SOFT_CAP,
                DEFAULT_PROJECT_HARD_CAP,
            ),
            fingerprint_inputs: Vec::new(),
            candidate_ids: all_component_ids.clone(),
            prior_model_sha: prior_model_sha.clone(),
            lane_b_revisions: 0,
        };
        let project_res = self.call_agent(project_req).await?;
        projection.project = Some(project_res.output);

        Ok(projection)
    }

    /// One agent invocation. Acquires transport + stage semaphores,
    /// consults the persistent transcript cache, on miss routes to
    /// `tool_loop_http` or `tool_loop_mcp` by transport flavour, then
    /// runs Lane A schema validation with one retry on failure.
    pub async fn call_agent(&self, request: AgentRequest) -> Result<AgentResult, AgentError> {
        let _transport_permit = self.semaphores.acquire_transport(request.transport).await;

        // Fill in fingerprint-input shas. PR-3 closeout left these as
        // zero sentinels (sync `fingerprint_inputs` cannot read disk
        // without async-trait modification); the runtime fills them
        // here via `tokio::fs::read` + `sha2::Sha256::digest`. Keeping
        // the sentinel + async-fill at the runtime preserves the
        // `Tool` trait's sync surface.
        let mut input_shas: Vec<[u8; 32]> = Vec::with_capacity(request.fingerprint_inputs.len());
        let mut recorded_inputs: Vec<FingerprintInputSpotCheck> =
            Vec::with_capacity(request.fingerprint_inputs.len());
        for fi in &request.fingerprint_inputs {
            let abs = fi.path.clone();
            // PR-4 design choice (per brief): keep `FingerprintInput`'s
            // sync sentinel; resolve shas here via async fs read +
            // sha256. Avoids async-trait modification of `Tool`.
            let bytes = tokio::fs::read(&abs)
                .await
                .map_err(|e| AgentError::Backend(format!("fingerprint input read failed: {e}")))?;
            let sha: [u8; 32] = Sha256::digest(&bytes).into();
            input_shas.push(sha);
            recorded_inputs.push(FingerprintInputSpotCheck {
                path: fi.path.to_string_lossy().into_owned(),
                recorded_sha: sha,
            });
        }

        let backend_fp = self.backend_router.fingerprint();
        let prior_sha_bytes = request.prior_model_sha.as_ref().and_then(|s| {
            let mut bytes = [0u8; 32];
            decode_hex_into(&s.0, &mut bytes).ok().map(|_| bytes)
        });
        let fingerprint = AgentInputFingerprint {
            stage_id: request.stage.as_str().to_string(),
            agent_id: agent_id(&request),
            agent_version: "v0".to_string(),
            prompt_template_sha: backend_fp.template_sha,
            tool_catalog_sha: self.tools.catalog_sha(),
            model_id: backend_fp.model_id.clone(),
            backend_version: backend_fp.backend_version.clone(),
            transport_flavour: request.transport.as_str().to_string(),
            target_input_shas: input_shas,
            iteration_number: request.iteration,
            prior_model_sha: prior_sha_bytes,
            // PR-5: non-dispatch agents never short-circuit via override
            // files, so the override-content-sha contributor is `None`
            // here. The dispatch path constructs its own fingerprint
            // with `override_content_sha: Some(...)` when the override
            // file is present.
            override_content_sha: None,
        };

        let fingerprint_hex = fingerprint.to_cache_key();
        self.event_bus.emit(AgentEvent::AgentStart {
            agent_id: agent_id(&request),
            parent_id: None,
            stage: request.stage.as_str().to_string(),
            target: request.target_id.clone(),
            fingerprint: fingerprint_hex.clone(),
            started_at: now_iso(),
            transport: request.transport,
        });

        // Two-tier transcript cache lookup, probe-first. The engine
        // exposes a sync `call_agent_cached` whose compute closure is
        // also sync; the runtime's tool loop is async, which means a
        // pre-PR-4-follow-up implementation that drove the loop above
        // `call_agent_cached` ran the loop *unconditionally*, defeating
        // the cache short-circuit. Instead we split the engine API into
        // [`LlmResponseCache::probe_agent_pair`] +
        // [`LlmResponseCache::write_agent_pair`] and orchestrate
        // probe → async compute on miss → write here.
        //
        // The "no concrete current_sha_fn" decision matches PR-2's
        // forward-pointer comment: PR-4 wires a const-`None` lookup
        // so the spot-check trivially passes for the empty-input case
        // and re-evicts the entry on any non-empty input that the
        // current run can't re-hash. PR-7 wires the real
        // `AtlasDatabase`-backed sha lookup.
        let current_sha_fn = |_path: &str| None;

        if let Some(cached) = self.cache.probe_agent_pair(
            IndexStage::L8,
            &fingerprint,
            &recorded_inputs,
            current_sha_fn,
        ) {
            // Cache hit: emit CacheHit + AgentComplete from the cached
            // transcript/output and return without invoking the backend
            // or running Lane A. The cached entry was Lane-A-clean at
            // the time it was written (recast §6.4 gates writes on the
            // `Ok` arm), so it remains so on replay.
            let output = decode_output_bytes(&cached.output_bytes)?;
            let grade = engine_to_grade(&cached.confidence_grade);
            let provider_label = self.backend_router.fingerprint().model_id;
            let output_sha = sha256_hex(&cached.output_bytes);
            self.event_bus.emit(AgentEvent::CacheHit {
                agent_id: agent_id(&request),
                fingerprint: fingerprint_hex.clone(),
                replayed_at: now_iso(),
                source: CacheHitSource::AgentCache,
            });
            self.event_bus.emit(AgentEvent::AgentComplete {
                agent_id: agent_id(&request),
                output_sha,
                confidence_grade: grade.clone(),
                // Cache hit: no backend tokens spent, no wall-clock
                // budget consumed. Subscribers can disambiguate via the
                // preceding `CacheHit` event.
                tokens_in: 0,
                tokens_out: 0,
                ms: 0,
                provider: provider_label,
            });
            return Ok(AgentResult {
                output,
                grade,
                transcript_bytes: cached.transcript_bytes,
                output_bytes: cached.output_bytes,
            });
        }

        // Cache miss: drive the async tool loop, run Lane B audit
        // (skipped on Strong/Moderate grades), then write back on
        // success. The order is deliberate: a producer whose output is
        // Lane-A-clean but Lane-B-rejected must never land in the
        // transcript cache — replay would otherwise resurrect rejected
        // output.
        let tool_outcome = self
            .run_tool_loop_with_lane_a(&request, &fingerprint_hex)
            .await?;
        let lane_a_retries = tool_outcome.lane_a_retries;
        let runtime_result = tool_outcome.result;
        let producer_transcript = tool_outcome.transcript;

        // Lane B audit (recast §4.3, brainstorm §7). PR-4 wires the
        // real cross-provider auditor closure: pre-flight verdict
        // cache → cross-provider backend lookup → audit-prompt
        // round-trip → YAML verdict parse → on-disk persistence.
        // Closure-internal errors map to
        // `AuditVerdict::HardFail(reason)` because the closure's
        // signature can't propagate `Result` outward;
        // `resolve_audit_verdict` translates HardFail into
        // `AgentError::LaneBFail`.
        let producer_provider = request.transport.provider();
        let producer_backend = self.backend_router.clone();
        let for_provider_fn = self.for_provider.clone();
        let bus_ref = self.event_bus.as_ref();
        let producer_agent_id = agent_id(&request);

        let audit_stage = request.stage;
        let audit_target_id = request.target_id.clone();
        let audit_dir_for_closure = self.audit_dir.clone();
        let producer_model_id = self.backend_router.fingerprint().model_id.clone();
        let producer_output_for_closure = runtime_result.output.clone();
        let producer_output_bytes_for_closure = runtime_result.output_bytes.clone();
        let producer_agent_id_for_closure = producer_agent_id.clone();
        let transcript_for_closure = producer_transcript.clone();

        let verdict = audit::lane_b_audit(
            bus_ref,
            &producer_agent_id,
            &runtime_result.grade,
            producer_provider,
            &producer_backend,
            for_provider_fn
                .as_deref()
                .map(|f| f as &(dyn Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync)),
            move |choice: audit::AuditorChoice| {
                let audit_dir = audit_dir_for_closure;
                let target_id = audit_target_id;
                let agent_id_payload = producer_agent_id_for_closure;
                let producer_output = producer_output_for_closure;
                let producer_output_bytes = producer_output_bytes_for_closure;
                let producer_model = producer_model_id;
                let transcript = transcript_for_closure;
                async move {
                    run_real_audit(
                        choice,
                        audit::lane_b::provider_label(producer_provider),
                        &producer_model,
                        &producer_output,
                        &producer_output_bytes,
                        &transcript,
                        audit_stage,
                        &target_id,
                        &agent_id_payload,
                        &audit_dir,
                    )
                    .await
                }
            },
        )
        .await;

        let cumulative_retries = lane_a_retries.saturating_add(request.lane_b_revisions);
        match resolve_audit_verdict(&verdict, cumulative_retries) {
            ResolvedAuditAction::Proceed => {
                // Lane B accepted (or skipped). Fall through to cache
                // write + AgentComplete with the original result.
            }
            ResolvedAuditAction::HardFail(reason) => {
                self.event_bus.emit(AgentEvent::HardFail {
                    agent_id: producer_agent_id.clone(),
                    error_kind: "lane_b".to_string(),
                    error_summary: reason.clone(),
                    retry_count: cumulative_retries,
                });
                return Err(AgentError::LaneBFail(reason));
            }
            ResolvedAuditAction::RequestRevision(reason) => {
                // PR-4: re-invoke the producer with the auditor's
                // critique threaded into the system-prompt addendum.
                // `lane_b_revisions` increments so the recursive call's
                // `resolve_audit_verdict` sees the cumulative budget
                // and escalates the next `RequestRevision` to
                // `HardFail`. The cache write is NOT performed for the
                // rejected output — only the revised output (if it
                // passes audit) lands in the cache.
                let prior_output_rendered = render_producer_output_text(&runtime_result.output);
                let retries_remaining = 1u32.saturating_sub(cumulative_retries);
                let addendum =
                    build_revision_addendum(&prior_output_rendered, &reason, retries_remaining);
                let mut revised = request.clone();
                revised.initial_prompt = format!("{}\n\n{}", revised.initial_prompt, addendum);
                revised.lane_b_revisions = request.lane_b_revisions.saturating_add(1);
                // Box the recursive future so it's `Sized`; the
                // recursion depth is bounded by the cumulative-budget
                // rule (max 2 frames in practice).
                return Box::pin(self.call_agent(revised)).await;
            }
        }

        self.cache.write_agent_pair(
            IndexStage::L8,
            &fingerprint,
            &runtime_result.transcript_bytes,
            &runtime_result.output_bytes,
        );

        let output = runtime_result.output;
        let grade = runtime_result.grade;
        let transcript_bytes = runtime_result.transcript_bytes;
        let output_bytes = runtime_result.output_bytes;

        let provider_label = self.backend_router.fingerprint().model_id;
        let output_sha = sha256_hex(&output_bytes);
        self.event_bus.emit(AgentEvent::AgentComplete {
            agent_id: agent_id(&request),
            output_sha,
            confidence_grade: grade.clone(),
            tokens_in: 0,
            tokens_out: 0,
            ms: 0,
            provider: provider_label,
        });

        Ok(AgentResult {
            output,
            grade,
            transcript_bytes,
            output_bytes,
        })
    }

    /// Drive the inner tool loop (HTTP or MCP by transport flavour),
    /// run Lane A on the result, retry once on schema-fail, otherwise
    /// hard-fail. Returns the [`AgentResult`] plus the per-call Lane A
    /// retry count so the caller's Lane B harness can apply the
    /// cumulative-budget rule (recast §4.3).
    async fn run_tool_loop_with_lane_a(
        &self,
        request: &AgentRequest,
        fingerprint_hex: &str,
    ) -> Result<ToolLoopOutcome, AgentError> {
        let mut conversation = request.initial_prompt.clone();
        let mut last_err: Option<SchemaError> = None;
        let mut lane_a_retries: u32 = 0;
        for attempt in 0..2 {
            let mut transcript = Transcript::new();
            let backend: &dyn LlmBackend = self.backend_router.as_ref();
            let outcome = match request.transport {
                TransportFlavour::HttpAnthropic => {
                    run_tool_loop_http(
                        backend,
                        &self.tools,
                        &tool_context_for(request),
                        &self.semaphores,
                        request.stage,
                        Provider::Anthropic,
                        conversation.clone(),
                        self.default_max_steps,
                        &mut transcript,
                    )
                    .await
                }
                TransportFlavour::HttpOpenai => {
                    run_tool_loop_http(
                        backend,
                        &self.tools,
                        &tool_context_for(request),
                        &self.semaphores,
                        request.stage,
                        Provider::OpenAi,
                        conversation.clone(),
                        self.default_max_steps,
                        &mut transcript,
                    )
                    .await
                }
                TransportFlavour::ClaudeCode | TransportFlavour::Codex => {
                    // PR-A: subprocess transports drive a per-call
                    // `serve_client` against the runtime's MCP server.
                    // The `mcp_config_path` is materialised by the CLI
                    // entry point (PR-7) and refined by PR-B; the
                    // placeholder here lets the structural plumbing
                    // exercise without a real config file.
                    let Some(mcp_server) = self.mcp_server.as_ref() else {
                        return Err(AgentError::Backend(
                            "subprocess transport selected but \
                             AgentRuntime.mcp_server is None; the CLI \
                             pipeline must provide an Arc<McpServer> \
                             when --agent-runtime uses a subprocess \
                             backend"
                                .to_string(),
                        ));
                    };
                    let mcp_config_path = std::path::Path::new("/dev/null");
                    let config = match request.transport {
                        TransportFlavour::ClaudeCode => {
                            crate::mcp::serve_client::claude_code_config(
                                mcp_config_path,
                                &conversation,
                            )
                        }
                        TransportFlavour::Codex => {
                            crate::mcp::serve_client::codex_config(mcp_config_path, &conversation)
                        }
                        _ => unreachable!(),
                    };
                    crate::mcp::serve_client::serve_client(
                        Arc::clone(mcp_server),
                        request.transport,
                        conversation.clone(),
                        config,
                    )
                    .await
                    .map(|(output, _subprocess_mcp_transcript)| output)
                }
            };
            let output = outcome?;
            match lane_a_validate(&output, request.stage, &request.candidate_ids, &transcript).await
            {
                Ok(grade) => {
                    // PR-2: Lane A now returns the evidence-floor-clamped
                    // grade rather than hardcoding `Strong`. The clamp
                    // pulls the LLM's claim down to whatever the
                    // deterministic evidence supports — see
                    // `crate::runtime::audit::evidence::grade_ceiling`.
                    let grade_engine = grade_to_engine(&grade);
                    // PR-4: keep a clone of the transcript before
                    // `into_bytes` consumes it — Lane B's audit closure
                    // renders this for the auditor's view of the
                    // producer's evidence trail.
                    let transcript_for_audit = transcript.clone();
                    let transcript_bytes = transcript.into_bytes(grade_engine);
                    let output_bytes = serde_json::to_vec(&output.value)
                        .map_err(|e| AgentError::Backend(format!("output encode failed: {e}")))?;
                    return Ok(ToolLoopOutcome {
                        result: AgentResult {
                            output,
                            grade,
                            transcript_bytes,
                            output_bytes,
                        },
                        lane_a_retries,
                        transcript: transcript_for_audit,
                    });
                }
                Err(err) => {
                    last_err = Some(err.clone());
                    if attempt == 0 {
                        // Retry once with an appended schema-fail
                        // notice. PR-5 may switch to structured
                        // turns; PR-4's conversation is text.
                        conversation.push_str(&format!(
                            "\n\n[lane_a_retry] previous response failed schema validation: {err}. \
                             Emit a valid response.\n"
                        ));
                        lane_a_retries = 1;
                        continue;
                    }
                    // Second fail — hard fail.
                    self.event_bus.emit(AgentEvent::HardFail {
                        agent_id: agent_id(request),
                        error_kind: "lane_a".to_string(),
                        error_summary: err.to_string(),
                        retry_count: 1,
                    });
                    return Err(AgentError::LaneAFail(err));
                }
            }
        }
        // Unreachable in practice — the loop above either returns
        // Ok on first/second success or Err on second fail. But to
        // keep the compiler happy without `unreachable!`, fall
        // through to the last observed error.
        Err(AgentError::LaneAFail(last_err.unwrap_or_else(|| {
            SchemaError::MalformedComponent(format!(
                "internal: lane_a loop exited without a result (fingerprint={fingerprint_hex})"
            ))
        })))
    }
}

/// PR-4: agent-id formatter delegating to `Agent::id()`. The free
/// function is preserved as a one-call-site convenience; the canonical
/// formatter is `Agent::id()` (see `crate::runtime::agent`).
fn agent_id(req: &AgentRequest) -> String {
    Agent::from(req).id()
}

/// Lane B verdict → caller action mapping.
///
/// `Skipped` / `Accept` → `Proceed`. `HardFail(reason)` → `HardFail`.
/// `RequestRevision(reason)` → either `RequestRevision` (room left in
/// the cumulative budget) or `HardFail` (Lane A + prior Lane B
/// revisions already spent the agent's quota per recast §4.3). The
/// `Degraded` wrapper is unwrapped and resolved recursively.
///
/// `cumulative_retries` is the *combined* Lane A retry count plus the
/// number of Lane B revisions already fired against this agent target
/// across recursive `call_agent` invocations. The cap is `>= 1`: any
/// further `RequestRevision` after a single retry-of-any-kind has
/// already fired escalates to `HardFail`.
fn resolve_audit_verdict(verdict: &AuditVerdict, cumulative_retries: u32) -> ResolvedAuditAction {
    match verdict {
        AuditVerdict::Skipped | AuditVerdict::Accept => ResolvedAuditAction::Proceed,
        AuditVerdict::HardFail(reason) => ResolvedAuditAction::HardFail(reason.clone()),
        AuditVerdict::RequestRevision(reason) => {
            if cumulative_retries >= 1 {
                // Cumulative budget exhausted (lane_a + lane_b combined
                // already burned the single retry slot). Escalate to a
                // hard fail.
                ResolvedAuditAction::HardFail(format!(
                    "lane_b request_revision after retry budget exhausted: {reason}"
                ))
            } else {
                ResolvedAuditAction::RequestRevision(reason.clone())
            }
        }
        AuditVerdict::Degraded(inner) => resolve_audit_verdict(inner, cumulative_retries),
    }
}

/// PR-4: build the revision system-prompt addendum (brainstorm §7.3).
/// Re-invokes the producer with its prior output + the auditor's
/// critique embedded so the producer can target the specific issue
/// rather than re-generating from scratch.
fn build_revision_addendum(
    producer_previous_output: &str,
    auditor_reason: &str,
    retries_remaining: u32,
) -> String {
    format!(
        "PRIOR ATTEMPT:\n{producer_previous_output}\n\n\
         AUDITOR'S CRITIQUE:\n{auditor_reason}\n\n\
         Revise your output to address the auditor's critique. You may invoke \
         tools again if additional evidence is needed. Cumulative retry budget \
         remaining: {retries_remaining}."
    )
}

/// PR-4: render the producer's output as text for the audit prompt.
/// Prefers the originating fenced YAML body (when available) so the
/// auditor judges the exact bytes the producer emitted; falls back to a
/// canonical JSON serialization of `output.value` for backends that
/// don't carry text blocks (e.g. the test backend's `response.output`
/// envelope).
fn render_producer_output_text(output: &AgentOutput) -> String {
    if !output.text.is_empty() {
        return output.text.clone();
    }
    serde_json::to_string_pretty(&output.value).unwrap_or_else(|_| output.value.to_string())
}

/// PR-4: the auditor closure body. Pre-flights the verdict cache,
/// calls the auditor backend, parses the fenced YAML response,
/// persists the verdict pair on disk, and returns the `AuditVerdict`
/// for `lane_b_audit` to wrap. Closure-internal errors map to
/// `AuditVerdict::HardFail(reason)` — `lane_b_audit`'s `audit_fn`
/// signature can't propagate `Result` outward; the calling-frame
/// `resolve_audit_verdict` translates HardFail to `LaneBFail`.
#[allow(clippy::too_many_arguments)]
async fn run_real_audit(
    choice: audit::AuditorChoice,
    producer_provider_label: &'static str,
    producer_model_id: &str,
    producer_output: &AgentOutput,
    producer_output_bytes: &[u8],
    producer_transcript: &Transcript,
    stage: Stage,
    target_id: &str,
    agent_id_payload: &str,
    audit_dir: &Path,
) -> AuditVerdict {
    let auditor_provider = choice.provider();
    let auditor_provider_label = audit::lane_b::provider_label(auditor_provider);
    let auditor_backend = choice.backend().clone();
    let auditor_model_id = auditor_backend.fingerprint().model_id;
    let producer_output_sha = sha256_hex(producer_output_bytes);

    // Pre-flight cache: replay a fresh verdict whose producer sha
    // matches.
    match audit::read_verdict_if_complete(audit_dir, stage, target_id) {
        Ok(Some(cached)) => {
            if cached.producer.output_sha == producer_output_sha {
                return verdict_from_cached(cached);
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(
                error = %e,
                "audit verdict cache read failed; falling through to fresh audit"
            );
        }
    }

    // Compose audit prompt.
    let producer_output_rendered = render_producer_output_text(producer_output);
    let transcript_rendered = audit::render_transcript_for_audit(producer_transcript);
    let prompt = audit::build_audit_prompt(
        producer_provider_label,
        auditor_provider_label,
        stage,
        &producer_output_rendered,
        &transcript_rendered,
    );

    // Call auditor. The `LlmRequest` carries the prompt under
    // `inputs.conversation` matching `build_llm_request_with_tools`'s
    // shape so HTTP backends route it identically.
    let llm_request = LlmRequest {
        prompt_template: atlas_llm::PromptId::Classify,
        inputs: serde_json::json!({
            "conversation": prompt,
            "tools": [],
        }),
        schema: atlas_llm::ResponseSchema::accept_any(),
    };

    let response = match auditor_backend.call_async(&llm_request).await {
        Ok(v) => v,
        Err(e) => {
            return AuditVerdict::HardFail(format!("auditor backend call failed: {e}"));
        }
    };

    // Extract the text payload from the response (Anthropic / OpenAI
    // shapes both flow through `parse_final_output`'s logic).
    let parsed = crate::runtime::tool_loop_http::parse_final_output(&response);
    let response_text = if !parsed.text.is_empty() {
        parsed.text
    } else if let Some(s) = response.as_str() {
        s.to_string()
    } else {
        serde_json::to_string(&parsed.value).unwrap_or_default()
    };

    // Extract + parse YAML fence.
    let yaml_body = match crate::runtime::prompt_examples::extract_yaml_fence(&response_text) {
        Ok(b) => b,
        Err(e) => {
            return AuditVerdict::HardFail(format!(
                "auditor response missing fenced YAML verdict: {e}"
            ));
        }
    };
    let emitted: audit::AuditorEmittedVerdict = match serde_yaml::from_str(yaml_body) {
        Ok(v) => v,
        Err(e) => {
            return AuditVerdict::HardFail(format!("auditor verdict YAML deserialize failed: {e}"));
        }
    };

    // Persist verdict to disk for re-run replay.
    let (tokens_in, tokens_out) = extract_audit_token_counts(&response);
    let on_disk = audit::AuditVerdictOnDisk {
        agent_id: agent_id_payload.to_string(),
        stage: stage.into(),
        producer: audit::ProducerMeta {
            provider: producer_provider_label.to_string(),
            model: producer_model_id.to_string(),
            output_sha: producer_output_sha,
        },
        auditor: audit::AuditorVerdictMeta {
            provider: auditor_provider_label.to_string(),
            model: auditor_model_id,
            verdict: emitted.verdict,
            reason: emitted.reason.clone(),
        },
        audit_tokens: audit::TokenCounts {
            tokens_in,
            tokens_out,
        },
        audited_at: now_iso(),
    };
    if let Err(e) =
        audit::write_verdict_pair(audit_dir, stage, target_id, &on_disk, &transcript_rendered)
    {
        tracing::warn!(error = %e, "audit verdict persistence failed (non-fatal)");
    }

    verdict_from_emitted(emitted)
}

/// PR-4: map a freshly-parsed auditor verdict YAML to the in-memory
/// [`AuditVerdict`] enum. The `Degraded` wrapper is applied by
/// `lane_b_audit`, not here.
fn verdict_from_emitted(emitted: audit::AuditorEmittedVerdict) -> AuditVerdict {
    match emitted.verdict {
        audit::VerdictKind::Accept => AuditVerdict::Accept,
        audit::VerdictKind::RequestRevision => AuditVerdict::RequestRevision(emitted.reason),
        audit::VerdictKind::HardFail => AuditVerdict::HardFail(emitted.reason),
        audit::VerdictKind::Skipped => AuditVerdict::Skipped,
    }
}

/// PR-4: map a cached on-disk verdict to the in-memory enum on the
/// cache-replay path.
fn verdict_from_cached(cached: audit::AuditVerdictOnDisk) -> AuditVerdict {
    match cached.auditor.verdict {
        audit::VerdictKind::Accept => AuditVerdict::Accept,
        audit::VerdictKind::RequestRevision => AuditVerdict::RequestRevision(cached.auditor.reason),
        audit::VerdictKind::HardFail => AuditVerdict::HardFail(cached.auditor.reason),
        audit::VerdictKind::Skipped => AuditVerdict::Skipped,
    }
}

/// PR-4: extract `(tokens_in, tokens_out)` from an auditor response.
/// Tries both the Anthropic shape (`usage.input_tokens` /
/// `usage.output_tokens`) and the OpenAI shape (`usage.prompt_tokens`
/// / `usage.completion_tokens`); falls back to `(0, 0)` if neither is
/// present (the test backend's response shape carries no usage block).
fn extract_audit_token_counts(response: &Value) -> (u64, u64) {
    let usage = match response.get("usage") {
        Some(u) => u,
        None => return (0, 0),
    };
    let tokens_in = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let tokens_out = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    (tokens_in, tokens_out)
}

/// Caller-facing action shape produced by [`resolve_audit_verdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedAuditAction {
    /// Audit accepted (or skipped) — continue with the producer's
    /// result unchanged.
    Proceed,
    /// Audit asked for a revision — caller may fire one Lane B retry.
    /// PR-5's minimum-viable wiring observes this branch but does not
    /// yet drive the prompt-revision retry harness (see the
    /// `PR-7-ENRICHES-PROMPT-WITH-REVISION-REASON` comment in
    /// `call_agent`).
    #[allow(dead_code)]
    RequestRevision(String),
    /// Audit hard-failed — caller must propagate
    /// [`AgentError::LaneBFail`].
    HardFail(String),
}

/// Build a `ToolContext` for `request`. PR-4: workspace_root is the
/// caller's responsibility; for now the runtime threads through the
/// `Workspace.root()` field via the dispatcher. The dispatcher embeds
/// the root into each `AgentRequest`'s initial prompt; a future PR
/// will carry the root explicitly on `AgentRequest`. For PR-4 the
/// smoke test wires the workspace root manually.
fn tool_context_for(_request: &AgentRequest) -> crate::ToolContext {
    // PR-4 minimal: the smoke test does not invoke any filesystem
    // tool, so the workspace root is a placeholder. PR-5 wires the
    // real per-call workspace root once the dispatch shape stabilises.
    crate::ToolContext {
        workspace_root: std::path::PathBuf::from("/"),
    }
}

/// PR-3: default soft-cap on the per-component Classify agent's
/// tool-iteration budget (brainstorm §6.1; decision row 4). Embedded
/// in the prompt and threaded through `AgentRequest::max_steps` so
/// prompt-text and request-budget cannot drift.
pub const DEFAULT_CLASSIFY_SOFT_CAP: u32 = 6;

/// PR-3: default hard-cap on the per-component Classify agent.
pub const DEFAULT_CLASSIFY_HARD_CAP: u32 = 12;

/// PR-3: default soft-cap on the per-subsystem Reduce agent
/// (brainstorm §6.2).
pub const DEFAULT_REDUCE_SOFT_CAP: u32 = 4;

/// PR-3: default hard-cap on the per-subsystem Reduce agent.
pub const DEFAULT_REDUCE_HARD_CAP: u32 = 8;

/// PR-3: default soft-cap on the workspace-level Project agent
/// (brainstorm §6.3).
pub const DEFAULT_PROJECT_SOFT_CAP: u32 = 4;

/// PR-3: default hard-cap on the workspace-level Project agent.
pub const DEFAULT_PROJECT_HARD_CAP: u32 = 8;

/// PR-3: production per-component Classify prompt.
///
/// The agent is asked to (a) read the component's primary manifest,
/// (b) inspect at least one source entry-point, and (c) emit exactly
/// one fenced ```yaml block matching the
/// [`outputs::ClassifyAgentOutput`] shape.
///
/// `evidence_pointers` is ordered by convention: `[primary_manifest,
/// source_entrypoint, ...]`. Lane A's classify-stage evidence floor
/// (`audit::evidence::classify_evidence`) reads `evidence_pointers[0]`
/// as the manifest path the transcript must show was read, and
/// `evidence_pointers[1]` (when present) as the source-entry-point.
/// `expected_classify_tool_id` is derived from `kind` so the LLM only
/// needs to declare its kind correctly — the runtime infers which
/// parser tool should have fired.
///
/// `soft_cap` / `hard_cap` are embedded verbatim so the prompt-text
/// and the caller's [`AgentRequest::max_steps`] cannot drift.
/// PR-3's drift-catcher test
/// (`crates/atlas-agents/tests/classify_prompt_shape.rs`) asserts the
/// embedded YAML example deserializes via `ClassifyAgentOutput` AND
/// both caps appear in the prompt body.
pub fn build_classify_prompt(
    workspace_root: &Path,
    component: &ComponentPartition,
    soft_cap: u32,
    hard_cap: u32,
) -> String {
    format!(
        r#"You are Atlas's classify agent. Classify component `{component_id}` \
(subsystem `{subsystem_id}`) under workspace `{root}`.

Use the available manifest-parser tools (parse_cargo_toml, \
parse_package_json, parse_pyproject_toml, parse_dockerfile, \
parse_compose, ...) and language classifiers to read the component's \
primary manifest BEFORE assigning a kind / language / lifecycle. Then \
read at least one source entry-point (lib.rs, index.ts, __init__.py, \
the Dockerfile's FROM line, ...) to confirm.

Iteration budget: soft cap {soft_cap}; hard cap {hard_cap}. Stop \
emitting new tool calls once you can ground every field in at least \
one of: a manifest read + a parser-tool call + a source-entrypoint \
read.

Emit your final answer as exactly ONE fenced yaml block matching this \
shape:

```yaml
component_id: "{component_id}"
kind: "rust-library"
language: "rust"
lifecycle: "build"
subsystem_hint: "{subsystem_id}"
evidence_pointers:
  - path: "crates/{component_id}/Cargo.toml"
    line_range: [1, 30]
  - path: "crates/{component_id}/src/lib.rs"
confidence_grade: "moderate"
```

Field rules:
- `component_id` MUST equal `{component_id}` exactly (quoted).
- `kind` is an open-vocabulary kebab-case string. Use the canonical \
  Atlas vocabulary when it fits (`rust-library`, `rust-binary`, \
  `typescript-package`, `python-package`, `docker-image`, \
  `csharp-project`, ...). Quote it.
- `language` is the dominant programming language as a kebab-case \
  string (`rust`, `typescript`, `python`, ...). Quote it.
- `lifecycle` is one of the closed component-ontology values: \
  `design`, `codegen`, `build`, `test`, `deploy`, `runtime`, \
  `dev-workflow`. Quote it.
- `subsystem_hint` may correct the runtime-supplied subsystem if your \
  reading of the manifest disagrees; otherwise echo `{subsystem_id}`.
- `evidence_pointers` is REQUIRED and ORDERED: index 0 is the primary \
  manifest path you read; index 1 (when present) is the source \
  entry-point you read. Additional indices may carry supporting \
  evidence. Each `path` is workspace-relative.
- `confidence_grade` ∈ {{"strong", "moderate", "weak", "declines"}}.

`confidence_grade` rubric:
- "strong": primary manifest READ and source entry-point READ and \
  the classifier tool whose name matches the declared `kind` was \
  CALLED.
- "moderate": primary manifest READ and the classifier tool was \
  CALLED, but no source entry-point read (or the kind was inferred \
  from the manifest alone).
- "weak": primary manifest READ but no classifier tool called (the \
  kind/language are best-guess from filename / directory structure).
- "declines": the primary manifest could not be read, OR there isn't \
  enough evidence to commit to a kind/language — emit a best-guess \
  + this grade so a downstream consumer or human reviewer can \
  intervene.

Quote any identity-shaped scalar (component id, kind, language) that \
could collide with YAML's implicit-typing rules — for example a \
component literally called "true", "1.10", or "0123" must appear as \
`"true"`, `"1.10"`, `"0123"`.
"#,
        component_id = component.id,
        subsystem_id = component.subsystem_id,
        root = workspace_root.display(),
        soft_cap = soft_cap,
        hard_cap = hard_cap,
    )
}

/// PR-3: production per-subsystem Reduce prompt.
///
/// The reducer consumes per-component classify outputs and emits ONE
/// fenced ```yaml block matching the [`outputs::ReduceAgentOutput`]
/// shape — including refactoring cues (framing #2 use-case b) and
/// internal edges between the subsystem's components.
///
/// `classify_outputs` carries the already-classified per-component
/// outputs as a `(component_id, kind, language)` tuple. The reducer
/// uses these for context; the prompt does not embed the raw classify
/// YAML to keep prompt size bounded.
pub fn build_reduce_prompt(
    workspace_root: &Path,
    subsystem: &SubsystemPartition,
    classify_outputs: &[(String, String, String)],
    soft_cap: u32,
    hard_cap: u32,
) -> String {
    let component_rollup = if classify_outputs.is_empty() {
        format!(
            "(no per-component classify outputs available; \
             dispatched members: {:?})",
            subsystem.members
        )
    } else {
        classify_outputs
            .iter()
            .map(|(id, kind, lang)| format!("  - id: {id}; kind: {kind}; language: {lang}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let example_components: Vec<String> = subsystem
        .members
        .iter()
        .take(2)
        .map(|s| format!("\"{}\"", s))
        .collect();
    let example_components_yaml = if example_components.is_empty() {
        "  - \"example-component\"".to_string()
    } else {
        example_components
            .iter()
            .map(|c| format!("  - {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"You are Atlas's reduce agent. Reduce the per-component \
classify outputs into a subsystem-level summary for subsystem \
`{subsystem_id}` (workspace `{root}`).

Per-component classify outputs you were handed:
{component_rollup}

Use the available tools (manifest parsers, language classifiers, \
surface analysers) ONLY to verify cross-component facts: shared \
contracts between components, internal edges (component → component) \
inside this subsystem, and refactoring cues spanning two or more \
components. Avoid re-classifying individual components — that work is \
already done.

Iteration budget: soft cap {soft_cap}; hard cap {hard_cap}. Stop \
emitting new tool calls once every claim in your output is grounded.

Emit your final answer as exactly ONE fenced yaml block matching this \
shape:

```yaml
subsystem_id: "{subsystem_id}"
purpose: "One to three sentences describing what this subsystem does, \
LLM-consumable as standalone context."
declared_child_component_ids:
{example_components_yaml}
component_ids:
{example_components_yaml}
key_contracts:
  - id: "tools/parse_cargo_toml"
    kind: "tool-handle"
    source_path:
      path: "crates/example/src/tools.rs"
internal_edges:
  - from: "example-component"
    to: "example-component"
    kind: "depends-on"
refactoring_cues:
  - kind: "abstraction-opportunity"
    component_ids: ["example-component"]
    rationale: "One sentence explaining the cue."
    evidence_pointers:
      - path: "crates/example/src/lib.rs"
evidence_pointers:
  - path: "crates/example/Cargo.toml"
confidence_grade: "moderate"
```

Field rules:
- `subsystem_id` MUST equal `{subsystem_id}` exactly (quoted).
- `purpose` is 1-3 sentences. Frame it as standalone LLM context — a \
  downstream tool should be able to understand what this subsystem \
  does from this sentence alone.
- `declared_child_component_ids` MUST echo back the exact list of \
  per-component classify outputs you were handed (the component ids \
  above). Lane A's reduce-stage evidence floor reads this as the \
  denominator of the coverage ratio.
- `component_ids` is the set of children you actually accounted for in \
  this subsystem reduce. To score Strong, this MUST equal \
  `declared_child_component_ids`.
- `key_contracts` lists the cross-component contracts the subsystem \
  exposes (traits, tool-handles, HTTP endpoints, IDL definitions, ...). \
  `kind` is free-text; quote it.
- `internal_edges` lists component→component relationships INSIDE \
  this subsystem. `kind` should match the component-ontology edge \
  vocabulary (`depends-on`, `calls`, `provides-contract`, \
  `implements-contract`, ...). Quote it.
- `refactoring_cues` is load-bearing — Atlas exists in part to \
  surface refactoring opportunities for downstream LLM consumers. \
  `kind` ∈ {{"duplication", "mis-modularised", \
  "abstraction-opportunity", "dependency-inversion", "dead-code", \
  "other"}}. Quote it.
- `evidence_pointers` cites the subsystem-level evidence you read \
  (cross-component manifests, README, design docs).
- `confidence_grade` ∈ {{"strong", "moderate", "weak", "declines"}}.

`confidence_grade` rubric:
- "strong": every child component appears in `component_ids`; every \
  `refactoring_cue` and `internal_edge` carries an evidence pointer; \
  `key_contracts` are grounded in source-path reads.
- "moderate": most children accounted for; some cues lack evidence \
  pointers.
- "weak": purpose written but contracts / edges / cues are sparse or \
  unverified.
- "declines": fewer than half the children consumed — surface the \
  best-effort reduce and a `declines` grade.

Quote any identity-shaped scalar (component id, subsystem id, \
contract id, edge kind) that could collide with YAML's implicit-typing \
rules.
"#,
        subsystem_id = subsystem.id,
        root = workspace_root.display(),
        component_rollup = component_rollup,
        example_components_yaml = example_components_yaml,
        soft_cap = soft_cap,
        hard_cap = hard_cap,
    )
}

/// PR-3: production workspace-level Project prompt.
///
/// The project agent consumes per-subsystem reduce outputs and emits
/// ONE fenced ```yaml block matching the
/// [`outputs::ProjectAgentOutput`] shape — including the
/// `doc_scaffold` outline (framing #2 use-case (c) — documentation
/// generation).
///
/// `reduce_outputs` is the rollup the runtime collected from each
/// subsystem reduce; the prompt embeds it as a compact list.
pub fn build_project_prompt(
    workspace_root: &Path,
    reduce_outputs: &[(String, String, u32)],
    soft_cap: u32,
    hard_cap: u32,
) -> String {
    let subsystem_rollup = if reduce_outputs.is_empty() {
        "(no subsystem reduces available)".to_string()
    } else {
        reduce_outputs
            .iter()
            .map(|(id, purpose, count)| {
                format!("  - id: {id}; component_count: {count}; purpose: {purpose}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let example_subsystem_ids: Vec<String> = reduce_outputs
        .iter()
        .take(2)
        .map(|(id, _, _)| format!("\"{}\"", id))
        .collect();
    let example_subsystem_yaml = if example_subsystem_ids.is_empty() {
        "  - \"example-subsystem\"".to_string()
    } else {
        example_subsystem_ids
            .iter()
            .map(|s| format!("  - {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"You are Atlas's project agent. Roll the per-subsystem reduce \
outputs up into a workspace-level architecture summary for workspace \
`{root}`. The output is the PRIMARY LLM-consumable artifact downstream \
tools read first.

Per-subsystem reduce outputs you were handed:
{subsystem_rollup}

Use the available tools sparingly here — most of the evidence has \
already been gathered. Read top-level docs (README.md, ARCHITECTURE.md, \
docs/*.md) if they exist; do not re-classify components.

Iteration budget: soft cap {soft_cap}; hard cap {hard_cap}.

Emit your final answer as exactly ONE fenced yaml block matching this \
shape:

```yaml
workspace_purpose: "Two to five sentences describing what this \
workspace as a whole does. Downstream LLM consumers read this first."
declared_subsystem_ids:
{example_subsystem_yaml}
subsystem_catalog:
  - subsystem_id: "example-subsystem"
    purpose: "One sentence summary."
    component_count: 3
cross_subsystem_edges:
  - from: "example-subsystem"
    to: "example-subsystem"
    kind: "depends-on"
workspace_refactoring_cues:
  - kind: "abstraction-opportunity"
    component_ids: ["example-component"]
    rationale: "One sentence explaining the cue."
    evidence_pointers:
      - path: "docs/architecture.md"
doc_scaffold:
  sections:
    - heading: "Architecture overview"
      source_references:
        - path: "docs/architecture.md"
      child_sections:
        - heading: "Per-subsystem deep-dives"
          source_references:
            - path: "docs/architecture.md"
confidence_grade: "moderate"
```

Field rules:
- `workspace_purpose` is 2-5 sentences. Frame it as standalone LLM \
  context — a downstream tool should understand the workspace's reason \
  to exist from this paragraph alone.
- `declared_subsystem_ids` MUST echo back EVERY subsystem id from the \
  rollup above. Lane A's project-stage evidence floor reads this as \
  the denominator of the coverage ratio.
- `subsystem_catalog` MUST contain one row per subsystem you accounted \
  for. To score Strong, every `declared_subsystem_ids` entry MUST \
  appear as a `subsystem_id` here.
- `cross_subsystem_edges` lists subsystem→subsystem relationships. \
  `kind` follows the component-ontology edge vocabulary.
- `workspace_refactoring_cues` is the workspace-level analog of the \
  per-subsystem `refactoring_cues` — cross-subsystem opportunities. \
  Same `kind` vocabulary as `RefactoringCueKind`.
- `doc_scaffold` is REQUIRED and load-bearing — downstream \
  documentation-generation tools fill the body of each section using \
  the cited `source_references`. Heading text should read as a \
  table-of-contents entry. Recurse via `child_sections` for \
  subheadings.
- `confidence_grade` ∈ {{"strong", "moderate", "weak", "declines"}}.

`confidence_grade` rubric:
- "strong": every subsystem appears in `subsystem_catalog`; \
  `doc_scaffold` covers every subsystem at least once; \
  `workspace_refactoring_cues` reference real edges with evidence \
  pointers.
- "moderate": most subsystems cataloged; `doc_scaffold` has gaps.
- "weak": `workspace_purpose` written but `subsystem_catalog` \
  incomplete or `doc_scaffold` shallow.
- "declines": cannot produce a coherent workspace-level view — emit \
  best-effort + this grade.

Quote any identity-shaped scalar that could collide with YAML's \
implicit-typing rules.
"#,
        root = workspace_root.display(),
        subsystem_rollup = subsystem_rollup,
        example_subsystem_yaml = example_subsystem_yaml,
        soft_cap = soft_cap,
        hard_cap = hard_cap,
    )
}

/// Convert the runtime `Grade` to the engine's `AgentGrade`. The two
/// enums are isomorphic but live in different crates per the
/// engine-below-agents layering rule.
fn grade_to_engine(grade: &Grade) -> AgentGrade {
    match grade {
        Grade::Strong => AgentGrade::Strong,
        Grade::Moderate => AgentGrade::Moderate,
        Grade::Weak => AgentGrade::Weak,
        Grade::Declines => AgentGrade::Declines,
    }
}

/// Inverse of `grade_to_engine`.
fn engine_to_grade(grade: &AgentGrade) -> Grade {
    match grade {
        AgentGrade::Strong => Grade::Strong,
        AgentGrade::Moderate => Grade::Moderate,
        AgentGrade::Weak => Grade::Weak,
        AgentGrade::Declines => Grade::Declines,
    }
}

/// Hex-encode a sha256 digest as a 64-char lowercase string. Used
/// for the `AgentComplete.output_sha` event field.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for &b in digest.as_slice() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decode a 64-char lowercase-hex string into `out`. Returns `Err(())`
/// on any malformed input. Used to thread `IterationBoundary`'s
/// hex-string prior model sha back into the cache fingerprint's
/// `Option<[u8; 32]>` shape.
fn decode_hex_into(s: &str, out: &mut [u8; 32]) -> Result<(), ()> {
    if s.len() != 64 {
        return Err(());
    }
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(())
}

fn hex_nibble(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

/// PR-4 ISO-8601 timestamp helper (UTC, second precision). PR-5 may
/// add millisecond precision for clearer trace ordering; PR-4 keeps
/// the format dependency-free.
///
/// PR-7 hoisted to `pub(super)` to consolidate the duplicated copy in
/// [`crate::runtime::dispatch`] (PR-5 closeout MEDIUM-3). One source of
/// truth for the event-timestamp shape.
pub(super) fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Render seconds since epoch — sortable, no chrono dep, good
    // enough for PR-4 telemetry. PR-7 wires the real wall-clock
    // formatter.
    format!("{secs}")
}

/// Decode an output-bytes blob back into an `AgentOutput`. PR-4 stores
/// the JSON-serialised `output.value` directly; the recorder/cache
/// path keeps shapes symmetric.
fn decode_output_bytes(bytes: &[u8]) -> Result<AgentOutput, AgentError> {
    // The cache may hand us either:
    //   - The bytes we wrote (raw JSON of `output.value`).
    //   - Bytes prefixed with the transcript-frame prefix (defensive;
    //     shouldn't happen for the `.output` path, but guard anyway).
    let raw = if bytes.starts_with(TRANSCRIPT_FRAME_PREFIX) {
        // Skip the framing header line.
        let after_prefix = &bytes[TRANSCRIPT_FRAME_PREFIX.len()..];
        let newline = after_prefix
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| AgentError::Cache("framed output missing newline".to_string()))?;
        &after_prefix[newline + 1..]
    } else {
        bytes
    };
    let value: Value = serde_json::from_slice(raw)
        .map_err(|e| AgentError::Cache(format!("output decode failed: {e}")))?;
    Ok(AgentOutput::from_value(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_error_lift_from_llm_error() {
        let err = AgentError::from_llm_error(LlmError::Invocation("nope".into()));
        match err {
            AgentError::Backend(msg) => assert!(msg.contains("nope")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn tool_catalog_default_contains_22_wrappers() {
        let cat = default_tool_catalog();
        assert_eq!(cat.len(), 22);
    }

    #[test]
    fn tool_catalog_catalog_sha_is_deterministic() {
        let a = default_tool_catalog().catalog_sha();
        let b = default_tool_catalog().catalog_sha();
        assert_eq!(a, b);
    }

    #[test]
    fn content_sha_to_hex_round_trips() {
        let s = ContentSha("abc123".to_string());
        assert_eq!(s.to_hex(), "abc123");
    }

    #[test]
    fn decode_hex_into_round_trips() {
        let mut out = [0u8; 32];
        let s = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        decode_hex_into(s, &mut out).unwrap();
        assert_eq!(out[0], 0xab);
        assert_eq!(out[1], 0xcd);
        assert_eq!(out[31], 0x89);
    }

    #[test]
    fn decode_hex_into_rejects_bad_length() {
        let mut out = [0u8; 32];
        assert!(decode_hex_into("abc", &mut out).is_err());
    }

    #[test]
    fn decode_output_bytes_handles_raw_json() {
        let v = AgentOutput::from_value(serde_json::json!({ "ok": 1 }));
        let bytes = serde_json::to_vec(&v.value).unwrap();
        let decoded = decode_output_bytes(&bytes).unwrap();
        assert_eq!(decoded.value, serde_json::json!({ "ok": 1 }));
    }

    #[test]
    fn l9_projection_is_empty_default() {
        assert!(L9Projection::default().is_empty());
    }
}
