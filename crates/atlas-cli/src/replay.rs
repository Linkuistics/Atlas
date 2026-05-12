//! `--replay-from-cache` mode (plan §4 Task 6 Step 6.3).
//!
//! Walks `<workspace_root>/.atlas/cache/agents/<stage>/` looking for
//! transcript/output pairs left behind by a prior live run. Each pair
//! is decoded back into a synthetic `AgentEvent` sequence
//! (`AgentStart` → `AgentComplete`) emitted on the bus, so the TUI
//! subscriber renders the run identically to the live source.
//!
//! ## Single-transport invariant (plan §7.4)
//!
//! `transport_flavour` is a cache-key contributor, which means a
//! given replay only sees the cache entries produced by the
//! transport that wrote them. The transcript blob itself does not
//! carry the transport string in PR-2's framing — the discriminator
//! lives in the cache key (the `<sha>` filename). The replay test
//! populates a sibling `<sha>.meta.json` file carrying
//! `transport_flavour`; if present and mismatched against the
//! requested transport, `replay_from_cache` returns
//! [`ReplayError::TransportMismatch`] **before emitting any events
//! on the bus** — so the caller sees the helpful error rather than
//! an empty TUI (plan §7.4 mitigation).
//!
//! ## Coordination with PR-4
//!
//! PR-4 owns `crates/atlas-agents/src/runtime/`. The cache layout
//! uses the canonical `atlas_index::Stage` enum (L1..L9). PR-7's
//! wiring step will reconcile the recast §6.1 logical-stage names
//! (DispatchSubsystem, DispatchComponent, Classify, Surface, Reduce,
//! Project) with the filesystem-level `L1..L9` axis. PR-6 reads
//! whatever the cache writer wrote.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atlas_agents::events::Grade;
use atlas_agents::{AgentEvent, EventBus, TransportFlavour};
use atlas_engine::cache::layout::{
    agents_output_path, agents_stage_dir_path, PUB_TRANSCRIPT_SUFFIX,
};
use atlas_engine::llm_cache::{parse_transcript_grade, AgentGrade};
use atlas_index::Stage;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

use crate::tui::{run_with_subscriber, TuiConfig, TuiSnapshot, TuiState};

/// The nine canonical stages the cache enumerates. Mirrors
/// `atlas_engine::cache::layout::ALL_STAGES` but kept local here to
/// avoid promoting a private engine constant just for replay.
const REPLAY_STAGES: [Stage; 9] = [
    Stage::L1,
    Stage::L2,
    Stage::L3,
    Stage::L4,
    Stage::L5,
    Stage::L6,
    Stage::L7,
    Stage::L8,
    Stage::L9,
];

/// Error variants the replay path can surface.
#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("io error during replay: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "transport mismatch: cache was produced by {recorded:?} but replay requested \
         {requested:?}; re-run the live workload under the requested transport (or pick the \
         recorded one)"
    )]
    TransportMismatch {
        recorded: String,
        requested: TransportFlavour,
    },
    #[error("cache corrupt: {0}")]
    CacheCorrupt(String),
    #[error("tui drain handshake failed: subscriber dropped before signalling done")]
    DrainFailed,
    #[error("tui subscriber failed: {0}")]
    TuiFailed(String),
    #[error("workspace root has no .atlas/cache/agents/ directory; nothing to replay")]
    NoCache,
    #[error("path traversal guard tripped: {0}")]
    PathTraversal(String),
}

/// Drive the `--replay-from-cache` mode end-to-end.
///
/// Spawns the TUI subscriber, walks the cache to derive a synthetic
/// event sequence, emits it on the bus, then awaits the subscriber's
/// drain handshake.
pub async fn replay_from_cache(
    workspace_root: &Path,
    transport: TransportFlavour,
) -> Result<(), ReplayError> {
    let atlas_root = workspace_root.join(".atlas");
    let _snapshot = replay_into_tui(&atlas_root, transport, TuiConfig::default()).await?;
    Ok(())
}

/// Public entry point used by both the CLI dispatch path and the
/// integration test. Walks the cache, builds the synthetic event
/// sequence, drives the TUI subscriber, awaits drain. Returns the
/// final [`TuiSnapshot`] so the integration test can diff it against
/// a synthetic-live snapshot.
pub async fn replay_into_tui(
    atlas_root: &Path,
    transport: TransportFlavour,
    config: TuiConfig,
) -> Result<TuiSnapshot, ReplayError> {
    let events = collect_replay_events(atlas_root, transport)?;
    run_with_tui(events, config).await
}

