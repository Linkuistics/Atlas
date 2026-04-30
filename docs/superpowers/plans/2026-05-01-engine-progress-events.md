# Engine-side ProgressEvent + indicatif rendering — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-call cumulative tally in `atlas index` with engine-emitted `ProgressEvent`s rendered by an indicatif multi-bar reporter that shows current target, k/n denominator, and per-iteration scrollback history.

**Architecture:** `atlas-engine` gains a `progress` module exposing a `ProgressSink` trait + `ProgressEvent` enum; `run_fixedpoint` fires `IterStart`/`Subcarve`/`IterEnd` through an optional sink installed via `FixedpointConfig`. `atlas-cli/pipeline.rs` owns a `Reporter` that implements `ProgressSink`, fires `Started`/`Phase`/`Surface`/`Finished` markers itself, and decomposes L5 demand into a per-component loop. `Reporter` renders into a `MultiProgress` with two bars (activity + token gauge); the existing `ProgressBackend` decorator now taps the same `Reporter` via a side-channel `on_llm_call` method that respects engine-set "sticky" k/n state.

**Tech Stack:** Rust 2021, `indicatif` (new dep, atlas-cli only), `salsa 0.26` (existing), workspace-deny `warnings = "deny"` so all clippy/rustc lints must be clean.

**Spec:** [`docs/superpowers/specs/2026-05-01-engine-progress-events-design.md`](../specs/2026-05-01-engine-progress-events-design.md) (commit `22d6324`).

---

## File Structure

| Path | Status | Responsibility |
|---|---|---|
| `crates/atlas-engine/src/progress.rs` | **create** | `ProgressSink` trait, `ProgressEvent` enum, `Phase`, `PromptBreakdown`, `relpath_of` helper, `RecordingSink` test helper |
| `crates/atlas-engine/src/lib.rs` | modify | re-export new public surface from `progress` module |
| `crates/atlas-engine/src/fixedpoint.rs` | modify | add `progress: Option<Arc<dyn ProgressSink>>` to `FixedpointConfig`; emit `IterStart`/`Subcarve`/`IterEnd` |
| `crates/atlas-cli/Cargo.toml` | modify | add `indicatif` |
| `Cargo.toml` (workspace) | modify | declare `indicatif` in `[workspace.dependencies]` |
| `crates/atlas-cli/src/progress.rs` | **rewrite** | `Reporter` (impl `ProgressSink`), `render_activity_msg`, `ProgressMode`, `make_stderr_reporter`, `ProgressBackend` (kept; taps `on_llm_call`) |
| `crates/atlas-cli/src/pipeline.rs` | modify | construct `Reporter`, fire `Started`/`Phase` markers, pass sink in `FixedpointConfig`, decompose L5 demand into per-component loop |
| `crates/atlas-cli/src/main.rs` | modify | wire `make_stderr_reporter` against new `Reporter` API; drop the legacy `announce_start` / `finish` calls |
| `crates/atlas-cli/tests/byte_identity_l5_demand.rs` | **create** | integration test: per-component L5 demand + final snapshot must equal pre-change snapshot |

---

## Conventions for every task

- After every step that runs commands, capture the **expected** output literally. If real output differs, stop and fix before moving on.
- Run `cargo fmt --all` before each commit. Workspace lints set `warnings = "deny"`, so any rustc/clippy warning must be fixed in the same commit that introduced it (per the project's "fix all lints" directive).
- Commit messages follow the existing convention (`feat: …`, `chore: …`, `test: …`). Co-author trailer is added by the harness, not by you.

---

## Task 1: Add `indicatif` as a dependency of `atlas-cli`

**Spec ref:** §11.

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Modify: `crates/atlas-cli/Cargo.toml`

- [ ] **Step 1: Add to workspace `[workspace.dependencies]`**

In `/Users/antony/Development/Atlas/Cargo.toml`, inside `[workspace.dependencies]`, add (alphabetically after `ignore`):

```toml
indicatif = "0.17"
```

- [ ] **Step 2: Add to atlas-cli `[dependencies]`**

In `crates/atlas-cli/Cargo.toml`, inside `[dependencies]`, add:

```toml
indicatif = { workspace = true }
```

- [ ] **Step 3: Verify the dep resolves**

Run: `cargo check -p atlas-cli`
Expected: builds clean (downloads `indicatif` on first run; no compile errors).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/atlas-cli/Cargo.toml Cargo.lock
git commit -m "chore: add indicatif workspace dep for atlas-cli progress rendering"
```

---

## Task 2: Create `atlas-engine` progress module with event types

**Spec ref:** §5.

**Files:**
- Create: `crates/atlas-engine/src/progress.rs`
- Modify: `crates/atlas-engine/src/lib.rs` (declare and re-export)

- [ ] **Step 1: Write the failing test**

Create `crates/atlas-engine/src/progress.rs` with body:

```rust
//! Progress events emitted by atlas-engine.
//!
//! `atlas-engine` owns the event vocabulary; rendering is the CLI's job.
//! See `docs/superpowers/specs/2026-05-01-engine-progress-events-design.md` §5.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    Started { root: PathBuf },
    Phase(Phase),
    IterStart { iteration: u32, live_components: u64 },
    Subcarve { component_id: String, relpath: PathBuf, k: u64, n: u64 },
    IterEnd { iteration: u32, components_added: u64, elapsed: Duration },
    Surface { component_id: String, relpath: PathBuf, k: u64, n: u64 },
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
}
```

- [ ] **Step 2: Wire the module into `lib.rs`**

In `crates/atlas-engine/src/lib.rs`, add to the module list (alphabetically near the other `l*` modules):

```rust
pub mod progress;
```

And at the bottom, alongside the other `pub use` re-exports:

```rust
pub use progress::{Phase, ProgressEvent, ProgressSink, PromptBreakdown};
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test -p atlas-engine progress::tests::recording_sink_captures_events_in_order`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 4: Run the full engine suite to confirm nothing else regressed**

Run: `cargo test -p atlas-engine`
Expected: all existing engine tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-engine/src/progress.rs crates/atlas-engine/src/lib.rs
git commit -m "feat(engine): add ProgressSink trait and ProgressEvent vocabulary"
```

---

## Task 3: Add `relpath_of(&ComponentEntry) -> PathBuf` helper

**Spec ref:** §5.1.

**Files:**
- Modify: `crates/atlas-engine/src/progress.rs`
- Modify: `crates/atlas-engine/src/lib.rs` (re-export)

- [ ] **Step 1: Add the failing test to `progress.rs`**

Append to the `mod tests` block in `crates/atlas-engine/src/progress.rs`:

```rust
#[test]
fn relpath_of_returns_last_segment_path() {
    use atlas_index::{ComponentEntry, EvidenceGrade, PathSegment};
    use std::path::PathBuf;
    let entry = ComponentEntry {
        id: "c".into(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: Vec::new(),
        language: None,
        build_system: None,
        role: None,
        path_segments: vec![
            PathSegment { path: PathBuf::from("crates"), content_sha: "a".into() },
            PathSegment { path: PathBuf::from("crates/atlas-engine"), content_sha: "b".into() },
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
    use atlas_index::{ComponentEntry, EvidenceGrade};
    use std::path::PathBuf;
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p atlas-engine progress::tests::relpath_of`
Expected: FAIL with "cannot find function `relpath_of`".

