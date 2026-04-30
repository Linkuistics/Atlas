//! L3 classification — decide whether a candidate directory is a
//! component and, if so, what kind. Resolution order (§4.1):
//!
//! 1. **Pin short-circuit.** An entry in `overrides.pins` keyed by the
//!    candidate's would-be id wins over everything; no LLM is
//!    consulted.
//! 2. **Deterministic rule.** The tabulated rules in
//!    [`crate::heuristics`] inspect the rationale bundle and any
//!    pre-loaded manifest contents; if one matches, it becomes the
//!    classification.
//! 3. **LLM fallback.** Ambiguous candidates are handed to
//!    [`atlas_llm::PromptId::Classify`] with the rationale bundle and
//!    manifest snippets as inputs; the response is parsed into
//!    [`Classification`].
//!
//! The query is keyed by `(workspace, candidate_dir)` rather than by a
//! full `Candidate`. Salsa memoises the underlying L1 signal queries,
//! so rebuilding the bundle inside L3 is free and keeps the query-key
//! type to primitives. This avoids forcing `salsa::Update` onto
//! nested signal types.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_index::{OverridesFile, PinValue};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use component_ontology::{EvidenceGrade, LifecycleScope};
use serde_json::{json, Value};

use crate::db::{AtlasDatabase, Workspace};
use crate::heuristics::{classify_deterministic, ManifestContents};
use crate::l1_queries::{doc_headings, file_content, git_boundaries, manifests_in, shebangs};
use crate::types::{Classification, ComponentKind, RationaleBundle};

/// Maximum bytes of each manifest passed to the LLM. Generous enough
/// for any real Cargo.toml / package.json, small enough to keep
/// prompts cheap.
const MANIFEST_SNIPPET_LIMIT: usize = 16 * 1024;

/// Classify the candidate whose directory is `candidate_dir`. Returns
/// `Arc<Classification>` so callers can cache the result cheaply.
pub fn is_component(
    db: &AtlasDatabase,
    workspace: Workspace,
    candidate_dir: PathBuf,
) -> Arc<Classification> {
    // Pin short-circuit runs before any expensive read of manifest
    // bytes so a user's hand-authored decision always wins without
    // LLM expense.
    let dyn_db: &dyn salsa::Database = db;
    let overrides = workspace.components_overrides(dyn_db).clone();
    let root = workspace.root(dyn_db).clone();
    if let Some(classification) = pinned_classification(&overrides, &candidate_dir, &root) {
        return Arc::new(classification);
    }

    // Build the rationale bundle by scoping L1 queries to the
    // candidate directory.
    let bundle = build_bundle(db, workspace, &candidate_dir);

    // Pre-load manifest contents for the deterministic rules + LLM
    // inputs.  Content reads go through `file_content` (untracked
    // helper), which pulls through the tracked `File::bytes` edge.
    let snippets = load_manifest_snippets(db, &bundle.manifests);
    let manifest_contents = ManifestContents {
        cargo_toml: snippet_text(&snippets, "Cargo.toml"),
        package_json: snippet_text(&snippets, "package.json"),
        pyproject_toml: snippet_text(&snippets, "pyproject.toml"),
    };

    let candidate = crate::types::Candidate {
        dir: candidate_dir.clone(),
        rationale_bundle: bundle.clone(),
    };
    if let Some(classification) = classify_deterministic(&candidate, &manifest_contents) {
        return Arc::new(classification);
    }

    // LLM fallback. Errors propagate as a weak "unknown" classification
    // — the engine intentionally does not panic on an LLM hiccup, so
    // higher-level tooling can surface the rationale.
    Arc::new(classify_via_llm(
        db,
        &root,
        &candidate_dir,
        &bundle,
        &snippets,
    ))
}