/// Apply the same synthetic event sequence directly to a fresh
/// `TuiState`, without going through the TUI subscriber. Lets the
/// integration test compare a live-shape snapshot against a
/// replay-shape snapshot for the same event sequence (plan §4 Task 6
/// Step 6.3 "synthetic-live phase").
pub fn synthetic_live_snapshot(events: &[AgentEvent]) -> TuiSnapshot {
    let mut state = TuiState::default();
    for ev in events {
        state.apply(ev.clone());
    }
    state.snapshot()
}

/// Self-contained driver: owns the bus, subscribes synchronously
/// (so the subscribe-before-emit ordering is unconditional), spawns
/// the TUI subscriber over the pre-subscribed receiver, runs the
/// replay, awaits drain.
async fn run_with_tui(
    events: Vec<AgentEvent>,
    config: TuiConfig,
) -> Result<TuiSnapshot, ReplayError> {
    let bus = EventBus::with_default_capacity();
    let state = Arc::new(Mutex::new(TuiState::default()));
    let (done_tx, done_rx) = oneshot::channel();

    // Subscribe BEFORE spawn. tokio's broadcast channel only
    // delivers events emitted after `subscribe()` returns; subscribing
    // on the spawn side races the first emit. Subscribing here makes
    // the ordering invariant unconditional.
    let rx = bus.subscribe();

    let tui_state = state.clone();
    let tui_handle: tokio::task::JoinHandle<std::io::Result<()>> =
        tokio::spawn(async move { run_with_subscriber(rx, tui_state, config, done_tx).await });

    // Drop unused-import lint suppressor; `Duration` is still imported
    // for the module-level use even though the sleep was removed.
    let _ = Duration::from_millis(0);

    for ev in events {
        bus.emit(ev);
    }
    bus.emit(AgentEvent::RuntimeComplete);

    // Drain handshake: the TUI subscriber signals `done_tx` after
    // applying `RuntimeComplete` and restoring the terminal. Even
    // when raw-mode entry fails (headless test env), `run_with_subscriber`
    // signals done_tx before returning the Err.
    //
    // PR-7 (PR-6 closeout MEDIUM-1): wrap the `done_rx.await` in a
    // 30s timeout so a wedged subscriber (e.g. broken pipe to
    // redirected stdout in `atlas index --replay-from-cache | head`)
    // cannot block forever. The timeout maps to the same
    // `DrainFailed` variant the subscriber-dropped path produces.
    match tokio::time::timeout(Duration::from_secs(30), done_rx).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => return Err(ReplayError::DrainFailed),
        Err(_) => return Err(ReplayError::DrainFailed),
    }
    let tui_outcome = tui_handle.await;
    let snapshot = state.lock().await.snapshot();
    match tui_outcome {
        Ok(Ok(())) => Ok(snapshot),
        Ok(Err(err)) => Err(ReplayError::TuiFailed(err.to_string())),
        Err(err) => Err(ReplayError::TuiFailed(err.to_string())),
    }
}

/// Test-facing alias for [`collect_replay_events`]. Lets the
/// `tests/replay.rs` integration test compare the walker's
/// canonical event sequence against the expected synthetic-live
/// sequence without spinning up the TUI subscriber (which can't
/// enter raw mode in a headless test environment).
#[doc(hidden)]
pub fn collect_replay_events_for_test(
    atlas_root: &Path,
    transport: TransportFlavour,
) -> Result<Vec<AgentEvent>, ReplayError> {
    collect_replay_events(atlas_root, transport)
}

