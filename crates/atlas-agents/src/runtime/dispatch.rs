//! Workspace → subsystem → component partitioning.
//!
//! PR-5 relaxes PR-4's mandatory-override stance to **LLM-with-shortcircuit**:
//! when `subsystems.overrides.yaml` / `components.overrides.yaml` exists at
//! the workspace root, the dispatcher short-circuits the LLM dispatch
//! agent, validates the override file via Lane A, emits a
//! `CacheHit { source: DispatchedFromOverride }` event, and returns the
//! parsed partitions. When no override is present, an LLM-driven
//! dispatch agent fires; Lane A validates the result; the runtime
//! materialises a transcript-cache entry under the same fingerprint shape
//! as the synthetic-from-override path.
//!
//! # Cache-invariant rule (recast §6.1)
//!
//! The dispatch agent's fingerprint includes `override_content_sha`
//! (or the sentinel `None` if absent):
//!
//! - **Adding an override** changes the contributor from `None` to
//!   `Some(sha)` and invalidates any prior LLM-dispatch transcript.
//!   The next dispatch fires the override-shortcircuit path and writes
//!   a new synthetic transcript.
//! - **Removing an override** changes the contributor from `Some(sha)`
//!   to `None` and invalidates the synthetic-from-override transcript.
//!   The next dispatch fires the LLM agent.
//! - **Editing an override** changes the `sha` and invalidates both
//!   prior shapes.
//!
//! This rule is enforced by [`atlas_engine::llm_cache::AgentInputFingerprint`]'s
//! `override_content_sha` field — every dispatch fingerprint contributes
//! it, so the persistent transcript cache evicts cleanly across the
//! override-add / override-remove / override-edit transitions.
//!
//! Both override files are parsed via `serde_yaml::from_str` with
//! `#[serde(deny_unknown_fields)]` shape, so typos surface as parse
//! errors rather than silent drops. The parse result is Lane-A
//! validated for structural sanity (non-empty id, etc.) before being
//! handed to `AgentRuntime`.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use atlas_engine::llm_cache::{
    frame_transcript_with_grade, AgentGrade, AgentInputFingerprint, FingerprintInputSpotCheck,
};
use atlas_index::Stage as IndexStage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::events::{AgentEvent, CacheHitSource};

use super::audit::{L1CandidateRef, Stage};
use super::prompt_examples::{extract_yaml_fence, FenceExtractError};
use super::yaml_strict::deserialize_string_strict;
use super::{now_iso, AgentError, AgentRequest, AgentRuntime, Workspace};

/// PR-2: default soft cap on the dispatch agent's tool-iteration budget.
/// Embedded in the prompt and threaded through `AgentRequest::max_steps`
/// so prompt-text and request-budget cannot drift (decision row 4).
pub const DEFAULT_DISPATCH_SOFT_CAP: u32 = 15;

/// PR-2: default hard cap on the dispatch agent's tool-iteration budget.
pub const DEFAULT_DISPATCH_HARD_CAP: u32 = 30;

/// File name for the subsystems override pin.
pub const SUBSYSTEMS_OVERRIDE_FILENAME: &str = "subsystems.overrides.yaml";

/// File name for the components override pin.
pub const COMPONENTS_OVERRIDE_FILENAME: &str = "components.overrides.yaml";

/// One subsystem partition resolved from `subsystems.overrides.yaml`.
/// Minimal PR-4 shape: id + members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemPartition {
    /// Stable id of this subsystem (e.g. `"agents"`).
    pub id: String,
    /// Members — each entry is a component id.
    pub members: Vec<String>,
}

/// One component partition resolved from `components.overrides.yaml`,
/// after subsystem-membership filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentPartition {
    /// Stable component id.
    pub id: String,
    /// Subsystem the component belongs to (the dispatcher decided).
    pub subsystem_id: String,
    /// Optional field overrides parsed from the per-component override
    /// file (Phase 6 PR-3 / recast §4.3).
    #[serde(default)]
    pub field_overrides: ComponentFieldOverrides,
}

/// Subset of the `OverridesFile.field_overrides` shape that the
/// runtime needs to propagate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentFieldOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
}