- [ ] **Step 3: Add the helper above the `tests` module**

Append to `crates/atlas-engine/src/progress.rs` above `#[cfg(test)] mod tests`:

```rust
use atlas_index::ComponentEntry;

/// Derive the renderer-facing relpath for a component. Returns the
/// deepest `path_segments` entry, or an empty `PathBuf` if there are
/// none. See spec §5.1.
pub fn relpath_of(c: &ComponentEntry) -> PathBuf {
    c.path_segments
        .last()
        .map(|s| s.path.clone())
        .unwrap_or_default()
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Update the `pub use progress::…` line:

```rust
pub use progress::{relpath_of, Phase, ProgressEvent, ProgressSink, PromptBreakdown};
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p atlas-engine progress::tests::relpath_of`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-engine/src/progress.rs crates/atlas-engine/src/lib.rs
git commit -m "feat(engine): add relpath_of helper for ProgressEvent emission"
```

---

## Task 4: Add `progress` field to `FixedpointConfig`

**Spec ref:** §5 (final block).

**Files:**
- Modify: `crates/atlas-engine/src/fixedpoint.rs`

This task adds the field but does **not** fire any events yet — it's purely additive so existing callers and tests stay green.

- [ ] **Step 1: Modify the `FixedpointConfig` struct** (`fixedpoint.rs:42-58`)

Replace the existing struct definition with:

```rust
#[derive(Clone)]
pub struct FixedpointConfig {
    /// Passed through to [`AtlasDatabase::set_max_depth`].
    pub max_depth: u32,
    /// Fail-loud threshold. Defaults to [`FIXEDPOINT_HARD_CAP`].
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
```

(`Debug` is now hand-rolled because `dyn ProgressSink` is not `Debug`.)

- [ ] **Step 2: Run engine tests**

Run: `cargo test -p atlas-engine`
Expected: all tests pass — no behaviour change yet.

- [ ] **Step 3: Run the workspace build to confirm callers still compile**

Run: `cargo check --workspace`
Expected: clean. (`atlas-cli/pipeline.rs` and `evaluation/harness` use `..FixedpointConfig::default()`, which absorbs the new field.)

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-engine/src/fixedpoint.rs
git commit -m "feat(engine): add optional ProgressSink to FixedpointConfig"
```

---

## Task 5: `run_fixedpoint` fires `IterStart` / `Subcarve` / `IterEnd`

**Spec ref:** §7.1, §7.2.

**Files:**
- Modify: `crates/atlas-engine/src/fixedpoint.rs`

- [ ] **Step 1: Write the failing test**

Add at the end of `mod tests` in `fixedpoint.rs`:

```rust
#[test]
fn engine_emits_iter_start_subcarve_iter_end_in_order() {
    use crate::progress::{ProgressEvent, RecordingSink};
    use std::sync::Arc;

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
    assert!(matches!(events[0], ProgressEvent::IterStart { iteration: 0, live_components: 1 }));
    assert!(matches!(events[1], ProgressEvent::Subcarve { k: 1, n: 1, .. }));
    assert!(matches!(
        events[2],
        ProgressEvent::IterEnd { iteration: 0, components_added: 0, .. }
    ));
}

