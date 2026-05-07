//! Per-analyser subprocess pool.
//!
//! Phase 2 default: at most one live child per analyser. The first
//! dispatch lazily spawns the binary, performs the
//! [`crate::subprocess::handshake`] dance, then keeps the child alive
//! across subsequent dispatches. Pipeline shutdown drops the
//! [`ProcessPool`], which sends `SIGTERM`, waits up to 5 seconds,
//! then escalates to `SIGKILL`.
//!
//! ## Concurrency
//!
//! `ProcessPool` owns a `Mutex<Option<ChildProcess>>`. Concurrent
//! dispatches serialise on the mutex (Phase 2 has no concurrent
//! analyser invocation; Phase 3+ may grow the pool to N children).
//!
//! ## Timeouts
//!
//! Each `call()` runs the request/response cycle on a worker
//! thread; the calling thread waits on a `Receiver` with the
//! configured timeout. On timeout the parent kills the child and
//! returns [`crate::AnalyzerError::CallFailed`] with
//! `reason: "timeout"`. The next dispatch respawns.
//!
//! ## Crash isolation
//!
//! A child that exits non-zero, hangs past the timeout, or emits
//! malformed JSON does NOT poison the registry: the proxy clears
//! the cached `ChildProcess` and the next dispatch respawns. This
//! matches the §4 acceptance: "A subprocess that fails or times
//! out does not poison the registry; later dispatches respawn it."

#[cfg(unix)]
use libc;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crate::subprocess::handshake::{verify_capabilities, Capabilities, HandshakeError};
use crate::subprocess::transport::{read_frame, write_frame};
use crate::subprocess::wire_types::{Request, Response};
use crate::AnalyzerError;

/// Default analyse-call timeout when the spec did not configure
/// one. Covers the full request → response cycle (write, child
/// computes, child writes, parent reads).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Grace period between `SIGTERM` and `SIGKILL` during shutdown.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// One spawned child + its stdio handles. Owned by the
/// [`ProcessPool`]'s mutex; never accessed concurrently.
struct ChildProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Stderr handle. `None` only if the OS failed to pipe it (should
    /// not happen given `Stdio::piped()`). Read on the crash path to
    /// surface diagnostics in the `CallFailed` message.
    stderr: Option<ChildStderr>,
}