/// On-disk shape for `subsystems.overrides.yaml` AND wire-envelope
/// shape the dispatch-subsystems LLM emits.
///
/// PR-2: extended with `candidates_considered` + `confidence_grade`
/// fields so the LLM's evidence-floor envelope round-trips through
/// the same struct as the user-authored override file. The shortcircuit
/// path leaves both fields empty; the LLM-decided path populates them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemsOverrideFile {
    pub schema_version: u32,
    #[serde(default)]
    pub subsystems: Vec<SubsystemOverrideEntry>,
    /// PR-2: L1 candidates the dispatch agent inspected (with primary
    /// manifest paths). Lane A's evidence-floor scorer matches these
    /// against the transcript's `read_file_paths`.
    #[serde(default)]
    pub candidates_considered: Vec<L1CandidateRef>,
    /// PR-2: LLM's self-claimed confidence grade. Lane A clamps this
    /// against the deterministic evidence ceiling.
    #[serde(default)]
    pub confidence_grade: Option<String>,
}

/// One subsystem entry inside `subsystems.overrides.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemOverrideEntry {
    /// Kebab-case subsystem id. PR-2 applies the strict-string adapter
    /// to defend against YAML implicit-typing coercion of
    /// identity-shaped scalars (`id: true` → bool, `id: 1.10` → float,
    /// etc.).
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub id: String,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub evidence_grade: Option<String>,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<String>,
}

/// On-disk shape for `components.overrides.yaml` AND wire-envelope
/// shape the dispatch-components LLM emits. PR-2: parallel to
/// [`SubsystemsOverrideFile`], extended with the same evidence-floor
/// fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentsOverrideFile {
    pub schema_version: u32,
    #[serde(default)]
    pub components: std::collections::BTreeMap<String, ComponentOverrideEntry>,
    /// PR-2: same shape as on [`SubsystemsOverrideFile`].
    #[serde(default)]
    pub candidates_considered: Vec<L1CandidateRef>,
    /// PR-2: same shape as on [`SubsystemsOverrideFile`].
    #[serde(default)]
    pub confidence_grade: Option<String>,
}

/// One component entry inside `components.overrides.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentOverrideEntry {
    #[serde(default)]
    pub subsystem: Option<String>,
    #[serde(default, rename = "overrides")]
    pub field_overrides: ComponentFieldOverrides,
}

