//! L5 surface extraction — for one component, call Stage 1 and produce
//! a parsed [`SurfaceRecord`].
//!
//! The query is not `#[salsa::tracked]`. Like L3 it takes
//! `&AtlasDatabase` and drops through the non-Salsa LLM backend;
//! incremental memoisation at the response level happens in
//! [`crate::llm_cache`] which is keyed by the fingerprint + request
//! inputs. That satisfies the "zero LLM calls on no-op re-run" contract
//! without requiring a Salsa downcast.
//!
//! ## Pin short-circuit
//!
//! `overrides.pins[<id>]["surface"]` may carry a `PinValue::Value` whose
//! `value` is a YAML-serialised [`SurfaceRecord`]. When present, L5
//! parses it and returns it directly — no LLM call. This is a rare
//! manual escape hatch (design §4.1 L5) for components whose surface
//! the LLM cannot produce well on its own.

use std::path::PathBuf;
use std::sync::Arc;

use atlas_analyzers::{
    cached_csharp_subprocess_proxy, cached_dart_subprocess_proxy, cached_elixir_subprocess_proxy,
    cached_lispkit_subprocess_proxy, cached_racket_subprocess_proxy, cached_subprocess_proxy,
    csharp_subprocess_spec, dart_subprocess_spec, elixir_subprocess_spec, extract_rust_surface,
    extract_ts_js_surface, lispkit_subprocess_spec, locate_csharp_analyzer_binary,
    locate_dart_analyzer_binary, locate_elixir_analyzer_binary, locate_lispkit_analyzer_binary,
    locate_python_analyzer_binary, locate_racket_analyzer_binary, python_subprocess_spec,
    racket_subprocess_spec, Analyzer, AnalyzerResult, RustSourceInputs, SubprocessOutput,
    TsJsSourceInputs,
};
use atlas_index::{Binding, ComponentEntry, Contract, LibraryApi, OverridesFile, PinValue, Stage};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use component_ontology::ComponentId;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::AtlasDatabase;
use crate::l1_queries::file_content;
use crate::l4_tree::all_components;
use crate::roots::best_root_for;
use crate::surface_types::SurfaceRecord;

/// The shipped Atlas Stage 1 prompt, embedded at compile time. Exposed
/// so the atlas-cli driver can compute the prompt SHA that feeds
/// [`atlas_llm::LlmFingerprint`] without re-reading the file from
/// disk.
pub const EMBEDDED_STAGE1_SURFACE_PROMPT: &str =
    include_str!("../../../defaults/prompts/stage1-surface.md");

/// Driver version baked into the L5 stage fingerprint (PR-10). Bumping
/// this invalidates every L5 persistent-cache entry — the only reason
/// to bump it is a structural change in `surface_of`'s call shape (new
/// inputs key, new contract on the response shape). Cosmetic edits do
/// not justify a bump.
pub const L5_DRIVER_VERSION: &str = "1.0.0";

/// Keys produced by [`build_inputs`]. Single source of truth for the
/// bidirectional template/builder coverage check in
/// [`crate::prompt_token_coverage`]: validated against
/// `stage1-surface.md` at compile time, and against the runtime builder
/// output by a unit test.
pub(crate) const BUILD_INPUTS_KEYS: &[&str] = &[
    "COMPONENT_ID",
    "COMPONENT_PATHS",
    "COMPONENT_CONTENT_SHAS",
    "CATALOG_COMPONENTS",
];

/// Per-segment content SHAs are baked into the inputs JSON so a
/// file-content change reshapes the cache key, even though the LLM
/// doesn't see the SHAs in the prompt.
pub(crate) const CACHE_ONLY_KEYS: &[&str] = &["COMPONENT_CONTENT_SHAS"];

