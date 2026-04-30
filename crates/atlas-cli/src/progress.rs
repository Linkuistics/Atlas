//! Progress reporting for `atlas index`.
//!
//! [`ProgressBackend`] is a thin decorator around any [`LlmBackend`].
//! After each forwarded call it taps a shared [`ProgressReporter`],
//! which writes a one-line running tally to a sink (stderr, in
//! production). The reporter sees only un-cached calls, because
//! `AtlasDatabase::call_llm_cached` consults the in-memory cache
//! before reaching the backend — so the tally reflects real work
//! rather than memo hits, and a fully-cached re-run is silent.
//!
//! When the sink is a TTY the line is rewritten in place via `\r`;
//! when it is not (CI logs, file redirection), each update is appended
//! as its own line so the output remains grep-friendly.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId, TokenCounter};
use serde_json::Value;

/// Triple-state mapped from `--progress` / `--no-progress` /
/// (neither): `Auto` enables progress when stderr is a TTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    classify: u64,
    surface: u64,
    edges: u64,
    subcarve: u64,
}

impl Counts {
    fn record(&mut self, id: PromptId) {
        match id {
            PromptId::Classify => self.classify += 1,
            PromptId::Stage1Surface => self.surface += 1,
            PromptId::Stage2Edges => self.edges += 1,
            PromptId::Subcarve => self.subcarve += 1,
        }
    }

    fn total(&self) -> u64 {
        self.classify + self.surface + self.edges + self.subcarve
    }
}

struct ReporterState {
    counts: Counts,
    sink: Box<dyn Write + Send>,
    inline: bool,
    counter: Option<Arc<TokenCounter>>,
    last_line_len: usize,
}

/// Shared state behind a mutex so the [`LlmBackend`] (which is
/// `Send + Sync`) can call `record` from any thread Salsa decides
/// to evaluate on.
pub struct ProgressReporter {
    state: Mutex<ReporterState>,
}

impl ProgressReporter {
    pub fn new(
        sink: Box<dyn Write + Send>,
        inline: bool,
        counter: Option<Arc<TokenCounter>>,
    ) -> Self {
        Self {
            state: Mutex::new(ReporterState {
                counts: Counts::default(),
                sink,
                inline,
                counter,
                last_line_len: 0,
            }),
        }
    }

    /// Print a one-time start line so the user sees something the
    /// moment indexing begins, before the first LLM call lands.
    pub fn announce_start(&self, root: &Path) {
        let mut s = self.lock();
        let _ = writeln!(s.sink, "[atlas] indexing {}", root.display());
        let _ = s.sink.flush();
    }

    /// Increment counters for `id` and emit an updated tally. Called
    /// by [`ProgressBackend`] after the inner backend returns.
    pub fn record(&self, id: PromptId) {
        let mut s = self.lock();
        s.counts.record(id);
        let line = render_line(&s.counts, s.counter.as_deref());
        if s.inline {
            // Pad with spaces to overwrite a longer previous line, then
            // park the cursor at column 0 so the next overwrite starts
            // cleanly.
            let pad = s.last_line_len.saturating_sub(line.len());
            let _ = write!(s.sink, "\r{line}{:pad$}", "", pad = pad);
            s.last_line_len = line.len();
        } else {
            let _ = writeln!(s.sink, "{line}");
        }
        let _ = s.sink.flush();
    }