/// PR-5: dispatch the workspace into subsystem partitions. Short-circuits
/// to the override file when present; otherwise fires the LLM-decided
/// dispatch agent through [`AgentRuntime::call_agent`] (which threads
/// Lane A + cache + Lane B per the standard agent contract).
///
/// **Override path:** when `subsystems.overrides.yaml` is present, the
/// dispatcher reads + Lane-A-validates the file, emits a
/// `CacheHit { source: DispatchedFromOverride }` event, materialises a
/// synthetic transcript-cache entry keyed on the override's content sha,
/// and returns the parsed partitions.
///
/// **LLM path (PR-5 follow-up):** when no override is present, the
/// dispatcher constructs an `AgentRequest` for
/// `Stage::DispatchSubsystem` and routes it through `call_agent`. The
/// returned `AgentOutput.value` must be a JSON object matching the
/// in-memory `SubsystemsOverrideFile` shape (`schema_version` +
/// `subsystems: [...]`) — wire envelope chosen for symmetry with the
/// override file. PR-7 replaces the prompt-template placeholder with
/// the production prompt template; the parser/cache path is final.
pub async fn dispatch_subsystems(
    runtime: &AgentRuntime,
    workspace: &Workspace,
) -> Result<Vec<SubsystemPartition>, AgentError> {
    let path = workspace.root().join(SUBSYSTEMS_OVERRIDE_FILENAME);
    if path.exists() {
        let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
            AgentError::OverrideRequired(format!("failed to read {}: {e}", path.display()))
        })?;
        let parsed = parse_subsystems_yaml(&text, &path)?;
        let override_sha = sha256_bytes(text.as_bytes());
        emit_dispatch_shortcircuit_hit(
            runtime,
            Stage::DispatchSubsystem,
            "_workspace",
            override_sha,
        );
        let projected = subsystems_from_parsed(parsed, &path)?;
        write_synthetic_dispatch_transcript(
            runtime,
            Stage::DispatchSubsystem,
            "_workspace",
            override_sha,
            text.as_bytes(),
            &serde_json::to_vec(&projected).unwrap_or_default(),
        );
        return Ok(projected);
    }
    // No override file: fire the LLM-dispatch agent via `call_agent`.
    // Lane A's candidate-id check is skipped here (`candidate_ids` is
    // empty) because the dispatch agent decides the candidate set rather
    // than consult one — `lane_a_validate` documents this behaviour. The
    // resulting fingerprint carries `override_content_sha: None` per
    // `call_agent`'s non-dispatch wiring; cache-invariant rule (recast
    // §6.1) still holds because the override-shortcircuit path produces
    // `Some(sha)` for the same `(stage, target_id, iteration=0)` tuple.
    //
    // PR-2: production dispatch prompt + YAML-canonical output envelope.
    // Lane A's evidence-floor scorer (downstream of `call_agent`) clamps
    // the LLM's self-grade against the deterministic transcript-derived
    // ceiling.
    let request = AgentRequest {
        stage: Stage::DispatchSubsystem,
        target_id: "_workspace".to_string(),
        iteration: 0,
        transport: runtime.default_transport,
        initial_prompt: build_dispatch_subsystems_prompt(
            workspace.root(),
            DEFAULT_DISPATCH_SOFT_CAP,
            DEFAULT_DISPATCH_HARD_CAP,
        ),
        fingerprint_inputs: Vec::new(),
        candidate_ids: HashSet::new(),
        prior_model_sha: None,
    };
    let result = runtime.call_agent(request).await?;
    parse_subsystems_from_output(&result.output.text)
}

/// PR-5: dispatch a subsystem's components. Same shortcircuit shape as
/// `dispatch_subsystems`; LLM-decided path on absent override.
pub async fn dispatch_components(
    runtime: &AgentRuntime,
    workspace: &Workspace,
    subsystem: &SubsystemPartition,
) -> Result<Vec<ComponentPartition>, AgentError> {
    let path = workspace.root().join(COMPONENTS_OVERRIDE_FILENAME);
    if path.exists() {
        let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
            AgentError::OverrideRequired(format!("failed to read {}: {e}", path.display()))
        })?;
        let parsed = parse_components_yaml(&text, &path)?;
        let override_sha = sha256_bytes(text.as_bytes());
        emit_dispatch_shortcircuit_hit(
            runtime,
            Stage::DispatchComponent,
            &subsystem.id,
            override_sha,
        );
        let partitions = components_from_parsed(parsed, subsystem);
        write_synthetic_dispatch_transcript(
            runtime,
            Stage::DispatchComponent,
            &subsystem.id,
            override_sha,
            text.as_bytes(),
            &serde_json::to_vec(&partitions).unwrap_or_default(),
        );
        return Ok(partitions);
    }
    // No override file: fire the LLM-dispatch agent. The envelope shape
    // mirrors `ComponentsOverrideFile` so the same `components_from_parsed`
    // helper projects to `ComponentPartition`.
    //
    // PR-2: production dispatch prompt + YAML-canonical output envelope.
    let request = AgentRequest {
        stage: Stage::DispatchComponent,
        target_id: subsystem.id.clone(),
        iteration: 0,
        transport: runtime.default_transport,
        initial_prompt: build_dispatch_components_prompt(
            workspace.root(),
            subsystem,
            DEFAULT_DISPATCH_SOFT_CAP,
            DEFAULT_DISPATCH_HARD_CAP,
        ),
        fingerprint_inputs: Vec::new(),
        candidate_ids: HashSet::new(),
        prior_model_sha: None,
    };
    let result = runtime.call_agent(request).await?;
    parse_components_from_output(&result.output.text, subsystem)
}

