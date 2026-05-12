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
pub mod semaphores;
pub mod tool_loop_http;
pub mod tool_loop_mcp;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use atlas_engine::llm_cache::{
    AgentGrade, AgentInputFingerprint, FingerprintInputSpotCheck, LlmResponseCache,
    TRANSCRIPT_FRAME_PREFIX,
};
use atlas_index::Stage as IndexStage;
use atlas_llm::{LlmBackend, LlmError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::events::{AgentEvent, CacheHitSource, EventBus, Grade};
use crate::transport::{Provider, TransportFlavour};
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
    /// existing `current_sha_fn` placeholder pattern, costs no
    /// `LlmBackend`-trait surgery, and keeps `Provider` confined to
    /// `atlas-agents`. PR-7 plugs in a closure that delegates to the
    /// real `BackendRouter::backend_for_provider` (which PR-7 adds);
    /// tests inject simpler mocks via [`AgentRuntime::with_for_provider`].
    pub for_provider: Option<Arc<ForProviderFn>>,
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

        // Pass 2: drive per-component Classify + Surface, then
        // per-subsystem Reduce, then a workspace-level Project.
        for (subsystem, components) in subsystem_components {
            for component in &components {
                let classify_req = AgentRequest {
                    stage: Stage::Classify,
                    target_id: component.id.clone(),
                    iteration: iter,
                    transport: self.default_transport,
                    initial_prompt: build_classify_prompt(workspace.root(), component),
                    fingerprint_inputs: Vec::new(),
                    candidate_ids: all_component_ids.clone(),
                    prior_model_sha: prior_model_sha.clone(),
                };
                let classify_res = self.call_agent(classify_req).await?;
                projection
                    .components
                    .insert(component.id.clone(), classify_res.output);
            }

            let reduce_req = AgentRequest {
                stage: Stage::Reduce,
                target_id: subsystem.id.clone(),
                iteration: iter,
                transport: self.default_transport,
                initial_prompt: build_reduce_prompt(&subsystem, &components),
                fingerprint_inputs: Vec::new(),
                candidate_ids: all_component_ids.clone(),
                prior_model_sha: prior_model_sha.clone(),
            };
            let reduce_res = self.call_agent(reduce_req).await?;
            projection
                .subsystems
                .insert(subsystem.id.clone(), reduce_res.output);
        }

        // Workspace-level project.
        let project_req = AgentRequest {
            stage: Stage::Project,
            target_id: "_workspace".to_string(),
            iteration: iter,
            transport: self.default_transport,
            initial_prompt: "project the workspace projection".to_string(),
            fingerprint_inputs: Vec::new(),
            candidate_ids: all_component_ids.clone(),
            prior_model_sha: prior_model_sha.clone(),
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

        // Lane B audit (recast §4.3, brainstorm §6 (iii)). The current
        // tool-loop always produces `Grade::Strong` on success, so
        // `lane_b_audit` returns `Skipped` under the existing PR-4 test
        // backend — the wiring is the spec deliverable; empirical
        // firing depends on the producer's grade, which is a PR-7+
        // concern (the production prompt template lets the model emit
        // `Grade::Weak`/`Grade::Declines` self-grades).
        let producer_provider = request.transport.provider();
        let producer_backend = self.backend_router.clone();
        let for_provider_fn = self.for_provider.clone();
        let bus_ref = self.event_bus.as_ref();
        let producer_agent_id = agent_id(&request);
        let verdict = audit::lane_b_audit(
            bus_ref,
            &producer_agent_id,
            &runtime_result.grade,
            producer_provider,
            &producer_backend,
            for_provider_fn
                .as_deref()
                .map(|f| f as &(dyn Fn(Provider) -> Option<Arc<dyn LlmBackend>> + Send + Sync)),
            // PR-7-WIRES-REAL-AUDITOR: the auditor closure is a stub;
            // PR-7 plumbs the real audit-prompt round-trip against
            // the chosen backend. Under PR-5 the closure returns
            // `Accept` so when Lane B fires (Weak/Declines grade) the
            // producer result stands; this is the documented
            // minimum-viable wiring (FIX 2 step 2).
            |_auditor_backend| async { audit::AuditVerdict::Accept },
        )
        .await;
        match resolve_audit_verdict(&verdict, lane_a_retries) {
            ResolvedAuditAction::Proceed => {
                // Lane B accepted (or skipped). Fall through to cache
                // write + AgentComplete with the original result.
            }
            ResolvedAuditAction::HardFail(reason) => {
                self.event_bus.emit(AgentEvent::HardFail {
                    agent_id: producer_agent_id.clone(),
                    error_kind: "lane_b".to_string(),
                    error_summary: reason.clone(),
                    retry_count: lane_a_retries,
                });
                return Err(AgentError::LaneBFail(reason));
            }
            ResolvedAuditAction::RequestRevision(_reason) => {
                // PR-7-ENRICHES-PROMPT-WITH-REVISION-REASON: full
                // revision-retry harness is a PR-7 deliverable; PR-5's
                // minimum-viable Lane B wiring records the verdict
                // (via the `AuditVerdict` event already emitted
                // inside `lane_b_audit`) and accepts the producer's
                // result by falling through to the cache-write
                // below. Cumulative-retry budget is honoured here:
                // if Lane A already retried, the resolver above maps
                // `RequestRevision` → `HardFail` rather than
                // falling through to this branch.
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
                    // PR-4: subprocess transports route through the
                    // MCP observation harness. The runtime caller
                    // (PR-7) is responsible for wiring the
                    // subprocess's stdin/stdout to a `serve_client`
                    // task on the MCP server. PR-4 itself never spawns
                    // a subprocess — the smoke test uses TestBackend.
                    // We surface a clear error so an accidental PR-4
                    // deployment doesn't silently no-op.
                    return Err(AgentError::Backend(
                        "PR-4 runtime does not drive subprocess transports directly; \
                         PR-7 wires the MCP `serve_client` task"
                            .to_string(),
                    ));
                }
            };
            let output = outcome?;
            match lane_a_validate(&output, request.stage, &request.candidate_ids).await {
                Ok(()) => {
                    let grade = Grade::Strong;
                    let grade_engine = grade_to_engine(&grade);
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

/// PR-5 follow-up: Lane B verdict → caller action mapping.
///
/// `Skipped` / `Accept` → `Proceed`. `HardFail(reason)` → `HardFail`.
/// `RequestRevision(reason)` → either `RequestRevision` (room left in
/// the cumulative budget) or `HardFail` (Lane A already retried — the
/// agent has spent its quota per recast §4.3). The `Degraded` wrapper
/// is unwrapped and resolved recursively.
fn resolve_audit_verdict(verdict: &AuditVerdict, lane_a_retries: u32) -> ResolvedAuditAction {
    match verdict {
        AuditVerdict::Skipped | AuditVerdict::Accept => ResolvedAuditAction::Proceed,
        AuditVerdict::HardFail(reason) => ResolvedAuditAction::HardFail(reason.clone()),
        AuditVerdict::RequestRevision(reason) => {
            if lane_a_retries >= 1 {
                // Cumulative budget exhausted (Lane A burned the only
                // retry slot). Escalate to a hard fail.
                ResolvedAuditAction::HardFail(format!(
                    "lane_b request_revision after lane_a retry exhausted budget: {reason}"
                ))
            } else {
                ResolvedAuditAction::RequestRevision(reason.clone())
            }
        }
        AuditVerdict::Degraded(inner) => resolve_audit_verdict(inner, lane_a_retries),
    }
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

/// Render the per-component Classify prompt. PR-4 keeps it
/// content-free — the smoke test only inspects routing and Lane A,
/// not the actual classifier prompt; PR-5 will replace this with the
/// production prompt template.
fn build_classify_prompt(_root: &Path, component: &ComponentPartition) -> String {
    format!(
        "classify component id={} subsystem={} (PR-4 placeholder)",
        component.id, component.subsystem_id
    )
}

/// Render the per-subsystem reduce prompt. PR-4 placeholder; PR-5
/// fills in the production prompt template.
fn build_reduce_prompt(
    subsystem: &SubsystemPartition,
    components: &[ComponentPartition],
) -> String {
    format!(
        "reduce subsystem={} components=[{}] (PR-4 placeholder)",
        subsystem.id,
        components
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
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