fn build_bundle(db: &AtlasDatabase, workspace: Workspace, candidate_dir: &Path) -> RationaleBundle {
    let dyn_db: &dyn salsa::Database = db;
    let manifests_here: Vec<PathBuf> = manifests_in(dyn_db, workspace, candidate_dir.to_path_buf())
        .iter()
        .filter(|m| m.parent() == Some(candidate_dir))
        .cloned()
        .collect();

    let git_dirs_here = git_boundaries(dyn_db, workspace, candidate_dir.to_path_buf());
    let is_git_root = git_dirs_here.iter().any(|d| d == candidate_dir);

    let doc_headings_here = doc_headings(dyn_db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();
    let shebangs_here = shebangs(dyn_db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();

    RationaleBundle {
        manifests: manifests_here,
        is_git_root,
        doc_headings: doc_headings_here,
        shebangs: shebangs_here,
    }
}

fn load_manifest_snippets(db: &AtlasDatabase, paths: &[PathBuf]) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    for path in paths {
        let bytes = match file_content(db, path) {
            Some(bytes) => bytes,
            None => continue,
        };
        let limit = bytes.len().min(MANIFEST_SNIPPET_LIMIT);
        let Ok(text) = std::str::from_utf8(&bytes[..limit]) else {
            continue;
        };
        out.insert(path.clone(), text.to_string());
    }
    out
}

fn snippet_text<'a>(snippets: &'a BTreeMap<PathBuf, String>, basename: &str) -> Option<&'a str> {
    snippets
        .iter()
        .find(|(path, _)| path.file_name().and_then(|n| n.to_str()) == Some(basename))
        .map(|(_, text)| text.as_str())
}

/// Look up a pin for the candidate under every key form L3 recognises.
/// Returns the pin's `Classification` or `None`.
///
/// Keys tried, in order:
/// 1. The candidate's dir-path relative to the workspace root (e.g.
///    `crates/foo`).
/// 2. The candidate dir's leaf basename (e.g. `foo`).
/// 3. The `id` of any `overrides.additions` entry whose first
///    `path_segment` points at this dir — so an authored addition +
///    pin pair both key off the addition's explicit id.
fn pinned_classification(
    overrides: &OverridesFile,
    candidate_dir: &Path,
    workspace_root: &Path,
) -> Option<Classification> {
    let rel = candidate_dir
        .strip_prefix(workspace_root)
        .unwrap_or(candidate_dir);
    let rel_str = path_to_forward_slash(rel);
    let basename = candidate_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut keys_tried: Vec<String> = Vec::new();
    if !rel_str.is_empty() {
        keys_tried.push(rel_str);
    }
    if !basename.is_empty() {
        keys_tried.push(basename);
    }
    for addition in &overrides.additions {
        if let Some(first_seg) = addition.path_segments.first() {
            let abs_path = if first_seg.path.is_absolute() {
                first_seg.path.clone()
            } else {
                workspace_root.join(&first_seg.path)
            };
            if abs_path == candidate_dir {
                keys_tried.push(addition.id.clone());
            }
        }
    }

    for key in &keys_tried {
        if let Some(pins) = overrides.pins.get(key) {
            return Some(pins_to_classification(pins));
        }
    }
    None
}

fn pins_to_classification(pins: &BTreeMap<String, PinValue>) -> Classification {
    let kind_str = pin_string(pins.get("kind"));
    let kind = kind_str
        .as_deref()
        .and_then(ComponentKind::parse)
        .unwrap_or(ComponentKind::NonComponent);
    let language = pin_string(pins.get("language"));
    let build_system = pin_string(pins.get("build_system"));
    let role = pin_string(pins.get("role"));

    Classification {
        kind,
        language,
        build_system,
        lifecycle_roles: Vec::new(),
        role,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: pins.keys().map(|k| format!("pin:{k}")).collect(),
        rationale: "human pin".to_string(),
        is_boundary: !matches!(pins.get("suppress"), Some(PinValue::Suppress { .. })),
    }
}

fn pin_string(pin: Option<&PinValue>) -> Option<String> {
    match pin {
        Some(PinValue::Value { value, .. }) => Some(value.clone()),
        _ => None,
    }
}