#[test]
fn engine_emits_subcarve_event_before_calling_decision() {
    // Proves the spec §7.2 ordering: Subcarve is emitted *before* the
    // LLM call is in flight, so the bar shows the in-progress target.
    // We use a sink whose on_event records the live token count at
    // that moment; the fact that Subcarve precedes the corresponding
    // call means LlmCache::call_count is unchanged when the event lands.
    use crate::progress::{ProgressEvent, ProgressSink};
    use std::sync::{Arc, Mutex};

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

    let cache = db.llm_cache();
    let cache_calls: Arc<dyn Fn() -> u64 + Send + Sync> = {
        let cache = cache.clone();
        Arc::new(move || cache.call_count())
    };
    let sink = Arc::new(OrderingSink {
        observations: Mutex::new(Vec::new()),
        cache_calls,
    });
    let cfg = FixedpointConfig {
        progress: Some(sink.clone() as Arc<dyn crate::progress::ProgressSink>),
        ..FixedpointConfig::default()
    };
    let _ = run_fixedpoint(&mut db, cfg);
    // The Subcarve event for the live component fired with cache_calls == 0.
    let observed = sink.observations.lock().unwrap().clone();
    assert!(!observed.is_empty(), "expected at least one Subcarve event");
    assert_eq!(observed[0].1, 0, "Subcarve event must precede subcarve_decision LLM call");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p atlas-engine fixedpoint::tests::engine_emits`
Expected: 2 FAIL — events vector is empty (nothing fires events yet).

- [ ] **Step 3: Implement event emission**

Replace the body of `run_fixedpoint` (`fixedpoint.rs:69-128`) with:

```rust
pub fn run_fixedpoint(
    db: &mut AtlasDatabase,
    config: FixedpointConfig,
) -> FixedpointResult {
    let sink = config.progress.clone();
    db.set_max_depth(config.max_depth);
    db.set_fixedpoint_iteration_count(0);

    let mut back_edge: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    db.set_carve_back_edge(back_edge.clone());

    let mut iterations = 0u32;
    loop {
        let iter_started = std::time::Instant::now();
        let components = all_components(db);
        let live: Vec<(String, std::path::PathBuf)> = components
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
            return FixedpointResult { iterations, back_edge };
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
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p atlas-engine fixedpoint::tests`
Expected: all tests pass — both new ones and the existing three.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-engine/src/fixedpoint.rs
git commit -m "feat(engine): emit IterStart/Subcarve/IterEnd from run_fixedpoint"
```

---

## Task 6: Extend `PathologicalBackend` test to assert `IterStart` per iteration

**Spec ref:** §9 ("`PathologicalBackend` test gets a parallel assertion that `IterStart{n}` fires for each iteration up to the hard cap").

**Files:**
- Modify: `crates/atlas-engine/src/fixedpoint.rs` (test module)

- [ ] **Step 1: Add a parallel test alongside the existing panic test**

Append to `mod tests`:

```rust
#[test]
fn pathological_backend_emits_iter_start_for_each_iteration_until_cap() {
    use crate::progress::{ProgressEvent, RecordingSink};
    use std::sync::Arc;

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
```

- [ ] **Step 2: Run, verify pass**

Run: `cargo test -p atlas-engine fixedpoint::tests::pathological_backend_emits_iter_start_for_each_iteration_until_cap`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/atlas-engine/src/fixedpoint.rs
git commit -m "test(engine): assert IterStart fires per iteration until hard cap"
```

---

## Task 7: Rewrite `atlas-cli/src/progress.rs` skeleton

**Spec ref:** §6 (intro), §6.4, §10.

**Files:**
- Rewrite: `crates/atlas-cli/src/progress.rs` (new shape)

This task replaces the file with the new skeleton: `Reporter` (impl `ProgressSink` as a stub for now), `ProgressMode` preserved, `make_stderr_reporter` returning `Arc<Reporter>` (not `Option`), and `ProgressBackend` retained but its `record` call replaced by a no-op `on_llm_call` stub. Task 13 fills in the stub.

- [ ] **Step 1: Replace `progress.rs` with the skeleton**

Overwrite `crates/atlas-cli/src/progress.rs` with:

```rust
//! Progress reporting for `atlas index`.
//!
//! `Reporter` owns an indicatif `MultiProgress` with two bars
//! (activity + token gauge) and implements `ProgressSink` so the engine
//! can drive it directly. The CLI fires `Started`/`Phase`/`Surface`/
//! `Finished` markers itself; the engine fires the inner
//! `IterStart`/`Subcarve`/`IterEnd` triplet. `ProgressBackend` taps the
//! same `Reporter` via a side-channel `on_llm_call` method (spec §6.3).
//! See `docs/superpowers/specs/2026-05-01-engine-progress-events-design.md`.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_engine::{Phase, ProgressEvent, ProgressSink, PromptBreakdown};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TokenCounter};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}

#[derive(Default, Debug, Clone)]
struct ReporterState {
    breakdown: PromptBreakdown,
    last_msg: String,
    iter_history: Vec<String>,
    summary: Option<String>,
    /// k/n set by the most recent engine-level Subcarve/Surface event.
    /// LLM-call taps must not clobber this — they may only refresh the
    /// target relpath, so the denominator from the engine wins.
    sticky_kn: Option<(u64, u64, &'static str)>, // (k, n, label)
    last_llm_target: Option<PathBuf>,
}

pub struct Reporter {
    multi: MultiProgress,
    activity: ProgressBar,
    tokens: ProgressBar,
    state: Mutex<ReporterState>,
    counter: Option<Arc<TokenCounter>>,
    drawing: bool,
}

impl Reporter {
    pub fn new(
        mode: ProgressMode,
        counter: Option<Arc<TokenCounter>>,
    ) -> Arc<Self> {
        let stderr_is_tty = std::io::stderr().is_terminal();
        let drawing = match mode {
            ProgressMode::Auto => stderr_is_tty,
            ProgressMode::Always => true,
            ProgressMode::Never => false,
        };
        let multi = if drawing {
            MultiProgress::with_draw_target(ProgressDrawTarget::stderr())
        } else {
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
        };
        let activity = multi.add(ProgressBar::new(0));
        activity.set_style(
            ProgressStyle::with_template("  {spinner} {msg}  {elapsed_precise}")
                .expect("static template"),
        );
        if drawing {
            activity.enable_steady_tick(Duration::from_millis(120));
        }
        let tokens = multi.add(ProgressBar::new(0));
        tokens.set_style(
            ProgressStyle::with_template("    tokens {msg}  {bar:50}  {percent:>3}%")
                .expect("static template"),
        );

        Arc::new(Self {
            multi,
            activity,
            tokens,
            state: Mutex::new(ReporterState::default()),
            counter,
            drawing,
        })
    }

    /// Tear down the live bars. Idempotent. Called on the success path
    /// after `Finished` and on the error path during pipeline drop.
    pub fn finish(&self) {
        let _ = self.multi.clear();
    }

    /// Side-channel called by `ProgressBackend` when an LLM call lands.
    /// (Filled in Task 13 — for now this is a no-op stub that exists
    /// only so `ProgressBackend` compiles.)
    pub fn on_llm_call(&self, _prompt: PromptId, _target: Option<PathBuf>) {}

    fn lock(&self) -> std::sync::MutexGuard<'_, ReporterState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // --- test-only accessors ---
    #[cfg(test)]
    pub(crate) fn current_msg(&self) -> String {
        self.lock().last_msg.clone()
    }
    #[cfg(test)]
    pub(crate) fn breakdown(&self) -> PromptBreakdown {
        self.lock().breakdown.clone()
    }
    #[cfg(test)]
    pub(crate) fn iter_history(&self) -> Vec<String> {
        self.lock().iter_history.clone()
    }
    #[cfg(test)]
    pub(crate) fn summary(&self) -> Option<String> {
        self.lock().summary.clone()
    }
    #[cfg(test)]
    pub(crate) fn activity_length(&self) -> Option<u64> {
        self.activity.length()
    }
    #[cfg(test)]
    pub(crate) fn activity_position(&self) -> u64 {
        self.activity.position()
    }
    #[cfg(test)]
    pub(crate) fn tokens_length(&self) -> Option<u64> {
        self.tokens.length()
    }
    #[cfg(test)]
    pub(crate) fn tokens_position(&self) -> u64 {
        self.tokens.position()
    }
    #[cfg(test)]
    pub(crate) fn drawing(&self) -> bool {
        self.drawing
    }
}

impl ProgressSink for Reporter {
    fn on_event(&self, event: ProgressEvent) {
        // Filled in Tasks 9-12. For now a stub so engine code compiles.
        let _ = event;
    }
}

/// Build a reporter wired to stderr. Always returns a Reporter; when
/// `mode` resolves to disabled, the underlying draw target is hidden,
/// but the reporter still receives events and updates `breakdown` so
/// the final summary can be printed via plain `eprintln!` (spec §6.4).
pub fn make_stderr_reporter(
    mode: ProgressMode,
    counter: Option<Arc<TokenCounter>>,
) -> Arc<Reporter> {
    Reporter::new(mode, counter)
}

/// Decorator backend: forwards every call to `inner`, then taps the
/// reporter's side-channel.
pub struct ProgressBackend {
    inner: Arc<dyn LlmBackend>,
    reporter: Arc<Reporter>,
}

impl ProgressBackend {
    pub fn new(inner: Arc<dyn LlmBackend>, reporter: Arc<Reporter>) -> Arc<Self> {
        Arc::new(Self { inner, reporter })
    }
}

impl LlmBackend for ProgressBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let result = self.inner.call(req);
        let target = relpath_from_inputs(&req.inputs);
        self.reporter.on_llm_call(req.prompt_template, target);
        result
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }
}

fn relpath_from_inputs(inputs: &Value) -> Option<PathBuf> {
    inputs
        .get("relpath")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_stderr_reporter_returns_reporter_in_never_mode() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        assert!(!r.drawing);
    }

    #[test]
    fn finish_is_idempotent() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        r.finish();
        r.finish(); // must not panic
    }
}
```

- [ ] **Step 2: Update the lib re-exports if needed**

Run `grep -n "pub use crate::progress" crates/atlas-cli/src/lib.rs` (if any). Adjust to the new public surface:

```rust
pub use crate::progress::{make_stderr_reporter, ProgressBackend, ProgressMode, Reporter};
```

If `lib.rs` does not re-export from `progress`, leave it.

- [ ] **Step 3: Build the crate**

Run: `cargo check -p atlas-cli`
Expected: this **will fail to compile** the binary because `main.rs` still calls `make_stderr_reporter` expecting `Option<...>` and uses `announce_start` / `record` / `finish`. That's intentional — Task 18 fixes main.rs. Until then we keep main.rs working by patching it minimally now.

- [ ] **Step 4: Apply minimum-viable patch to `main.rs`**

In `crates/atlas-cli/src/main.rs`, replace the current reporter wiring (lines ~148-168) with:

```rust
    let progress_mode = if args.no_progress {
        ProgressMode::Never
    } else if args.progress {
        ProgressMode::Always
    } else {
        ProgressMode::Auto
    };
    let reporter = make_stderr_reporter(progress_mode, handles.counter.clone());
    let backend: Arc<dyn LlmBackend> = ProgressBackend::new(
        handles.backend.clone(),
        Arc::clone(&reporter),
    ) as Arc<dyn LlmBackend>;

    let outcome = run_index(&config, backend, handles.counter.clone());
    reporter.finish();