/// Produce the Stage 1 surface record for the component whose id is
/// `id`. Pin-short-circuits through [`surface_pin`] before any LLM
/// call; otherwise invokes the backend via
/// [`AtlasDatabase::call_llm_cached`] so repeated calls at the same
/// revision are free.
///
/// If the id does not resolve to an internal component (or the
/// component has no path_segments), returns a default
/// [`SurfaceRecord`] — the engine is intentionally non-panicking on
/// unknown ids so callers can probe freely.
pub fn surface_of(db: &AtlasDatabase, id: ComponentId) -> Arc<SurfaceRecord> {
    // Resolve the component. `all_components` does the L2→L4 walk;
    // Salsa caches the result across repeated surface_of calls that
    // share a revision.
    let components = all_components(db);
    let Some(entry) = components.iter().find(|c| c.id == id && !c.deleted) else {
        return Arc::new(SurfaceRecord::default());
    };

    let workspace = db.workspace();
    let overrides = workspace
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    if let Some(pinned) = surface_pin(&overrides, &id) {
        return Arc::new(pinned);
    }

    let peer_ids: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted && c.id != id)
        .map(|c| c.id.as_str().to_string())
        .collect();

    let inputs = build_inputs(entry, &peer_ids);
    let request = LlmRequest {
        prompt_template: PromptId::Stage1Surface,
        inputs: inputs.clone(),
        schema: ResponseSchema::accept_any(),
    };

    // PR-10: L5 stage fingerprint per design §8.1. Contributors:
    //
    // - `analyzer_registry_sha` — registry-shape change invalidates
    //   (consistent with L3);
    // - `llm_fingerprint` — model / backend / template / ontology;
    // - `prompt_sha` — sha of the rendered (canonical-JSON) inputs
    //   the backend will receive, so peer-id churn or path-segment
    //   churn invalidates;
    // - `file_content_sha` — every path segment's content sha, so a
    //   file-content change inside the component invalidates only
    //   the entries that named that component's content.
    //
    // The component ID itself is not contributed separately because
    // (a) the `prompt_sha` already includes it (it appears in the
    // canonical-JSON inputs) and (b) two components with the same
    // id, peers, and content cannot exist within a single workspace
    // — id uniqueness is an L4 invariant.
    let llm_fp = workspace
        .llm_fingerprint(db as &dyn salsa::Database)
        .clone();
    let rendered_prompt_sha = sha256_hex_bytes(
        serde_json::to_string(&inputs)
            .unwrap_or_default()
            .as_bytes(),
    );
    let registry_sha = db.analyzer_registry().registry_sha();
    // Phase 2 PR-3: Python components contribute the python-analyzer
    // binary's content sha to the L5 fingerprint via tag 0x06. PR-6 / PR-7 / PR-8
    // generalise the same posture for C# / Dart / Elixir; each contribution
    // is conditional on the component's language so unrelated components are
    // not coupled to other language analysers' binary churn.
    let python_binary_sha = if entry_is_python(entry) {
        locate_python_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    let csharp_binary_sha = if entry_is_csharp(entry) {
        locate_csharp_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    let dart_binary_sha = if entry_is_dart(entry) {
        locate_dart_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    let elixir_binary_sha = if entry_is_elixir(entry) {
        locate_elixir_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    // Phase 2 PR-9: Racket components contribute the racket-analyzer
    // binary's content sha to the L5 fingerprint via tag 0x06.
    let racket_binary_sha = if entry_is_racket(entry) {
        locate_racket_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    // Phase 2 PR-10: LispKit components contribute the lispkit-analyzer
    // binary's content sha to the L5 fingerprint via tag 0x06.
    let lispkit_binary_sha = if entry_is_lispkit(entry) {
        locate_lispkit_analyzer_binary().and_then(|p| atlas_analyzers::hash_binary(&p).ok())
    } else {
        None
    };
    let l5_fingerprint = {
        let mut fb = crate::FingerprintBuilder::new(Stage::L5, "l5-driver", L5_DRIVER_VERSION);
        fb.add_analyzer_registry_sha(&registry_sha);
        fb.add_llm_fingerprint(llm_fp.as_ref());
        fb.add_prompt_sha(&rendered_prompt_sha);
        for seg in &entry.path_segments {
            fb.add_file_content_sha(&seg.content_sha);
        }
        if let Some(sha) = &python_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        if let Some(sha) = &csharp_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        if let Some(sha) = &dart_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        if let Some(sha) = &elixir_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        if let Some(sha) = &racket_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        if let Some(sha) = &lispkit_binary_sha {
            fb.add_analyzer_binary_sha(sha);
        }
        fb.finalise()
    };

    let value = match db.call_llm_cached_with_fp(Stage::L5, &l5_fingerprint, &request) {
        Ok(v) => v,
        Err(err) => {
            // Conservative failure mode: empty surface annotated with
            // the error in `notes`. The caller (L6 / CLI) can decide
            // whether to surface this or skip. Matches L3's "soft fail"
            // behaviour rather than panicking deep in the pipeline.
            return Arc::new(SurfaceRecord {
                notes: format!("LLM call failed: {err}"),
                ..SurfaceRecord::default()
            });
        }
    };

    match parse_surface_response(&value) {
        Ok(record) => Arc::new(record),
        Err(reason) => Arc::new(SurfaceRecord {
            notes: format!("LLM response parse failed: {reason}"),
            ..SurfaceRecord::default()
        }),
    }
}

/// Combined surface artefacts for one component: the LLM-derived
/// [`SurfaceRecord`] (the inner record, unchanged from PR-5) plus the
/// PR-7 contract / binding / library-api projections produced by the
/// deterministic Rust-surface analyser.
///
/// Plan §4 PR-7 wording — "extend `SurfaceRecord` (kept as the inner
/// record) with new top-level fields" — is honoured by carrying the
/// inner `SurfaceRecord` verbatim and adding three peer fields:
/// `contracts`, `bindings`, `library_apis`. The L9
/// `surfaces_yaml_snapshot` projects from this struct onto
/// [`atlas_index::SurfacesFile`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceArtefacts {
    /// LLM-derived purpose / consumes / produces / etc. record.
    /// PR-5's existing shape — see [`SurfaceRecord`].
    pub record: SurfaceRecord,
    /// Code-derived contracts the component defines (currently
    /// `data-format` contracts from `pub struct ... #[derive(Serialize,
    /// Deserialize)]`). One [`Contract`] per defining binding.
    pub contracts: Vec<Contract>,
    /// Bindings extracted by the deterministic Rust-surface analyser.
    /// Mirrors the bindings appearing in `contracts[i].definition_binding`.
    pub bindings: Vec<Binding>,
    /// In-process library APIs. Phase 1 emits Rust only — at most one
    /// entry, populated when the component exposes any `pub` items.
    pub library_apis: Vec<LibraryApi>,
}

/// Produce the full surface artefact bundle for one component. Calls
/// the LLM via [`surface_of`] for the `SurfaceRecord` inner, then runs
/// the deterministic Rust-surface analyser
/// ([`atlas_analyzers::extract_rust_surface`]) over the component's
/// `src/lib.rs` and `src/main.rs` (when present) to produce contracts,
/// bindings, and library-api items.
///
/// Phase 1 limitations (plan §6 risks):
///
/// - Nested `pub mod foo { pub struct Bar; }` is missed (top-level
///   scan only).
/// - Only Rust components produce contracts / library-apis; non-Rust
///   components return an empty artefact set in those fields.
/// - Component path resolution uses
///   [`crate::roots::best_root_for`]; multi-segment components emit
///   from the first segment that contains a recognised source file.
pub fn surface_artefacts_of(db: &AtlasDatabase, id: ComponentId) -> Arc<SurfaceArtefacts> {
    // 1. Inner record via the existing LLM-driven path.
    let record = (*surface_of(db, id.clone())).clone();

    // 2. Resolve the component's on-disk source files to feed into
    //    the deterministic Rust-surface analyser. Component path
    //    segments are relative to one of the workspace roots; pick
    //    the longest-prefix match.
    let workspace = db.workspace();
    let roots = workspace.roots(db as &dyn salsa::Database).clone();
    let components = all_components(db);
    let Some(entry) = components.iter().find(|c| c.id == id && !c.deleted) else {
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    };

    // 3a-python. Python branch — Phase 2 PR-3's first subprocess
    //     analyser. A component is handled here when it carries
    //     `python` in its language set or its kind is one of the
    //     Python kinds (`python-library`, `python-app`, the new
    //     `python-package` once a Python analyser-registered
    //     classifier emits it). The L5 driver constructs a
    //     [`SubprocessAnalyzerProxy`] on demand against the
    //     `python-analyzer` binary located via
    //     [`locate_python_analyzer_binary`] and invokes the proxy
    //     directly. If the binary cannot be located (running outside
    //     a cargo target tree or against a workspace where the
    //     python analyser wasn't built), the artefact set is empty
    //     for the component — same posture as the TS/JS branch when
    //     no source is recognised.
    if entry_is_python(entry) {
        if let Some(artefacts) = python_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        // Fallthrough: no python-analyzer binary located. Emit empty
        // artefacts (same as the catch-all branch below).
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a-csharp. C# branch — Phase 2 PR-6. A component is handled here
    //     when it carries `csharp` in its language set or its kind is
    //     one of the C# kinds (`csharp-project`, `csharp-solution`).
    //     The L5 driver constructs a [`SubprocessAnalyzerProxy`] on
    //     demand against the `csharp-analyzer` binary and invokes it
    //     directly. If the binary cannot be located the artefact set is
    //     empty for the component.
    if entry_is_csharp(entry) {
        if let Some(artefacts) = csharp_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a-dart. Dart branch — Phase 2 PR-7's subprocess analyser. A
    //     component is handled here when it carries `dart` in its language
    //     set or its kind is one of the Dart/Flutter kinds. The L5 driver
    //     constructs a [`SubprocessAnalyzerProxy`] on demand against the
    //     `dart-analyzer` binary located via [`locate_dart_analyzer_binary`].
    //     If the binary cannot be located, the artefact set is empty —
    //     same posture as the Python branch.
    if entry_is_dart(entry) {
        if let Some(artefacts) = dart_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a-elixir. Elixir branch — Phase 2 PR-8's subprocess analyser.
    //     A component is handled here when it carries `elixir` in its
    //     language set or its kind is `elixir-project`. The L5 driver
    //     constructs a [`SubprocessAnalyzerProxy`] on demand against the
    //     `elixir-analyzer` binary located via
    //     [`locate_elixir_analyzer_binary`] and invokes the proxy
    //     directly. If the binary cannot be located (running outside a
    //     cargo target tree), the artefact set is empty for the
    //     component.
    if entry_is_elixir(entry) {
        if let Some(artefacts) = elixir_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a-racket. Racket branch — Phase 2 PR-9's subprocess analyser.
    //     A component is handled here when it carries `racket` in its
    //     language set or its kind is `racket-package`. The L5 driver
    //     constructs a [`SubprocessAnalyzerProxy`] on demand against the
    //     `racket-analyzer` binary located via
    //     [`locate_racket_analyzer_binary`] and invokes the proxy
    //     directly. If the binary cannot be located, the artefact set is
    //     empty — same posture as the Python branch.
    if entry_is_racket(entry) {
        if let Some(artefacts) = racket_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a-lispkit. LispKit branch — Phase 2 PR-10's subprocess analyser.
    //     A component is handled here when its kind is `lispkit-package`
    //     or it carries `scheme` / `lispkit` in its language set.
    if entry_is_lispkit(entry) {
        if let Some(artefacts) = lispkit_surface_artefacts(db, entry, &roots, &record) {
            return artefacts;
        }
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 3a. TypeScript / JavaScript branch — drive the TS/JS-surface
    //     extractor in-process. A component is handled here when it
    //     carries "typescript" or "javascript" in its language set, or
    //     its kind is a recognised TS/JS package kind.
    let is_ts_js = entry.languages.contains("typescript")
        || entry.languages.contains("javascript")
        || entry.kind == "typescript-package"
        || entry.kind == "javascript-package";

    if is_ts_js {
        let is_typescript =
            entry.languages.contains("typescript") || entry.kind == "typescript-package";

        let mut sources: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let mut package_json: Option<Vec<u8>> = None;

        for segment in &entry.path_segments {
            let candidate_roots: Vec<PathBuf> = if segment.path.is_absolute() {
                vec![PathBuf::new()]
            } else if let Some(owning_root) = best_root_for(&roots, &segment.path) {
                vec![owning_root.to_path_buf()]
            } else {
                roots.clone()
            };

            for root in &candidate_roots {
                let absolute_dir = if segment.path.is_absolute() {
                    segment.path.clone()
                } else {
                    root.join(&segment.path)
                };

                // Collect well-known source files: `src/<name>.<ext>`.
                // Phase 1 probes the `src/` subdirectory only; deeper
                // nesting is Phase 2 (full tree walk). This is the
                // simplest pattern that works for the integration
                // fixture (which has just `src/index.ts`).
                for filename in &[
                    "src/index.ts",
                    "src/index.tsx",
                    "src/index.js",
                    "src/index.jsx",
                    "src/main.ts",
                    "src/main.tsx",
                    "src/main.js",
                    "src/main.jsx",
                ] {
                    let candidate = absolute_dir.join(filename);
                    if let Some(bytes) = file_content(db, &candidate) {
                        let rel = PathBuf::from(filename);
                        if !sources.iter().any(|(p, _)| p == &rel) {
                            sources.push((rel, (*bytes).clone()));
                        }
                    }
                }

                // Also read `package.json` for entrypoint resolution.
                if package_json.is_none() {
                    let pkg_candidate = absolute_dir.join("package.json");
                    if let Some(bytes) = file_content(db, &pkg_candidate) {
                        package_json = Some((*bytes).clone());
                    }
                }
            }
        }

        let inputs = TsJsSourceInputs {
            sources,
            package_json,
            is_typescript,
        };
        let surface_output = extract_ts_js_surface(id.as_str(), &inputs);

        return Arc::new(SurfaceArtefacts {
            record,
            contracts: surface_output.contracts,
            bindings: surface_output.bindings,
            library_apis: surface_output.library_apis,
        });
    }

    // 3b. Skip non-Rust, non-TS/JS components: only the deterministic
    //     scanner produces output, and it only knows Rust. A polyglot
    //     component that *includes* Rust still gets the Rust subset.
    if !entry.languages.contains("rust") && entry.kind != "rust-library" && entry.kind != "rust-cli"
    {
        return Arc::new(SurfaceArtefacts {
            record,
            ..Default::default()
        });
    }

    // 4. Read the well-known Rust source files. Phase 1 looks at
    //    `src/lib.rs` and `src/main.rs` only; nested modules are
    //    Phase 2 (rust-analyzer wire-up).
    let mut sources: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for segment in &entry.path_segments {
        // Absolute segment paths short-circuit: there is exactly one
        // candidate (the segment itself), no roots involved. We handle
        // this as an early step so the relative-path branch below can
        // reason in terms of `roots` alone.
        if segment.path.is_absolute() {
            for filename in ["src/lib.rs", "src/main.rs"] {
                let candidate = segment.path.join(filename);
                if let Some(bytes) = file_content(db, &candidate) {
                    let rel = PathBuf::from(filename);
                    if !sources.iter().any(|(p, _)| p == &rel) {
                        sources.push((rel, (*bytes).clone()));
                    }
                }
            }
            continue;
        }

        // Relative segment: probe each candidate root for the one whose
        // `<root>/<segment>/src/{lib,main}.rs` resolves to known file
        // bytes. Load-bearing for cross-tree (peer-root) components
        // whose `path_segments[0].path` is empty or root-leaf-only —
        // for those, `<root>/<segment>` is itself a directory under
        // every root and the only signal that distinguishes them is the
        // presence of the Rust source files we're about to read.
        let candidate_roots: Vec<PathBuf> =
            if let Some(owning_root) = best_root_for(&roots, &segment.path) {
                // The relative path happens to also be a descendant of
                // some root (rare; produced by overrides that synthesise
                // a path that resolves under one root unambiguously).
                vec![owning_root.to_path_buf()]
            } else {
                roots.clone()
            };

        for filename in ["src/lib.rs", "src/main.rs"] {
            for root in &candidate_roots {
                let candidate = root.join(&segment.path).join(filename);
                if let Some(bytes) = file_content(db, &candidate) {
                    let rel = PathBuf::from(filename);
                    // De-duplicate against earlier segments contributing
                    // the same relative path (rare, but possible for
                    // overlapping segment definitions).
                    if !sources.iter().any(|(p, _)| p == &rel) {
                        sources.push((rel, (*bytes).clone()));
                    }
                    break;
                }
            }
        }
    }

    let inputs = RustSourceInputs { sources };
    let surface_output = extract_rust_surface(id.as_str(), &inputs);

    Arc::new(SurfaceArtefacts {
        record,
        contracts: surface_output.contracts,
        bindings: surface_output.bindings,
        library_apis: surface_output.library_apis,
    })
}

/// True when the component looks like a Python component to the L5
/// branch logic. Centralised here so `surface_of`'s fingerprint
/// computation and `surface_artefacts_of`'s python branch share the
/// same predicate.
fn entry_is_python(entry: &ComponentEntry) -> bool {
    entry.languages.contains("python")
        || entry.kind == "python-library"
        || entry.kind == "python-app"
        || entry.kind == "python-package"
}

/// True when the component looks like a Racket component to the L5
/// branch logic.
fn entry_is_racket(entry: &ComponentEntry) -> bool {
    entry.languages.contains("racket") || entry.kind == "racket-package"
}

/// True when the component looks like a LispKit component to the L5
/// branch logic (Phase 2 PR-10).
fn entry_is_lispkit(entry: &ComponentEntry) -> bool {
    entry.kind == "lispkit-package"
        || entry.languages.contains("scheme")
        || entry.languages.contains("lispkit")
}

/// Drive a Python-component's surface extraction through PR-2's
/// subprocess transport.
///
/// Discovery: walks `entry.path_segments` against the workspace roots
/// to resolve the absolute candidate dir, locates the
/// `python-analyzer` binary via
/// [`atlas_analyzers::locate_python_analyzer_binary`], constructs a
/// [`SubprocessAnalyzerProxy`] against it, and invokes the proxy's
/// `analyse` method. The proxy spawns the child on first call,
/// performs the wire handshake, and returns a `Confident` payload
/// carrying the JSON surface produced by `extract_python_surface`.
///
/// Returns `None` when the binary cannot be located — the caller
/// degrades to empty artefacts. Other failure modes (handshake
/// mismatch, child crash, malformed payload) return `Some` with an
/// empty artefact set so the failure is observable in tests but does
/// not panic the pipeline.
fn python_surface_artefacts(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_python_analyzer_binary()?;

    // Resolve the candidate dir — first segment that resolves
    // against any root wins.
    let absolute_dir = resolve_component_dir_first_segment(entry, roots)?;

    // Build a minimal `Target` for the proxy. We pre-load
    // `pyproject.toml` (the manifest the analyser cares about) so
    // the subprocess sees it via `Target.manifests`. The python
    // analyser's filesystem walk handles the source files itself
    // (per the binary's docstring).
    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    let pyproject_path = absolute_dir.join("pyproject.toml");
    if let Some(bytes) = file_content(db, &pyproject_path) {
        let bytes_vec = (*bytes).clone();
        let content_sha = sha256_hex_bytes(&bytes_vec);
        manifests.push(atlas_analyzers::TargetFile {
            name: "pyproject.toml".into(),
            relpath: PathBuf::from("pyproject.toml"),
            bytes: bytes_vec,
            content_sha,
        });
    }
    let mut languages = std::collections::BTreeSet::new();
    languages.insert("python".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = python_subprocess_spec(binary);
    // F-CQ-1: amortise proxy construction (which hashes the binary
    // and primes the process pool) across all Python components in
    // the workspace. Without the cache, a workspace with N Python
    // components incurred N binary hashes + N spawns; with it,
    // exactly one of each.
    let proxy = cached_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis) =
        decode_subprocess_surface_payload(&payload, entry.id.as_str(), "python");
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts: Vec::new(),
        bindings,
        library_apis,
    }))
}

/// Resolve a component's first path segment against the workspace
/// roots, returning the absolute on-disk dir for the candidate.
/// Language-agnostic: used by both the Python and Racket surface
/// extraction paths. Mirrors the per-segment walk used by the TS/JS
/// branch but stops at the first match (single-segment component paths
/// are the canonical form for Python and Racket).
fn resolve_component_dir_first_segment(
    entry: &ComponentEntry,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    let segment = entry.path_segments.first()?;
    if segment.path.is_absolute() {
        return Some(segment.path.clone());
    }
    if let Some(owning_root) = best_root_for(roots, &segment.path) {
        return Some(owning_root.join(&segment.path));
    }
    for root in roots {
        let absolute = root.join(&segment.path);
        if absolute.is_dir() {
            return Some(absolute);
        }
    }
    Some(roots.first()?.join(&segment.path))
}

/// Drive a Racket-component's surface extraction through PR-2's
/// subprocess transport. Mirrors [`python_surface_artefacts`] exactly
/// but uses the `racket-analyzer` binary.
fn racket_surface_artefacts(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_racket_analyzer_binary()?;

    let absolute_dir = resolve_component_dir_first_segment(entry, roots)?;

    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    let info_rkt_path = absolute_dir.join("info.rkt");
    if let Some(bytes) = file_content(db, &info_rkt_path) {
        let bytes_vec = (*bytes).clone();
        let content_sha = sha256_hex_bytes(&bytes_vec);
        manifests.push(atlas_analyzers::TargetFile {
            name: "info.rkt".into(),
            relpath: PathBuf::from("info.rkt"),
            bytes: bytes_vec,
            content_sha,
        });
    }
    let mut languages = std::collections::BTreeSet::new();
    languages.insert("racket".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = racket_subprocess_spec(binary);
    let proxy = cached_racket_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis) = decode_racket_surface_payload(&payload, entry.id.as_str());
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts: Vec::new(),
        bindings,
        library_apis,
    }))
}

/// Decode the JSON payload returned by the racket-analyzer subprocess.
/// The wire shape is identical to the python-analyzer's shape; the
/// same decoder logic applies with `language: "racket"` substituted.
fn decode_racket_surface_payload(
    payload: &Value,
    component_id: &str,
) -> (Vec<Binding>, Vec<LibraryApi>) {
    use atlas_index::{ContractKind, PubItem, PubItemKind, Visibility};
    use std::collections::BTreeMap as StdBTreeMap;

    let Some(obj) = payload.as_object() else {
        return (Vec::new(), Vec::new());
    };

    let mut bindings: Vec<Binding> = Vec::new();
    if let Some(arr) = obj.get("bindings").and_then(Value::as_array) {
        for v in arr {
            let Some(b) = v.as_object() else { continue };
            let language = b
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("racket")
                .to_string();
            let symbol = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let file = b
                .get("file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let span = b
                .get("span")
                .and_then(Value::as_array)
                .and_then(|a| {
                    let s = a.first().and_then(Value::as_u64)? as usize;
                    let e = a.get(1).and_then(Value::as_u64)? as usize;
                    Some((s, e))
                })
                .unwrap_or((0, 0));
            let content_sha = b
                .get("content_sha")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let visibility = b
                .get("visibility")
                .map(|v| {
                    serde_json::from_value::<Visibility>(v.clone())
                        .unwrap_or(Visibility::Conventional)
                })
                .unwrap_or(Visibility::Conventional);
            let module_path = b
                .get("module_path")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let attributes: StdBTreeMap<String, serde_yaml::Value> = b
                .get("attributes")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            let yaml: serde_yaml::Value = serde_json::from_value(v.clone()).ok()?;
                            Some((k.clone(), yaml))
                        })
                        .collect()
                })
                .unwrap_or_default();
            bindings.push(Binding {
                language,
                symbol,
                file,
                span,
                content_sha,
                visibility,
                module_path,
                attributes,
            });
        }
    }

    let mut library_apis: Vec<LibraryApi> = Vec::new();
    if let Some(arr) = obj.get("library_apis").and_then(Value::as_array) {
        for v in arr {
            let Some(api) = v.as_object() else { continue };
            let id = api
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("{component_id}/public-api"));
            let language = api
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("racket")
                .to_string();
            let fingerprint = api
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pub_items: Vec<PubItem> = api
                .get("pub_items")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_object()?;
                            let name = p.get("name").and_then(Value::as_str)?.to_string();
                            let file = p.get("file").and_then(Value::as_str).map(PathBuf::from)?;
                            let kind_str = p.get("kind").and_then(Value::as_str)?;
                            let kind = match kind_str {
                                "struct" => PubItemKind::Struct,
                                "enum" => PubItemKind::Enum,
                                "fn" => PubItemKind::Fn,
                                "trait" => PubItemKind::Trait,
                                "mod" => PubItemKind::Mod,
                                "type-alias" => PubItemKind::TypeAlias,
                                "const" => PubItemKind::Const,
                                "static" => PubItemKind::Static,
                                "union" => PubItemKind::Union,
                                "macro" => PubItemKind::Macro,
                                _ => return None,
                            };
                            Some(PubItem { name, file, kind })
                        })
                        .collect()
                })
                .unwrap_or_default();
            library_apis.push(LibraryApi {
                id,
                kind: ContractKind::LibraryApi,
                language,
                fingerprint,
                pub_items,
            });
        }
    }

    (bindings, library_apis)
}

