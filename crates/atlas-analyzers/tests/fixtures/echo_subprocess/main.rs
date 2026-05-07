//! Reference echo-subprocess binary for the PR-2 transport tests.
//!
//! Behaviour is configured by command-line flags (NOT environment
//! variables — env vars on a parent process are global and would
//! clobber across parallel cargo-tests). The flag-driven shape
//! lets each test pass its own scenario without cross-test leak.
//!
//! Recognised flags (all optional; all key=value):
//!
//! - `--id=<s>`: handshake `id` (default `echo-subprocess`).
//! - `--version=<s>`: handshake `version` (default `0.1.0`).
//! - `--stage=<l1..l9>`: handshake `stage` (default `l3`).
//! - `--cost-class=<class>`: handshake `cost_class` (default
//!   `deterministic-cheap`).
//! - `--applicability-json=<json>`: handshake
//!   `applicability_predicate` (default `{"always": true}`).
//! - `--crash-before-handshake`: exit 1 before writing anything.
//! - `--crash-after-handshake`: emit handshake, then exit 1.
//! - `--hang-after-handshake`: emit handshake, then sleep
//!   forever waiting on stdin.
//! - `--slow-response-ms=<n>`: sleep `n` ms before each reply.
//! - `--garbage-handshake`: emit non-JSON handshake bytes.
//!
//! Behaviour after handshake (all benign requests):
//!
//! - `Request::Applies` → `Confident { payload: true }`.
//! - `Request::FingerprintInputs` → `Confident` whose payload is
//!   one `file_content_sha` per pre-loaded manifest.
//! - `Request::Analyse` → `Confident` whose payload echoes the
//!   target dir.

use atlas_analyzers::subprocess::handshake::Capabilities;
use atlas_analyzers::subprocess::transport::{read_frame, write_frame};
use atlas_analyzers::subprocess::wire_types::{Request, Response, WireFingerprintInput};
use atlas_index::{ApplicabilityPredicate, CostClass, Stage};
use std::env;
use std::io::{self, BufWriter};
use std::time::Duration;

fn parse_stage(s: &str) -> Stage {
    match s {
        "l1" => Stage::L1,
        "l2" => Stage::L2,
        "l3" => Stage::L3,
        "l4" => Stage::L4,
        "l5" => Stage::L5,
        "l6" => Stage::L6,
        "l7" => Stage::L7,
        "l8" => Stage::L8,
        "l9" => Stage::L9,
        _ => Stage::L3,
    }
}

fn parse_cost_class(s: &str) -> CostClass {
    match s {
        "deterministic-cheap" => CostClass::DeterministicCheap,
        "deterministic-expensive" => CostClass::DeterministicExpensive,
        "llm-cheap" => CostClass::LlmCheap,
        "llm-expensive" => CostClass::LlmExpensive,
        _ => CostClass::DeterministicCheap,
    }
}

#[derive(Default)]
struct EchoArgs {
    id: Option<String>,
    version: Option<String>,
    stage: Option<String>,
    cost_class: Option<String>,
    applicability_json: Option<String>,
    crash_before_handshake: bool,
    crash_after_handshake: bool,
    hang_after_handshake: bool,
    slow_response_ms: Option<u64>,
    garbage_handshake: bool,
}

fn parse_args() -> EchoArgs {
    let mut args = EchoArgs::default();
    for arg in env::args().skip(1) {
        let stripped = arg.strip_prefix("--").unwrap_or(&arg);
        let (key, value) = stripped.split_once('=').unwrap_or((stripped, ""));
        match key {
            "id" => args.id = Some(value.to_string()),
            "version" => args.version = Some(value.to_string()),
            "stage" => args.stage = Some(value.to_string()),
            "cost-class" => args.cost_class = Some(value.to_string()),
            "applicability-json" => args.applicability_json = Some(value.to_string()),
            "crash-before-handshake" => args.crash_before_handshake = true,
            "crash-after-handshake" => args.crash_after_handshake = true,
            "hang-after-handshake" => args.hang_after_handshake = true,
            "slow-response-ms" => args.slow_response_ms = value.parse().ok(),
            "garbage-handshake" => args.garbage_handshake = true,
            _ => { /* ignore unknown — keeps test wiring simple */ }
        }
    }
    args
}

fn capabilities_from_args(args: &EchoArgs) -> Capabilities {
    let id = args.id.clone().unwrap_or_else(|| "echo-subprocess".into());
    let version = args.version.clone().unwrap_or_else(|| "0.1.0".into());
    let stage = parse_stage(args.stage.as_deref().unwrap_or("l3"));
    let cost_class = parse_cost_class(args.cost_class.as_deref().unwrap_or("deterministic-cheap"));
    let applicability = match args.applicability_json.as_deref() {
        Some(s) => serde_json::from_str(s).unwrap_or_default(),
        None => ApplicabilityPredicate {
            always: true,
            ..Default::default()
        },
    };
    Capabilities {
        id,
        version,
        stage,
        cost_class,
        applicability_predicate: applicability,
    }
}

fn main() {
    let args = parse_args();
    if args.crash_before_handshake {
        std::process::exit(1);
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout_buf = BufWriter::new(stdout.lock());

    if args.garbage_handshake {
        // Length prefix says 4 bytes, payload is `xxxx` — not
        // JSON. The parent must surface MalformedInput.
        use std::io::Write;
        let bytes = b"xxxx";
        let len = (bytes.len() as u32).to_be_bytes();
        stdout_buf.write_all(&len).ok();
        stdout_buf.write_all(bytes).ok();
        stdout_buf.flush().ok();
        // Hang so the parent decides what to do.
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    let caps = capabilities_from_args(&args);
    let caps_bytes = serde_json::to_vec(&caps).expect("serialising Capabilities");
    write_frame(&mut stdout_buf, &caps_bytes).expect("writing handshake frame");

    if args.crash_after_handshake {
        std::process::exit(1);
    }
    if args.hang_after_handshake {
        // Read stdin forever without ever replying.
        loop {
            let _ = read_frame(&mut stdin);
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    loop {
        let req_bytes = match read_frame(&mut stdin) {
            Ok(b) => b,
            Err(_) => return, // parent closed stdin → graceful exit
        };
        if let Some(ms) = args.slow_response_ms {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let request: Request = match serde_json::from_slice(&req_bytes) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::Error {
                    message: format!("undecodable request: {e}"),
                    error_kind: Some("malformed_input".into()),
                };
                let bytes = serde_json::to_vec(&resp).expect("serialising Error response");
                write_frame(&mut stdout_buf, &bytes).ok();
                continue;
            }
        };
        let response = match request {
            Request::Applies { .. } => Response::Confident {
                payload: serde_json::Value::Bool(true),
            },
            Request::FingerprintInputs { target } => {
                let inputs: Vec<WireFingerprintInput> = target
                    .manifests
                    .iter()
                    .map(|m| WireFingerprintInput::FileContentSha {
                        sha: m.content_sha.clone(),
                    })
                    .collect();
                Response::Confident {
                    payload: serde_json::to_value(inputs).expect("serialising fingerprint inputs"),
                }
            }
            Request::Analyse { target } => Response::Confident {
                payload: serde_json::json!({"echoed_dir": target.dir}),
            },
        };
        let bytes = serde_json::to_vec(&response).expect("serialising response");
        if write_frame(&mut stdout_buf, &bytes).is_err() {
            return;
        }
    }
}