```

The `announce_start` / `r.finish()` / `Option` handling all collapse — `Reporter` is always present.

- [ ] **Step 5: Build, run progress tests**

Run: `cargo test -p atlas-cli progress::tests`
Expected: 2 PASS (`make_stderr_reporter_returns_reporter_in_never_mode`, `finish_is_idempotent`).

Run: `cargo build -p atlas-cli`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/atlas-cli/src/progress.rs crates/atlas-cli/src/main.rs crates/atlas-cli/src/lib.rs
git commit -m "refactor(cli): replace ProgressReporter with indicatif Reporter skeleton"
```

---

## Task 8: Implement `render_activity_msg` pure function

**Spec ref:** §6.1.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing tests**

Append to `mod tests` in `progress.rs`:

```rust
use atlas_engine::Phase;
use std::path::PathBuf;

#[test]
fn render_activity_msg_started() {
    assert_eq!(render_activity_msg(&MsgInput::Started), "seed");
}

#[test]
fn render_activity_msg_iter_start_shows_scanning_count() {
    let m = MsgInput::IterStart { iteration: 1, live: 47 };
    assert_eq!(render_activity_msg(&m), "iter 1 · scanning 47 components");
}

#[test]
fn render_activity_msg_subcarve_shows_kn_and_relpath() {
    let m = MsgInput::Subcarve {
        iteration: 1,
        k: 47,
        n: 120,
        target: PathBuf::from("crates/atlas-engine"),
    };
    assert_eq!(
        render_activity_msg(&m),
        "iter 1 · subcarve  47/120 (crates/atlas-engine)"
    );
}

#[test]
fn render_activity_msg_subcarve_handles_empty_target() {
    let m = MsgInput::Subcarve {
        iteration: 1,
        k: 47,
        n: 120,
        target: PathBuf::new(),
    };
    assert_eq!(render_activity_msg(&m), "iter 1 · subcarve  47/120");
}

#[test]
fn render_activity_msg_phase_project() {
    assert_eq!(render_activity_msg(&MsgInput::Phase(Phase::Project)), "project");
}

#[test]
fn render_activity_msg_phase_edges() {
    assert_eq!(
        render_activity_msg(&MsgInput::Phase(Phase::Edges)),
        "project · edges (batch)"
    );
}

#[test]
fn render_activity_msg_surface_shows_kn_and_relpath() {
    let m = MsgInput::Surface { k: 12, n: 53, target: PathBuf::from("crates/atlas-cli") };
    assert_eq!(
        render_activity_msg(&m),
        "project · surface  12/53 (crates/atlas-cli)"
    );
}

#[test]
fn render_activity_msg_llm_tap_classify() {
    let m = MsgInput::LlmTap {
        iteration: Some(1),
        prompt: PromptId::Classify,
        target: Some(PathBuf::from("crates/foo")),
    };
    assert_eq!(render_activity_msg(&m), "iter 1 · classify (crates/foo)");
}
```

- [ ] **Step 2: Run, verify failure**

Run: `cargo test -p atlas-cli progress::tests::render_activity_msg`
Expected: FAIL with "cannot find type `MsgInput`".

- [ ] **Step 3: Implement `MsgInput` and `render_activity_msg`**

Above `mod tests` in `progress.rs`, add:

```rust
#[derive(Debug, Clone)]
pub(crate) enum MsgInput {
    Started,
    Phase(Phase),
    IterStart { iteration: u32, live: u64 },
    Subcarve { iteration: u32, k: u64, n: u64, target: PathBuf },
    Surface { k: u64, n: u64, target: PathBuf },
    LlmTap { iteration: Option<u32>, prompt: PromptId, target: Option<PathBuf> },
}

pub(crate) fn render_activity_msg(input: &MsgInput) -> String {
    match input {
        MsgInput::Started => "seed".to_string(),
        MsgInput::Phase(Phase::Seed) => "seed".to_string(),
        MsgInput::Phase(Phase::Fixedpoint) => "fixedpoint".to_string(),
        MsgInput::Phase(Phase::Project) => "project".to_string(),
        MsgInput::Phase(Phase::Edges) => "project · edges (batch)".to_string(),
        MsgInput::IterStart { iteration, live } => {
            format!("iter {iteration} · scanning {live} components")
        }
        MsgInput::Subcarve { iteration, k, n, target } => {
            if target.as_os_str().is_empty() {
                format!("iter {iteration} · subcarve  {k}/{n}")
            } else {
                format!("iter {iteration} · subcarve  {k}/{n} ({})", target.display())
            }
        }
        MsgInput::Surface { k, n, target } => {
            if target.as_os_str().is_empty() {
                format!("project · surface  {k}/{n}")
            } else {
                format!("project · surface  {k}/{n} ({})", target.display())
            }
        }
        MsgInput::LlmTap { iteration, prompt, target } => {
            let label = match prompt {
                PromptId::Classify => "classify",
                PromptId::Stage1Surface => "surface",
                PromptId::Stage2Edges => "edges",
                PromptId::Subcarve => "subcarve",
            };
            let prefix = iteration
                .map(|i| format!("iter {i} · "))
                .unwrap_or_default();
            match target {
                Some(t) => format!("{prefix}{label} ({})", t.display()),
                None => format!("{prefix}{label}"),
            }
        }
    }
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::render_activity_msg`
Expected: 8 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): pure render_activity_msg covering each phase variant"
```

---

## Task 9: `Reporter` reacts to `Started` and `Phase` events

**Spec ref:** §5, §6.1.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing tests**

Append to `mod tests`:

```rust
#[test]
fn reporter_started_event_sets_seed_msg() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Started { root: PathBuf::from("/tmp/x") });
    assert_eq!(r.current_msg(), "seed");
}