/// PR-2: production dispatch-subsystems prompt. The agent is asked to
/// (a) enumerate L1 candidates from the workspace, (b) read each
/// candidate's primary manifest before grouping, and (c) emit one
/// fenced ```yaml block matching the [`SubsystemsOverrideFile`] shape
/// (extended with `candidates_considered` + `confidence_grade` so
/// Lane A's evidence floor can clamp the LLM's self-grade).
///
/// The test backend keys canned responses on the `"dispatch subsystems"`
/// substring; that phrase is preserved in the prompt's opening line.
///
/// `soft_cap` / `hard_cap` are embedded verbatim so the prompt-text
/// and the caller's [`AgentRequest::max_steps`] (decision row 4) cannot
/// drift. PR-2's drift-catcher test in `tests/dispatch_prompt_shape.rs`
/// asserts both caps appear in the prompt.
pub fn build_dispatch_subsystems_prompt(
    workspace_root: &Path,
    soft_cap: u32,
    hard_cap: u32,
) -> String {
    format!(
        r#"You are Atlas's dispatch subsystems agent. Inspect the workspace at \
{root} and partition its components into subsystems.

Use the available manifest-parser tools (parse_cargo_toml, parse_compose, \
parse_dockerfile, parse_package_json) and language classifiers to gather \
evidence. Read each L1 candidate's primary manifest BEFORE assigning it \
to a subsystem.

Iteration budget: soft cap {soft_cap}; hard cap {hard_cap}. Stop emitting \
new tool calls once you can ground every subsystem assignment in at \
least one manifest read.

Emit your final answer as exactly ONE fenced yaml block matching this \
shape:

```yaml
schema_version: 1
candidates_considered:
  - id: "example-component"
    primary_manifest_path: "crates/example/Cargo.toml"
subsystems:
  - id: "core"
    members:
      - "example-component"
confidence_grade: "moderate"
```

The `candidates_considered` field MUST list every L1 candidate you \
inspected together with the manifest path you read for it. Lane A \
compares this list against your tool-call transcript: claims unsupported \
by manifest reads are clamped downward.

`confidence_grade` rubric (decision row 5):
- "strong": every subsystem member's primary manifest was read AND \
  classified; structural evidence is unambiguous.
- "moderate": most members read; one or two grouping decisions rest \
  on heuristic naming evidence rather than manifest content.
- "weak": several grouping decisions rest on naming alone; manifest \
  reads partial.
- "declines": insufficient evidence to commit; emit a best-guess \
  partition + this grade so a human reviewer can intervene.

Quote any identity-shaped scalar (subsystem id, component id) that \
could collide with YAML's implicit-typing rules — for example a \
component literally called "true", "1.10", or "0123" must appear as \
`"true"`, `"1.10"`, `"0123"`.
"#,
        root = workspace_root.display(),
        soft_cap = soft_cap,
        hard_cap = hard_cap,
    )
}

/// PR-2: production dispatch-components prompt. Same shape as
/// [`build_dispatch_subsystems_prompt`], scoped to a single subsystem's
/// component candidates. The test backend keys canned responses on
/// the `"dispatch components"` substring.
pub fn build_dispatch_components_prompt(
    workspace_root: &Path,
    subsystem: &SubsystemPartition,
    soft_cap: u32,
    hard_cap: u32,
) -> String {
    format!(
        r#"You are Atlas's dispatch components agent. The workspace at {root} \
has been partitioned into subsystems; your job is to enumerate components \
for subsystem `{subsystem_id}` (already-known members: {members:?}).

Use the available manifest-parser tools (parse_cargo_toml, parse_compose, \
parse_dockerfile, parse_package_json) and language classifiers / surface \
analysers to discover any additional components belonging to this \
subsystem. Read each candidate's primary manifest BEFORE assigning it.

Iteration budget: soft cap {soft_cap}; hard cap {hard_cap}.

Emit your final answer as exactly ONE fenced yaml block matching this \
shape:

```yaml
schema_version: 1
candidates_considered:
  - id: "example-component"
    primary_manifest_path: "crates/example/Cargo.toml"
components:
  example-component:
    subsystem: "{subsystem_id}"
    overrides:
      kind: "rust-library"
confidence_grade: "moderate"
```

The `candidates_considered` field MUST list every component candidate \
you inspected with the manifest path you read for it. Lane A's evidence \
floor compares this list against your transcript and clamps your \
self-grade.

`confidence_grade` rubric (decision row 5):
- "strong": every component's primary manifest was read AND classified; \
  the field-override hints (language/kind/lifecycle) are grounded.
- "moderate": most components read; one or two assignments rest on \
  naming heuristics.
- "weak": several assignments rest on naming alone.
- "declines": insufficient evidence — emit a best-guess + this grade.

Quote any identity-shaped scalar (component id, override value) that \
could collide with YAML's implicit-typing rules.
"#,
        root = workspace_root.display(),
        subsystem_id = subsystem.id,
        members = subsystem.members,
        soft_cap = soft_cap,
        hard_cap = hard_cap,
    )
}