/// Decode the JSON payload returned by a subprocess analyser
/// into typed `Binding` / `LibraryApi` values. The wire shape is
/// defined inside each analyser binary (`AnalysePayload`,
/// `WireBinding`, `WireLibraryApi`); we re-derive the same types
/// here as they're load-bearing for the engine-side projection.
///
/// `default_language` is used when the wire payload omits the
/// `language` field on a binding or library-api object.  Pass
/// `"python"` for the Python analyser and `"scheme"` for LispKit.
fn decode_subprocess_surface_payload(
    payload: &Value,
    component_id: &str,
    default_language: &str,
) -> (Vec<Binding>, Vec<LibraryApi>) {
    use atlas_index::{ContractKind, PubItem, PubItemKind, Visibility};
    use std::collections::BTreeMap as StdBTreeMap;

    let Some(obj) = payload.as_object() else {
        return (Vec::new(), Vec::new());
    };

    // Decode bindings.
    let mut bindings: Vec<Binding> = Vec::new();
    if let Some(arr) = obj.get("bindings").and_then(Value::as_array) {
        for v in arr {
            let Some(b) = v.as_object() else { continue };
            let language = b
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or(default_language)
                .to_string();
            let symbol = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let file = b
                .get("file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let span = b
                .get("span")
                .and_then(Value::as_array)
                .and_then(|a| {
                    let s = a.first().and_then(Value::as_u64)? as usize;
                    let e = a.get(1).and_then(Value::as_u64)? as usize;
                    Some((s, e))
                })
                .unwrap_or((0, 0));
            let content_sha = b
                .get("content_sha")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let visibility = b
                .get("visibility")
                .map(|v| {
                    serde_json::from_value::<Visibility>(v.clone())
                        .unwrap_or(Visibility::Conventional)
                })
                .unwrap_or(Visibility::Conventional);
            let module_path = b
                .get("module_path")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let attributes: StdBTreeMap<String, serde_yaml::Value> = b
                .get("attributes")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            // Convert JSON → YAML losslessly via the
                            // generic `serde_json::from_value`
                            // adapter into `serde_yaml::Value`. The
                            // earlier two-step idiom
                            // (`serde_yaml::from_str(&v.to_string())`)
                            // round-tripped *strings* through the
                            // YAML parser, which corrupts values
                            // containing YAML-special characters
                            // (`#`, `:`, `|`, `&`, `*`, `!`, …) —
                            // see PR-3 code-quality F-CQ-3 for the
                            // pathological case. `from_value` walks
                            // the JSON tree directly and emits the
                            // matching YAML primitive, so a JSON
                            // string `"key: value"` round-trips as a
                            // YAML *scalar* string rather than being
                            // re-parsed as a YAML mapping.
                            let yaml: serde_yaml::Value = serde_json::from_value(v.clone()).ok()?;
                            Some((k.clone(), yaml))
                        })
                        .collect()
                })
                .unwrap_or_default();
            bindings.push(Binding {
                language,
                symbol,
                file,
                span,
                content_sha,
                visibility,
                module_path,
                attributes,
            });
        }
    }

    // Decode library APIs.
    let mut library_apis: Vec<LibraryApi> = Vec::new();
    if let Some(arr) = obj.get("library_apis").and_then(Value::as_array) {
        for v in arr {
            let Some(api) = v.as_object() else { continue };
            let id = api
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("{component_id}/public-api"));
            let language = api
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or(default_language)
                .to_string();
            let fingerprint = api
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pub_items: Vec<PubItem> = api
                .get("pub_items")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_object()?;
                            let name = p.get("name").and_then(Value::as_str)?.to_string();
                            let file = p.get("file").and_then(Value::as_str).map(PathBuf::from)?;
                            let kind_str = p.get("kind").and_then(Value::as_str)?;
                            let kind = match kind_str {
                                "struct" => PubItemKind::Struct,
                                "enum" => PubItemKind::Enum,
                                "fn" => PubItemKind::Fn,
                                "trait" => PubItemKind::Trait,
                                "mod" => PubItemKind::Mod,
                                "type-alias" => PubItemKind::TypeAlias,
                                "const" => PubItemKind::Const,
                                "static" => PubItemKind::Static,
                                "union" => PubItemKind::Union,
                                "macro" => PubItemKind::Macro,
                                _ => return None,
                            };
                            Some(PubItem { name, file, kind })
                        })
                        .collect()
                })
                .unwrap_or_default();
            library_apis.push(LibraryApi {
                id,
                kind: ContractKind::LibraryApi,
                language,
                fingerprint,
                pub_items,
            });
        }
    }

    (bindings, library_apis)
}

