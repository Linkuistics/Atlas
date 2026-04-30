# Engine-side ProgressEvent + indicatif rendering

**Status:** design approved
**Date:** 2026-05-01

## 1. Problem

The progress line `atlas index` emits today (added in commit `6fc46b1`) is a
running cumulative tally of LLM calls — `[atlas] classify=47 surface=3 edges=2
subcarve=1 | tokens=18.4k/200k`. On a multi-hour or multi-day run against a
large codebase it answers none of the questions a watching user actually has:

1. **What is Atlas doing right now?** No current target.
2. **How far through this phase am I?** No `k/n` denominators.
3. **Is it still alive between LLM calls?** `ProgressBackend` is a decorator
   on `LlmBackend`, so it sits outside `LlmResponseCache` — fully-cached
   re-runs are completely silent, and no signal fires while the engine is
   doing Salsa work between LLM calls.

A pre-existing memory note
(`phase-level-progress-needs-engine-side-hooks-in-run-fixedpoint`) flags this
exact gap as deferred v1 work: meaningful progress requires the engine to
emit events from inside `run_fixedpoint`, not just decorate the LLM backend.

## 2. Goals

- Show the user a current target and a `k/n` denominator wherever both are
  known. **`k/n` is the headline number;** elapsed and ETA are secondary.
- Emit useful signal even on cached re-runs and between LLM calls.
- Keep `atlas-engine` free of presentation logic and free of any TUI
  dependency — events are structured data; the CLI formats them.
- Preserve the existing `--progress` / `--no-progress` flag semantics
  (Auto / Always / Never).
- Preserve byte-identical YAML output. The new demand pattern in the
  Project phase must produce the same `ComponentsFile` /
  `RelatedComponentsFile` as today.

## 3. Non-goals

- ETAs that try to predict whole-run time. Fixedpoint convergence depth is
  unpredictable and per-iteration cost varies wildly. ETA is enabled only
  during phases where both `n` is meaningful and a rolling rate is honest
  (subcarve loop, surface loop). Outside those, the bar shows elapsed only.
- Full ratatui-style TUI. `atlas index` is a batch operation, not a
  navigable workload — `indicatif`'s multi-bar rendering is the right
  level of richness without taking over the terminal in raw mode.
- Per-call instrumentation inside Salsa-tracked queries (L3 / L5 / L6 /
  L7). Salsa 0.26 doesn't expose a downcast from `&dyn salsa::Database` to
  `AtlasDatabase` (memory note
  `is_component_not_salsa_tracked_salsa_0_26_lacks_downcast`), and on
  cached re-runs those queries are memo hits anyway. The engine fires
  events at iteration and subcarve-loop boundaries; the LLM-side
  `ProgressBackend` covers per-call visibility on uncached calls.

## 4. Architecture

Three layers, each with one job.

```
┌────────────────────────────────────┐
│  atlas-engine                       │
│  ───────────                        │
│  pub trait ProgressSink (events)    │   new module: engine/src/progress.rs
│  pub enum  ProgressEvent            │
│                                     │
│  run_fixedpoint() fires:            │
│    IterStart, Subcarve, IterEnd     │
│  l5_surface_for() callable per-id   │   already exists (Salsa-tracked)
└──────────────┬──────────────────────┘
               │ Arc<dyn ProgressSink>
┌──────────────▼──────────────────────┐
│  atlas-cli/pipeline.rs               │
│  ────────────────────────            │
│  Owns the Reporter; passes it as     │
│  the ProgressSink to FixedpointConfig│
│  Fires Phase markers around each     │
│  step (Seed, Project::Surface k/n…)  │
└──────────────┬──────────────────────┘
               │ engine events  +  LLM-call taps
┌──────────────▼──────────────────────┐
│  atlas-cli/progress.rs (rewrite)     │
│  ──────────────────────              │
│  Reporter = MultiProgress + 2 bars   │
│   (activity bar, token gauge)        │
│  impl ProgressSink for Reporter      │
│  ProgressBackend taps LlmRequest →   │
│    extracts target → updates bar msg │
└─────────────────────────────────────┘
```

**Key principle:** the engine emits structured data only. The renderer is
purely a CLI concern. This keeps `atlas-engine` testable with a mock sink
and keeps the door open to swap the renderer later.

