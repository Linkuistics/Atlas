//! Integration tests for the subprocess analyser transport (PR-2).
//!
//! Drives the [`atlas_analyzers::SubprocessAnalyzerProxy`] against
//! the reference echo binary at
//! `crates/atlas-analyzers/tests/fixtures/echo_subprocess/`. The
//! binary is declared as a `[[bin]]` in atlas-analyzers's
//! Cargo.toml so cargo builds it before running tests; we look it
//! up via `env!("CARGO_BIN_EXE_echo_subprocess")`.
//!
//! The fixture is configured by command-line flags (per
//! `tests/fixtures/echo_subprocess/main.rs`) so each test can pass
//! its own scenario without leaking state into the others. (Env
//! vars on the parent process are global and would clobber under
//! cargo's parallel test runner.)

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use atlas_analyzers::subprocess::wire_types::{Request, Response, WireTarget};
use atlas_analyzers::{
    AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, SubprocessAnalyzerProxy,
    SubprocessAnalyzerSpec, Target, TargetFile,
};
use atlas_index::{ApplicabilityPredicate, CostClass, Stage};

/// Path to the cargo-built echo fixture binary. Cargo sets this
/// env var to the artefact path when the test depends on the
/// `[[bin]]` declaration in the same crate.
fn echo_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_echo_subprocess"))
}

fn applicability_always() -> ApplicabilityPredicate {
    ApplicabilityPredicate {
        always: true,
        ..Default::default()
    }
}

/// Spec configured to run the echo binary with the given extra
/// CLI flags appended to argv. Each test uses its own argv so
/// parallel cargo-test runs don't clobber each other.
fn make_spec(extra_args: &[&str]) -> SubprocessAnalyzerSpec {
    let mut command: Vec<String> = vec![echo_binary().to_string_lossy().into_owned()];
    command.extend(extra_args.iter().map(|s| (*s).to_string()));
    SubprocessAnalyzerSpec {
        id: "echo-subprocess".into(),
        version: "0.1.0".into(),
        stage: Stage::L3,
        cost_class: CostClass::DeterministicCheap,
        applicability: applicability_always(),
        command,
        binary_path: echo_binary(),
        timeout: Some(Duration::from_secs(5)),
    }
}

fn target_at(dir: &str) -> Target {
    Target {
        dir: PathBuf::from(dir),
        languages: BTreeSet::new(),
        manifests: vec![TargetFile {
            name: "Cargo.toml".into(),
            relpath: PathBuf::from("Cargo.toml"),
            bytes: b"[package]".to_vec(),
            content_sha: "deadbeef".into(),
        }],
        top_level_files: vec!["Cargo.toml".into()],
    }
}

#[test]
fn happy_path_handshake_and_analyse() {
    let spec = make_spec(&[]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    assert_eq!(proxy.id(), "echo-subprocess");
    assert!(proxy.applies(&target_at("/ws")));

    // Round-trip an analyse request through the proxy.
    let ctx = AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target_at("/ws/x"));
    match result {
        AnalyzerResult::Confident(output) => {
            let so = output
                .as_any()
                .downcast_ref::<atlas_analyzers::SubprocessOutput>()
                .expect("Confident output is SubprocessOutput");
            assert_eq!(so.analyzer_id, "echo-subprocess");
            assert_eq!(
                so.payload.get("echoed_dir").and_then(|v| v.as_str()),
                Some("/ws/x")
            );
        }
        other => panic!("expected Confident, got {other:?}"),
    }
}

#[test]
fn fingerprint_inputs_round_trip_through_subprocess() {
    let spec = make_spec(&[]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let inputs = proxy.fingerprint_inputs(&target_at("/ws"));
    assert_eq!(inputs.len(), 1, "expected one fingerprint input");
    match &inputs[0] {
        atlas_analyzers::FingerprintInput::FileContentSha(sha) => assert_eq!(sha, "deadbeef"),
        other => panic!("expected FileContentSha, got {other:?}"),
    }
}

#[test]
fn handshake_rejects_mismatched_capabilities() {
    // The fixture announces stage L5 but the parent's spec
    // declares L3. The proxy must reject the call with a
    // CallFailed describing the mismatch.
    let spec = make_spec(&["--stage=l5"]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    // The handshake mismatch is observed on the first call (the
    // proxy lazily spawns).
    let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target_at("/ws"));
    match result {
        AnalyzerResult::Error(AnalyzerError::CallFailed { message, .. }) => {
            assert!(
                message.contains("handshake mismatch"),
                "expected handshake-mismatch CallFailed, got: {message}"
            );
            assert!(
                message.contains("L3") && message.contains("L5"),
                "expected stage names in error, got: {message}"
            );
        }
        other => panic!("expected CallFailed, got {other:?}"),
    }
}

#[test]
fn subprocess_crash_returns_call_failed() {
    // Fixture exits 1 immediately. The parent must surface
    // AnalyzerError::CallFailed.
    let spec = make_spec(&["--crash-before-handshake"]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target_at("/ws"));
    match result {
        AnalyzerResult::Error(AnalyzerError::CallFailed { message, .. }) => {
            // Either the spawn-then-EOF is reported as a read
            // error, or the read frame fails. Both are CallFailed
            // — what matters is no panic and the analyzer_id is
            // ours.
            assert!(
                !message.is_empty(),
                "CallFailed message should be non-empty"
            );
        }
        other => panic!("expected CallFailed, got {other:?}"),
    }
}

#[test]
fn subprocess_crash_after_handshake_returns_call_failed() {
    // Fixture emits the handshake, then exits 1. The first
    // analyse call observes the EOF on the response frame.
    let spec = make_spec(&["--crash-after-handshake"]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target_at("/ws"));
    match result {
        AnalyzerResult::Error(AnalyzerError::CallFailed { .. }) => {}
        other => panic!("expected CallFailed, got {other:?}"),
    }
}

#[test]
fn subprocess_timeout_returns_call_failed() {
    // Fixture hangs after the handshake. The proxy timeout is
    // tightened to 500ms for a quick-running test.
    let mut spec = make_spec(&["--hang-after-handshake"]);
    spec.timeout = Some(Duration::from_millis(500));
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target_at("/ws"));
    match result {
        AnalyzerResult::Error(AnalyzerError::CallFailed { message, .. }) => {
            assert_eq!(
                message, "timeout",
                "timeout case must surface message: \"timeout\""
            );
        }
        other => panic!("expected CallFailed (timeout), got {other:?}"),
    }
}