/// True when the component looks like a C# component to the L5
/// branch logic.
fn entry_is_csharp(entry: &ComponentEntry) -> bool {
    entry.languages.contains("csharp")
        || entry.kind == "csharp-project"
        || entry.kind == "csharp-solution"
}

/// Drive a C#-component's surface extraction through the subprocess
/// transport. Mirrors `python_surface_artefacts`.
///
/// Returns `None` when the binary cannot be located. Other failure
/// modes return `Some` with an empty artefact set.
fn csharp_surface_artefacts(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_csharp_analyzer_binary()?;

    let absolute_dir = resolve_csharp_component_dir(entry, roots)?;

    // Pre-load the *.csproj manifest (the analyser cares about it for
    // reference extraction). Walk the candidate dir for *.csproj.
    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&absolute_dir) {
        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.ends_with(".csproj") || name.ends_with(".sln") {
                if let Some(bytes) = file_content(db, &path) {
                    let bytes_vec = (*bytes).clone();
                    let content_sha = sha256_hex_bytes(&bytes_vec);
                    manifests.push(atlas_analyzers::TargetFile {
                        name,
                        relpath: path
                            .strip_prefix(&absolute_dir)
                            .unwrap_or(&path)
                            .to_path_buf(),
                        bytes: bytes_vec,
                        content_sha,
                    });
                }
            }
        }
    }

    let mut languages = std::collections::BTreeSet::new();
    languages.insert("csharp".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = csharp_subprocess_spec(binary);
    let proxy = cached_csharp_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis) = decode_csharp_surface_payload(&payload, entry.id.as_str());
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts: Vec::new(),
        bindings,
        library_apis,
    }))
}

/// Resolve a C# component's first path segment against the workspace
/// roots, returning the absolute on-disk dir. Mirrors
/// `resolve_python_component_dir`.
fn resolve_csharp_component_dir(entry: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let segment = entry.path_segments.first()?;
    if segment.path.is_absolute() {
        return Some(segment.path.clone());
    }
    if let Some(owning_root) = best_root_for(roots, &segment.path) {
        return Some(owning_root.join(&segment.path));
    }
    for root in roots {
        let absolute = root.join(&segment.path);
        if absolute.is_dir() {
            return Some(absolute);
        }
    }
    Some(roots.first()?.join(&segment.path))
}