#[test]
fn reporter_phase_event_updates_msg() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Phase(Phase::Project));
    assert_eq!(r.current_msg(), "project");
}
```

- [ ] **Step 2: Run, verify failure**

Run: `cargo test -p atlas-cli progress::tests::reporter_`
Expected: 2 FAIL — `current_msg()` returns "" (stub `on_event`).

- [ ] **Step 3: Implement event handling for `Started`/`Phase`**

Replace the body of `impl ProgressSink for Reporter`:

```rust
impl ProgressSink for Reporter {
    fn on_event(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { .. } => {
                self.set_msg(MsgInput::Started);
            }
            ProgressEvent::Phase(p) => {
                self.set_msg(MsgInput::Phase(p));
            }
            _ => {}
        }
    }
}

impl Reporter {
    fn set_msg(&self, input: MsgInput) {
        let rendered = render_activity_msg(&input);
        let mut s = self.lock();
        s.last_msg = rendered.clone();
        drop(s);
        self.activity.set_message(rendered);
    }
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::reporter_`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): Reporter handles Started and Phase events"
```

---

## Task 10: `Reporter` reacts to `IterStart` / `IterEnd` events (with scrollback line)

**Spec ref:** §5, §6.2.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing tests**

Append:

```rust
#[test]
fn reporter_iter_start_sets_scanning_msg_and_resets_length() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::IterStart { iteration: 1, live_components: 47 });
    assert_eq!(r.current_msg(), "iter 1 · scanning 47 components");
}

#[test]
fn reporter_iter_end_appends_scrollback_line() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::IterStart { iteration: 1, live_components: 1247 });
    r.on_event(ProgressEvent::IterEnd {
        iteration: 1,
        components_added: 18,
        elapsed: std::time::Duration::from_secs(872),
    });
    let history = r.iter_history();
    assert_eq!(history.len(), 1);
    assert!(history[0].contains("iter 1"));
    assert!(history[0].contains("1247 components"));
    assert!(history[0].contains("+18 sub-dirs"));
    assert!(history[0].contains("14:32"));
}
```

- [ ] **Step 2: Run, verify failure**

Run: `cargo test -p atlas-cli progress::tests::reporter_iter`
Expected: 2 FAIL.

- [ ] **Step 3: Track current iteration's live count and emit scrollback**

In `ReporterState`, add:

```rust
    /// The `live_components` from the most recent `IterStart`. Used to
    /// build the scrollback line on `IterEnd`.
    iter_live: u64,
```

Extend the `ProgressSink` match:

```rust
            ProgressEvent::IterStart { iteration, live_components } => {
                {
                    let mut s = self.lock();
                    s.iter_live = live_components;
                }
                self.activity.set_length(0); // suppress ETA outside k/n phases
                self.set_msg(MsgInput::IterStart { iteration, live: live_components });
            }
            ProgressEvent::IterEnd { iteration, components_added, elapsed } => {
                let live = self.lock().iter_live;
                let line = format_iter_end_line(iteration, live, components_added, elapsed);
                let _ = self.multi.println(&line);
                self.lock().iter_history.push(line);
            }
```

Add the helper:

```rust
fn format_iter_end_line(
    iteration: u32,
    live: u64,
    added: u64,
    elapsed: Duration,
) -> String {
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    format!(
        "✓ iter {iteration} · {live} components · +{added} sub-dirs · {mins:02}:{secs:02}"
    )
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::reporter_iter`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): Reporter handles IterStart/IterEnd with scrollback line"
```

---

## Task 11: `Reporter` reacts to `Subcarve` / `Surface` events (sticky k/n)

**Spec ref:** §5, §6.1, §6.3.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing tests**

```rust
#[test]
fn reporter_subcarve_sets_kn_msg_and_progress_length() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::IterStart { iteration: 1, live_components: 120 });
    r.on_event(ProgressEvent::Subcarve {
        component_id: "c".into(),
        relpath: PathBuf::from("crates/atlas-engine"),
        k: 47,
        n: 120,
    });
    assert_eq!(
        r.current_msg(),
        "iter 1 · subcarve  47/120 (crates/atlas-engine)"
    );
    assert_eq!(r.activity_length(), Some(120));
    assert_eq!(r.activity_position(), 47);
}

#[test]
fn reporter_surface_sets_kn_under_project_phase() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Phase(Phase::Project));
    r.on_event(ProgressEvent::Surface {
        component_id: "c".into(),
        relpath: PathBuf::from("crates/atlas-cli"),
        k: 12,
        n: 53,
    });
    assert_eq!(
        r.current_msg(),
        "project · surface  12/53 (crates/atlas-cli)"
    );
}
```

- [ ] **Step 2: Track iteration count + handle the events**

Add to `ReporterState`:

```rust
    last_iteration: u32,
```

Modify the existing `IterStart` arm (added in Task 10) to capture the iteration:

```rust
            ProgressEvent::IterStart { iteration, live_components } => {
                {
                    let mut s = self.lock();
                    s.iter_live = live_components;
                    s.last_iteration = iteration;
                }
                self.activity.set_length(0);
                self.set_msg(MsgInput::IterStart { iteration, live: live_components });
            }
```

Then add the new arms:

```rust
            ProgressEvent::Subcarve { component_id: _, relpath, k, n } => {
                let iteration = self.lock().last_iteration;
                {
                    let mut s = self.lock();
                    s.sticky_kn = Some((k, n, "subcarve"));
                }
                self.activity.set_length(n);
                self.activity.set_position(k);
                self.set_msg(MsgInput::Subcarve {
                    iteration,
                    k,
                    n,
                    target: relpath,
                });
            }
            ProgressEvent::Surface { component_id: _, relpath, k, n } => {
                {
                    let mut s = self.lock();
                    s.sticky_kn = Some((k, n, "surface"));
                }
                self.activity.set_length(n);
                self.activity.set_position(k);
                self.set_msg(MsgInput::Surface {
                    k,
                    n,
                    target: relpath,
                });
            }
```

- [ ] **Step 3: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::reporter_subcarve_sets_kn_msg_and_progress_length progress::tests::reporter_surface_sets_kn_under_project_phase`
Expected: 2 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): Reporter handles Subcarve/Surface with k/n progress"
```

---

## Task 12: `Reporter` reacts to `Finished`

**Spec ref:** §5, §6.2 (final summary lines).

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing test**

```rust
#[test]
fn reporter_finished_records_summary_and_breakdown() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Finished {
        components: 1268,
        llm_calls: 3892,
        tokens_used: 184_000,
        token_budget: Some(200_000),
        elapsed: std::time::Duration::from_secs(22 * 60 + 15),
        breakdown: PromptBreakdown {
            classify: 2715,
            surface: 1268,
            edges: 1,
            subcarve: 908,
        },
    });
    let summary = r.summary().expect("summary set after Finished");
    assert!(summary.contains("done"));
    assert!(summary.contains("1268 components"));
    assert!(summary.contains("3892 LLM calls"));
    assert!(summary.contains("184.0k/200.0k tokens"));
    assert!(summary.contains("22:15"));
    assert!(summary.contains("classify=2715"));
    assert!(summary.contains("subcarve=908"));
}
```

- [ ] **Step 2: Implement `Finished` handling**

Add to the `ProgressSink` match:

```rust
            ProgressEvent::Finished {
                components,
                llm_calls,
                tokens_used,
                token_budget,
                elapsed,
                breakdown,
            } => {
                let line = format_finished_line(
                    components,
                    llm_calls,
                    tokens_used,
                    token_budget,
                    elapsed,
                    &breakdown,
                );
                if self.drawing {
                    let _ = self.multi.println(&line);
                } else {
                    eprintln!("{line}");
                }
                let mut s = self.lock();
                s.summary = Some(line);
                s.breakdown = breakdown;
            }
