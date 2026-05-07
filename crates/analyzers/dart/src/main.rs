//! `dart-analyzer` subprocess entry point (Atlas vNext Phase 2 PR-7).
//!
//! The binary speaks the stdio JSON wire protocol defined by
//! [`atlas_analyzers::subprocess`] (length-prefixed framing,
//! `applies` / `fingerprint_inputs` / `analyse` request kinds):
//!
//! 1. On startup, write a [`Capabilities`] handshake frame announcing
//!    `id: "dart-surface-analyzer"`, `stage: L5`,
//!    `cost_class: deterministic-expensive`, and the
//!    `applicability_predicate` keyed on the Dart language tag
//!    plus `pubspec.yaml`.
//! 2. For each subsequent [`Request`] frame on stdin, parse it,
//!    drive [`atlas_dart_analyzer::extract_dart_surface`] when the
//!    request is `analyse`, and write the response frame on stdout.
//! 3. EOF on stdin → graceful exit.
//!
//! ## Phase 2 deterministic-only
//!
//! The subprocess does NOT have LLM access. All extraction is structural,
//! driven by the hand-rolled Dart lexer in `lib.rs`.
//!
//! ## Source-file discovery
//!
//! The wire protocol's `Target.manifests` only carries pre-loaded manifests.
//! The analyser walks `Target.dir` directly via `std::fs::read_dir` to
//! enumerate Dart sources under `lib/`, then parses each.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use atlas_analyzers::subprocess::handshake::Capabilities;
use atlas_analyzers::subprocess::transport::{read_frame, write_frame};
use atlas_analyzers::subprocess::wire_types::{
    Request, Response, WireFingerprintInput, WireTarget,
};
use atlas_index::{ApplicabilityPredicate, CostClass, Stage};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use atlas_dart_analyzer::{extract_dart_surface, pub_item_kind_str, ANALYZER_ID, ANALYZER_VERSION};

/// Wire-side representation of one [`atlas_index::Binding`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireBinding {
    language: String,
    symbol: String,
    file: String,
    span: (usize, usize),
    content_sha: String,
    visibility: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    module_path: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    attributes: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireLibraryApi {
    id: String,
    kind: String,
    language: String,
    fingerprint: String,
    pub_items: Vec<WirePubItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WirePubItem {
    name: String,
    file: String,
    kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnalysePayload {
    bindings: Vec<WireBinding>,
    library_apis: Vec<WireLibraryApi>,
}

fn capabilities() -> Capabilities {
    Capabilities {
        id: ANALYZER_ID.into(),
        version: ANALYZER_VERSION.into(),
        stage: Stage::L5,
        cost_class: CostClass::DeterministicExpensive,
        applicability_predicate: ApplicabilityPredicate {
            languages: vec!["dart".into()],
            file_globs: vec!["**/pubspec.yaml".into()],
            manifest_types: vec!["dart".into()],
            ..Default::default()
        },
    }
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout_buf = BufWriter::new(stdout.lock());

    // 1. Write the handshake.
    let caps = capabilities();
    let caps_bytes = serde_json::to_vec(&caps).expect("Capabilities serialises");
    if write_frame(&mut stdout_buf, &caps_bytes).is_err() {
        return;
    }

    // 2. Request loop.
    loop {
        let req_bytes = match read_frame(&mut stdin) {
            Ok(b) => b,
            Err(_) => return,
        };
        let request: Request = match serde_json::from_slice(&req_bytes) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::Error {
                    message: format!("undecodable request: {e}"),
                    error_kind: Some("malformed_input".into()),
                };
                send(&mut stdout_buf, &resp);
                continue;
            }
        };

        let response = handle_request(&request);
        send(&mut stdout_buf, &response);
    }
}

fn send<W: Write>(dst: &mut W, response: &Response) {
    if let Ok(bytes) = serde_json::to_vec(response) {
        let _ = write_frame(dst, &bytes);
    }
}