    /// Terminate any in-place line so subsequent stderr output starts
    /// on a fresh line. Safe to call when nothing was reported.
    pub fn finish(&self) {
        let mut s = self.lock();
        if s.inline && s.counts.total() > 0 {
            let _ = writeln!(s.sink);
        }
        let _ = s.sink.flush();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ReporterState> {
        // A poisoned mutex means an earlier panic mid-write — recover
        // and keep going; progress is best-effort.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Build a reporter for stderr based on `mode`. Returns `None` when
/// progress should be disabled (`Never`, or `Auto` with non-TTY
/// stderr); the caller skips wrapping the backend in that case.
pub fn make_stderr_reporter(
    mode: ProgressMode,
    counter: Option<Arc<TokenCounter>>,
) -> Option<Arc<ProgressReporter>> {
    let stderr = std::io::stderr();
    let stderr_is_tty = stderr.is_terminal();
    let enabled = match mode {
        ProgressMode::Auto => stderr_is_tty,
        ProgressMode::Always => true,
        ProgressMode::Never => false,
    };
    if !enabled {
        return None;
    }
    let sink: Box<dyn Write + Send> = Box::new(stderr);
    Some(Arc::new(ProgressReporter::new(
        sink,
        stderr_is_tty,
        counter,
    )))
}

fn render_line(counts: &Counts, counter: Option<&TokenCounter>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);
    if counts.classify > 0 {
        parts.push(format!("classify={}", counts.classify));
    }
    if counts.surface > 0 {
        parts.push(format!("surface={}", counts.surface));
    }
    if counts.edges > 0 {
        parts.push(format!("edges={}", counts.edges));
    }
    if counts.subcarve > 0 {
        parts.push(format!("subcarve={}", counts.subcarve));
    }
    if parts.is_empty() {
        parts.push("starting".to_string());
    }

    let tokens = match counter {
        Some(c) => format!(
            " | tokens={}/{}",
            abbreviate(c.used()),
            abbreviate(c.budget())
        ),
        None => String::new(),
    };
    format!("[atlas] {}{}", parts.join(" "), tokens)
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

/// Decorator backend: forwards every call to `inner`, then taps the
/// reporter. Recording happens after the inner call returns so the
/// `TokenCounter` already reflects this call's cost (the budget
/// wrapper charges post-return).
pub struct ProgressBackend {
    inner: Arc<dyn LlmBackend>,
    reporter: Arc<ProgressReporter>,
}

impl ProgressBackend {
    pub fn new(inner: Arc<dyn LlmBackend>, reporter: Arc<ProgressReporter>) -> Arc<Self> {
        Arc::new(Self { inner, reporter })
    }
}

impl LlmBackend for ProgressBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let result = self.inner.call(req);
        // Record on attempt, not just success — a failed call still
        // represents work the user is waiting through.
        self.reporter.record(req.prompt_template);
        result
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.inner.fingerprint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_llm::{ResponseSchema, TestBackend};

    /// `Vec<u8>` writer wrapped so a clone of the Arc lets the test
    /// inspect what was written without consuming the sink.
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn fresh_sink() -> (Arc<Mutex<Vec<u8>>>, Box<dyn Write + Send>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = SharedSink(Arc::clone(&buf));
        (buf, Box::new(sink))
    }

    fn dump(buf: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn abbreviate_uses_thousands_above_threshold() {
        assert_eq!(abbreviate(0), "0");
        assert_eq!(abbreviate(9_999), "9999");
        assert_eq!(abbreviate(10_000), "10.0k");
        assert_eq!(abbreviate(18_400), "18.4k");
        assert_eq!(abbreviate(2_500_000), "2.50M");
    }

    #[test]
    fn render_line_omits_zero_counters() {
        let counts = Counts {
            classify: 47,
            surface: 12,
            ..Counts::default()
        };
        let line = render_line(&counts, None);
        assert_eq!(line, "[atlas] classify=47 surface=12");
        assert!(!line.contains("subcarve"));
        assert!(!line.contains("edges"));
    }

    #[test]
    fn render_line_shows_starting_when_no_calls_yet() {
        let line = render_line(&Counts::default(), None);
        assert_eq!(line, "[atlas] starting");
    }

    #[test]
    fn render_line_includes_tokens_when_counter_present() {
        let counter = Arc::new(TokenCounter::new(200_000));
        // `used()` is what the reporter reads; `charge` updates it
        // and succeeds while under budget.
        counter.charge(18_400).unwrap();
        let counts = Counts {
            classify: 47,
            ..Counts::default()
        };
        let line = render_line(&counts, Some(&counter));
        assert_eq!(line, "[atlas] classify=47 | tokens=18.4k/200.0k");
    }

    #[test]
    fn line_based_reporter_appends_newline_per_record() {
        let (buf, sink) = fresh_sink();
        let reporter = ProgressReporter::new(sink, false, None);
        reporter.record(PromptId::Classify);
        reporter.record(PromptId::Classify);
        reporter.record(PromptId::Stage1Surface);
        let out = dump(&buf);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "[atlas] classify=1");
        assert_eq!(lines[1], "[atlas] classify=2");
        assert_eq!(lines[2], "[atlas] classify=2 surface=1");
    }

    #[test]
    fn inline_reporter_uses_carriage_return_and_pads_for_shrink() {
        let (buf, sink) = fresh_sink();
        let reporter = ProgressReporter::new(sink, true, None);
        reporter.record(PromptId::Classify); // "[atlas] classify=1" len=18
        reporter.record(PromptId::Stage1Surface); // "[atlas] classify=1 surface=1" len=28
        let out = dump(&buf);
        // Both writes must be \r-prefixed; second must not start a new line.
        assert!(out.starts_with('\r'));
        assert!(!out.contains('\n'));
        assert!(out.contains("classify=1 surface=1"));
    }

    #[test]
    fn finish_terminates_inline_line_with_newline() {
        let (buf, sink) = fresh_sink();
        let reporter = ProgressReporter::new(sink, true, None);
        reporter.record(PromptId::Classify);
        reporter.finish();
        let out = dump(&buf);
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn finish_is_silent_when_nothing_was_recorded() {
        let (buf, sink) = fresh_sink();
        let reporter = ProgressReporter::new(sink, true, None);
        reporter.finish();
        assert!(dump(&buf).is_empty());
    }

    #[test]
    fn announce_start_writes_root_path() {
        let (buf, sink) = fresh_sink();
        let reporter = ProgressReporter::new(sink, true, None);
        reporter.announce_start(Path::new("/tmp/example"));
        let out = dump(&buf);
        assert!(out.contains("[atlas] indexing /tmp/example"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn progress_backend_records_after_forwarding_call() {
        let (buf, sink) = fresh_sink();
        let reporter = Arc::new(ProgressReporter::new(sink, false, None));
        let test_backend = TestBackend::new();
        test_backend.respond(
            PromptId::Classify,
            serde_json::json!({"k": "v"}),
            serde_json::json!({"ok": true}),
        );
        let inner: Arc<dyn LlmBackend> = Arc::new(test_backend);
        let backend = ProgressBackend::new(inner, Arc::clone(&reporter));
        let resp = backend
            .call(&LlmRequest {
                prompt_template: PromptId::Classify,
                inputs: serde_json::json!({"k": "v"}),
                schema: ResponseSchema::accept_any(),
            })
            .unwrap();
        assert_eq!(resp, serde_json::json!({"ok": true}));
        let out = dump(&buf);
        assert!(out.contains("classify=1"));
    }

    #[test]
    fn make_stderr_reporter_returns_none_for_never_mode() {
        assert!(make_stderr_reporter(ProgressMode::Never, None).is_none());
    }
}