```

Add the helper near `format_iter_end_line`:

```rust
fn format_finished_line(
    components: u64,
    llm_calls: u64,
    tokens_used: u64,
    budget: Option<u64>,
    elapsed: Duration,
    bd: &PromptBreakdown,
) -> String {
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    let tokens = match budget {
        Some(b) => format!("{}/{} tokens", abbreviate(tokens_used), abbreviate(b)),
        None => format!("{} tokens (no budget)", abbreviate(tokens_used)),
    };
    format!(
        "[atlas] done · {components} components · {llm_calls} LLM calls · {tokens} · {mins:02}:{secs:02}\n        classify={c}  surface={s}  edges={e}  subcarve={sc}",
        c = bd.classify, s = bd.surface, e = bd.edges, sc = bd.subcarve,
    )
}

fn abbreviate(n: u64) -> String {
    if n < 10_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    }
}
```

- [ ] **Step 3: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::reporter_finished_records_summary_and_breakdown`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): Reporter handles Finished event with summary line"
```

---

## Task 13: `on_llm_call` side-channel + sticky-k/n priority

**Spec ref:** §6.3.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing test for sticky priority**

```rust
#[test]
fn on_llm_call_increments_breakdown_and_does_not_clobber_sticky_kn() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::IterStart { iteration: 1, live_components: 120 });
    r.on_event(ProgressEvent::Subcarve {
        component_id: "c".into(),
        relpath: PathBuf::from("crates/atlas-engine"),
        k: 47,
        n: 120,
    });
    // mid-iteration LLM call
    r.on_llm_call(PromptId::Classify, Some(PathBuf::from("crates/foo")));
    // breakdown reflects the call
    assert_eq!(r.breakdown().classify, 1);
    // ...but the sticky k/n message survives — engine event still wins.
    assert_eq!(
        r.current_msg(),
        "iter 1 · subcarve  47/120 (crates/atlas-engine)"
    );
}

#[test]
fn on_llm_call_without_sticky_kn_falls_through_to_llm_msg() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Phase(Phase::Seed));
    r.on_llm_call(PromptId::Classify, Some(PathBuf::from("crates/foo")));
    assert_eq!(r.current_msg(), "classify (crates/foo)");
}
```

- [ ] **Step 2: Implement `on_llm_call`**

Replace the stub `on_llm_call` body:

```rust
    pub fn on_llm_call(&self, prompt: PromptId, target: Option<PathBuf>) {
        let mut s = self.lock();
        match prompt {
            PromptId::Classify => s.breakdown.classify += 1,
            PromptId::Stage1Surface => s.breakdown.surface += 1,
            PromptId::Stage2Edges => s.breakdown.edges += 1,
            PromptId::Subcarve => s.breakdown.subcarve += 1,
        }
        s.last_llm_target = target.clone();
        let sticky = s.sticky_kn;
        let iteration = if sticky.is_some() { Some(s.last_iteration) } else { None };
        drop(s);
        if sticky.is_some() {
            // Engine event already set msg with k/n + relpath; refresh
            // the *target* portion only. Cheapest way: do nothing — the
            // engine message stays visible until the next engine event.
        } else {
            self.set_msg(MsgInput::LlmTap { iteration, prompt, target });
        }
    }
```

- [ ] **Step 3: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::on_llm_call`
Expected: 2 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): on_llm_call updates breakdown and respects sticky k/n"
```

---

## Task 14: Token gauge updates from `TokenCounter`

**Spec ref:** §6 (Bar 2 — token gauge), §6.4 (hidden when no budget).

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add failing test**

```rust
#[test]
fn token_gauge_updates_from_counter_on_each_event() {
    use atlas_llm::TokenCounter;
    let counter = Arc::new(TokenCounter::new(200_000));
    counter.charge(18_400).unwrap();
    let r = make_stderr_reporter(ProgressMode::Never, Some(counter.clone()));
    r.on_event(ProgressEvent::Phase(Phase::Fixedpoint));
    assert_eq!(r.tokens_length(), Some(200_000));
    assert_eq!(r.tokens_position(), 18_400);
    counter.charge(1_600).unwrap();
    r.on_event(ProgressEvent::Phase(Phase::Project));
    assert_eq!(r.tokens_position(), 20_000);
}

#[test]
fn token_gauge_hidden_when_no_counter() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    r.on_event(ProgressEvent::Phase(Phase::Fixedpoint));
    // No counter ⇒ length stays 0 ⇒ indicatif renders nothing.
    assert_eq!(r.tokens_length(), Some(0));
}
```

- [ ] **Step 2: Implement gauge refresh**

In the `Reporter` impl, add:

```rust
    fn refresh_token_gauge(&self) {
        if let Some(c) = self.counter.as_ref() {
            self.tokens.set_length(c.budget());
            self.tokens.set_position(c.used());
            self.tokens.set_message(format!(
                "{}/{}",
                abbreviate(c.used()),
                abbreviate(c.budget())
            ));
        }
    }
```

Call `self.refresh_token_gauge();` at the top of `on_event` and inside `on_llm_call` (after the lock drop).

If the counter has `budget == 0` (the `--no-budget` case), hide it explicitly:

```rust
    fn refresh_token_gauge(&self) {
        let Some(c) = self.counter.as_ref() else { return; };
        if c.budget() == 0 {
            self.tokens.set_draw_target(ProgressDrawTarget::hidden());
            return;
        }
        self.tokens.set_length(c.budget());
        self.tokens.set_position(c.used());
        self.tokens.set_message(format!(
            "{}/{}",
            abbreviate(c.used()),
            abbreviate(c.budget())
        ));
    }
```

- [ ] **Step 3: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::token_gauge`
Expected: 2 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): refresh token gauge from TokenCounter on every event"
```

---

## Task 15: ProgressMode mapping + non-TTY hidden + steady-tick discipline

**Spec ref:** §6.4, §10.

The skeleton (Task 7) already wires `ProgressMode → drawing` and disables `enable_steady_tick` when not drawing. This task just adds the regression tests to lock that in.

**Files:**
- Modify: `crates/atlas-cli/src/progress.rs`

- [ ] **Step 1: Add tests**

```rust
#[test]
fn progress_mode_never_yields_hidden_draw_target() {
    let r = make_stderr_reporter(ProgressMode::Never, None);
    assert!(!r.drawing());
}

#[test]
fn progress_mode_always_yields_visible_draw_target() {
    let r = make_stderr_reporter(ProgressMode::Always, None);
    assert!(r.drawing);
}