/// Parse the LLM dispatch-subsystems agent's output text into
/// `Vec<SubsystemPartition>`. The text is expected to contain exactly
/// one fenced ```yaml block matching the [`SubsystemsOverrideFile`]
/// shape (PR-2 migration: YAML-canonical interchange).
fn parse_subsystems_from_output(text: &str) -> Result<Vec<SubsystemPartition>, AgentError> {
    let yaml_body = extract_yaml_fence(text).map_err(fence_extract_to_agent_error)?;
    let parsed: SubsystemsOverrideFile = serde_yaml::from_str(yaml_body).map_err(|e| {
        AgentError::LlmOutputMalformed(format!(
            "dispatch agent emitted output that did not match \
             SubsystemsOverrideFile shape: {e}; raw yaml body = {yaml_body}"
        ))
    })?;
    // The override path uses a `Path` for diagnostics; the LLM path
    // passes a synthetic `"<dispatch-agent-output>"` so structural-fail
    // messages are still readable.
    subsystems_from_parsed(parsed, Path::new("<dispatch-agent-output>"))
}

/// Parse the LLM dispatch-components agent's output text into
/// `Vec<ComponentPartition>`, filtered to `subsystem`. Same shape as
/// [`parse_subsystems_from_output`].
fn parse_components_from_output(
    text: &str,
    subsystem: &SubsystemPartition,
) -> Result<Vec<ComponentPartition>, AgentError> {
    let yaml_body = extract_yaml_fence(text).map_err(fence_extract_to_agent_error)?;
    let parsed: ComponentsOverrideFile = serde_yaml::from_str(yaml_body).map_err(|e| {
        AgentError::LlmOutputMalformed(format!(
            "dispatch agent emitted output that did not match \
             ComponentsOverrideFile shape: {e}; raw yaml body = {yaml_body}"
        ))
    })?;
    Ok(components_from_parsed(parsed, subsystem))
}

/// Lift a fence-extraction failure to the runtime's `AgentError`
/// shape. PR-2: surfaced as `LlmOutputMalformed` so Lane A's retry
/// path can ask the LLM to re-emit a single fenced ```yaml block.
fn fence_extract_to_agent_error(e: FenceExtractError) -> AgentError {
    AgentError::LlmOutputMalformed(format!("dispatch agent output is not yaml-fenced: {e}"))
}

/// PR-5 helper: parse `subsystems.overrides.yaml` into the raw on-disk
/// shape. Pulled out of the dispatch function so the parse + Lane A
/// pass are testable in isolation.
fn parse_subsystems_yaml(text: &str, path: &Path) -> Result<SubsystemsOverrideFile, AgentError> {
    serde_yaml::from_str(text).map_err(|e| {
        AgentError::OverrideRequired(format!("failed to parse {}: {e}", path.display()))
    })
}

/// PR-5 helper: parse `components.overrides.yaml`.
fn parse_components_yaml(text: &str, path: &Path) -> Result<ComponentsOverrideFile, AgentError> {
    serde_yaml::from_str(text).map_err(|e| {
        AgentError::OverrideRequired(format!("failed to parse {}: {e}", path.display()))
    })
}