/// Decode the JSON payload returned by the csharp-analyzer subprocess
/// into typed `Binding` / `LibraryApi` values. The wire shape is
/// identical to the Python analyser's shape (same `AnalysePayload`
/// struct mirrored in `csharp-analyzer`'s `main.rs`); this decoder
/// is a near-copy of `decode_python_surface_payload` with language
/// defaulting to `"csharp"`.
fn decode_csharp_surface_payload(
    payload: &Value,
    component_id: &str,
) -> (Vec<Binding>, Vec<LibraryApi>) {
    use atlas_index::{ContractKind, PubItem, PubItemKind, Visibility};
    use std::collections::BTreeMap as StdBTreeMap;

    let Some(obj) = payload.as_object() else {
        return (Vec::new(), Vec::new());
    };

    let mut bindings: Vec<Binding> = Vec::new();
    if let Some(arr) = obj.get("bindings").and_then(Value::as_array) {
        for v in arr {
            let Some(b) = v.as_object() else { continue };
            let language = b
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("csharp")
                .to_string();
            let symbol = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let file = b
                .get("file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let span = b
                .get("span")
                .and_then(Value::as_array)
                .and_then(|a| {
                    let s = a.first().and_then(Value::as_u64)? as usize;
                    let e = a.get(1).and_then(Value::as_u64)? as usize;
                    Some((s, e))
                })
                .unwrap_or((0, 0));
            let content_sha = b
                .get("content_sha")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let visibility = b
                .get("visibility")
                .map(|v| {
                    serde_json::from_value::<Visibility>(v.clone())
                        .unwrap_or(Visibility::Conventional)
                })
                .unwrap_or(Visibility::Conventional);
            let module_path = b
                .get("module_path")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let attributes: StdBTreeMap<String, serde_yaml::Value> = b
                .get("attributes")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            // JSON → YAML via serde_json::from_value (PR-3 F-CQ-3
                            // fix — avoids YAML-special-character corruption).
                            let yaml: serde_yaml::Value = serde_json::from_value(v.clone()).ok()?;
                            Some((k.clone(), yaml))
                        })
                        .collect()
                })
                .unwrap_or_default();
            bindings.push(Binding {
                language,
                symbol,
                file,
                span,
                content_sha,
                visibility,
                module_path,
                attributes,
            });
        }
    }

    let mut library_apis: Vec<LibraryApi> = Vec::new();
    if let Some(arr) = obj.get("library_apis").and_then(Value::as_array) {
        for v in arr {
            let Some(api) = v.as_object() else { continue };
            let id = api
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("{component_id}/public-api"));
            let language = api
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("csharp")
                .to_string();
            let fingerprint = api
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pub_items: Vec<PubItem> = api
                .get("pub_items")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_object()?;
                            let name = p.get("name").and_then(Value::as_str)?.to_string();
                            let file = p.get("file").and_then(Value::as_str).map(PathBuf::from)?;
                            let kind_str = p.get("kind").and_then(Value::as_str)?;
                            let kind = match kind_str {
                                "struct" => PubItemKind::Struct,
                                "enum" => PubItemKind::Enum,
                                "fn" => PubItemKind::Fn,
                                "trait" => PubItemKind::Trait,
                                "mod" => PubItemKind::Mod,
                                "type-alias" => PubItemKind::TypeAlias,
                                "const" => PubItemKind::Const,
                                "static" => PubItemKind::Static,
                                "union" => PubItemKind::Union,
                                "macro" => PubItemKind::Macro,
                                _ => return None,
                            };
                            Some(PubItem { name, file, kind })
                        })
                        .collect()
                })
                .unwrap_or_default();
            library_apis.push(LibraryApi {
                id,
                kind: ContractKind::LibraryApi,
                language,
                fingerprint,
                pub_items,
            });
        }
    }

    (bindings, library_apis)
}

/// True when the component looks like a Dart/Flutter component.
fn entry_is_dart(entry: &ComponentEntry) -> bool {
    entry.languages.contains("dart")
        || entry.kind == "dart-package"
        || entry.kind == "flutter-package"
        || entry.kind == "dart-library"
        || entry.kind == "dart-app"
        || entry.kind == "flutter-app"
}

/// Drive a Dart component's surface extraction through PR-2's subprocess
/// transport. Mirrors [`python_surface_artefacts`] structurally.
///
/// Returns `None` when the `dart-analyzer` binary cannot be located.
fn dart_surface_artefacts(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_dart_analyzer_binary()?;

    let absolute_dir = resolve_dart_component_dir(entry, roots)?;

    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    let pubspec_path = absolute_dir.join("pubspec.yaml");
    if let Some(bytes) = file_content(db, &pubspec_path) {
        let bytes_vec = (*bytes).clone();
        let content_sha = sha256_hex_bytes(&bytes_vec);
        manifests.push(atlas_analyzers::TargetFile {
            name: "pubspec.yaml".into(),
            relpath: PathBuf::from("pubspec.yaml"),
            bytes: bytes_vec,
            content_sha,
        });
    }
    let mut languages = std::collections::BTreeSet::new();
    languages.insert("dart".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = dart_subprocess_spec(binary);
    let proxy = cached_dart_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis) = decode_dart_surface_payload(&payload, entry.id.as_str());
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts: Vec::new(),
        bindings,
        library_apis,
    }))
}

/// Resolve a Dart component's first path segment against the workspace roots.
fn resolve_dart_component_dir(entry: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let segment = entry.path_segments.first()?;
    if segment.path.is_absolute() {
        return Some(segment.path.clone());
    }
    if let Some(owning_root) = best_root_for(roots, &segment.path) {
        return Some(owning_root.join(&segment.path));
    }
    for root in roots {
        let absolute = root.join(&segment.path);
        if absolute.is_dir() {
            return Some(absolute);
        }
    }
    Some(roots.first()?.join(&segment.path))
}

/// Decode the JSON payload returned by the dart-analyzer subprocess.
/// Mirrors [`decode_python_surface_payload`] with `"dart"` as the default
/// language tag.
fn decode_dart_surface_payload(
    payload: &Value,
    component_id: &str,
) -> (Vec<Binding>, Vec<LibraryApi>) {
    use atlas_index::{ContractKind, PubItem, PubItemKind, Visibility};
    use std::collections::BTreeMap as StdBTreeMap;

    let Some(obj) = payload.as_object() else {
        return (Vec::new(), Vec::new());
    };

    let mut bindings: Vec<Binding> = Vec::new();
    if let Some(arr) = obj.get("bindings").and_then(Value::as_array) {
        for v in arr {
            let Some(b) = v.as_object() else { continue };
            let language = b
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("dart")
                .to_string();
            let symbol = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let file = b
                .get("file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let span = b
                .get("span")
                .and_then(Value::as_array)
                .and_then(|a| {
                    let s = a.first().and_then(Value::as_u64)? as usize;
                    let e = a.get(1).and_then(Value::as_u64)? as usize;
                    Some((s, e))
                })
                .unwrap_or((0, 0));
            let content_sha = b
                .get("content_sha")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let visibility = b
                .get("visibility")
                .map(|v| {
                    serde_json::from_value::<Visibility>(v.clone())
                        .unwrap_or(Visibility::Conventional)
                })
                .unwrap_or(Visibility::Conventional);
            let module_path = b
                .get("module_path")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let attributes: StdBTreeMap<String, serde_yaml::Value> = b
                .get("attributes")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            // JSON → YAML via serde_json::from_value (PR-3 F-CQ-3
                            // fix — avoids YAML-special-character corruption).
                            let yaml: serde_yaml::Value = serde_json::from_value(v.clone()).ok()?;
                            Some((k.clone(), yaml))
                        })
                        .collect()
                })
                .unwrap_or_default();
            bindings.push(Binding {
                language,
                symbol,
                file,
                span,
                content_sha,
                visibility,
                module_path,
                attributes,
            });
        }
    }

    let mut library_apis: Vec<LibraryApi> = Vec::new();
    if let Some(arr) = obj.get("library_apis").and_then(Value::as_array) {
        for v in arr {
            let Some(api) = v.as_object() else { continue };
            let id = api
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("{component_id}/public-api"));
            let language = api
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("dart")
                .to_string();
            let fingerprint = api
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pub_items: Vec<PubItem> = api
                .get("pub_items")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_object()?;
                            let name = p.get("name").and_then(Value::as_str)?.to_string();
                            let file = p.get("file").and_then(Value::as_str).map(PathBuf::from)?;
                            let kind_str = p.get("kind").and_then(Value::as_str)?;
                            let kind = match kind_str {
                                "struct" => PubItemKind::Struct,
                                "enum" => PubItemKind::Enum,
                                "fn" => PubItemKind::Fn,
                                "trait" => PubItemKind::Trait,
                                "mod" => PubItemKind::Mod,
                                "type-alias" => PubItemKind::TypeAlias,
                                "const" => PubItemKind::Const,
                                "static" => PubItemKind::Static,
                                "union" => PubItemKind::Union,
                                "macro" => PubItemKind::Macro,
                                _ => return None,
                            };
                            Some(PubItem { name, file, kind })
                        })
                        .collect()
                })
                .unwrap_or_default();
            library_apis.push(LibraryApi {
                id,
                kind: ContractKind::LibraryApi,
                language,
                fingerprint,
                pub_items,
            });
        }
    }

    (bindings, library_apis)
}

/// True when the component looks like an Elixir component to the L5
/// branch logic.
fn entry_is_elixir(entry: &ComponentEntry) -> bool {
    entry.languages.contains("elixir") || entry.kind == "elixir-project"
}