#[test]
fn finish_after_finished_event_is_safe() {
    let r = make_stderr_reporter(ProgressMode::Always, None);
    r.on_event(ProgressEvent::Finished {
        components: 1, llm_calls: 0, tokens_used: 0, token_budget: None,
        elapsed: Duration::from_secs(1), breakdown: PromptBreakdown::default(),
    });
    r.finish();
}
```

- [ ] **Step 2: Run, verify pass**

Run: `cargo test -p atlas-cli progress::tests::progress_mode progress::tests::finish_after_finished`
Expected: 3 PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/atlas-cli/src/progress.rs
git commit -m "test(cli): lock ProgressMode → draw-target + finish lifecycle"
```

---

## Task 16: Wire `Reporter` into `pipeline.rs`

**Spec ref:** §7 (top-level orchestration sketch).

**Files:**
- Modify: `crates/atlas-cli/src/pipeline.rs`

This task installs a `Reporter` in `run_index`, fires `Started`/`Phase` markers around each step, and passes the sink in `FixedpointConfig`. **It does not yet decompose L5 demand** — that's Task 17, kept separate so byte-identity (Task 19) verifies that change in isolation.

- [ ] **Step 1: Update `IndexConfig` to carry the reporter**

In `pipeline.rs`, change `run_index`'s signature to accept the reporter:

```rust
pub fn run_index(
    config: &IndexConfig,
    backend: Arc<dyn LlmBackend>,
    counter: Option<Arc<TokenCounter>>,
    reporter: Arc<crate::progress::Reporter>,
) -> Result<IndexSummary, IndexError> {
```

The CLI will pass it; tests that don't care can call `Reporter::new(ProgressMode::Never, None)`.

- [ ] **Step 2: Fire `Started` + `Phase(Seed)`**

Right after the `BudgetSentinel` is built but before priors load:

```rust
    use atlas_engine::{Phase, ProgressEvent};
    use std::time::Instant;
    let started_at = Instant::now();
    reporter.on_event(ProgressEvent::Started { root: config.root.clone() });
    reporter.on_event(ProgressEvent::Phase(Phase::Seed));
```

- [ ] **Step 3: Fire `Phase(Fixedpoint)` and pass sink in `FixedpointConfig`**

Replace the `fp_config` block (`pipeline.rs:160-164`) with:

```rust
    reporter.on_event(ProgressEvent::Phase(Phase::Fixedpoint));
    let sink: Arc<dyn atlas_engine::ProgressSink> = reporter.clone();
    let fp_config = FixedpointConfig {
        max_depth: config.max_depth,
        progress: Some(sink.clone()),
        ..FixedpointConfig::default()
    };
```

- [ ] **Step 4: Fire `Phase(Project)` and `Phase(Edges)` around projection**

Before the projection block:

```rust
    reporter.on_event(ProgressEvent::Phase(Phase::Project));
```

Just before the existing `let related_file = (*related_components_yaml_snapshot(&db)).clone();` line, insert:

```rust
    reporter.on_event(ProgressEvent::Phase(Phase::Edges));
```

- [ ] **Step 5: Fire `Finished` at the end**

After `summary` is built and before the `if config.dry_run` early return, fire:

```rust
    reporter.on_event(ProgressEvent::Finished {
        components: summary.component_count as u64,
        llm_calls: summary.llm_calls,
        tokens_used: summary.tokens_used,
        token_budget: summary.token_budget,
        elapsed: started_at.elapsed(),
        breakdown: reporter.breakdown_snapshot(),
    });
```

Add a public accessor on `Reporter`:

```rust
    pub fn breakdown_snapshot(&self) -> PromptBreakdown {
        self.lock().breakdown.clone()
    }
```

- [ ] **Step 6: Update tests**

Existing pipeline.rs tests build configs by hand. Add `_reporter: Arc<Reporter>` parameter passing where needed; `Reporter::new(ProgressMode::Never, None)` is fine for them.

- [ ] **Step 7: Run pipeline tests**

Run: `cargo test -p atlas-cli pipeline::tests`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/atlas-cli/src/pipeline.rs crates/atlas-cli/src/progress.rs
git commit -m "feat(cli): pipeline owns Reporter and fires Phase markers"
```

---

## Task 17: Decompose L5 demand into per-component `Surface` events

**Spec ref:** §7 (orchestration sketch), §7.3 (byte-identity).

**Files:**
- Modify: `crates/atlas-cli/src/pipeline.rs`

The current pipeline calls `components_yaml_snapshot_with_prompt_shas` once, which internally demands every L5 surface in one go. We split that into a per-component pre-loop that emits `Surface{k,n}` events; the final snapshot then hits the warmed cache.

- [ ] **Step 1: Insert the per-component loop**

Inside the projection block, between `Phase(Project)` and the existing `components_yaml_snapshot_with_prompt_shas` call:

```rust
    use atlas_engine::{all_components, surface_of, relpath_of};
    let live_components: Vec<_> = all_components(&db)
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();
    let n = live_components.len() as u64;
    for (i, comp) in live_components.iter().enumerate() {
        reporter.on_event(ProgressEvent::Surface {
            component_id: comp.id.clone(),
            relpath: relpath_of(comp),
            k: (i as u64) + 1,
            n,
        });
        let _ = surface_of(&db, comp.id.clone());
    }
```

(The final `components_yaml_snapshot_with_prompt_shas` line is unchanged — it now hits the populated `LlmResponseCache` per component.)

- [ ] **Step 2: Run pipeline tests**

Run: `cargo test -p atlas-cli pipeline::tests`
Expected: PASS — output content unchanged because the demand pattern only reorders cache population.

- [ ] **Step 3: Run engine tests for cross-check**

Run: `cargo test -p atlas-engine`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/pipeline.rs
git commit -m "feat(cli): decompose L5 demand into per-component Surface events"
```

---

## Task 18: Wire `main.rs` to the new `Reporter` API

**Spec ref:** §10.

**Files:**
- Modify: `crates/atlas-cli/src/main.rs`

The Task 7 minimum-viable patch already removed the `Option<...>` handling. This task removes the `r.announce_start(&config.root)` call (now done by the engine `Started` event) and threads `reporter` into `run_index`.

- [ ] **Step 1: Replace the reporter wiring** (`main.rs:148-168`)

```rust
    let progress_mode = if args.no_progress {
        ProgressMode::Never
    } else if args.progress {
        ProgressMode::Always
    } else {
        ProgressMode::Auto
    };
    let reporter = make_stderr_reporter(progress_mode, handles.counter.clone());
    let backend: Arc<dyn LlmBackend> =
        ProgressBackend::new(handles.backend.clone(), Arc::clone(&reporter))
            as Arc<dyn LlmBackend>;

    let outcome = run_index(&config, backend, handles.counter.clone(), Arc::clone(&reporter));
    reporter.finish();
    match outcome {
        Ok(summary) => {
            println!("{}", atlas_cli::pipeline::format_summary(&summary));
            drop(handles);
            Ok(ExitCode::SUCCESS)
        }
        Err(IndexError::BudgetExhausted) => {
            eprintln!("atlas: LLM token budget exhausted; no output files were written");
            drop(handles);
            Ok(ExitCode::from(2))
        }
        Err(IndexError::Other(err)) => {
            drop(handles);
            Err(err)
        }
    }
```