/// Project a parsed `SubsystemsOverrideFile` into the runtime's
/// `SubsystemPartition` shape, validating Lane-A-style structural
/// constraints (non-empty unique ids).
fn subsystems_from_parsed(
    parsed: SubsystemsOverrideFile,
    path: &Path,
) -> Result<Vec<SubsystemPartition>, AgentError> {
    let mut ids = BTreeSet::new();
    let mut partitions = Vec::with_capacity(parsed.subsystems.len());
    for entry in parsed.subsystems {
        if entry.id.is_empty() {
            return Err(AgentError::OverrideRequired(format!(
                "{}: subsystem entry with empty id",
                path.display()
            )));
        }
        if !ids.insert(entry.id.clone()) {
            return Err(AgentError::OverrideRequired(format!(
                "{}: duplicate subsystem id `{}`",
                path.display(),
                entry.id
            )));
        }
        partitions.push(SubsystemPartition {
            id: entry.id,
            members: entry.members,
        });
    }
    Ok(partitions)
}

/// Project a parsed `ComponentsOverrideFile` into the runtime's
/// `ComponentPartition` shape, filtered to `subsystem`.
fn components_from_parsed(
    parsed: ComponentsOverrideFile,
    subsystem: &SubsystemPartition,
) -> Vec<ComponentPartition> {
    let members: BTreeSet<&str> = subsystem.members.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for (id, entry) in parsed.components {
        let assigned_by_override = entry
            .subsystem
            .as_deref()
            .map(|s| s == subsystem.id)
            .unwrap_or(false);
        let assigned_by_members = members.contains(id.as_str());
        if !(assigned_by_override || assigned_by_members) {
            continue;
        }
        out.push(ComponentPartition {
            id: id.clone(),
            subsystem_id: subsystem.id.clone(),
            field_overrides: entry.field_overrides,
        });
    }
    out
}

/// SHA-256 of `bytes` as a 32-byte array. Used to compute the
/// `override_content_sha` cache-key contributor.
fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Emit `CacheHit { source: DispatchedFromOverride }` for the
/// shortcircuit dispatch path. The `fingerprint` field carries a
/// dispatch-fingerprint hex string so subscribers (PR-6 TUI,
/// `agent_cache_writer`) can correlate the hit with downstream
/// transcript-cache state.
fn emit_dispatch_shortcircuit_hit(
    runtime: &AgentRuntime,
    stage: Stage,
    target_id: &str,
    override_sha: [u8; 32],
) {
    let fingerprint = dispatch_fingerprint(runtime, stage, target_id, Some(override_sha));
    let agent_id = format!("{}::{}#i0", stage.as_str(), target_id);
    runtime.event_bus.emit(AgentEvent::CacheHit {
        agent_id,
        fingerprint: fingerprint.to_cache_key(),
        replayed_at: now_iso(),
        source: CacheHitSource::DispatchedFromOverride,
    });
}

/// Compute the dispatch agent's fingerprint with the override-content-sha
/// contributor set. Centralised here so the cache-invariant rule is
/// implemented in one place.
///
/// The dispatch agent's per-call fingerprint contributions:
///
/// - `stage_id` — `dispatch_subsystem` or `dispatch_component`.
/// - `agent_id` — `<stage>::<target>#i0` (dispatch is iteration-0).
/// - `target_input_shas` — empty (the override file's content sha is
///   the per-call input, carried under `override_content_sha`).
/// - `iteration_number` — `0` (dispatch is pre-iteration).
/// - `prior_model_sha` — `None`.
/// - `override_content_sha` — `Some(sha)` when an override is present;
///   `None` for the LLM-decided path.
///
/// Pure helper; surfaced `pub(crate)` so tests in
/// `tests/dispatch_shortcircuit.rs` can assert key equality directly
/// without re-deriving the fingerprint shape.
pub fn dispatch_fingerprint(
    runtime: &AgentRuntime,
    stage: Stage,
    target_id: &str,
    override_sha: Option<[u8; 32]>,
) -> AgentInputFingerprint {
    let backend_fp = runtime.backend_router.fingerprint();
    AgentInputFingerprint {
        stage_id: stage.as_str().to_string(),
        agent_id: format!("{}::{}#i0", stage.as_str(), target_id),
        agent_version: "v0".to_string(),
        prompt_template_sha: backend_fp.template_sha,
        tool_catalog_sha: runtime.tools.catalog_sha(),
        model_id: backend_fp.model_id.clone(),
        backend_version: backend_fp.backend_version.clone(),
        transport_flavour: runtime.default_transport.as_str().to_string(),
        target_input_shas: Vec::new(),
        iteration_number: 0,
        prior_model_sha: None,
        override_content_sha: override_sha,
    }
}