/// Drive an Elixir-component's surface extraction through PR-2's
/// subprocess transport (Phase 2 PR-8).
///
/// Mirrors `python_surface_artefacts` but targets the `elixir-analyzer`
/// binary and the `elixir` language domain. Returns `None` when the
/// binary cannot be located.
fn elixir_surface_artefacts(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_elixir_analyzer_binary()?;

    let absolute_dir = resolve_elixir_component_dir(entry, roots)?;

    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    let mix_exs_path = absolute_dir.join("mix.exs");
    if let Some(bytes) = file_content(db, &mix_exs_path) {
        let bytes_vec = (*bytes).clone();
        let content_sha = sha256_hex_bytes(&bytes_vec);
        manifests.push(atlas_analyzers::TargetFile {
            name: "mix.exs".into(),
            relpath: PathBuf::from("mix.exs"),
            bytes: bytes_vec,
            content_sha,
        });
    }
    let mut languages = std::collections::BTreeSet::new();
    languages.insert("elixir".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = elixir_subprocess_spec(binary);
    let proxy = cached_elixir_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis, contracts) =
        decode_elixir_surface_payload(&payload, entry.id.as_str());
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts,
        bindings,
        library_apis,
    }))
}

/// Resolve an Elixir component's first path segment against the
/// workspace roots. Mirrors `resolve_python_component_dir`.
fn resolve_elixir_component_dir(entry: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let segment = entry.path_segments.first()?;
    if segment.path.is_absolute() {
        return Some(segment.path.clone());
    }
    if let Some(owning_root) = best_root_for(roots, &segment.path) {
        return Some(owning_root.join(&segment.path));
    }
    for root in roots {
        let absolute = root.join(&segment.path);
        if absolute.is_dir() {
            return Some(absolute);
        }
    }
    Some(roots.first()?.join(&segment.path))
}

/// Decode the JSON payload returned by the elixir-analyzer subprocess
/// into typed `Binding` / `LibraryApi` / `Contract` values.
///
/// The wire shape is defined in the elixir-analyzer binary
/// (`AnalysePayload`, `WireBinding`, `WireLibraryApi`, `WireContract`).
fn decode_elixir_surface_payload(
    payload: &Value,
    component_id: &str,
) -> (Vec<Binding>, Vec<LibraryApi>, Vec<Contract>) {
    use atlas_index::{ContractKind, PubItem, PubItemKind, Visibility};
    use std::collections::BTreeMap as StdBTreeMap;

    let Some(obj) = payload.as_object() else {
        return (Vec::new(), Vec::new(), Vec::new());
    };

    // Decode bindings (same shape as the Python decoder).
    let mut bindings: Vec<Binding> = Vec::new();
    if let Some(arr) = obj.get("bindings").and_then(Value::as_array) {
        for v in arr {
            let Some(b) = v.as_object() else { continue };
            let language = b
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("elixir")
                .to_string();
            let symbol = b
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let file = b
                .get("file")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let span = b
                .get("span")
                .and_then(Value::as_array)
                .and_then(|a| {
                    let s = a.first().and_then(Value::as_u64)? as usize;
                    let e = a.get(1).and_then(Value::as_u64)? as usize;
                    Some((s, e))
                })
                .unwrap_or((0, 0));
            let content_sha = b
                .get("content_sha")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let visibility = b
                .get("visibility")
                .map(|v| {
                    serde_json::from_value::<Visibility>(v.clone())
                        .unwrap_or(Visibility::Conventional)
                })
                .unwrap_or(Visibility::Conventional);
            let module_path = b
                .get("module_path")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let attributes: StdBTreeMap<String, serde_yaml::Value> = b
                .get("attributes")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| {
                            let yaml: serde_yaml::Value = serde_json::from_value(v.clone()).ok()?;
                            Some((k.clone(), yaml))
                        })
                        .collect()
                })
                .unwrap_or_default();
            bindings.push(Binding {
                language,
                symbol,
                file,
                span,
                content_sha,
                visibility,
                module_path,
                attributes,
            });
        }
    }

    // Decode library APIs.
    let mut library_apis: Vec<LibraryApi> = Vec::new();
    if let Some(arr) = obj.get("library_apis").and_then(Value::as_array) {
        for v in arr {
            let Some(api) = v.as_object() else { continue };
            let id = api
                .get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| format!("{component_id}/public-api"));
            let language = api
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("elixir")
                .to_string();
            let fingerprint = api
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pub_items: Vec<PubItem> = api
                .get("pub_items")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let p = p.as_object()?;
                            let name = p.get("name").and_then(Value::as_str)?.to_string();
                            let file = p.get("file").and_then(Value::as_str).map(PathBuf::from)?;
                            let kind_str = p.get("kind").and_then(Value::as_str)?;
                            let kind = match kind_str {
                                "struct" => PubItemKind::Struct,
                                "enum" => PubItemKind::Enum,
                                "fn" => PubItemKind::Fn,
                                "trait" => PubItemKind::Trait,
                                "mod" => PubItemKind::Mod,
                                "type-alias" => PubItemKind::TypeAlias,
                                "const" => PubItemKind::Const,
                                "static" => PubItemKind::Static,
                                "union" => PubItemKind::Union,
                                "macro" => PubItemKind::Macro,
                                _ => return None,
                            };
                            Some(PubItem { name, file, kind })
                        })
                        .collect()
                })
                .unwrap_or_default();
            library_apis.push(LibraryApi {
                id,
                kind: ContractKind::LibraryApi,
                language,
                fingerprint,
                pub_items,
            });
        }
    }

    // Decode behaviour contracts.
    let mut contracts: Vec<Contract> = Vec::new();
    if let Some(arr) = obj.get("contracts").and_then(Value::as_array) {
        for v in arr {
            let Some(c) = v.as_object() else { continue };
            let id = c
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let kind_str = c.get("kind").and_then(Value::as_str).unwrap_or("");
            let kind = if kind_str == "behaviour" {
                ContractKind::Behaviour
            } else {
                ContractKind::LibraryApi
            };
            let description = c
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let fingerprint = c
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // Decode optional definition_binding — required by Contract struct.
            let definition_binding = c
                .get("definition_binding")
                .and_then(Value::as_object)
                .and_then(|b| {
                    let symbol = b.get("symbol").and_then(Value::as_str)?.to_string();
                    let file = b
                        .get("file")
                        .and_then(Value::as_str)
                        .map(PathBuf::from)
                        .unwrap_or_default();
                    let span = b
                        .get("span")
                        .and_then(Value::as_array)
                        .and_then(|a| {
                            let s = a.first().and_then(Value::as_u64)? as usize;
                            let e = a.get(1).and_then(Value::as_u64)? as usize;
                            Some((s, e))
                        })
                        .unwrap_or((0, 0));
                    let content_sha = b
                        .get("content_sha")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let module_path = b
                        .get("module_path")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(Binding {
                        language: "elixir".into(),
                        symbol,
                        file,
                        span,
                        content_sha,
                        visibility: Visibility::Conventional,
                        module_path,
                        attributes: StdBTreeMap::new(),
                    })
                });

            if let Some(def_binding) = definition_binding {
                contracts.push(Contract {
                    id,
                    kind,
                    fingerprint,
                    definition_binding: def_binding,
                    description,
                });
            }
        }
    }

    // Note: `implemented_contracts` (populated by `@behaviour` in the
    // elixir analyzer) are intentionally not decoded here. `SurfaceArtefacts`
    // does not yet carry an `implemented_contracts` field (Phase 2 scope
    // boundary). A subsequent PR will add that field and the decode logic.

    (bindings, library_apis, contracts)
}

/// Drive a LispKit component's surface extraction through the
/// subprocess transport (Phase 2 PR-10).
///
/// Mirrors [`python_surface_artefacts`]: walks path segments, locates
/// the `lispkit-analyzer` binary, constructs a
/// [`SubprocessAnalyzerProxy`] against it, and invokes the proxy.
/// Returns `None` when the binary cannot be located.
fn lispkit_surface_artefacts(
    _db: &AtlasDatabase,
    entry: &ComponentEntry,
    roots: &[PathBuf],
    record: &SurfaceRecord,
) -> Option<Arc<SurfaceArtefacts>> {
    let binary = locate_lispkit_analyzer_binary()?;

    let absolute_dir = resolve_lispkit_component_dir(entry, roots)?;

    // Build a minimal Target for the proxy. Pre-load any `*.sld`
    // manifests the engine has read (if the engine pre-loaded them
    // via `manifest_patterns`).
    let mut manifests: Vec<atlas_analyzers::TargetFile> = Vec::new();
    // Try to pre-load any .sld file already known by the engine
    // (the manifest scan may have surfaced one).
    for segment in &entry.path_segments {
        let candidate_dir = if segment.path.is_absolute() {
            segment.path.clone()
        } else if let Some(owning_root) = best_root_for(roots, &segment.path) {
            owning_root.join(&segment.path)
        } else if let Some(root) = roots.first() {
            root.join(&segment.path)
        } else {
            segment.path.clone()
        };
        if let Ok(entries) = std::fs::read_dir(&candidate_dir) {
            for e in entries.flatten() {
                let path = e.path();
                if path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s == "sld")
                {
                    if let Ok(bytes) = std::fs::read(&path) {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("lib.sld")
                            .to_string();
                        let content_sha = sha256_hex_bytes(&bytes);
                        let relpath = PathBuf::from(&name);
                        manifests.push(atlas_analyzers::TargetFile {
                            name,
                            relpath,
                            bytes,
                            content_sha,
                        });
                    }
                }
            }
        }
    }
    let mut languages = std::collections::BTreeSet::new();
    languages.insert("scheme".to_string());
    let target = atlas_analyzers::Target {
        dir: absolute_dir,
        languages,
        manifests,
        top_level_files: Vec::new(),
    };

    let spec = lispkit_subprocess_spec(binary);
    let proxy = cached_lispkit_subprocess_proxy(spec).ok()?;
    let ctx = atlas_analyzers::AnalysisContext::deterministic_only();
    let result = proxy.analyse(&ctx, &target);

    let payload = match result {
        AnalyzerResult::Confident(output) => output
            .as_any()
            .downcast_ref::<SubprocessOutput>()?
            .payload
            .clone(),
        _ => {
            return Some(Arc::new(SurfaceArtefacts {
                record: record.clone(),
                ..Default::default()
            }));
        }
    };

    let (bindings, library_apis) = decode_lispkit_surface_payload(&payload, entry.id.as_str());
    Some(Arc::new(SurfaceArtefacts {
        record: record.clone(),
        contracts: Vec::new(),
        bindings,
        library_apis,
    }))
}

