//! Fixedpoint driver for the L8 back-edge.
//!
//! On each iteration:
//!
//! 1. Demand [`all_components`] on the current inputs.
//! 2. For every live component, call
//!    [`crate::l8_recurse::subcarve_decision`]. Its sub-dirs — if any —
//!    are merged into an accumulating back-edge map keyed by parent id.
//! 3. If the merge added at least one new `(id, sub_dir)` pair, stamp
//!    the map onto `workspace.carve_back_edge` and loop. If nothing
//!    changed, exit: the fixedpoint has converged.
//! 4. Abort with a descriptive panic at
//!    [`FIXEDPOINT_HARD_CAP`] iterations — the design-doc §8.2 ceiling.
//!
//! Iteration count is exposed via
//! [`AtlasDatabase::fixedpoint_iteration_count`] so the CLI and the
//! evaluation harness can report "converged in N rounds".
//!
//! ## Why monotonic growth
//!
//! The merge adds sub-dirs without ever removing them. A well-behaved
//! backend returns the same sub_dirs for identical inputs (the LLM
//! cache guarantees this), so a converged iteration adds nothing and
//! the loop exits. A pathological backend that keeps proposing novel
//! sub-dirs grows the map until the hard cap fires — that's the
//! intended failure mode, not a correctness hole.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::db::{AtlasDatabase, DEFAULT_MAX_DEPTH};
use crate::l4_tree::all_components;
use crate::l8_recurse::subcarve_decision;

/// Hard cap on fixedpoint iterations. Design §8.2 — "Atlas must
/// converge within 8 rounds on every input; a run that needs more is
/// prima facie evidence of a pathological classifier."
pub const FIXEDPOINT_HARD_CAP: u32 = 8;

/// Configuration for one driver run.
#[derive(Clone)]
pub struct FixedpointConfig {
    /// Passed through to [`AtlasDatabase::set_max_depth`]. Controls how
    /// deep L8 will recurse regardless of signal strength.
    pub max_depth: u32,
    /// Fail-loud threshold. Overridable for tests that deliberately
    /// provoke divergence; defaults to [`FIXEDPOINT_HARD_CAP`].
    pub hard_cap: u32,
    /// Optional progress sink. When `None`, the driver runs silently
    /// (current behaviour — preserved for engine tests and the harness).
    pub progress: Option<std::sync::Arc<dyn crate::progress::ProgressSink>>,
}

impl std::fmt::Debug for FixedpointConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixedpointConfig")
            .field("max_depth", &self.max_depth)
            .field("hard_cap", &self.hard_cap)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl Default for FixedpointConfig {
    fn default() -> Self {
        FixedpointConfig {
            max_depth: DEFAULT_MAX_DEPTH,
            hard_cap: FIXEDPOINT_HARD_CAP,
            progress: None,
        }
    }
}

/// Outcome of a driver run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedpointResult {
    pub iterations: u32,
    pub back_edge: BTreeMap<String, Vec<PathBuf>>,
}