**Two parallel event sources, one sink.** The engine fires `ProgressEvent`
through `ProgressSink::on_event`. The existing `ProgressBackend` fires its
own LLM-call tap directly on the `Reporter` via a separate side-channel
method (e.g., `Reporter::on_llm_call(prompt_id, target)`). Funnelling LLM
calls through `ProgressEvent` would couple the engine to the LLM backend
for no benefit — the engine never directly sees the LLM call.

Both update the same `Reporter` state behind a single mutex.

## 5. Event vocabulary

In `crates/atlas-engine/src/progress.rs`:

```rust
pub trait ProgressSink: Send + Sync {
    fn on_event(&self, event: ProgressEvent);
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Indexing has started. Fired once, from pipeline.rs.
    Started { root: PathBuf },

    /// A high-level phase has begun. Fired by pipeline.rs at each
    /// step boundary; the engine itself never fires Phase events.
    Phase(Phase),

    /// A fixedpoint iteration has begun.
    /// `live_components` is the count we're about to iterate over.
    IterStart { iteration: u32, live_components: u64 },

    /// A subcarve decision is being made for one component. Fired
    /// from inside run_fixedpoint's `for id in live_ids` loop, BEFORE
    /// subcarve_decision() is called (so the bar updates while the
    /// caller is waiting on the LLM). `k` is 1-based. `relpath` is
    /// derived from `ComponentEntry::path_segments` at event-emission
    /// time (see §5.1) — the engine hands the renderer a
    /// ready-to-display PathBuf so the renderer doesn't need to know
    /// about path-segment internals.
    Subcarve { component_id: String, relpath: PathBuf, k: u64, n: u64 },

    /// A fixedpoint iteration has ended. `components_added` counts
    /// new (id, sub_dir) pairs merged into the back-edge during this
    /// iteration; 0 means the loop is about to exit.
    IterEnd { iteration: u32, components_added: u64, elapsed: Duration },

    /// L5 surface generation for one component. Fired by pipeline.rs
    /// during the Project phase as it demands surface_for(id) per
    /// component.
    Surface { component_id: String, relpath: PathBuf, k: u64, n: u64 },

    /// Indexing finished. Carries the final summary so the reporter
    /// can render the completion line + cumulative breakdown.
    Finished {
        components: u64,
        llm_calls: u64,
        tokens_used: u64,
        token_budget: Option<u64>,
        elapsed: Duration,
        breakdown: PromptBreakdown,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Seed,
    Fixedpoint,
    Project,    // covers L5/L6/L9 demand
    Edges,      // L6 batch — single marker, no k/n
}

#[derive(Debug, Clone, Default)]
pub struct PromptBreakdown {
    pub classify: u64,
    pub surface: u64,
    pub edges: u64,
    pub subcarve: u64,
}
```

### 5.1. `relpath` derivation

`ComponentEntry` (in `atlas-contracts/atlas-index/src/schema.rs`) carries
`path_segments: Vec<PathSegment>`, where each `PathSegment.path` is a
`PathBuf` "relative to `ComponentsFile::root`" (per the existing schema
doc-comment). The deepest segment — `path_segments.last()` — represents
the component's own directory.

The engine derives `relpath` for emission like so:

```rust
fn relpath_of(c: &ComponentEntry) -> PathBuf {
    c.path_segments
        .last()
        .map(|s| s.path.clone())
        .unwrap_or_default()
}
```

A component with no segments yields an empty `PathBuf`; the renderer
prints an empty target gracefully (`subcarve 47/120`).

**Notes:**

- `Subcarve` and `Surface` carry both `component_id` and `relpath` so the
  renderer doesn't need to look anything up — sink stays stateless.
- `IterEnd::elapsed` is per-iteration. The renderer uses it with
  `MultiProgress::println` to leave a permanent scrollback line per
  iteration, surviving the run.
- No `LlmCall` event in the enum — LLM calls flow through
  `ProgressBackend`'s side-channel.
- No `Stalled` event — `indicatif::enable_steady_tick` covers liveness.

**`FixedpointConfig` change** — additive field; default `None` keeps every
existing caller (engine tests, integration tests, evaluation harness)
compiling unchanged:

```rust
pub struct FixedpointConfig {
    pub max_depth: u32,
    pub hard_cap: u32,
    pub progress: Option<Arc<dyn ProgressSink>>,   // new
}
```

## 6. Rendering — indicatif layout

The reporter owns a `MultiProgress` with two bars.