/// Resolve a LispKit component's first path segment against the
/// workspace roots, returning the absolute on-disk dir.
fn resolve_lispkit_component_dir(entry: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let segment = entry.path_segments.first()?;
    if segment.path.is_absolute() {
        return Some(segment.path.clone());
    }
    if let Some(owning_root) = best_root_for(roots, &segment.path) {
        return Some(owning_root.join(&segment.path));
    }
    for root in roots {
        let absolute = root.join(&segment.path);
        if absolute.is_dir() {
            return Some(absolute);
        }
    }
    Some(roots.first()?.join(&segment.path))
}

/// Decode the JSON payload returned by the lispkit-analyzer subprocess.
/// Wire shape is identical to the python-analyzer's `AnalysePayload`.
fn decode_lispkit_surface_payload(
    payload: &Value,
    component_id: &str,
) -> (Vec<Binding>, Vec<LibraryApi>) {
    // Re-use the shared decode logic — the wire shapes are identical
    // (both carry `bindings` + `library_apis` with the same field
    // names). Pass `"scheme"` as the default language so that any
    // binding/api that omits the `language` field on the wire is
    // correctly labelled rather than inheriting the Python default.
    decode_subprocess_surface_payload(payload, component_id, "scheme")
}

/// JSON input document for the Stage 1 prompt. The key set is stable
/// across the live code so cache-key equality is a proxy for
/// "inputs unchanged".
///
/// Fields:
///
/// - `COMPONENT_ID` — rendered as `{{COMPONENT_ID}}` in the prompt so
///   the model knows which catalog id it is analysing.
/// - `COMPONENT_PATHS` — a JSON array of the relative path segments
///   the component spans (design §4.1 L5 notes that a component may
///   span multiple segments). Rendered as `{{COMPONENT_PATHS}}`.
/// - `COMPONENT_CONTENT_SHAS` — the matching per-segment content SHAs
///   so a file-level content change invalidates the cache key. Not
///   referenced by the prompt prose; present only for cache-key
///   fidelity.
/// - `CATALOG_COMPONENTS` — marker-formatted list of peer component
///   ids so `{{CATALOG_COMPONENTS}}` substitution in the shipped
///   prompt has something to expand to.
fn build_inputs(component: &ComponentEntry, peer_ids: &[String]) -> Value {
    let component_paths: Vec<String> = component
        .path_segments
        .iter()
        .map(|seg| path_to_forward_slash(&seg.path))
        .collect();
    let content_shas: Vec<String> = component
        .path_segments
        .iter()
        .map(|seg| seg.content_sha.clone())
        .collect();
    let catalog_block = render_catalog_for_prompt(peer_ids);

    json!({
        "COMPONENT_ID": component.id.as_str(),
        "COMPONENT_PATHS": component_paths,
        "COMPONENT_CONTENT_SHAS": content_shas,
        "CATALOG_COMPONENTS": catalog_block,
    })
}

/// Test-only escape hatch: other L-layer tests need to know exactly
/// what [`build_inputs`] produces so they can register a canned
/// TestBackend response against that shape. Kept here rather than in
/// a shared fixtures module so the real function stays private.
#[cfg(test)]
pub(crate) fn build_inputs_for_tests(component: &ComponentEntry, peer_ids: &[String]) -> Value {
    build_inputs(component, peer_ids)
}

/// Parameterless variant for the unified prompt/builder coverage
/// matrix in [`crate::prompt_token_coverage`]. Constructs a minimal
/// stub component so the matrix can call all four builders uniformly.
#[cfg(test)]
pub(crate) fn build_inputs_with_stubs_for_tests() -> Value {
    let component = ComponentEntry {
        id: ComponentId::parse("demo").unwrap(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: Vec::new(),
        languages: std::collections::BTreeSet::new(),
        build_system: None,
        role: None,
        path_segments: vec![atlas_index::PathSegment {
            path: std::path::PathBuf::from("crates/demo"),
            content_sha: "0".repeat(64),
        }],
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: component_ontology::EvidenceGrade::Strong,
        evidence_fields: Vec::new(),
        rationale: String::new(),
        deleted: false,
    };
    build_inputs(&component, &[])
}

/// SHA-256 of `bytes` rendered as 64-character lowercase hex. Used by
/// the L5 fingerprint construction to derive a `prompt_sha` from the
/// canonical-JSON inputs the backend will receive.
fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut out = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut out, "{b:02x}").expect("writing to String never fails");
    }
    out
}