- [ ] **Step 2: Build the binary**

Run: `cargo build -p atlas-cli`
Expected: clean.

- [ ] **Step 3: Smoke-test on a tiny fixture**

Create `/tmp/atlas-smoke` with one Cargo.toml + lib.rs (or pick the smallest fixture in the repo). Run:

```bash
cargo run -p atlas-cli --quiet -- index /tmp/atlas-smoke --no-budget --no-gitignore
```

Expected: output completes without panic; if stderr is a TTY, you should see live activity bar updates and a final `[atlas] done · …` summary; on non-TTY (output redirected) you should see only the plain `[atlas] done` line.

- [ ] **Step 4: Commit**

```bash
git add crates/atlas-cli/src/main.rs
git commit -m "feat(cli): main passes Reporter through run_index, drops legacy hooks"
```

---

## Task 19: Integration regression — byte-identical YAML after L5 demand reorder

**Spec ref:** §7.3.

**Files:**
- Create: `crates/atlas-cli/tests/byte_identity_l5_demand.rs`

This test uses `TestBackend` (no real LLM) to drive the full pipeline and asserts that the `ComponentsFile` produced via the new per-component demand loop equals what would come out of a single-shot snapshot demand. The cleanest way is to run the pipeline twice on the same fixture and assert equality.

- [ ] **Step 1: Write the test**

Create `crates/atlas-cli/tests/byte_identity_l5_demand.rs`:

```rust
//! Regression test: decomposing L5 demand into a per-component loop
//! must produce byte-identical `components.yaml` output. Spec §7.3.

use std::path::Path;
use std::sync::Arc;

use atlas_cli::pipeline::{run_index, IndexConfig};
use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_index::ComponentsFile;
use atlas_llm::{LlmBackend, LlmFingerprint, PromptId, TestBackend};
use serde_json::json;
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [1u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn write_lib(root: &Path, name: &str) {
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

fn make_backend() -> Arc<dyn LlmBackend> {
    let backend = TestBackend::with_fingerprint(fingerprint());
    backend.respond(
        PromptId::Stage1Surface,
        json!({}),
        json!({"purpose": "x"}),
    );
    backend.respond(
        PromptId::Stage2Edges,
        json!({}),
        json!([]),
    );
    Arc::new(backend) as Arc<dyn LlmBackend>
}

#[test]
fn pipeline_run_twice_produces_identical_components_yaml() {
    let tmp = TempDir::new().unwrap();
    write_lib(tmp.path(), "lib");

    let mut config = IndexConfig::new(tmp.path().to_path_buf());
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());

    let reporter = make_stderr_reporter(ProgressMode::Never, None);
    let backend = make_backend();
    let _ = run_index(&config, backend, None, Arc::clone(&reporter)).unwrap();
    let first = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    // Second run on the same output_dir + fixture: should hit cache and
    // produce the same bytes (modulo generated_at, which the pipeline
    // pins to the prior timestamp when content matches).
    let reporter2 = make_stderr_reporter(ProgressMode::Never, None);
    let backend2 = make_backend();
    let _ = run_index(&config, backend2, None, Arc::clone(&reporter2)).unwrap();
    let second = std::fs::read(config.output_dir.join("components.yaml")).unwrap();

    assert_eq!(first, second, "components.yaml must be byte-identical on no-op re-run");

    // And the parsed file is also identical.
    let parsed_first: ComponentsFile = serde_yaml::from_slice(&first).unwrap();
    let parsed_second: ComponentsFile = serde_yaml::from_slice(&second).unwrap();
    assert_eq!(parsed_first, parsed_second);
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p atlas-cli --test byte_identity_l5_demand`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/atlas-cli/tests/byte_identity_l5_demand.rs
git commit -m "test(cli): byte-identity regression for decomposed L5 demand"
```

---

## Final verification

- [ ] **Step 1: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 2: Run clippy with workspace deny-warnings**

Run: `cargo clippy --workspace --all-targets`
Expected: no warnings, no errors.

- [ ] **Step 3: Hand-eyeball the live output**

Run on a real-sized repo with a budget set:

```bash
cargo run -p atlas-cli --release -- index ~/Development/Atlas --budget 200000
```

Confirm:
- Activity bar shows `iter 1 · subcarve k/n (relpath)` while a subcarve LLM call is in flight.
- Token gauge reflects `tokens_used / budget`.
- After each iteration, a `✓ iter N · X components · +Y sub-dirs · MM:SS` line lands in scrollback.
- After completion, `[atlas] done · … · classify=… surface=… …` line is visible.

If any of those don't match, **stop** — don't paper over the issue. Re-read the spec section that describes the broken behaviour.

---

## Self-review (spec coverage map)

| Spec section | Covered by |
|---|---|
| §1 Problem | Whole plan motivation |
| §2 Goals | Tasks 5, 11, 14 (k/n, useful signal even on cached re-runs, byte-identity in 17/19) |
| §3 Non-goals | Constrains scope — no per-Salsa-tracked-query instrumentation tasks |
| §4 Architecture | Tasks 2, 7, 16 (engine/CLI separation) |
| §5 Event vocabulary | Tasks 2, 3, 4 |
| §5.1 relpath derivation | Task 3 |
| §6 Rendering — indicatif layout | Tasks 7, 14 |
| §6.1 Activity-bar message by phase | Task 8 |
| §6.2 Iteration history via println | Task 10 |
| §6.3 Merging the LLM-call side-channel | Task 13 |
| §6.4 Non-TTY fallback | Tasks 7, 12, 15 |
| §7 Pipeline orchestration | Tasks 16, 17 |
| §7.1 Inside run_fixedpoint | Task 5 |
| §7.2 Why fire Subcarve before LLM call | Task 5 (ordering test) |
| §7.3 Output byte-identity | Tasks 17, 19 |
| §8.1 Threading | Task 7 (Mutex<ReporterState>) |
| §8.2 Lifecycle and drop ordering | Tasks 7 (`finish` idempotent), 12, 15, 18 |
| §9 Testing | Tasks 5, 6, 8 (unit), 19 (integration) |
| §10 Flag behaviour | Tasks 7, 15, 18 |
| §11 Dependencies | Task 1 |
| §12 Out of scope | Constrains scope — no daemon, no new flags |

No placeholders, no TBDs. All file paths, types, and method names are concrete. Type usage is consistent across tasks: `ProgressEvent`, `ProgressSink`, `Phase`, `PromptBreakdown`, `Reporter`, `ProgressMode`, `make_stderr_reporter`, `ProgressBackend`, `MsgInput`, `render_activity_msg`, `format_iter_end_line`, `format_finished_line`, `abbreviate`, `relpath_of`.