fn path_to_forward_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn classify_via_llm(
    db: &AtlasDatabase,
    workspace_root: &Path,
    candidate_dir: &Path,
    bundle: &RationaleBundle,
    snippets: &BTreeMap<PathBuf, String>,
) -> Classification {
    let backend = db.backend();
    let request = LlmRequest {
        prompt_template: PromptId::Classify,
        inputs: build_llm_inputs(workspace_root, candidate_dir, bundle, snippets),
        schema: ResponseSchema::accept_any(),
    };

    match backend.call(&request) {
        Ok(value) => parse_llm_response(value).unwrap_or_else(|reason| {
            unknown_classification(format!("LLM response parse failed: {reason}"))
        }),
        Err(err) => unknown_classification(format!("LLM call failed: {err}")),
    }
}

fn build_llm_inputs(
    workspace_root: &Path,
    candidate_dir: &Path,
    bundle: &RationaleBundle,
    snippets: &BTreeMap<PathBuf, String>,
) -> Value {
    let rel = candidate_dir
        .strip_prefix(workspace_root)
        .unwrap_or(candidate_dir);
    let rel_str = path_to_forward_slash(rel);

    let manifests_rel: Vec<String> = bundle
        .manifests
        .iter()
        .map(|m| {
            let rel = m.strip_prefix(workspace_root).unwrap_or(m);
            path_to_forward_slash(rel)
        })
        .collect();

    let doc_headings_json: Vec<Value> = bundle
        .doc_headings
        .iter()
        .map(|h| {
            let rel = h.path.strip_prefix(workspace_root).unwrap_or(&h.path);
            json!({
                "path": path_to_forward_slash(rel),
                "level": h.level,
                "text": h.text,
            })
        })
        .collect();

    let shebangs_json: Vec<Value> = bundle
        .shebangs
        .iter()
        .map(|s| {
            let rel = s.path.strip_prefix(workspace_root).unwrap_or(&s.path);
            json!({
                "path": path_to_forward_slash(rel),
                "interpreter": s.interpreter,
            })
        })
        .collect();

    let manifest_contents_json: BTreeMap<String, String> = snippets
        .iter()
        .map(|(path, text)| {
            let rel = path.strip_prefix(workspace_root).unwrap_or(path);
            (path_to_forward_slash(rel), text.clone())
        })
        .collect();

    json!({
        "dir_relative": rel_str,
        "rationale_bundle": {
            "manifests": manifests_rel,
            "is_git_root": bundle.is_git_root,
            "doc_headings": doc_headings_json,
            "shebangs": shebangs_json,
        },
        "manifest_contents": manifest_contents_json,
    })
}

fn parse_llm_response(value: Value) -> Result<Classification, String> {
    // Accept a Classification shape plus a handful of minor
    // deviations the LLM may introduce (missing optional fields,
    // integer-typed levels, etc.).
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected JSON object, got {}", value))?;

    let kind_str = object
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "response missing string `kind`".to_string())?;
    let kind =
        ComponentKind::parse(kind_str).ok_or_else(|| format!("unknown kind `{kind_str}`"))?;

    let language = object
        .get("language")
        .and_then(|v| v.as_str())
        .map(String::from);
    let build_system = object
        .get("build_system")
        .and_then(|v| v.as_str())
        .map(String::from);
    let role = object
        .get("role")
        .and_then(|v| v.as_str())
        .map(String::from);
    let rationale = object
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_boundary = object
        .get("is_boundary")
        .and_then(|v| v.as_bool())
        .unwrap_or(!matches!(kind, ComponentKind::NonComponent));
    let evidence_grade = object
        .get("evidence_grade")
        .and_then(|v| v.as_str())
        .and_then(EvidenceGrade::parse)
        .unwrap_or(EvidenceGrade::Medium);
    let evidence_fields = object
        .get("evidence_fields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let lifecycle_roles = object
        .get("lifecycle_roles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(LifecycleScope::parse))
                .collect()
        })
        .unwrap_or_default();

    Ok(Classification {
        kind,
        language,
        build_system,
        lifecycle_roles,
        role,
        evidence_grade,
        evidence_fields,
        rationale,
        is_boundary,
    })
}