/// Write the synthetic transcript-cache entry for an override-shortcircuit
/// dispatch hit. The transcript is the override-yaml bytes framed with
/// `Grade::Strong` so replay sees byte-identical output to an LLM-decided
/// dispatch that emitted `Strong`.
///
/// Cache-invariant rule: the fingerprint here carries
/// `override_content_sha: Some(override_sha)`, so an override removal
/// changes the key shape and the existing synthetic entry becomes
/// unreachable (PR-5 cache-eviction-on-input-drift, recast §6.3).
fn write_synthetic_dispatch_transcript(
    runtime: &AgentRuntime,
    stage: Stage,
    target_id: &str,
    override_sha: [u8; 32],
    override_bytes: &[u8],
    output_bytes: &[u8],
) {
    let fingerprint = dispatch_fingerprint(runtime, stage, target_id, Some(override_sha));
    // Spot-check entry mirrors the override file path so a subsequent
    // probe with a mismatched current_sha_fn evicts the entry
    // (recast §6.3). PR-5's runtime wires `current_sha_fn = |_| None`
    // — the spot-check's absence is documented; the synthetic write
    // primarily exists so PR-6's `--replay-from-cache` can replay
    // dispatch hits without recomputing.
    let _spot_check: Vec<FingerprintInputSpotCheck> = Vec::new();
    let transcript_bytes = frame_transcript_with_grade(&AgentGrade::Strong, override_bytes);
    runtime.cache.write_agent_pair(
        IndexStage::L8,
        &fingerprint,
        &transcript_bytes,
        output_bytes,
    );
}