/// Drive the engine to a fixedpoint on the L8 back-edge. Panics if the
/// hard cap fires — matching the design's "abort loudly" stance.
pub fn run_fixedpoint(db: &mut AtlasDatabase, config: FixedpointConfig) -> FixedpointResult {
    let sink = config.progress.clone();
    db.set_max_depth(config.max_depth);
    db.set_fixedpoint_iteration_count(0);

    let mut back_edge: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    db.set_carve_back_edge(back_edge.clone());

    let mut iterations = 0u32;
    loop {
        let iter_started = std::time::Instant::now();
        let components = all_components(db);
        let live: Vec<(String, PathBuf)> = components
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| (c.id.clone(), crate::progress::relpath_of(c)))
            .collect();
        drop(components);

        if let Some(s) = sink.as_ref() {
            s.on_event(crate::progress::ProgressEvent::IterStart {
                iteration: iterations,
                live_components: live.len() as u64,
            });
        }

        let n = live.len() as u64;
        let mut added = 0u64;
        let mut changed = false;
        for (k, (id, relpath)) in live.iter().enumerate() {
            if let Some(s) = sink.as_ref() {
                s.on_event(crate::progress::ProgressEvent::Subcarve {
                    component_id: id.clone(),
                    relpath: relpath.clone(),
                    k: (k as u64) + 1,
                    n,
                });
            }
            let decision = subcarve_decision(db, id.clone());
            if !decision.should_subcarve || decision.sub_dirs.is_empty() {
                continue;
            }
            let entry = back_edge.entry(id.clone()).or_default();
            for sub in decision.sub_dirs {
                if !entry.iter().any(|existing| existing == &sub) {
                    entry.push(sub);
                    added += 1;
                    changed = true;
                }
            }
        }

        if let Some(s) = sink.as_ref() {
            s.on_event(crate::progress::ProgressEvent::IterEnd {
                iteration: iterations,
                components_added: added,
                elapsed: iter_started.elapsed(),
            });
        }

        if !changed {
            return FixedpointResult {
                iterations,
                back_edge,
            };
        }

        iterations = iterations.saturating_add(1);
        db.set_fixedpoint_iteration_count(iterations);
        db.set_carve_back_edge(back_edge.clone());

        if iterations >= config.hard_cap {
            panic!(
                "Atlas fixedpoint did not converge after {iterations} iterations \
                 (hard cap {cap}). {n} components still have growing carve plans. \
                 This is prima facie evidence of a pathological classifier that \
                 keeps proposing new sub-carves; audit the backend or widen the \
                 hard_cap deliberately if you have a justifying input.",
                iterations = iterations,
                cap = config.hard_cap,
                n = back_edge.len(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AtlasDatabase;
    use crate::ingest::seed_filesystem;
    use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TestBackend};
    use serde_json::{json, Value};
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [9u8; 32],
            ontology_sha: [1u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn write_cli_crate(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main(){}\n").unwrap();
    }

    #[test]
    fn converges_immediately_when_no_component_can_subcarve() {
        // A RustCli short-circuits at policy — no LLM, no back-edge.
        // The driver should return after zero iterations.
        let tmp = TempDir::new().unwrap();
        write_cli_crate(tmp.path(), "cli");
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let result = run_fixedpoint(&mut db, FixedpointConfig::default());
        assert_eq!(result.iterations, 0);
        assert!(result.back_edge.is_empty());
        assert_eq!(db.fixedpoint_iteration_count(), 0);
    }

    /// Pathological backend: every call returns a fresh, never-before-
    /// seen sub_dir. Its only purpose is to exercise the hard-cap panic
    /// path.
    struct PathologicalBackend {
        counter: AtomicU32,
        fingerprint: LlmFingerprint,
    }

    impl PathologicalBackend {
        fn new() -> Self {
            PathologicalBackend {
                counter: AtomicU32::new(0),
                fingerprint: fingerprint(),
            }
        }
    }

    impl LlmBackend for PathologicalBackend {
        fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
            match req.prompt_template {
                PromptId::Subcarve => {
                    let n = self.counter.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({
                        "should_subcarve": true,
                        "sub_dirs": [format!("src/novel-{n}")],
                        "rationale": "pathological",
                    }))
                }
                // Any other prompt (e.g. Stage1/Stage2 triggered by
                // L5/L6 via L7's modularity_hint → edge_graph) returns
                // empty-shaped output so we don't panic incidentally.
                PromptId::Stage1Surface => Ok(json!({ "purpose": "p" })),
                PromptId::Stage2Edges => Ok(Value::Array(Vec::new())),
                PromptId::Classify => Ok(json!({
                    "kind": "rust-library",
                    "rationale": "r",
                })),
            }
        }

        fn fingerprint(&self) -> LlmFingerprint {
            self.fingerprint.clone()
        }
    }

    fn write_lib_crate(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
    }

    #[test]
    #[should_panic(expected = "did not converge")]
    fn pathological_backend_panics_at_hard_cap() {
        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "lib");
        let backend: Arc<dyn LlmBackend> = Arc::new(PathologicalBackend::new());
        let mut db = AtlasDatabase::new(backend, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let _ = run_fixedpoint(
            &mut db,
            FixedpointConfig {
                max_depth: 8,
                hard_cap: 3,
                ..FixedpointConfig::default()
            },
        );
    }

    #[test]
    fn iteration_counter_reflects_number_of_productive_rounds() {
        // A library with one stable LLM response → one productive round
        // (adds the sub_dir), then a second round that re-confirms the
        // same sub_dir and terminates. Final counter: 1.
        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "lib");
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        // Canning a response for every input shape the driver will
        // probe is impractical — instead, stub the response shape
        // directly against whatever inputs the first call builds.
        // We use a loop: drive one iteration manually via should_subcarve
        // to capture the inputs, then register the canned response.
        //
        // But capturing inputs requires accessing build_subcarve_inputs
        // — which is private. Register for all components.
        let live_id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let entry = all_components(&db)
            .iter()
            .find(|c| c.id == live_id)
            .unwrap()
            .clone();

        let signals_first_round = crate::subcarve_policy::SubcarveSignals {
            kind: crate::types::ComponentKind::RustLibrary,
            current_depth: 0,
            max_depth: 4, // MUST match the FixedpointConfig.max_depth below
            seam_density: 0.0,
            modularity_hint: None,
            cliques_touching: Vec::new(),
            pin_suppressed_children: Vec::new(),
        };
        let inputs =
            crate::l8_recurse::build_subcarve_inputs_for_tests(&entry, &signals_first_round);
        backend.respond(
            PromptId::Subcarve,
            inputs,
            json!({
                "should_subcarve": true,
                "sub_dirs": ["src/sub-a"],
                "rationale": "split",
            }),
        );

        // Run the driver. After the first iteration, workspace.carve_back_edge
        // picks up `src/sub-a`; L2 re-walks and finds no manifests there
        // (it's an empty dir), so L4 does not add a new component. The
        // second pass therefore re-confirms the same decision → no
        // growth → terminates.
        let result = run_fixedpoint(
            &mut db,
            FixedpointConfig {
                max_depth: 4,
                hard_cap: 8,
                ..FixedpointConfig::default()
            },
        );
        assert!(
            result.iterations >= 1,
            "expected at least one productive iteration"
        );
        assert!(
            result.back_edge.contains_key(&live_id),
            "library should have a carve plan, got {:?}",
            result.back_edge
        );
        assert_eq!(db.fixedpoint_iteration_count(), result.iterations);
    }

    #[test]
    fn engine_emits_iter_start_subcarve_iter_end_in_order() {
        use crate::progress::{ProgressEvent, RecordingSink};

        let tmp = TempDir::new().unwrap();
        write_cli_crate(tmp.path(), "cli");
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let sink = RecordingSink::new();
        let cfg = FixedpointConfig {
            progress: Some(sink.clone() as Arc<dyn crate::progress::ProgressSink>),
            ..FixedpointConfig::default()
        };
        let _ = run_fixedpoint(&mut db, cfg);

        let events = sink.snapshot();
        // Expect IterStart{0, 1}, Subcarve{_, 1, 1}, IterEnd{0, 0, _}.
        assert!(matches!(
            events[0],
            ProgressEvent::IterStart {
                iteration: 0,
                live_components: 1
            }
        ));
        assert!(matches!(
            events[1],
            ProgressEvent::Subcarve { k: 1, n: 1, .. }
        ));
        assert!(matches!(
            events[2],
            ProgressEvent::IterEnd {
                iteration: 0,
                components_added: 0,
                ..
            }
        ));
    }

    #[test]
    fn engine_emits_subcarve_event_before_calling_decision() {
        // Proves the spec §7.2 ordering: Subcarve is emitted *before* the
        // LLM call is in flight, so the bar shows the in-progress target.
        // The sink records the live cache call_count when the event lands;
        // because Subcarve precedes the corresponding LLM round-trip, the
        // counter is unchanged at emission time.
        use crate::progress::{ProgressEvent, ProgressSink};
        use std::sync::Mutex;

        struct OrderingSink {
            observations: Mutex<Vec<(String, u64)>>,
            cache_calls: Arc<dyn Fn() -> u64 + Send + Sync>,
        }
        impl ProgressSink for OrderingSink {
            fn on_event(&self, event: ProgressEvent) {
                if let ProgressEvent::Subcarve { component_id, .. } = event {
                    self.observations
                        .lock()
                        .unwrap()
                        .push((component_id, (self.cache_calls)()));
                }
            }
        }

        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "lib");
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let cache = db.llm_cache().clone();
        let cache_calls: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new(move || cache.call_count());
        let sink = Arc::new(OrderingSink {
            observations: Mutex::new(Vec::new()),
            cache_calls,
        });
        let cfg = FixedpointConfig {
            progress: Some(sink.clone() as Arc<dyn crate::progress::ProgressSink>),
            ..FixedpointConfig::default()
        };
        let _ = run_fixedpoint(&mut db, cfg);
        let observed = sink.observations.lock().unwrap().clone();
        assert!(!observed.is_empty(), "expected at least one Subcarve event");
        assert_eq!(
            observed[0].1, 0,
            "Subcarve event must precede subcarve_decision LLM call"
        );
    }

    #[test]
    fn pathological_backend_emits_iter_start_for_each_iteration_until_cap() {
        use crate::progress::{ProgressEvent, RecordingSink};

        let tmp = TempDir::new().unwrap();
        write_lib_crate(tmp.path(), "lib");
        let backend: Arc<dyn LlmBackend> = Arc::new(PathologicalBackend::new());
        let mut db = AtlasDatabase::new(backend, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();

        let sink = RecordingSink::new();
        let cfg = FixedpointConfig {
            max_depth: 8,
            hard_cap: 3,
            progress: Some(sink.clone() as Arc<dyn crate::progress::ProgressSink>),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_fixedpoint(&mut db, cfg)
        }));
        assert!(result.is_err(), "expected hard-cap panic");

        let events = sink.snapshot();
        let iter_starts: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                ProgressEvent::IterStart { iteration, .. } => Some(*iteration),
                _ => None,
            })
            .collect();
        // Hard cap is 3. The driver fires IterStart for iterations 0, 1, 2
        // (the panic occurs after the third merge step).
        assert_eq!(iter_starts, vec![0, 1, 2]);
    }
}
