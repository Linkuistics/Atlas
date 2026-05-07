//! `python-analyzer` subprocess entry point (Atlas vNext Phase 2 PR-3).
//!
//! The binary speaks the stdio JSON wire protocol defined by
//! [`atlas_analyzers::subprocess`] (length-prefixed framing,
//! `applies` / `fingerprint_inputs` / `analyse` request kinds):
//!
//! 1. On startup, write a [`Capabilities`] handshake frame announcing
//!    `id: "python-surface-analyzer"`, `stage: L5`,
//!    `cost_class: deterministic-expensive`, and the
//!    `applicability_predicate` keyed on the Python language tag
//!    plus `pyproject.toml` / `setup.py` / `requirements.txt`.
//! 2. For each subsequent [`Request`] frame on stdin, parse it,
//!    drive [`atlas_python_analyzer::extract_python_surface`] when
//!    the request is `analyse`, and write the response frame on
//!    stdout.
//! 3. EOF on stdin → graceful exit.
//!
//! ## Phase 2 deterministic-only
//!
//! The subprocess does NOT have LLM access (per PR-2 / PR-3 charter
//! in the plan). All extraction is structural, driven by
//! `rustpython-parser` over the pre-loaded Python source bytes.
//!
//! ## Source-file discovery
//!
//! The wire protocol's `Target.manifests` only carries the manifests
//! the parent pre-loaded (canonically `pyproject.toml`); it does NOT
//! ship every `*.py` source file in the component. The analyser
//! walks the candidate dir at `Target.dir` directly via
//! `std::fs::read_dir` to enumerate Python sources, then parses each.
//! This is the "Phase 2 minimum" pattern the TS/JS in-process
//! analyser also follows; a future driver may stream source bytes
//! through the wire envelope to keep the analyser sandboxable.

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

use atlas_python_analyzer::{extract_python_surface, ANALYZER_ID, ANALYZER_VERSION};

/// Wire-side representation of one [`atlas_index::Binding`]. Keeps
/// the on-wire shape decoupled from the in-process struct so future
/// schema mutations on the parent's side don't force the subprocess
/// to be rebuilt in lockstep — the parent decodes this into a
/// `Binding` after framing has been resolved.
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
        // The L5 dispatcher consults this analyser whenever the
        // candidate's manifest set or language tags signal Python.
        // Multiple non-empty fields combine with OR semantics in
        // [`atlas_analyzers::subprocess`]'s `applicability_matches`.
        applicability_predicate: ApplicabilityPredicate {
            languages: vec!["python".into()],
            file_globs: vec![
                "**/pyproject.toml".into(),
                "**/setup.py".into(),
                "**/requirements.txt".into(),
            ],
            manifest_types: vec!["python".into()],
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
            // Parent closed stdin → graceful exit.
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
        // The proxy short-circuits `applies` locally against the
        // declared predicate; the wire form is honoured here for
        // future direct dispatch.
        Request::Applies { .. } => Response::Confident {
            payload: serde_json::Value::Bool(true),
        },
        Request::FingerprintInputs { target } => {
            // Contributors:
            //
            // - Every pre-loaded manifest's content_sha (so a
            //   `pyproject.toml` change reshapes the L5 cache key).
            // - Every Python source file's content_sha discovered
            //   under `target.dir` (the analyser's actual input).
            let mut inputs: Vec<WireFingerprintInput> = target
                .manifests
                .iter()
                .map(|m| WireFingerprintInput::FileContentSha {
                    sha: m.content_sha.clone(),
                })
                .collect();
            for (_rel, bytes) in walk_python_sources(Path::new(&target.dir)) {
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

    let pyproject_toml = pyproject_bytes_from_target(target);
    let sources = walk_python_sources(&dir);
    let inputs = atlas_python_analyzer::PythonSourceInputs {
        sources,
        pyproject_toml,
    };
    let surface = extract_python_surface(&component_id, &inputs);

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
        // The wire form carries the kebab-case string for cross-
        // language consistency; the parent decodes it back into the
        // typed enum.
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

fn pub_item_kind_str(kind: atlas_index::PubItemKind) -> &'static str {
    use atlas_index::PubItemKind;
    match kind {
        PubItemKind::Struct => "struct",
        PubItemKind::Enum => "enum",
        PubItemKind::Fn => "fn",
        PubItemKind::Trait => "trait",
        PubItemKind::Mod => "mod",
        PubItemKind::TypeAlias => "type-alias",
        PubItemKind::Const => "const",
        PubItemKind::Static => "static",
        PubItemKind::Union => "union",
        PubItemKind::Macro => "macro",
    }
}

/// Extract the `pyproject.toml` bytes from the parent's pre-loaded
/// manifest list. The `bytes_b64` field is base64; we decode here.
/// Returns `None` if no pyproject is shipped.
fn pyproject_bytes_from_target(target: &WireTarget) -> Option<Vec<u8>> {
    target
        .manifests
        .iter()
        .find(|m| m.name == "pyproject.toml")
        .and_then(|m| BASE64.decode(&m.bytes_b64).ok())
}

/// Component id heuristic for the analyse path: prefer the
/// `pyproject.toml`'s declared project name; fall back to the dir
/// basename. The actual canonical id is set by the parent's L4 walk;
/// this string only flows into `LibraryApi.id` (`<id>/public-api`)
/// and is overwritten if the parent wishes to canonicalise.
fn component_id_from_target(target: &WireTarget) -> String {
    if let Some(bytes) = pyproject_bytes_from_target(target) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Some(name) = atlas_python_analyzer::extract_pyproject_project_name(text) {
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

/// Walk the candidate directory recursively, returning every `*.py`
/// (and `*.pyi`) file as `(relative_path, bytes)`. Symlinks are not
/// followed; files larger than 4 MiB are skipped (defensive against
/// generated mega-files). The walk is depth-first and lexicographic
/// so output ordering is stable across runs on the same tree.
fn walk_python_sources(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    const MAX_PY_FILE_BYTES: u64 = 4 * 1024 * 1024;
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
            // Skip standard hidden / virtualenv noise.
            let basename = entry.file_name();
            if let Some(s) = basename.to_str() {
                if s.starts_with('.') || s == "__pycache__" || s == "venv" || s == ".venv" {
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
            if metadata.len() > MAX_PY_FILE_BYTES {
                continue;
            }
            let extension = path.extension().and_then(OsStr::to_str);
            if !matches!(extension, Some("py") | Some("pyi")) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            // Make the relpath relative to the original dir so
            // `Binding.file` is portable across runs / machines.
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