// PR-7 (PR-5 closeout MEDIUM-3): `now_iso` consolidated to
// `runtime::mod::now_iso` (visible as `pub(super)`). One source of
// truth for the event-timestamp shape.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_tool_catalog;
    use crate::events::EventBus;
    use crate::transport::TransportFlavour;
    use crate::Semaphores;
    use atlas_engine::llm_cache::LlmResponseCache;
    use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};
    use std::sync::Arc;

    /// Test-only `LlmBackend` so the dispatch tests don't drag in the
    /// PR-4 single-iteration smoke fixture's `StagedBackend`.
    struct StubBackend;
    #[async_trait::async_trait]
    impl LlmBackend for StubBackend {
        fn call(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
            Err(LlmError::Invocation("unused".into()))
        }
        async fn call_async(&self, _req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
            Err(LlmError::Invocation("unused".into()))
        }
        fn fingerprint(&self) -> LlmFingerprint {
            LlmFingerprint {
                template_sha: [0u8; 32],
                ontology_sha: [0u8; 32],
                model_id: "stub".to_string(),
                backend_version: "v0".to_string(),
            }
        }
    }

    fn make_runtime() -> AgentRuntime {
        AgentRuntime {
            backend_router: Arc::new(StubBackend),
            tools: Arc::new(default_tool_catalog()),
            cache: Arc::new(LlmResponseCache::new()),
            event_bus: Arc::new(EventBus::new(64)),
            semaphores: Semaphores::defaults(),
            default_transport: TransportFlavour::HttpAnthropic,
            default_max_steps: 4,
            max_iterations: 1,
            for_provider: None,
        }
    }

    fn write_file(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).expect("write fixture");
    }

    #[tokio::test]
    async fn dispatch_subsystems_fires_llm_agent_when_override_missing() {
        // PR-5 follow-up: with no override file, dispatch routes through
        // `call_agent`. The `StubBackend` declines every call, so the
        // result surfaces as `AgentError::Backend` (the lifted LlmError)
        // — *not* `AgentError::OverrideRequired`. This unit-level test
        // asserts the routing happened (we hit the backend) rather than
        // the success path; the integration test in
        // `tests/dispatch_shortcircuit.rs` exercises a wired backend.
        let dir = tempfile::tempdir().unwrap();
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
        assert!(
            !matches!(err, AgentError::OverrideRequired(_)),
            "no override path must NOT short-circuit to OverrideRequired; got {err:?}"
        );
    }

    #[tokio::test]
    async fn dispatch_subsystems_parses_minimal_file() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            SUBSYSTEMS_OVERRIDE_FILENAME,
            "schema_version: 1\nsubsystems:\n  - id: agents\n    members: [foo, bar]\n",
        );
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let partitions = dispatch_subsystems(&runtime, &workspace).await.unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].id, "agents");
        assert_eq!(partitions[0].members, vec!["foo", "bar"]);
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_duplicate_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            SUBSYSTEMS_OVERRIDE_FILENAME,
            "schema_version: 1\nsubsystems:\n  - id: a\n    members: []\n  - id: a\n    members: []\n",
        );
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("duplicate")));
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_empty_id() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            SUBSYSTEMS_OVERRIDE_FILENAME,
            "schema_version: 1\nsubsystems:\n  - id: ''\n    members: []\n",
        );
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("empty id")));
    }

    #[tokio::test]
    async fn dispatch_subsystems_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            SUBSYSTEMS_OVERRIDE_FILENAME,
            "schema_version: 1\nsubsystems:\n  - id: a\n    members: []\n    typo_field: 1\n",
        );
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let err = dispatch_subsystems(&runtime, &workspace).await.unwrap_err();
        assert!(matches!(err, AgentError::OverrideRequired(ref msg) if msg.contains("typo_field")));
    }

    #[tokio::test]
    async fn dispatch_components_filters_to_subsystem_members() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            COMPONENTS_OVERRIDE_FILENAME,
            "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n  bar:\n    subsystem: cli\n",
        );
        let subsystem = SubsystemPartition {
            id: "agents".to_string(),
            members: vec!["foo".to_string()],
        };
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let parts = dispatch_components(&runtime, &workspace, &subsystem)
            .await
            .unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].id, "foo");
        assert_eq!(parts[0].subsystem_id, "agents");
    }

    #[tokio::test]
    async fn dispatch_components_honours_field_overrides() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            COMPONENTS_OVERRIDE_FILENAME,
            "schema_version: 1\ncomponents:\n  foo:\n    subsystem: agents\n    overrides:\n      kind: rust-library\n",
        );
        let subsystem = SubsystemPartition {
            id: "agents".to_string(),
            members: vec!["foo".to_string()],
        };
        let runtime = make_runtime();
        let workspace = Workspace::new(dir.path());
        let parts = dispatch_components(&runtime, &workspace, &subsystem)
            .await
            .unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0].field_overrides.kind.as_deref(),
            Some("rust-library")
        );
    }

    #[test]
    fn dispatch_fingerprint_changes_when_override_added() {
        // Adding an override (None -> Some) must produce a different
        // cache key. This is the PR-5 cache-invariant rule.
        let runtime = make_runtime();
        let no_override =
            dispatch_fingerprint(&runtime, Stage::DispatchSubsystem, "_workspace", None);
        let with_override = dispatch_fingerprint(
            &runtime,
            Stage::DispatchSubsystem,
            "_workspace",
            Some([7u8; 32]),
        );
        assert_ne!(no_override.to_cache_key(), with_override.to_cache_key());
    }

    #[test]
    fn dispatch_fingerprint_changes_when_override_edited() {
        // Editing an override (Some(a) -> Some(b)) must produce a
        // different cache key.
        let runtime = make_runtime();
        let a = dispatch_fingerprint(
            &runtime,
            Stage::DispatchSubsystem,
            "_workspace",
            Some([7u8; 32]),
        );
        let b = dispatch_fingerprint(
            &runtime,
            Stage::DispatchSubsystem,
            "_workspace",
            Some([8u8; 32]),
        );
        assert_ne!(a.to_cache_key(), b.to_cache_key());
    }
}