fn render_catalog_for_prompt(peer_ids: &[String]) -> String {
    if peer_ids.is_empty() {
        return "_(none — this component is the only catalog entry)_".to_string();
    }
    peer_ids
        .iter()
        .map(|n| format!("- {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn path_to_forward_slash(path: &std::path::Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Parse the LLM response into a [`SurfaceRecord`]. Accepts two
/// shapes:
///
/// 1. A JSON object matching `SurfaceRecord` directly.
/// 2. A JSON object with a `surface` key whose value matches
///    `SurfaceRecord` — mirrors Ravel-Lite's `SurfaceFile`-wrapped
///    shape for compatibility with a backend that forwards the
///    entire file.
fn parse_surface_response(value: &Value) -> Result<SurfaceRecord, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected JSON object, got {}", value_kind(value)))?;

    let body = if let Some(inner) = object.get("surface") {
        inner
    } else {
        value
    };
    serde_json::from_value::<SurfaceRecord>(body.clone()).map_err(|e| format!("{e}"))
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Look up a surface-level pin for `id`. Pins live in
/// `overrides.pins[<id>]["surface"]` with a `PinValue::Value` whose
/// string is a YAML-encoded [`SurfaceRecord`]. Returns `None` unless
/// the pin is present AND parses cleanly — a malformed pin is
/// reported via `notes` by the caller's fallback, not silently
/// ignored.
fn surface_pin(overrides: &OverridesFile, id: &ComponentId) -> Option<SurfaceRecord> {
    let pins = overrides.pins.get(id)?;
    let entry = pins.get("surface")?;
    let PinValue::Value { value, .. } = entry else {
        return None;
    };
    serde_yaml::from_str::<SurfaceRecord>(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AtlasDatabase;
    use crate::ingest::seed_filesystem;
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn stage1_surface_prompt_exposes_required_substitution_tokens() {
        for token in [
            "{{COMPONENT_ID}}",
            "{{COMPONENT_PATHS}}",
            "{{CATALOG_COMPONENTS}}",
        ] {
            assert!(
                EMBEDDED_STAGE1_SURFACE_PROMPT.contains(token),
                "stage1-surface.md must expose `{token}` — the engine's \
                 `build_inputs` populates it and the prompt is expected \
                 to reference it"
            );
        }
    }

    #[test]
    fn stage1_surface_prompt_has_no_ravel_lite_protocol_remnants() {
        // The Ravel-Lite-era prompt told the model to write YAML to a
        // path passed via `{{SURFACE_OUTPUT_PATH}}` and referred to
        // the upstream tool by name. Atlas's `ClaudeCodeBackend`
        // expects JSON on stdout; those instructions are now wrong
        // and must stay gone.
        for forbidden in ["SURFACE_OUTPUT_PATH", "ravel-lite", "Ravel-Lite"] {
            assert!(
                !EMBEDDED_STAGE1_SURFACE_PROMPT.contains(forbidden),
                "stage1-surface.md still contains `{forbidden}` — this is \
                 a leftover Ravel-Lite protocol reference that conflicts \
                 with Atlas's JSON-on-stdout contract"
            );
        }
    }

    #[test]
    fn stage1_surface_prompt_has_no_residual_project_word() {
        // The prompt was migrated from Ravel-Lite's project-oriented
        // vocabulary; any stray `project` word means the migration
        // missed a spot.
        for stem in ["project", "Project", "PROJECT"] {
            assert!(
                !EMBEDDED_STAGE1_SURFACE_PROMPT.contains(stem),
                "stage1-surface.md contains stray `{stem}` token"
            );
        }
    }

    // Bidirectional token coverage between stage1-surface.md and
    // build_inputs is enforced at compile time by
    // `prompt_token_coverage.rs`.

    // ---------------------------------------------------------------
    // Fixture helpers for surface_of integration tests. The backend
    // is owned by an `Arc<TestBackend>` kept by the test; the
    // database receives a cheaply-cloned `Arc<dyn LlmBackend>` that
    // points at the same heap object, so `backend.respond(...)`
    // calls from the test land in the map the database reads.
    // ---------------------------------------------------------------

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn canned_surface() -> Value {
        json!({
            "purpose": "Does the alpha thing.",
            "consumes_files": ["~/.config/alpha/*.yaml"],
            "produces_files": ["/tmp/out/*.json"],
            "network_endpoints": ["grpc://alpha:50051"],
            "data_formats": ["AlphaRecord"],
            "external_tools_spawned": ["git"],
            "explicit_cross_component_mentions": ["Beta"],
            "interaction_role_hints": ["generator"],
            "notes": "",
        })
    }

    fn write_cargo_lib_fixture(root: &std::path::Path, crate_name: &str) {
        let crate_dir = root.join(crate_name);
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{crate_name}\"\n"),
        )
        .unwrap();
        std::fs::write(crate_dir.join("src").join("lib.rs"), "// lib\n").unwrap();
        std::fs::write(crate_dir.join("README.md"), format!("# {crate_name}\n")).unwrap();
    }

    /// Build an AtlasDatabase from a filesystem fixture rooted in
    /// `tmp`, returning `(db, backend)` so tests can register canned
    /// responses on the very same backend instance the engine calls.
    fn db_with_shared_backend(tmp: &TempDir) -> (AtlasDatabase, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn atlas_llm::LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, vec![tmp.path().to_path_buf()], fingerprint());
        seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();
        (db, backend)
    }

    /// The engine includes the component's path_segments'
    /// content_shas in its cache-key inputs, which the test cannot
    /// know until after seeding. Returns the exact inputs
    /// [`surface_of`] will build.
    fn inputs_for_id(db: &AtlasDatabase, id: &ComponentId) -> Value {
        let components = all_components(db);
        let entry = components
            .iter()
            .find(|c| &c.id == id && !c.deleted)
            .expect("id must resolve to a live component");
        let peer_ids: Vec<String> = components
            .iter()
            .filter(|c| !c.deleted && &c.id != id)
            .map(|c| c.id.as_str().to_string())
            .collect();
        build_inputs(entry, &peer_ids)
    }

    #[test]
    fn surface_of_parses_canned_stage1_response_for_single_component_fixture() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "alpha");
        let (db, backend) = db_with_shared_backend(&tmp);

        let components = all_components(&db);
        let id = components
            .iter()
            .find(|c| !c.deleted)
            .expect("fixture must produce a component")
            .id
            .clone();
        let inputs = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs, canned_surface());

        let record = surface_of(&db, id);
        assert_eq!(record.purpose, "Does the alpha thing.");
        assert_eq!(
            record.explicit_cross_component_mentions,
            vec!["Beta".to_string()]
        );
        assert_eq!(record.interaction_role_hints.len(), 1);
    }

    #[test]
    fn surface_of_hits_cache_on_second_call_with_unchanged_inputs() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "beta");
        let (db, backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let inputs = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs, canned_surface());

        // Reset counter — previous tests in the same module may have
        // been counted if cache state were shared. It is not: every
        // AtlasDatabase owns its own LlmResponseCache.
        assert_eq!(db.llm_cache().call_count(), 0);
        let _first = surface_of(&db, id.clone());
        assert_eq!(db.llm_cache().call_count(), 1);
        let _second = surface_of(&db, id.clone());
        assert_eq!(
            db.llm_cache().call_count(),
            1,
            "second identical call must hit the cache — this is the \
             'zero LLM calls on no-op re-run' contract in miniature"
        );
    }

    #[test]
    fn surface_of_misses_cache_when_file_content_changes() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "gamma");
        let (mut db, backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let inputs_before = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs_before, canned_surface());

        let _ = surface_of(&db, id.clone());
        let calls_after_first = db.llm_cache().call_count();
        assert_eq!(calls_after_first, 1);

        // Mutate a file — its content_sha propagates up to
        // path_segments[0].content_sha, invalidating the cache key.
        let lib_path = tmp.path().join("gamma").join("src").join("lib.rs");
        std::fs::write(&lib_path, "// modified\n").unwrap();
        seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();

        // The new content_sha produces a new input shape. Register
        // the new inputs with the same canned response so the
        // response parses cleanly; the assertion is on call count,
        // not response shape.
        let inputs_after = inputs_for_id(&db, &id);
        backend.respond(PromptId::Stage1Surface, inputs_after, canned_surface());

        let _ = surface_of(&db, id);
        assert_eq!(
            db.llm_cache().call_count(),
            2,
            "content-sha change must force a cache miss"
        );
    }

    #[test]
    fn surface_pin_short_circuits_before_backend() {
        use atlas_index::{OverridesFile, PinValue};
        use std::collections::BTreeMap;

        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "delta");
        let (mut db, _backend) = db_with_shared_backend(&tmp);

        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();

        // Install a surface pin that encodes a minimal SurfaceRecord as
        // YAML in the pin value. No canned response is registered —
        // if the pin is not honoured the TestBackend would error.
        let mut field_pins = BTreeMap::new();
        let pinned_yaml = "purpose: pinned\nnotes: hand-authored\n";
        field_pins.insert(
            "surface".to_string(),
            PinValue::Value {
                value: pinned_yaml.to_string(),
                reason: None,
            },
        );
        let mut pins = BTreeMap::new();
        pins.insert(id.clone(), field_pins);
        db.set_components_overrides(OverridesFile {
            pins,
            ..OverridesFile::default()
        });

        let record = surface_of(&db, id);
        assert_eq!(record.purpose, "pinned");
        assert_eq!(record.notes, "hand-authored");
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "pinned surface must not touch the backend"
        );
    }

    #[test]
    fn surface_of_returns_default_for_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib_fixture(tmp.path(), "epsilon");
        let (db, _backend) = db_with_shared_backend(&tmp);

        let record = surface_of(&db, ComponentId::parse("does-not-exist").unwrap());
        assert_eq!(record.as_ref(), &SurfaceRecord::default());
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "unknown id must not make an LLM call"
        );
    }

    #[test]
    fn parse_surface_response_accepts_bare_record_shape() {
        let v = json!({ "purpose": "p" });
        let got = parse_surface_response(&v).unwrap();
        assert_eq!(got.purpose, "p");
    }

    #[test]
    fn parse_surface_response_accepts_surface_wrapped_shape() {
        let v = json!({ "surface": { "purpose": "w" } });
        let got = parse_surface_response(&v).unwrap();
        assert_eq!(got.purpose, "w");
    }

    #[test]
    fn parse_surface_response_rejects_non_object() {
        let err = parse_surface_response(&json!("string-value")).unwrap_err();
        assert!(err.contains("object"), "{err}");
    }

    /// PR-3 code-quality F-CQ-3 regression pin. The earlier
    /// JSON-string→YAML conversion went via `serde_yaml::from_str(&v.to_string())`,
    /// which round-tripped attribute *string values* through the YAML
    /// parser. For values containing YAML-special characters (`:` is
    /// the canonical case), the round-trip silently re-parsed the
    /// string as a YAML mapping or other compound, corrupting the
    /// payload. The fix uses `serde_json::from_value` directly, which
    /// walks the JSON tree and emits the matching YAML primitive.
    /// This test pins the pathological `"key: value"` case so a
    /// future refactor can't regress it.
    #[test]
    fn decode_subprocess_surface_payload_preserves_yaml_special_chars_in_string_values() {
        // Wave 3 C# attribute values may carry `:` / `#` / etc.; the
        // fix-up's regression pin uses `:` (the most pernicious) plus
        // a `#` for good measure.
        let payload = json!({
            "bindings": [
                {
                    "language": "python",
                    "symbol": "demo",
                    "file": "demo.py",
                    "span": [0, 1],
                    "content_sha": "0".repeat(64),
                    "visibility": { "kind": "conventional" },
                    "module_path": [],
                    "attributes": {
                        // Two pathological values: a string containing
                        // `:` (would otherwise be reparsed as a YAML
                        // mapping `{ "key": " value" }`), and a
                        // `# `-prefixed string (would otherwise be
                        // treated as a YAML comment and resolve to
                        // null).
                        "literal_with_colon": "key: value",
                        "literal_with_hash": "# not a comment",
                        // Also pin the simple-bool / array shapes the
                        // earlier idiom did get right, so we don't
                        // accidentally regress them.
                        "private": true,
                        "decorator_chain": ["dataclass"],
                    },
                }
            ],
            "library_apis": [],
        });
        let (bindings, _apis) = decode_subprocess_surface_payload(&payload, "comp", "python");
        assert_eq!(bindings.len(), 1);
        let attrs = &bindings[0].attributes;
        assert_eq!(
            attrs.get("literal_with_colon"),
            Some(&serde_yaml::Value::String("key: value".into())),
            "`:`-bearing string must round-trip as a YAML scalar, not be \
             reparsed as a mapping"
        );
        assert_eq!(
            attrs.get("literal_with_hash"),
            Some(&serde_yaml::Value::String("# not a comment".into())),
            "`#`-prefixed string must round-trip as a YAML scalar, not be \
             treated as a YAML comment"
        );
        assert_eq!(
            attrs.get("private"),
            Some(&serde_yaml::Value::Bool(true)),
            "boolean values must remain booleans after the JSON→YAML conversion"
        );
        assert_eq!(
            attrs.get("decorator_chain"),
            Some(&serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("dataclass".into())
            ])),
            "array values must remain sequences after the JSON→YAML conversion"
        );
    }
}