```
[atlas] indexing /Users/antony/Development/some-large-repo

  ⠋ iter 1 · subcarve  47/120 (crates/atlas-engine)                          0:42  ETA 02:18
    tokens 18.4k/200.0k  ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  9%
```

- **Bar 1 — activity bar.** Spinner + `{msg}` + `{elapsed_precise}` +
  optional `{eta_precise}`. `enable_steady_tick(120ms)` so the spinner
  ticks even when no event has fired in 30 seconds.
- **Bar 2 — token gauge.** Position = `TokenCounter::used()`, length =
  budget. Hidden when no budget is set (`--no-budget`). Updated on every
  event the reporter receives.

### 6.1. Activity-bar message by phase

| Phase / event | Rendered `{msg}` |
|---|---|
| `Started` | `seed` |
| `IterStart{1, 47}` | `iter 1 · scanning 47 components` |
| `Subcarve{id, "crates/atlas-engine", 47, 120}` | `iter 1 · subcarve  47/120 (crates/atlas-engine)` |
| LLM-call tap mid-iter on classify | `iter 1 · classify (crates/foo)` then back to subcarve when next event fires |
| `Phase(Project)` | `project` |
| `Surface{id, "crates/atlas-cli", 12, 53}` | `project · surface  12/53 (crates/atlas-cli)` |
| `Phase(Edges)` | `project · edges (batch)` |

### 6.2. Iteration history via `println`

`IterEnd` triggers `MultiProgress::println("✓ iter 1 · 47 components · +0
sub-dirs · 14:32")` — writes a permanent line to scrollback above the
live bars.

After the run, scrollback shows:

```
[atlas] indexing /Users/antony/Development/some-large-repo
✓ iter 1 · 1247 components · +18 sub-dirs · 14:32
✓ iter 2 · 1265 components · +3 sub-dirs · 02:18
✓ iter 3 · 1268 components · +0 sub-dirs · 01:04   (converged)
✓ project · 1268 surfaces · 4:21
[atlas] done · 1268 components · 3892 LLM calls · 184k/200k tokens · 22:15
        classify=2715  surface=1268  edges=1  subcarve=908
```

### 6.3. Merging the LLM-call side-channel

When `ProgressBackend::call(req)` lands, it extracts `req.inputs` →
relpath (best-effort: `inputs["relpath"]` if string, else fall back to
component id, else fall back to nothing) and calls
`Reporter::on_llm_call(prompt_id, target)`. The reporter:

1. Increments `breakdown.{classify|surface|edges|subcarve}`.
2. Updates `state.last_llm_target = Some(target)`.
3. Re-renders the activity-bar msg.

**Engine k/n events take priority** when rendering the message. During a
long subcarve loop, the engine fires `Subcarve{47/120, …}` synchronously
before the LLM call lands. If the LLM-call tap then *overwrote* the
message with `subcarve (crates/foo)` (no k/n), we'd lose the denominator
that's the whole point of this design. So engine events set a "sticky"
k/n that LLM taps refresh-but-don't-clobber.

`Surface` events and L5 LLM calls describe the same work twice (engine
fires `Surface{12/53, "crates/atlas-cli"}` *and* `ProgressBackend` sees
the matching surface `LlmRequest` with relpath `crates/atlas-cli`). They
naturally merge: engine event sets `k/n + target`, LLM tap then
increments the cumulative `breakdown.surface` count. No double-counting.

ETA is enabled on the activity bar only during `Subcarve` and `Surface`
phases. Indicatif's `set_length(n)` + `set_position(k)` per event
activates `{eta_precise}`. Outside those phases we `set_length(0)` which
makes indicatif suppress ETA.

### 6.4. Non-TTY fallback

Indicatif auto-detects non-TTY stderr. We additionally set
`ProgressBar::set_draw_target(ProgressDrawTarget::hidden())` when
`--no-progress` (or `Auto` + non-TTY) is in effect. The `Reporter`
becomes a near-no-op for live drawing but still updates internal
`breakdown` counts, so the final `Finished` line is still printed via
plain `eprintln!`.

## 7. Pipeline orchestration

The full life of a single `atlas index` run, with every event-emission
site annotated. Sketch — not final code.