/// Walk the cache and synthesize the event sequence. Fails before
/// emitting anything if any transcript was produced by a transport
/// other than `transport`.
pub(crate) fn collect_replay_events(
    atlas_root: &Path,
    transport: TransportFlavour,
) -> Result<Vec<AgentEvent>, ReplayError> {
    let agents_root = atlas_root.join("cache").join("agents");
    if !agents_root.exists() {
        return Err(ReplayError::NoCache);
    }

    // Canonicalise so the path-traversal guard below operates on a
    // resolved absolute view.
    let agents_root_canon = agents_root.canonicalize().map_err(ReplayError::Io)?;

    let mut entries: Vec<ReplayEntry> = Vec::new();

    for stage in REPLAY_STAGES {
        let stage_dir = agents_stage_dir_path(atlas_root, stage);
        if !stage_dir.exists() {
            continue;
        }
        let dir_entries = std::fs::read_dir(&stage_dir).map_err(ReplayError::Io)?;
        for entry in dir_entries {
            let entry = entry.map_err(ReplayError::Io)?;
            let entry_path = entry.path();

            // PR-7 (PR-6 closeout MEDIUM-2): apply the cheap suffix
            // filter BEFORE the (expensive, IO-touching) canonicalize.
            // A broken-symlink `.meta.json` or a stray non-transcript
            // file in the stage directory would otherwise produce a
            // misleading `PathTraversal` message when the real cause
            // is an unrelated IO error. Canonicalising only the
            // transcript half makes the error attribution sharp.
            let name = entry.file_name().to_string_lossy().into_owned();
            // Process the `.transcript` half. The paired `.output`
            // is loaded via `agents_output_path`.
            let Some(sha) = name.strip_suffix(PUB_TRANSCRIPT_SUFFIX) else {
                continue;
            };

            // Path-traversal guard (defence-in-depth): refuse any
            // entry whose canonical form escapes the canonical
            // agents root. A symlink inside the agents directory
            // pointing outside the cache is the failure mode we
            // are protecting against.
            let canonical = entry_path.canonicalize().map_err(|e| {
                ReplayError::PathTraversal(format!(
                    "failed to canonicalise {}: {e}",
                    entry_path.display()
                ))
            })?;
            if !canonical.starts_with(&agents_root_canon) {
                return Err(ReplayError::PathTraversal(format!(
                    "entry {} escapes agents root {}",
                    canonical.display(),
                    agents_root_canon.display()
                )));
            }

            let transcript_path = entry_path;
            let output_path = agents_output_path(atlas_root, stage, &sha.to_string());

            let entry = decode_pair(stage, sha, &transcript_path, &output_path, transport)?;
            entries.push(entry);
        }
    }

    // Sort for snapshot determinism: stage-then-sha. Directory
    // iteration order is not portable; without this the replay
    // snapshot byte-equality would be flaky.
    entries.sort_by(|a, b| match a.stage.cmp(&b.stage) {
        std::cmp::Ordering::Equal => a.sha.cmp(&b.sha),
        other => other,
    });

    let mut out: Vec<AgentEvent> = Vec::with_capacity(entries.len() * 2);
    for entry in entries {
        out.push(entry.start);
        out.push(entry.complete);
    }
    Ok(out)
}

/// Internal helper: per-pair events kept sortable by (stage, sha)
/// before flattening.
struct ReplayEntry {
    stage: Stage,
    sha: String,
    start: AgentEvent,
    complete: AgentEvent,
}