fn handle_request(request: &Request) -> Response {
    match request {
        Request::Applies { .. } => Response::Confident {
            payload: serde_json::Value::Bool(true),
        },
        Request::FingerprintInputs { target } => {
            let mut inputs: Vec<WireFingerprintInput> = target
                .manifests
                .iter()
                .map(|m| WireFingerprintInput::FileContentSha {
                    sha: m.content_sha.clone(),
                })
                .collect();
            for (_rel, bytes) in walk_dart_sources(Path::new(&target.dir)) {
                let sha = sha256_hex_bytes(&bytes);
                inputs.push(WireFingerprintInput::FileContentSha { sha });
            }
            match serde_json::to_value(inputs) {
                Ok(v) => Response::Confident { payload: v },
                Err(e) => Response::Error {
                    message: format!("serialising fingerprint_inputs failed: {e}"),
                    error_kind: None,
                },
            }
        }
        Request::Analyse { target } => analyse(target),
    }
}

fn analyse(target: &WireTarget) -> Response {
    let dir = PathBuf::from(&target.dir);
    let component_id = component_id_from_target(target);

    let pubspec_yaml = pubspec_bytes_from_target(target);
    let sources = walk_dart_sources(&dir);
    let inputs = atlas_dart_analyzer::DartSourceInputs {
        sources,
        pubspec_yaml,
    };
    let surface = extract_dart_surface(&component_id, &inputs);

    let payload = AnalysePayload {
        bindings: surface.bindings.iter().map(wire_binding).collect(),
        library_apis: surface.library_apis.iter().map(wire_library_api).collect(),
    };
    match serde_json::to_value(payload) {
        Ok(v) => Response::Confident { payload: v },
        Err(e) => Response::Error {
            message: format!("serialising analyse payload failed: {e}"),
            error_kind: None,
        },
    }
}

fn wire_binding(b: &atlas_index::Binding) -> WireBinding {
    let visibility = serde_json::to_value(&b.visibility).unwrap_or(serde_json::Value::Null);
    let attributes = b
        .attributes
        .iter()
        .map(|(k, v)| {
            let v_json = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
            (k.clone(), v_json)
        })
        .collect();
    WireBinding {
        language: b.language.clone(),
        symbol: b.symbol.clone(),
        file: b.file.to_string_lossy().into_owned(),
        span: b.span,
        content_sha: b.content_sha.clone(),
        visibility,
        module_path: b.module_path.clone(),
        attributes,
    }
}

fn wire_library_api(api: &atlas_index::LibraryApi) -> WireLibraryApi {
    WireLibraryApi {
        id: api.id.clone(),
        kind: "library-api".into(),
        language: api.language.clone(),
        fingerprint: api.fingerprint.clone(),
        pub_items: api
            .pub_items
            .iter()
            .map(|p| WirePubItem {
                name: p.name.clone(),
                file: p.file.to_string_lossy().into_owned(),
                kind: pub_item_kind_str(p.kind).to_string(),
            })
            .collect(),
    }
}

/// Extract the `pubspec.yaml` bytes from the parent's pre-loaded manifest list.
fn pubspec_bytes_from_target(target: &WireTarget) -> Option<Vec<u8>> {
    target
        .manifests
        .iter()
        .find(|m| m.name == "pubspec.yaml")
        .and_then(|m| BASE64.decode(&m.bytes_b64).ok())
}

/// Component id heuristic: prefer `pubspec.yaml`'s declared package name;
/// fall back to the directory basename.
fn component_id_from_target(target: &WireTarget) -> String {
    if let Some(bytes) = pubspec_bytes_from_target(target) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Some(name) = atlas_dart_analyzer::extract_pubspec_name(text) {
                return name;
            }
        }
    }
    Path::new(&target.dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Walk the candidate directory recursively, returning every `*.dart` file
/// as `(relative_path, bytes)`. Focuses on `lib/` subdirectory first, then
/// includes the rest. Symlinks are not followed; files larger than 4 MiB
/// are skipped.
fn walk_dart_sources(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    const MAX_DART_FILE_BYTES: u64 = 4 * 1024 * 1024;
    let mut out: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let basename = entry.file_name();
            if let Some(s) = basename.to_str() {
                // Skip common noise dirs.
                if s.starts_with('.')
                    || s == ".dart_tool"
                    || s == "build"
                    || s == ".pub-cache"
                    || s == "test"
                    || s == "example"
                {
                    continue;
                }
            }
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if metadata.len() > MAX_DART_FILE_BYTES {
                continue;
            }
            let extension = path.extension().and_then(OsStr::to_str);
            if !matches!(extension, Some("dart")) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let rel = path.strip_prefix(dir).unwrap_or(&path).to_path_buf();
            out.push((rel, bytes));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}