```rust
// crates/atlas-cli/src/pipeline.rs (excerpt)
pub fn run_index(config: IndexConfig) -> Result<IndexSummary, IndexError> {
    let reporter: Arc<Reporter> = Reporter::new(...);   // owns MultiProgress
    let sink: Arc<dyn ProgressSink> = reporter.clone();

    sink.on_event(Started { root: config.root.clone() });
    sink.on_event(Phase(Phase::Seed));

    // ── seed ────────────────────────────────────────────
    let mut db = AtlasDatabase::new(...);
    cache_io::load_into(&cache_path, db.llm_cache());
    seed_filesystem(&mut db, &config.root, config.respect_gitignore)?;
    db.set_prior_components(...);  // etc.

    // ── fixedpoint (engine fires all inner events) ──────
    sink.on_event(Phase(Phase::Fixedpoint));
    let fp_config = FixedpointConfig {
        max_depth: config.max_depth,
        progress: Some(sink.clone()),
        ..FixedpointConfig::default()
    };
    let fp_result = run_fixedpoint(&mut db, fp_config);

    // ── project ─────────────────────────────────────────
    sink.on_event(Phase(Phase::Project));
    let live_components: Vec<ComponentEntry> = all_components(&db)
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();
    let n = live_components.len() as u64;
    for (i, comp) in live_components.iter().enumerate() {
        sink.on_event(Surface {
            component_id: comp.id.clone(),
            relpath: relpath_of(comp),
            k: (i as u64) + 1,
            n,
        });
        // Demand the per-component surface entry point. surface_of is
        // not #[salsa::tracked] but routes through LlmResponseCache, so
        // per-id demand still primes the cache that the eventual
        // snapshot reads from. Output remains byte-identical (§7.3).
        let _ = surface_of(&db, comp.id.clone());
    }
    sink.on_event(Phase(Phase::Edges));   // L6 batch — single marker
    let _ = related_components_yaml_snapshot(&db);   // batch, no inner k/n

    // Final aggregation now that L5/L6 caches are warm:
    let components_file = components_yaml_snapshot_with_prompt_shas(&db, ...);
    let externals_file = external_components_yaml_snapshot(&db);
    let related_file = related_components_yaml_snapshot(&db);   // cache-hit
    // ... stable_generated_at, write atomically, etc.

    sink.on_event(Finished { components: ..., elapsed: ..., breakdown: ... });
    reporter.finish();   // tears down MultiProgress, prints summary
    Ok(summary)
}
```

### 7.1. Inside `run_fixedpoint`

Conditional event emissions only — no behavioural change when `progress`
is `None`. Sketch:

```rust
// crates/atlas-engine/src/fixedpoint.rs (excerpt)
pub fn run_fixedpoint(db: &mut AtlasDatabase, config: FixedpointConfig) -> FixedpointResult {
    let sink = config.progress.clone();
    db.set_max_depth(config.max_depth);
    db.set_fixedpoint_iteration_count(0);

    let mut back_edge: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    db.set_carve_back_edge(back_edge.clone());

    let mut iterations = 0u32;
    loop {
        let iter_started = Instant::now();
        let components = all_components(db);
        let live: Vec<(String, PathBuf)> = components
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| (c.id.clone(), relpath_of(c)))
            .collect();
        drop(components);

        if let Some(s) = &sink {
            s.on_event(IterStart {
                iteration: iterations,
                live_components: live.len() as u64,
            });
        }

        let n = live.len() as u64;
        let mut added = 0u64;
        let mut changed = false;
        for (k, (id, relpath)) in live.iter().enumerate() {
            if let Some(s) = &sink {
                s.on_event(Subcarve {
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
                if !entry.iter().any(|e| e == &sub) {
                    entry.push(sub);
                    added += 1;
                    changed = true;
                }
            }
        }

        if let Some(s) = &sink {
            s.on_event(IterEnd {
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
        if iterations >= config.hard_cap { panic!(...); }
    }
}
```

### 7.2. Why fire `Subcarve` *before* the LLM call

The current `ProgressBackend` records *after* the inner backend returns
(so the token counter is up to date). For engine `Subcarve` events the
opposite is right: emit *before* the call, so the activity bar shows
"subcarve 47/120 (crates/atlas-engine)" *while* the LLM call is in
flight (potentially 30+ seconds on a slow model). Otherwise the user
sees `46/120 (prev_target)` until the call returns — the wrong direction
of staleness.

### 7.3. Output byte-identity

