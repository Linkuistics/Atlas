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
use atlas_llm::{
    AgentEvent, AgentObserver, LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId,
    TokenCounter,
};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct ReporterState {
    breakdown: PromptBreakdown,
    last_msg: String,
    iter_history: Vec<String>,
    summary: Option<String>,
    /// k/n set by the most recent engine-level Subcarve/Surface event.
    /// LLM-call taps must not clobber this — they may only refresh the
    /// target relpath, so the denominator from the engine wins.
    sticky_kn: Option<(u64, u64, &'static str)>,
    last_llm_target: Option<PathBuf>,
    /// `live_components` from the most recent `IterStart`. Used to
    /// build the scrollback line on `IterEnd`.
    iter_live: u64,
    last_iteration: u32,
    /// Counter incremented per `AgentEvent::ToolUse`, reset on
    /// `AgentEvent::CallStart`.
    agent_tools: u64,
    /// `(name, summary)` of the most recent `AgentEvent::ToolUse`.
    agent_last_tool: Option<(String, String)>,
    /// Sticky flag set by `AgentEvent::ToolResult { ok: false }`.
    /// Cleared by the next `AgentEvent::ToolUse` so the `(✗)` marker
    /// only persists until the agent moves on.
    agent_last_failed: bool,
    /// Whether the agent bar is currently mounted (between `CallStart`
    /// and `CallEnd`). Tracked explicitly because indicatif's
    /// `is_hidden()` also returns true when stderr is not a TTY, which
    /// is true in test runs even after we attach a stderr draw target.
    agent_mounted: bool,
}

pub struct Reporter {
    multi: MultiProgress,
    activity: ProgressBar,
    agent: ProgressBar,
    tokens: ProgressBar,
    state: Mutex<ReporterState>,
    counter: Option<Arc<TokenCounter>>,
    drawing: bool,
}

impl Reporter {
    pub fn new(mode: ProgressMode, counter: Option<Arc<TokenCounter>>) -> Arc<Self> {
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
        let agent = multi.add(ProgressBar::new(0));
        agent.set_style(ProgressStyle::with_template("      {msg}").expect("static template"));
        agent.set_draw_target(ProgressDrawTarget::hidden());

        let tokens = multi.add(ProgressBar::new(0));
        tokens.set_style(
            ProgressStyle::with_template("    tokens {msg}  {bar:50}  {percent:>3}%")
                .expect("static template"),
        );

        Arc::new(Self {
            multi,
            activity,
            agent,
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

    /// Snapshot of the current per-prompt counters. Consumed by the
    /// pipeline when constructing the `Finished` event.
    pub fn breakdown_snapshot(&self) -> PromptBreakdown {
        self.lock().breakdown.clone()
    }

    /// Side-channel called by `ProgressBackend` when an LLM call lands.
    /// Increments the per-prompt counter and, when no engine-set k/n
    /// is sticky, refreshes the activity message with the call target
    /// (spec §6.3).
    pub fn on_llm_call(&self, prompt: PromptId, target: Option<PathBuf>) {
        self.refresh_token_gauge();
        let sticky_active;
        {
            let mut s = self.lock();
            match prompt {
                PromptId::Classify => s.breakdown.classify += 1,
                PromptId::Stage1Surface => s.breakdown.surface += 1,
                PromptId::Stage2Edges => s.breakdown.edges += 1,
                PromptId::Subcarve => s.breakdown.subcarve += 1,
            }
            s.last_llm_target = target.clone();
            sticky_active = s.sticky_kn.is_some();
        }
        if !sticky_active {
            self.set_msg(MsgInput::LlmTap {
                iteration: None,
                prompt,
                target,
            });
        }
    }

    fn set_msg(&self, input: MsgInput) {
        let rendered = render_activity_msg(&input);
        {
            let mut s = self.lock();
            s.last_msg = rendered.clone();
        }
        self.activity.set_message(rendered);
    }

    fn refresh_token_gauge(&self) {
        let Some(c) = self.counter.as_ref() else {
            return;
        };
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
    pub fn drawing(&self) -> bool {
        self.drawing
    }
    #[cfg(test)]
    pub(crate) fn agent_visible(&self) -> bool {
        self.lock().agent_mounted
    }
    #[cfg(test)]
    pub(crate) fn agent_tools(&self) -> u64 {
        self.lock().agent_tools
    }
    #[cfg(test)]
    pub(crate) fn agent_msg(&self) -> String {
        self.agent.message().to_string()
    }
}

impl ProgressSink for Reporter {
    fn on_event(&self, event: ProgressEvent) {
        self.refresh_token_gauge();
        match event {
            ProgressEvent::Started { .. } => {
                self.set_msg(MsgInput::Started);
            }
            ProgressEvent::Phase(p) => {
                self.set_msg(MsgInput::Phase(p));
            }
            ProgressEvent::IterStart {
                iteration,
                live_components,
            } => {
                {
                    let mut s = self.lock();
                    s.iter_live = live_components;
                    s.last_iteration = iteration;
                }
                // Suppress ETA outside k/n phases by zeroing length.
                self.activity.set_length(0);
                self.set_msg(MsgInput::IterStart {
                    iteration,
                    live: live_components,
                });
            }
            ProgressEvent::IterEnd {
                iteration,
                components_added,
                elapsed,
            } => {
                let live = self.lock().iter_live;
                let line = format_iter_end_line(iteration, live, components_added, elapsed);
                let _ = self.multi.println(&line);
                self.lock().iter_history.push(line);
            }
            ProgressEvent::Subcarve {
                component_id: _,
                relpath,
                k,
                n,
            } => {
                let iteration = {
                    let mut s = self.lock();
                    s.sticky_kn = Some((k, n, "subcarve"));
                    s.last_iteration
                };
                self.activity.set_length(n);
                self.activity.set_position(k);
                self.set_msg(MsgInput::Subcarve {
                    iteration,
                    k,
                    n,
                    target: relpath,
                });
            }
            ProgressEvent::Surface {
                component_id: _,
                relpath,
                k,
                n,
            } => {
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
        }
    }
}

impl AgentObserver for Reporter {
    fn on_event(&self, event: AgentEvent) {
        match event {
            AgentEvent::CallStart { prompt } => {
                {
                    let mut s = self.lock();
                    s.agent_tools = 0;
                    s.agent_last_tool = None;
                    s.agent_last_failed = false;
                    s.agent_mounted = self.drawing;
                }
                self.agent.set_draw_target(if self.drawing {
                    ProgressDrawTarget::stderr()
                } else {
                    ProgressDrawTarget::hidden()
                });
                self.agent
                    .set_message(format!("↳ starting {}", prompt_label(prompt)));
            }
            AgentEvent::ToolUse { name, summary } => {
                let line = {
                    let mut s = self.lock();
                    s.agent_tools = s.agent_tools.saturating_add(1);
                    s.agent_last_tool = Some((name, summary));
                    s.agent_last_failed = false;
                    render_agent_line(&s)
                };
                self.agent.set_message(line);
            }
            AgentEvent::ToolResult { ok } => {
                if !ok {
                    let line = {
                        let mut s = self.lock();
                        s.agent_last_failed = true;
                        render_agent_line(&s)
                    };
                    self.agent.set_message(line);
                }
            }
            AgentEvent::CallEnd => {
                self.lock().agent_mounted = false;
                self.agent.set_draw_target(ProgressDrawTarget::hidden());
            }
        }
    }
}

fn prompt_label(prompt: PromptId) -> &'static str {
    match prompt {
        PromptId::Classify => "classify",
        PromptId::Subcarve => "subcarve",
        PromptId::Stage1Surface => "surface",
        PromptId::Stage2Edges => "edges",
    }
}

const AGENT_LINE_DEFAULT_WIDTH: usize = 120;

pub(crate) fn render_agent_line(state: &ReporterState) -> String {
    render_agent_line_with_width(state, AGENT_LINE_DEFAULT_WIDTH)
}

/// Render the agent sub-line within `max_width` characters. The summary
/// (the tool argument) is the only part shortened with a trailing
/// ellipsis; the tool name, counter, and any `(✗)` marker are always
/// preserved (spec §3).
pub(crate) fn render_agent_line_with_width(state: &ReporterState, max_width: usize) -> String {
    let (name, summary) = state.agent_last_tool.clone().unwrap_or_default();
    let suffix = if state.agent_last_failed {
        " (✗)"
    } else {
        ""
    };
    let counter_part = format!(" · {} tools{}", state.agent_tools, suffix);
    let prefix_no_summary = format!("↳ {name}");
    if summary.is_empty() {
        return format!("{prefix_no_summary}{counter_part}");
    }
    let fixed_len = prefix_no_summary.chars().count() + 1 + counter_part.chars().count();
    if fixed_len + summary.chars().count() <= max_width {
        return format!("{prefix_no_summary} {summary}{counter_part}");
    }
    let budget = max_width.saturating_sub(fixed_len + 1);
    let kept: String = summary.chars().take(budget).collect();
    format!("{prefix_no_summary} {kept}…{counter_part}")
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

#[derive(Debug, Clone)]
pub(crate) enum MsgInput {
    Started,
    Phase(Phase),
    IterStart {
        iteration: u32,
        live: u64,
    },
    Subcarve {
        iteration: u32,
        k: u64,
        n: u64,
        target: PathBuf,
    },
    Surface {
        k: u64,
        n: u64,
        target: PathBuf,
    },
    LlmTap {
        iteration: Option<u32>,
        prompt: PromptId,
        target: Option<PathBuf>,
    },
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
        MsgInput::Subcarve {
            iteration,
            k,
            n,
            target,
        } => {
            if target.as_os_str().is_empty() {
                format!("iter {iteration} · subcarve  {k}/{n}")
            } else {
                format!(
                    "iter {iteration} · subcarve  {k}/{n} ({})",
                    target.display()
                )
            }
        }
        MsgInput::Surface { k, n, target } => {
            if target.as_os_str().is_empty() {
                format!("project · surface  {k}/{n}")
            } else {
                format!("project · surface  {k}/{n} ({})", target.display())
            }
        }
        MsgInput::LlmTap {
            iteration,
            prompt,
            target,
        } => {
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

fn format_iter_end_line(iteration: u32, live: u64, added: u64, elapsed: Duration) -> String {
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    format!("✓ iter {iteration} · {live} components · +{added} sub-dirs · {mins:02}:{secs:02}")
}

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
        c = bd.classify,
        s = bd.surface,
        e = bd.edges,
        sc = bd.subcarve,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- Task 7: skeleton smoke tests ---

    #[test]
    fn make_stderr_reporter_returns_reporter_in_never_mode() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        assert!(!r.drawing);
    }

    #[test]
    fn finish_is_idempotent() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        r.finish();
        r.finish();
    }

    // --- Task 8: render_activity_msg coverage ---

    #[test]
    fn render_activity_msg_started() {
        assert_eq!(render_activity_msg(&MsgInput::Started), "seed");
    }

    #[test]
    fn render_activity_msg_iter_start_shows_scanning_count() {
        let m = MsgInput::IterStart {
            iteration: 1,
            live: 47,
        };
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
        assert_eq!(
            render_activity_msg(&MsgInput::Phase(Phase::Project)),
            "project"
        );
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
        let m = MsgInput::Surface {
            k: 12,
            n: 53,
            target: PathBuf::from("crates/atlas-cli"),
        };
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

    // --- Task 9: Started / Phase ---

    #[test]
    fn reporter_started_event_sets_seed_msg() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Started {
                root: PathBuf::from("/tmp/x"),
            },
        );
        assert_eq!(r.current_msg(), "seed");
    }

    #[test]
    fn reporter_phase_event_updates_msg() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Project));
        assert_eq!(r.current_msg(), "project");
    }

    // --- Task 10: IterStart / IterEnd + scrollback ---

    #[test]
    fn reporter_iter_start_sets_scanning_msg_and_resets_length() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::IterStart {
                iteration: 1,
                live_components: 47,
            },
        );
        assert_eq!(r.current_msg(), "iter 1 · scanning 47 components");
    }

    #[test]
    fn reporter_iter_end_appends_scrollback_line() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::IterStart {
                iteration: 1,
                live_components: 1247,
            },
        );
        ProgressSink::on_event(
            &*r,
            ProgressEvent::IterEnd {
                iteration: 1,
                components_added: 18,
                elapsed: Duration::from_secs(872),
            },
        );
        let history = r.iter_history();
        assert_eq!(history.len(), 1);
        assert!(history[0].contains("iter 1"));
        assert!(history[0].contains("1247 components"));
        assert!(history[0].contains("+18 sub-dirs"));
        assert!(history[0].contains("14:32"));
    }

    // --- Task 11: Subcarve / Surface (sticky k/n) ---

    #[test]
    fn reporter_subcarve_sets_kn_msg_and_progress_length() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::IterStart {
                iteration: 1,
                live_components: 120,
            },
        );
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Subcarve {
                component_id: "c".into(),
                relpath: PathBuf::from("crates/atlas-engine"),
                k: 47,
                n: 120,
            },
        );
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
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Project));
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Surface {
                component_id: "c".into(),
                relpath: PathBuf::from("crates/atlas-cli"),
                k: 12,
                n: 53,
            },
        );
        assert_eq!(
            r.current_msg(),
            "project · surface  12/53 (crates/atlas-cli)"
        );
    }

    // --- Task 12: Finished ---

    #[test]
    fn reporter_finished_records_summary_and_breakdown() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Finished {
                components: 1268,
                llm_calls: 3892,
                tokens_used: 184_000,
                token_budget: Some(200_000),
                elapsed: Duration::from_secs(22 * 60 + 15),
                breakdown: PromptBreakdown {
                    classify: 2715,
                    surface: 1268,
                    edges: 1,
                    subcarve: 908,
                },
            },
        );
        let summary = r.summary().expect("summary set after Finished");
        assert!(summary.contains("done"));
        assert!(summary.contains("1268 components"));
        assert!(summary.contains("3892 LLM calls"));
        assert!(summary.contains("184.0k/200.0k tokens"));
        assert!(summary.contains("22:15"));
        assert!(summary.contains("classify=2715"));
        assert!(summary.contains("subcarve=908"));
    }

    // --- Task 13: on_llm_call sticky priority ---

    #[test]
    fn on_llm_call_increments_breakdown_and_does_not_clobber_sticky_kn() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(
            &*r,
            ProgressEvent::IterStart {
                iteration: 1,
                live_components: 120,
            },
        );
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Subcarve {
                component_id: "c".into(),
                relpath: PathBuf::from("crates/atlas-engine"),
                k: 47,
                n: 120,
            },
        );
        r.on_llm_call(PromptId::Classify, Some(PathBuf::from("crates/foo")));
        assert_eq!(r.breakdown().classify, 1);
        assert_eq!(
            r.current_msg(),
            "iter 1 · subcarve  47/120 (crates/atlas-engine)"
        );
    }

    #[test]
    fn on_llm_call_without_sticky_kn_falls_through_to_llm_msg() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Seed));
        r.on_llm_call(PromptId::Classify, Some(PathBuf::from("crates/foo")));
        assert_eq!(r.current_msg(), "classify (crates/foo)");
    }

    // --- Task 14: token gauge ---

    #[test]
    fn token_gauge_updates_from_counter_on_each_event() {
        let counter = Arc::new(TokenCounter::new(200_000));
        counter.charge(18_400).unwrap();
        let r = make_stderr_reporter(ProgressMode::Never, Some(counter.clone()));
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Fixedpoint));
        assert_eq!(r.tokens_length(), Some(200_000));
        assert_eq!(r.tokens_position(), 18_400);
        counter.charge(1_600).unwrap();
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Project));
        assert_eq!(r.tokens_position(), 20_000);
    }

    #[test]
    fn token_gauge_hidden_when_no_counter() {
        let r = make_stderr_reporter(ProgressMode::Never, None);
        ProgressSink::on_event(&*r, ProgressEvent::Phase(Phase::Fixedpoint));
        assert_eq!(r.tokens_length(), Some(0));
    }

    // --- Task 15: ProgressMode mapping + finish lifecycle ---

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
        ProgressSink::on_event(
            &*r,
            ProgressEvent::Finished {
                components: 1,
                llm_calls: 0,
                tokens_used: 0,
                token_budget: None,
                elapsed: Duration::from_secs(1),
                breakdown: PromptBreakdown::default(),
            },
        );
        r.finish();
    }

    // --- Task 8: agent bar starts hidden ---

    #[test]
    fn reporter_starts_with_agent_bar_hidden_and_zero_tools() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        assert!(!r.agent_visible());
        assert_eq!(r.agent_tools(), 0);
    }

    // --- Task 9: AgentObserver impl on Reporter ---

    #[test]
    fn reporter_call_start_makes_agent_bar_visible_and_resets_counters() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        {
            let mut s = r.lock();
            s.agent_tools = 5;
            s.agent_last_tool = Some(("Read".into(), "/x".into()));
            s.agent_last_failed = true;
        }
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        assert!(r.agent_visible());
        assert_eq!(r.agent_tools(), 0);
        let s = r.lock();
        assert!(s.agent_last_tool.is_none());
        assert!(!s.agent_last_failed);
    }

    #[test]
    fn reporter_tool_use_increments_counter_and_renders_line() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::ToolUse {
                name: "Read".into(),
                summary: "crates/atlas-engine/src/l8_recurse.rs".into(),
            },
        );
        assert_eq!(r.agent_tools(), 1);
        assert_eq!(
            r.agent_msg(),
            "↳ Read crates/atlas-engine/src/l8_recurse.rs · 1 tools"
        );
    }

    #[test]
    fn reporter_tool_result_failure_marks_line_with_cross() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::ToolUse {
                name: "Read".into(),
                summary: "/tmp/x".into(),
            },
        );
        AgentObserver::on_event(r.as_ref(), AgentEvent::ToolResult { ok: false });
        assert!(
            r.agent_msg().ends_with("(✗)"),
            "expected (✗) marker; got {:?}",
            r.agent_msg()
        );
    }

    #[test]
    fn reporter_tool_result_success_does_not_change_line() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::ToolUse {
                name: "Read".into(),
                summary: "/tmp/x".into(),
            },
        );
        let before = r.agent_msg();
        AgentObserver::on_event(r.as_ref(), AgentEvent::ToolResult { ok: true });
        assert_eq!(r.agent_msg(), before);
    }

    #[test]
    fn reporter_subsequent_tool_use_clears_failure_marker() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::ToolUse {
                name: "Read".into(),
                summary: "/x".into(),
            },
        );
        AgentObserver::on_event(r.as_ref(), AgentEvent::ToolResult { ok: false });
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::ToolUse {
                name: "Grep".into(),
                summary: "foo".into(),
            },
        );
        assert!(!r.agent_msg().contains("(✗)"));
    }

    #[test]
    fn reporter_call_end_hides_agent_bar() {
        let r = make_stderr_reporter(ProgressMode::Always, None);
        AgentObserver::on_event(
            r.as_ref(),
            AgentEvent::CallStart {
                prompt: PromptId::Subcarve,
            },
        );
        AgentObserver::on_event(r.as_ref(), AgentEvent::CallEnd);
        assert!(!r.agent_visible());
    }

    // --- Task 10: render_agent_line_with_width truncation ---

    #[test]
    fn render_agent_line_truncates_long_summary_with_ellipsis() {
        let s = ReporterState {
            agent_tools: 23,
            agent_last_tool: Some((
                "Read".into(),
                "crates/atlas-engine/src/very/deep/path/that/keeps/going/and/going/l8_recurse.rs"
                    .into(),
            )),
            ..ReporterState::default()
        };
        let out = render_agent_line_with_width(&s, 60);
        assert!(out.starts_with("↳ Read "), "tool name preserved: {out:?}");
        assert!(out.ends_with("· 23 tools"), "counter preserved: {out:?}");
        assert!(out.contains('…'), "summary ellipsised: {out:?}");
        assert!(
            out.chars().count() <= 60,
            "line within budget: {} > 60",
            out.chars().count()
        );
    }

    #[test]
    fn render_agent_line_does_not_truncate_short_summary() {
        let s = ReporterState {
            agent_tools: 1,
            agent_last_tool: Some(("Read".into(), "/x.rs".into())),
            ..ReporterState::default()
        };
        let out = render_agent_line_with_width(&s, 120);
        assert_eq!(out, "↳ Read /x.rs · 1 tools");
    }

    #[test]
    fn render_agent_line_truncation_keeps_failure_marker() {
        let s = ReporterState {
            agent_tools: 5,
            agent_last_tool: Some((
                "Read".into(),
                "very/long/path/that/will/get/truncated/aaaaaaaaaaaaaaaaaaaaaa.rs".into(),
            )),
            agent_last_failed: true,
            ..ReporterState::default()
        };
        let out = render_agent_line_with_width(&s, 50);
        assert!(out.ends_with("(✗)"), "failure marker preserved: {out:?}");
        assert!(out.contains('…'), "summary ellipsised: {out:?}");
    }
}