fn unknown_classification(reason: String) -> Classification {
    Classification {
        kind: ComponentKind::NonComponent,
        language: None,
        build_system: None,
        lifecycle_roles: Vec::new(),
        role: None,
        evidence_grade: EvidenceGrade::Weak,
        evidence_fields: Vec::new(),
        rationale: reason,
        is_boundary: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use atlas_index::{AlwaysTrue, ComponentEntry, PathSegment, PinValue};
    use component_ontology::EvidenceGrade;

    fn overrides_with_pin(id: &str, field: &str, value: &str) -> OverridesFile {
        let mut pins = BTreeMap::new();
        let mut field_pins = BTreeMap::new();
        field_pins.insert(
            field.to_string(),
            PinValue::Value {
                value: value.to_string(),
                reason: None,
            },
        );
        pins.insert(id.to_string(), field_pins);
        OverridesFile {
            pins,
            ..OverridesFile::default()
        }
    }

    #[test]
    fn pin_matches_relative_path_key() {
        let overrides = overrides_with_pin("crates/foo", "kind", "spec");
        let got = pinned_classification(&overrides, Path::new("/ws/crates/foo"), Path::new("/ws"))
            .expect("pin should match relative path key");
        assert_eq!(got.kind, ComponentKind::Spec);
        assert_eq!(got.rationale, "human pin");
    }

    #[test]
    fn pin_matches_basename_key() {
        // User-friendly fallback: pin by bare basename when the
        // relative-path form isn't used.
        let overrides = overrides_with_pin("foo", "kind", "spec");
        let got = pinned_classification(&overrides, Path::new("/ws/crates/foo"), Path::new("/ws"))
            .expect("pin should match basename key");
        assert_eq!(got.kind, ComponentKind::Spec);
    }

    #[test]
    fn pin_matches_addition_id() {
        // Authored addition + pin pair: the addition declares the id,
        // the pin key references that id.
        let mut overrides = overrides_with_pin("my-spec", "kind", "spec");
        overrides.additions.push(ComponentEntry {
            id: "my-spec".into(),
            parent: None,
            kind: "spec".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("specs/my-spec"),
                content_sha: "0".into(),
            }],
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "spec".into(),
            deleted: false,
        });
        let got =
            pinned_classification(&overrides, Path::new("/ws/specs/my-spec"), Path::new("/ws"))
                .expect("pin should match via addition id");
        assert_eq!(got.kind, ComponentKind::Spec);
    }

    #[test]
    fn suppress_pin_sets_is_boundary_false() {
        let mut field_pins = BTreeMap::new();
        field_pins.insert(
            "suppress".to_string(),
            PinValue::Suppress {
                suppress: AlwaysTrue,
            },
        );
        let mut pins = BTreeMap::new();
        pins.insert("foo".to_string(), field_pins);
        let overrides = OverridesFile {
            pins,
            ..OverridesFile::default()
        };
        let got = pinned_classification(&overrides, Path::new("/ws/foo"), Path::new("/ws"))
            .expect("suppress pin should produce a classification");
        assert!(!got.is_boundary);
    }

    #[test]
    fn parse_llm_response_accepts_minimal_object() {
        let value = serde_json::json!({
            "kind": "rust-library",
            "evidence_grade": "medium",
            "rationale": "some reason",
            "is_boundary": true,
        });
        let got = parse_llm_response(value).unwrap();
        assert_eq!(got.kind, ComponentKind::RustLibrary);
        assert!(got.is_boundary);
    }

    #[test]
    fn parse_llm_response_rejects_unknown_kind() {
        let value = serde_json::json!({
            "kind": "nonsense",
            "rationale": "x",
        });
        assert!(parse_llm_response(value).is_err());
    }

    #[test]
    fn parse_llm_response_defaults_is_boundary_by_kind() {
        // NonComponent defaults to is_boundary=false even when the
        // field is absent; every other kind defaults to true.
        let lib_value = serde_json::json!({
            "kind": "rust-library",
            "rationale": "r",
        });
        assert!(parse_llm_response(lib_value).unwrap().is_boundary);
        let nc_value = serde_json::json!({
            "kind": "non-component",
            "rationale": "r",
        });
        assert!(!parse_llm_response(nc_value).unwrap().is_boundary);
    }
}