The L5 demand loop in `pipeline.rs` is the only behaviour change in the
projection step. `surface_of` is not `#[salsa::tracked]`, but it routes
LLM calls through `AtlasDatabase::call_llm_cached` which consults
`LlmResponseCache` keyed by `(LlmFingerprint, PromptId,
canonical-JSON(inputs))`. Each loop iteration therefore does at most one
LLM call (cache-miss) or zero (cache-hit). The terminal
`components_yaml_snapshot_with_prompt_shas` call resolves the same
component surfaces and hits the now-populated cache, producing the same
`ComponentsFile` it would have produced without the loop. Output is
byte-identical to today; we've just reordered the demand pattern.

## 8. Threading, lifecycle, errors

### 8.1. Threading

`ProgressSink: Send + Sync`. Salsa may parallelise; the sink's
`on_event` must be safe to call from any thread. `Reporter` puts all
mutable state behind a single `Mutex<ReporterState>`. Indicatif's bars
are themselves `Send + Sync`, so we don't need a second mutex around
them.

A poisoned mutex is recovered (`unwrap_or_else(PoisonError::into_inner)`)
— progress is best-effort, never load-bearing for correctness.

### 8.2. Lifecycle and drop ordering

Happy path:

1. `Reporter::new()` constructs `MultiProgress`, adds bars, calls
   `enable_steady_tick` on the activity bar.
2. Pipeline runs; events fire.
3. `Finished` event prints the summary line via
   `MultiProgress::println` and the breakdown line below.
4. `reporter.finish()` calls `MultiProgress::clear()` to remove live
   bars (the `println` summary stays in scrollback).

Panic mid-run (fixedpoint hard cap or `BudgetExhausted`):

1. `pipeline.rs` already wraps `run_fixedpoint` in `catch_unwind`. The
   panic unwinds out of the engine; the reporter's mutex may end up
   poisoned, but no `Finished` event fires.
2. `pipeline.rs` drops the reporter as it returns the error. Drop
   order: bars cleared by `MultiProgress::Drop`, terminal state
   restored.
3. The CLI's outer error handler prints the error to stderr after the
   bars are gone — no garbled output.

## 9. Testing

**Engine side** — `crates/atlas-engine/src/fixedpoint.rs` tests use a
test-only `RecordingSink` that pushes every event into an
`Arc<Mutex<Vec<ProgressEvent>>>`. Add a test alongside the existing
three asserting the event sequence on a small fixture (one component,
no subcarve, converges immediately):
`[IterStart{0, 1}, Subcarve{_, 1, 1}, IterEnd{0, 0, _}]`. The
`PathologicalBackend` test gets a parallel assertion that `IterStart{n}`
fires for each iteration up to the hard cap.

**CLI side** — the existing `progress.rs` line-comparison tests are
replaced. New tests use indicatif's `ProgressDrawTarget::hidden()`, then
assert on:

- `Reporter` state after a synthetic event sequence (counts in
  `breakdown`, current msg via a test-only `current_msg()` accessor).
- `render_activity_msg` for each phase variant (pure function,
  easy unit-test surface).
- LLM-call taps don't clobber engine k/n state (the priority rule from
  §6.3).

## 10. Flag behaviour and CLI surface

Existing `--progress` / `--no-progress` flags (Auto / Always / Never)
are kept unchanged. The trinary maps onto indicatif:

- `Always` → `MultiProgress::with_draw_target(ProgressDrawTarget::stderr())`.
- `Auto` + TTY → same as `Always`.
- `Never`, or `Auto` + non-TTY →
  `ProgressDrawTarget::hidden()`. The reporter still receives events
  and updates `breakdown`, so the final `Finished` line is still
  printed via plain `eprintln!` (giving CI logs and `2> file` redirects
  a meaningful end-of-run summary).

Steady-tick is disabled when the draw target is hidden, both to avoid
wasted work and because there's nothing to draw.

## 11. Dependencies

- `indicatif` added to `crates/atlas-cli/Cargo.toml` only.
- `atlas-engine` stays free of indicatif and of any TUI machinery. The
  `ProgressSink` trait + `ProgressEvent` enum is the entire engine-side
  public surface for progress.

## 12. Out of scope

- ETA estimates that try to predict whole-run time (see §3).
- Per-call instrumentation inside Salsa-tracked queries (see §3).
- A separate `atlas progress` subcommand or a long-running daemon — this
  spec is exclusively about `atlas index`.
- Changes to `--progress` flag semantics or new flags. Existing
  Auto/Always/Never trinary covers what's needed.
