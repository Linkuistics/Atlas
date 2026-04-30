//! Progress events emitted by atlas-engine.
//!
//! `atlas-engine` owns the event vocabulary; rendering is the CLI's job.
//! See `docs/superpowers/specs/2026-05-01-engine-progress-events-design.md` §5.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_index::ComponentEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Seed,
    Fixedpoint,
    Project,
    Edges,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptBreakdown {
    pub classify: u64,
    pub surface: u64,
    pub edges: u64,
    pub subcarve: u64,
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Started {
        root: PathBuf,
    },
    Phase(Phase),
    IterStart {
        iteration: u32,
        live_components: u64,
    },
    Subcarve {
        component_id: String,
        relpath: PathBuf,
        k: u64,
        n: u64,
    },
    IterEnd {
        iteration: u32,
        components_added: u64,
        elapsed: Duration,
    },
    Surface {
        component_id: String,
        relpath: PathBuf,
        k: u64,
        n: u64,
    },
    Finished {
        components: u64,
        llm_calls: u64,
        tokens_used: u64,
        token_budget: Option<u64>,
        elapsed: Duration,
        breakdown: PromptBreakdown,
    },
}

pub trait ProgressSink: Send + Sync {
    fn on_event(&self, event: ProgressEvent);
}

/// Derive the renderer-facing relpath for a component. Returns the
/// deepest `path_segments` entry, or an empty `PathBuf` if there are
/// none. See spec §5.1.
pub fn relpath_of(c: &ComponentEntry) -> PathBuf {
    c.path_segments
        .last()
        .map(|s| s.path.clone())
        .unwrap_or_default()
}

/// Test helper: records every event into an inner `Vec`.
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<ProgressEvent>>,
}

impl RecordingSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> Vec<ProgressEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl ProgressSink for RecordingSink {
    fn on_event(&self, event: ProgressEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_captures_events_in_order() {
        let sink = RecordingSink::new();
        let dyn_sink: Arc<dyn ProgressSink> = sink.clone();
        dyn_sink.on_event(ProgressEvent::Phase(Phase::Seed));
        dyn_sink.on_event(ProgressEvent::Phase(Phase::Project));

        let events = sink.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], ProgressEvent::Phase(Phase::Seed)));
        assert!(matches!(events[1], ProgressEvent::Phase(Phase::Project)));
    }

    #[test]
    fn relpath_of_returns_last_segment_path() {
        use atlas_index::{EvidenceGrade, PathSegment};
        let entry = ComponentEntry {
            id: "c".into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![
                PathSegment {
                    path: PathBuf::from("crates"),
                    content_sha: "a".into(),
                },
                PathSegment {
                    path: PathBuf::from("crates/atlas-engine"),
                    content_sha: "b".into(),
                },
            ],
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: String::new(),
            deleted: false,
        };
        assert_eq!(relpath_of(&entry), PathBuf::from("crates/atlas-engine"));
    }

    #[test]
    fn relpath_of_returns_empty_when_no_segments() {
        use atlas_index::EvidenceGrade;
        let entry = ComponentEntry {
            id: "c".into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: Vec::new(),
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: String::new(),
            deleted: false,
        };
        assert_eq!(relpath_of(&entry), PathBuf::new());
    }
}