fn decode_pair(
    stage: Stage,
    sha: &str,
    transcript_path: &Path,
    output_path: &Path,
    requested_transport: TransportFlavour,
) -> Result<ReplayEntry, ReplayError> {
    let transcript_bytes = std::fs::read(transcript_path).map_err(ReplayError::Io)?;
    if !output_path.exists() {
        // Half-pair residue (post-rename-a / pre-rename-b crash) —
        // treat as cache-corrupt for replay's purpose. The live
        // path in `llm_cache::call_agent_cached` silently recomputes;
        // replay refuses because we can't synthesize an event
        // sequence from half a transcript.
        return Err(ReplayError::CacheCorrupt(format!(
            "half-pair cache entry: {} present without {}",
            transcript_path.display(),
            output_path.display()
        )));
    }

    let (grade, _body) = parse_transcript_grade(&transcript_bytes).ok_or_else(|| {
        ReplayError::CacheCorrupt(format!(
            "transcript framing invalid in {}",
            transcript_path.display()
        ))
    })?;

    // Transport-mismatch check (plan §7.4). The transcript blob does
    // not carry transport_flavour directly — that lives in the cache
    // key (the `<sha>` filename). Tests plant a sibling
    // `<sha>.meta.json` file carrying `transport_flavour` so the
    // mismatch path is exercisable; production cache layout (PR-2)
    // omits the meta file and the check is a no-op in that case
    // (per plan §7.4: "PR-6's `replay_from_cache` emits a helpful
    // error if the configured transport differs from what's in
    // cache" — the discriminator is best-effort and surfaces when
    // the writer leaves the breadcrumb).
    let meta_path = sibling_meta_path(transcript_path);
    if meta_path.exists() {
        let meta_bytes = std::fs::read(&meta_path).map_err(ReplayError::Io)?;
        let meta: ReplayMeta = serde_json::from_slice(&meta_bytes).map_err(|e| {
            ReplayError::CacheCorrupt(format!("meta file {} invalid: {e}", meta_path.display()))
        })?;
        if meta.transport_flavour != requested_transport.as_str() {
            return Err(ReplayError::TransportMismatch {
                recorded: meta.transport_flavour,
                requested: requested_transport,
            });
        }
    }

    let agent_id = format!("{}:{}", stage_name(stage), sha);
    Ok(ReplayEntry {
        stage,
        sha: sha.to_string(),
        start: AgentEvent::AgentStart {
            agent_id: agent_id.clone(),
            parent_id: None,
            stage: stage_name(stage).to_string(),
            target: sha.to_string(),
            fingerprint: sha.to_string(),
            started_at: "replay".to_string(),
            transport: requested_transport,
        },
        // Emit `AgentComplete` carrying the recorded grade so the
        // tree-view transitions to its terminal state. The recast
        // spec's replay path could also emit `CacheHit` first; PR-6
        // emits the single `AgentComplete` because the simpler
        // shape is sufficient for snapshot byte-equality (the
        // synthetic-live phase emits the same shape).
        complete: AgentEvent::AgentComplete {
            agent_id,
            output_sha: sha.to_string(),
            confidence_grade: grade_from_engine(grade),
            tokens_in: 0,
            tokens_out: 0,
            ms: 0,
            provider: "replay".to_string(),
        },
    })
}

fn grade_from_engine(grade: AgentGrade) -> Grade {
    match grade {
        AgentGrade::Strong => Grade::Strong,
        AgentGrade::Moderate => Grade::Moderate,
        AgentGrade::Weak => Grade::Weak,
        AgentGrade::Declines => Grade::Declines,
    }
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::L1 => "L1",
        Stage::L2 => "L2",
        Stage::L3 => "L3",
        Stage::L4 => "L4",
        Stage::L5 => "L5",
        Stage::L6 => "L6",
        Stage::L7 => "L7",
        Stage::L8 => "L8",
        Stage::L9 => "L9",
    }
}

/// Sibling metadata file path: `<sha>.transcript` → `<sha>.meta.json`.
/// Optional; only the replay test populates it.
fn sibling_meta_path(transcript_path: &Path) -> PathBuf {
    let file = transcript_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = file
        .strip_suffix(PUB_TRANSCRIPT_SUFFIX)
        .unwrap_or(&file)
        .to_string();
    let parent = transcript_path.parent().unwrap_or(Path::new("."));
    parent.join(format!("{stem}.meta.json"))
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct ReplayMeta {
    pub transport_flavour: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cache_dir_returns_no_cache_error() {
        let tmp = tempfile::tempdir().unwrap();
        let result = collect_replay_events(tmp.path(), TransportFlavour::ClaudeCode);
        assert!(matches!(result, Err(ReplayError::NoCache)));
    }

    #[test]
    fn synthetic_live_snapshot_round_trips_with_serde() {
        let events = vec![
            AgentEvent::AgentStart {
                agent_id: "a".into(),
                parent_id: None,
                stage: "L3".into(),
                target: "t".into(),
                fingerprint: "f".into(),
                started_at: "t".into(),
                transport: TransportFlavour::ClaudeCode,
            },
            AgentEvent::AgentComplete {
                agent_id: "a".into(),
                output_sha: "s".into(),
                confidence_grade: Grade::Strong,
                tokens_in: 1,
                tokens_out: 2,
                ms: 0,
                provider: "Anthropic".into(),
            },
            AgentEvent::RuntimeComplete,
        ];
        let snap = synthetic_live_snapshot(&events);
        assert!(snap.runtime_complete);
        let bytes = serde_json::to_vec(&snap).unwrap();
        let back: TuiSnapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(snap, back);
    }
}