impl ChildProcess {
    fn spawn(command: &[String], expected_caps: &Capabilities) -> Result<Self, AnalyzerError> {
        if command.is_empty() {
            return Err(AnalyzerError::CallFailed {
                analyzer_id: expected_caps.id.clone(),
                message: "subprocess command is empty".into(),
            });
        }
        let mut cmd = Command::new(&command[0]);
        if command.len() > 1 {
            cmd.args(&command[1..]);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| AnalyzerError::CallFailed {
            analyzer_id: expected_caps.id.clone(),
            message: format!("spawn `{}` failed: {e}", command[0]),
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AnalyzerError::CallFailed {
                analyzer_id: expected_caps.id.clone(),
                message: "child stdin not piped".into(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AnalyzerError::CallFailed {
                analyzer_id: expected_caps.id.clone(),
                message: "child stdout not piped".into(),
            })?;
        // Capture stderr so crash-path errors can include a tail of the
        // child's diagnostic output. The handle is stored as `Option`
        // since `take()` returns `Option`; in practice `Stdio::piped()`
        // always provides a handle.
        let stderr = child.stderr.take();
        let mut cp = ChildProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
        };
        // Consume the handshake frame the child must emit on
        // startup. Done synchronously — no point caching a child
        // that did not announce itself.
        let frame = read_frame(&mut cp.stdout).map_err(|e| AnalyzerError::CallFailed {
            analyzer_id: expected_caps.id.clone(),
            message: format!("reading handshake frame failed: {e}"),
        })?;
        let actual: Capabilities =
            serde_json::from_slice(&frame).map_err(|e| AnalyzerError::MalformedInput {
                analyzer_id: expected_caps.id.clone(),
                message: format!("decoding handshake frame failed: {e}"),
            })?;
        verify_capabilities(expected_caps, &actual).map_err(|e| match *e {
            HandshakeError::IdMismatch { .. }
            | HandshakeError::VersionMismatch { .. }
            | HandshakeError::StageMismatch { .. }
            | HandshakeError::CostClassMismatch { .. }
            | HandshakeError::ApplicabilityMismatch { .. } => AnalyzerError::CallFailed {
                analyzer_id: expected_caps.id.clone(),
                message: format!("handshake mismatch: {e}"),
            },
        })?;
        Ok(cp)
    }

    /// Send SIGTERM to the child process (Unix only).
    ///
    /// On Windows there is no SIGTERM equivalent; the caller will fall
    /// through to `child.kill()` (TerminateProcess) after the grace
    /// period.
    #[cfg(unix)]
    fn send_sigterm(&self) {
        // SAFETY: libc::kill is async-signal-safe; we pass a valid
        // pid obtained from the running child and a well-known signal
        // number. The return value is intentionally ignored — if the
        // process has already exited the kill(2) call returns ESRCH,
        // which is benign.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
    }

    fn shutdown(mut self) {
        // Send SIGTERM first (Unix). The spec requires
        // "SIGTERM → 5s grace → SIGKILL". On Windows there is no
        // SIGTERM so we fall through directly to kill() below.
        #[cfg(unix)]
        self.send_sigterm();

        // Drop stdin to signal EOF. This helps analysers that do not
        // install a SIGTERM handler but do exit on stdin close.
        let _ = self.stdin;
        drop(self.stderr);

        // Poll for a clean exit up to SHUTDOWN_GRACE; escalate to
        // SIGKILL if the child is still alive after the deadline.
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

/// Read up to `limit` bytes from `stderr` (synchronous). Called only
/// on the crash path after the child has exited or closed its stdout,
/// so the kernel has already closed the write end of the pipe —
/// `read_to_end` returns at EOF without blocking.
///
/// Returns an empty string if `stderr` is `None` or the read fails.
fn read_stderr_tail(stderr: Option<ChildStderr>, limit: usize) -> String {
    use std::io::Read;
    let handle = match stderr {
        Some(h) => h,
        None => return String::new(),
    };
    let mut buf = Vec::with_capacity(limit.min(4096));
    // Read with a hard byte cap: take() prevents reading past `limit`.
    let _ = handle.take(limit as u64).read_to_end(&mut buf);
    if buf.is_empty() {
        return String::new();
    }
    // Convert to UTF-8 lossily so control characters don't panic.
    let s = String::from_utf8_lossy(&buf);
    // Trim trailing whitespace for cleaner error messages.
    s.trim_end().to_string()
}

/// Per-analyser subprocess pool.
///
/// Construction does not spawn — the first [`ProcessPool::call`]
/// does. Drop terminates any live child via [`ChildProcess::shutdown`].
pub struct ProcessPool {
    /// argv of the analyser binary. Resolved against `$PATH` by
    /// `Command::new` (Phase 2 does not implement
    /// `override_search`; that is Phase 3+).
    command: Vec<String>,
    /// Capabilities the parent expects the child to announce.
    expected_caps: Capabilities,
    /// Per-call timeout. If absent, [`DEFAULT_TIMEOUT`] is used.
    timeout: Duration,
    /// The cached child. `None` until first call (or after a
    /// crash / timeout).
    child: Mutex<Option<ChildProcess>>,
    /// Path the binary content sha was computed from. Stored so
    /// debug logging can name the failing binary.
    #[allow(dead_code)]
    binary_path: PathBuf,
}

impl std::fmt::Debug for ProcessPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessPool")
            .field("id", &self.expected_caps.id)
            .field("command", &self.command)
            .field("timeout", &self.timeout)
            .field(
                "child_alive",
                &self.child.lock().map(|g| g.is_some()).unwrap_or(false),
            )
            .finish()
    }
}

impl ProcessPool {
    /// Construct a new pool. Does not spawn; the first
    /// [`ProcessPool::call`] performs the spawn + handshake.
    pub fn new(
        command: Vec<String>,
        expected_caps: Capabilities,
        timeout: Option<Duration>,
        binary_path: PathBuf,
    ) -> Self {
        ProcessPool {
            command,
            expected_caps,
            timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
            child: Mutex::new(None),
            binary_path,
        }
    }

    /// Issue a single request/response cycle. Lazily spawns the
    /// child on first call. On any failure (spawn, write, read,
    /// timeout) the cached child is cleared so the next call
    /// respawns; the failure is mapped onto an [`AnalyzerError`]
    /// for the caller.
    pub fn call(&self, request: &Request) -> Result<Response, AnalyzerError> {
        let mut guard = self.child.lock().map_err(|_| AnalyzerError::CallFailed {
            analyzer_id: self.expected_caps.id.clone(),
            message: "process-pool mutex poisoned".into(),
        })?;
        if guard.is_none() {
            let cp = ChildProcess::spawn(&self.command, &self.expected_caps)?;
            *guard = Some(cp);
        }
        let cp = guard.as_mut().expect("just spawned");

        // Serialise + write the request frame. A write error means
        // the child went away; clear the cache so the next call
        // respawns.
        let req_bytes = serde_json::to_vec(request).map_err(|e| {
            // Serialising a Request shouldn't fail — but if it
            // does, the child is innocent; do NOT kill it.
            AnalyzerError::MalformedInput {
                analyzer_id: self.expected_caps.id.clone(),
                message: format!("encoding request failed: {e}"),
            }
        })?;
        if let Err(e) = write_frame(&mut cp.stdin, &req_bytes) {
            // Write failed → child is suspect. Tear it down.
            let analyzer_id = self.expected_caps.id.clone();
            self.tear_down_locked(&mut guard);
            return Err(AnalyzerError::CallFailed {
                analyzer_id,
                message: format!("writing request frame failed: {e}"),
            });
        }

        // Read the response on a worker thread so the calling
        // thread can enforce the timeout. The worker takes
        // ownership of the BufReader (we hand it out and back via
        // a channel) because read_frame is a blocking call we
        // cannot interrupt cleanly. On timeout we kill the child;
        // the worker thread observes EOF / error and exits.
        let cp = guard.take().expect("just spawned");
        let ChildProcess {
            mut child,
            stdin,
            mut stdout,
            stderr,
        } = cp;
        let timeout = self.timeout;

        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, std::io::Error>>();
        let analyzer_id = self.expected_caps.id.clone();
        let worker = thread::spawn(move || {
            let result = read_frame(&mut stdout);
            // Best effort: if the receiver is gone (timeout), the
            // send fails and we just drop the result.
            let _ = tx.send(result);
            stdout
        });
        let recv = rx.recv_timeout(timeout);
        match recv {
            Ok(Ok(bytes)) => {
                // Response received; rejoin the worker to recover
                // the BufReader and restore the cached child.
                let stdout = worker.join().map_err(|_| AnalyzerError::CallFailed {
                    analyzer_id: analyzer_id.clone(),
                    message: "worker thread panicked".into(),
                })?;
                let cp = ChildProcess {
                    child,
                    stdin,
                    stdout,
                    stderr,
                };
                *guard = Some(cp);
                let response: Response = serde_json::from_slice(&bytes).map_err(|e| {
                    // Tear the child down — it emitted garbage.
                    let analyzer_id = self.expected_caps.id.clone();
                    self.tear_down_locked(&mut guard);
                    AnalyzerError::MalformedInput {
                        analyzer_id,
                        message: format!("decoding response frame failed: {e}"),
                    }
                })?;
                Ok(response)
            }
            Ok(Err(io_err)) => {
                // Read errored synchronously (e.g. child closed
                // its stdout). Reap the child to learn its exit
                // code. The child has already exited (or closed its
                // stdout end), so reading stderr returns at EOF
                // immediately.
                let _ = child.kill();
                let exit = child.wait().ok();
                drop(stdin);
                let _ = worker.join();
                // child slot stays None.
                let exit_msg = exit.map(|s| format!(" (exit: {s})")).unwrap_or_default();
                let stderr_tail = read_stderr_tail(stderr, 4096);
                let stderr_msg = if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!("; stderr: {stderr_tail}")
                };
                Err(AnalyzerError::CallFailed {
                    analyzer_id,
                    message: format!(
                        "subprocess exited unexpectedly: {io_err}{exit_msg}{stderr_msg}"
                    ),
                })
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Hung child: kill, reap, and surface a stable
                // CallFailed with the literal `timeout` reason
                // string the §4 acceptance pins.
                let _ = child.kill();
                let _ = child.wait();
                drop(stdin);
                let _ = worker.join();
                // After kill+wait the child is gone; read stderr
                // tail for diagnostics. The `timeout` prefix is kept
                // for the §4 acceptance pin.
                let stderr_tail = read_stderr_tail(stderr, 4096);
                let stderr_msg = if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!("; stderr: {stderr_tail}")
                };
                Err(AnalyzerError::CallFailed {
                    analyzer_id,
                    message: format!("timeout{stderr_msg}"),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The worker thread dropped the sender without
                // sending; treat as a read error.
                let _ = child.kill();
                let _ = child.wait();
                drop(stdin);
                let _ = worker.join();
                let stderr_tail = read_stderr_tail(stderr, 4096);
                let stderr_msg = if stderr_tail.is_empty() {
                    String::new()
                } else {
                    format!("; stderr: {stderr_tail}")
                };
                Err(AnalyzerError::CallFailed {
                    analyzer_id,
                    message: format!("worker disconnected before response{stderr_msg}"),
                })
            }
        }
    }

    /// Abandon the cached child without waiting on a clean exit.
    /// Used when the proxy has already detected the child is
    /// misbehaving and we just want to ensure the next call
    /// respawns.
    fn tear_down_locked(&self, guard: &mut std::sync::MutexGuard<'_, Option<ChildProcess>>) {
        if let Some(cp) = guard.take() {
            // Don't run the polite shutdown path; the child is
            // already broken, just kill it.
            let mut child = cp.child;
            drop(cp.stdin);
            drop(cp.stdout);
            drop(cp.stderr);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ProcessPool {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(cp) = guard.take() {
                cp.shutdown();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Placeholder: pool integration tests live in
    // `tests/subprocess_transport.rs` because they need the
    // `echo_subprocess` fixture binary. Unit tests here only cover
    // the no-process paths.

    #[test]
    fn empty_command_errors_out() {
        let caps = Capabilities {
            id: "x".into(),
            version: "v".into(),
            stage: atlas_index::Stage::L3,
            cost_class: atlas_index::CostClass::DeterministicCheap,
            applicability_predicate: Default::default(),
        };
        let pool = ProcessPool::new(Vec::new(), caps, None, PathBuf::from("/missing"));
        let req = Request::Applies {
            target: crate::subprocess::wire_types::WireTarget {
                dir: "/x".into(),
                languages: Vec::new(),
                manifests: Vec::new(),
                top_level_files: Vec::new(),
            },
        };
        let err = pool.call(&req).unwrap_err();
        match err {
            AnalyzerError::CallFailed {
                analyzer_id,
                message,
            } => {
                assert_eq!(analyzer_id, "x");
                assert!(message.contains("empty"));
            }
            other => panic!("expected CallFailed, got {other:?}"),
        }
    }

    #[test]
    fn missing_binary_errors_out() {
        let caps = Capabilities {
            id: "x".into(),
            version: "v".into(),
            stage: atlas_index::Stage::L3,
            cost_class: atlas_index::CostClass::DeterministicCheap,
            applicability_predicate: Default::default(),
        };
        let pool = ProcessPool::new(
            vec!["/this/does/not/exist".into()],
            caps,
            None,
            PathBuf::from("/this/does/not/exist"),
        );
        let req = Request::Applies {
            target: crate::subprocess::wire_types::WireTarget {
                dir: "/x".into(),
                languages: Vec::new(),
                manifests: Vec::new(),
                top_level_files: Vec::new(),
            },
        };
        let err = pool.call(&req).unwrap_err();
        match err {
            AnalyzerError::CallFailed {
                analyzer_id,
                message,
            } => {
                assert_eq!(analyzer_id, "x");
                assert!(message.contains("spawn"));
            }
            other => panic!("expected CallFailed, got {other:?}"),
        }
    }
}