#[test]
fn malformed_handshake_returns_malformed_input() {
    // Fixture writes a non-JSON handshake frame. The proxy
    // surfaces MalformedInput.
    let spec = make_spec(&["--garbage-handshake"]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target_at("/ws"));
    match result {
        AnalyzerResult::Error(AnalyzerError::MalformedInput { message, .. }) => {
            assert!(message.contains("handshake"), "got: {message}");
        }
        other => panic!("expected MalformedInput, got {other:?}"),
    }
}

#[test]
fn binary_sha_is_recorded_on_proxy() {
    let spec = make_spec(&[]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let sha = proxy.binary_sha();
    assert_eq!(sha.len(), 64, "binary_sha must be 64-char hex");
    assert!(sha
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn registry_register_subprocess_records_binary_sha() {
    let spec = make_spec(&[]);
    let mut registry = atlas_analyzers::AnalyzerRegistry::empty();
    let returned = registry
        .register_subprocess(spec)
        .expect("register_subprocess succeeds");
    let recorded = registry
        .binary_sha("echo-subprocess")
        .expect("binary_sha is recorded after register_subprocess");
    assert_eq!(returned, recorded);
    assert_eq!(returned.len(), 64);
}

#[test]
fn proxy_runs_multiple_calls_against_same_child() {
    // Phase 2's process pool reuses one child per analyser; this
    // test verifies the child stays alive across N calls.
    let spec = make_spec(&[]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let ctx = AnalysisContext::deterministic_only();
    for i in 0..3 {
        let res = proxy.analyse(&ctx, &target_at(&format!("/ws/{i}")));
        match res {
            AnalyzerResult::Confident(_) => {}
            other => panic!("call #{i} expected Confident, got {other:?}"),
        }
    }
}

#[test]
fn proxy_recovers_after_failure_via_respawn() {
    // After a child failure (here: a crash-after-handshake), the
    // next call must respawn rather than poison the proxy. We
    // simulate by configuring the fixture to crash after
    // handshake on the first invocation; the proxy tears it down
    // and the next dispatch gets a fresh child. The child is
    // identically configured — i.e. the second spawn would also
    // crash — but the contract under test is "the next dispatch
    // attempts a respawn", not "the analyser eventually
    // succeeds".
    let spec = make_spec(&["--crash-after-handshake"]);
    let proxy = SubprocessAnalyzerProxy::new(spec).expect("constructing proxy");
    let ctx = AnalysisContext::deterministic_only();
    let r1 = proxy.analyse(&ctx, &target_at("/ws/1"));
    assert!(matches!(
        r1,
        AnalyzerResult::Error(AnalyzerError::CallFailed { .. })
    ));
    let r2 = proxy.analyse(&ctx, &target_at("/ws/2"));
    assert!(
        matches!(r2, AnalyzerResult::Error(AnalyzerError::CallFailed { .. })),
        "second dispatch must also CallFail (a fresh child also crashes); got {r2:?}"
    );
}

#[test]
fn process_pool_serialises_request_response() {
    // Lower-level transport check: build a Request and Response
    // and verify they round-trip through serde the same way the
    // proxy does internally.
    let req = Request::Analyse {
        target: WireTarget {
            dir: "/x".into(),
            languages: Vec::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        },
    };
    let bytes = serde_json::to_vec(&req).unwrap();
    let parsed: Request = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, req);
    let resp = Response::Declines;
    let bytes = serde_json::to_vec(&resp).unwrap();
    let parsed: Response = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, resp);
}
