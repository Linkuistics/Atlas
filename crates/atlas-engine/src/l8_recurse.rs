//! L8 sub-carve decision — per-component "should we recurse, and if so,
//! into which sub-directories?".
//!
//! The policy table in [`crate::subcarve_policy`] resolves most cases
//! deterministically. For genuinely ambiguous ones — a Rust library
//! with no modularity hint, say — the LLM is consulted via
//! [`atlas_llm::PromptId::Subcarve`]. Either way, the result is a
//! [`SubcarveDecision`] consumed by [`crate::fixedpoint`].
//!
//! Workspaces are an exception: they always recurse (their members are
//! discovered by L2's manifest walk), so no sub-dirs are proposed and
//! no LLM is called. Non-components, externals, and the other
//! leaf-like kinds return an empty decision by policy.
//!
//! Like [`crate::l5_surface`] and [`crate::l6_edges`], L8 is a plain
//! function over [`AtlasDatabase`]: Salsa 0.26 cannot downcast
//! `&dyn salsa::Database` to access the LLM backend, so tracked
//! recursion would bypass the response cache. The fixedpoint back-edge
//! closes through `workspace.carve_back_edge` instead.

use std::path::PathBuf;
use std::sync::Arc;

use atlas_index::{ComponentEntry, PinValue};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use serde_json::{json, Value};

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;
use crate::l7_structural::{cliques, modularity_hint, seam_density, Clique};
use crate::subcarve_policy::{decide, PolicyDecision, SubcarveSignals};
use crate::types::ComponentKind;

/// Min-k for clique search feeding [`SubcarveSignals::cliques_touching`].
/// 3 matches the design §4.1 wording ("triangles of mutual reference are
/// a strong coupling signal"); a K2 clique is just an edge and would
/// flood the signal with noise.
const CLIQUES_TOUCHING_MIN_K: u32 = 3;

/// The shipped Atlas sub-carve prompt, embedded at compile time.
pub const EMBEDDED_SUBCARVE_PROMPT: &str = include_str!("../../../defaults/prompts/subcarve.md");

/// The full outcome of an L8 decision: whether to recurse, the
/// directories to open up as new L2 candidate roots, and the rationale
/// (for logs / overrides consumers).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubcarveDecision {
    pub should_subcarve: bool,
    pub sub_dirs: Vec<PathBuf>,
    pub rationale: String,
}

impl SubcarveDecision {
    fn stopped(reason: &str) -> Self {
        SubcarveDecision {
            should_subcarve: false,
            sub_dirs: Vec::new(),
            rationale: reason.to_string(),
        }
    }
}

/// Public boolean view of [`compute_decision`]. Matches the signature
/// shape from the task spec; callers that also want the sub-dirs should
/// use [`subcarve_plan`] directly (both go through the same internal
/// computation, so back-to-back calls don't double-count LLM traffic —
/// [`AtlasDatabase::call_llm_cached`] memoises per-request).
pub fn should_subcarve(db: &AtlasDatabase, id: String) -> bool {
    compute_decision(db, &id).should_subcarve
}

/// Directories to open up as new L2 candidate roots inside the
/// component whose id is `id`. Empty when the policy (or the LLM)
/// decides not to recurse.
pub fn subcarve_plan(db: &AtlasDatabase, id: String) -> Arc<Vec<PathBuf>> {
    Arc::new(compute_decision(db, &id).sub_dirs)
}

/// Full L8 result. Exposed separately so the fixedpoint driver can read
/// the rationale without repeating the decision walk.
pub fn subcarve_decision(db: &AtlasDatabase, id: String) -> SubcarveDecision {
    compute_decision(db, &id)
}

// ---------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------

fn compute_decision(db: &AtlasDatabase, id: &str) -> SubcarveDecision {
    let components = all_components(db);
    let Some(entry) = components.iter().find(|c| c.id == id && !c.deleted) else {
        return SubcarveDecision::stopped("unknown component id");
    };

    let kind = ComponentKind::parse(&entry.kind).unwrap_or(ComponentKind::NonComponent);
    let current_depth = compute_depth(&components, id);
    let max_depth = db.max_depth();
    let seam_density_value = seam_density(db, id.to_string());
    let modularity_hint_value = modularity_hint(db, id.to_string());
    let cliques_touching = cliques_touching_id(db, id);
    let pin_suppressed_children = pin_suppressed_children_of(db, id);

    let signals = SubcarveSignals {
        kind,
        current_depth,
        max_depth,
        seam_density: seam_density_value,
        modularity_hint: modularity_hint_value,
        cliques_touching,
        pin_suppressed_children,
    };

    match decide(&signals) {
        PolicyDecision::Stop => SubcarveDecision::stopped(&format!(
            "policy: stop (kind={}, depth={}/{})",
            entry.kind, current_depth, max_depth
        )),
        PolicyDecision::Recurse if matches!(kind, ComponentKind::Workspace) => {
            // Workspace members surface through L2's manifest walk.
            // Reporting `should_subcarve: true` without sub_dirs tells
            // the fixedpoint driver "yes, this component may grow"
            // without adding a bogus carve root.
            SubcarveDecision {
                should_subcarve: true,
                sub_dirs: Vec::new(),
                rationale: "workspace: members discovered via manifests".to_string(),
            }
        }
        PolicyDecision::Recurse | PolicyDecision::AskLlm => {
            ask_llm_for_subcarve(db, entry, &signals)
        }
    }
}

/// Walk parent links to compute depth. Root is depth 0; each parent step
/// adds 1. The L4 acyclicity invariant guarantees this terminates.
fn compute_depth(components: &[ComponentEntry], id: &str) -> u32 {
    let mut depth = 0u32;
    let mut cursor = id.to_string();
    loop {
        let Some(entry) = components.iter().find(|c| c.id == cursor) else {
            break;
        };
        match &entry.parent {
            Some(parent) => {
                depth = depth.saturating_add(1);
                cursor = parent.clone();
            }
            None => break,
        }
    }
    depth
}

fn cliques_touching_id(db: &AtlasDatabase, id: &str) -> Vec<Clique> {
    cliques(db, CLIQUES_TOUCHING_MIN_K)
        .iter()
        .filter(|c| c.members.iter().any(|m| m == id))
        .cloned()
        .collect()
}

fn pin_suppressed_children_of(db: &AtlasDatabase, id: &str) -> Vec<String> {
    let overrides = db
        .workspace()
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    overrides
        .pins
        .get(id)
        .and_then(|pins| pins.get("suppress_children"))
        .and_then(|pin| match pin {
            PinValue::SuppressChildren { suppress_children } => Some(suppress_children.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn ask_llm_for_subcarve(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    signals: &SubcarveSignals,
) -> SubcarveDecision {
    let request = LlmRequest {
        prompt_template: PromptId::Subcarve,
        inputs: build_subcarve_inputs(entry, signals),
        schema: ResponseSchema::accept_any(),
    };

    match db.call_llm_cached(&request) {
        Ok(value) => parse_subcarve_response(&value).unwrap_or_else(|reason| {
            SubcarveDecision::stopped(&format!("LLM response parse failed: {reason}"))
        }),
        Err(err) => SubcarveDecision::stopped(&format!("LLM call failed: {err}")),
    }
}

/// Test-only escape hatch: the fixedpoint tests need to register a
/// canned TestBackend response against the exact inputs `compute_decision`
/// will build. Kept private elsewhere so the production code continues
/// to own the shape.
#[cfg(test)]
pub(crate) fn build_subcarve_inputs_for_tests(
    entry: &ComponentEntry,
    signals: &SubcarveSignals,
) -> Value {
    build_subcarve_inputs(entry, signals)
}

/// Parameterless variant for the unified prompt/builder coverage matrix
/// in [`crate::prompt_token_coverage`]. Constructs minimal stub entry +
/// signals so the matrix can call all four builders uniformly.
#[cfg(test)]
pub(crate) fn build_subcarve_inputs_with_stubs_for_tests() -> Value {
    let entry = ComponentEntry {
        id: "demo".into(),
        parent: None,
        kind: "rust-library".into(),
        lifecycle_roles: Vec::new(),
        language: None,
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
    let signals = SubcarveSignals {
        kind: ComponentKind::RustLibrary,
        current_depth: 0,
        max_depth: 8,
        seam_density: 0.0,
        modularity_hint: None,
        cliques_touching: Vec::new(),
        pin_suppressed_children: Vec::new(),
    };
    build_subcarve_inputs(&entry, &signals)
}

/// Build the input JSON fed to `PromptId::Subcarve`. Shape matches the
/// prompt's `{{COMPONENT_ID}}`, `{{COMPONENT_KIND}}`,
/// `{{STRUCTURAL_SIGNALS}}`, `{{EDGE_NEIGHBOURHOOD}}` tokens.
fn build_subcarve_inputs(entry: &ComponentEntry, signals: &SubcarveSignals) -> Value {
    let paths: Vec<String> = entry
        .path_segments
        .iter()
        .map(|seg| path_to_forward_slash(&seg.path))
        .collect();

    let modularity_hint_json = match &signals.modularity_hint {
        Some(hint) => json!({
            "partition_a": hint.partition_a,
            "partition_b": hint.partition_b,
            "cross_edges": hint.cross_edges,
            "total_internal_edges": hint.total_internal_edges,
        }),
        None => Value::Null,
    };

    let cliques_json: Vec<Value> = signals
        .cliques_touching
        .iter()
        .map(|c| json!({ "members": c.members }))
        .collect();

    let structural = json!({
        "seam_density": format_seam_density(signals.seam_density),
        "modularity_hint": modularity_hint_json,
        "cliques_touching": cliques_json,
        "current_depth": signals.current_depth,
        "max_depth": signals.max_depth,
    });

    json!({
        "COMPONENT_ID": entry.id,
        "COMPONENT_KIND": entry.kind,
        "COMPONENT_PATHS": paths,
        "STRUCTURAL_SIGNALS": structural,
        "EDGE_NEIGHBOURHOOD": Value::Array(Vec::new()),
        "PIN_SUPPRESSED_CHILDREN": signals.pin_suppressed_children.clone(),
    })
}

/// Infinity round-trips poorly through JSON — serde_json emits `null`
/// which the LLM then sees as "no information". Clamp to a large finite
/// stand-in so the prompt renders a concrete number.
fn format_seam_density(value: f32) -> Value {
    if value.is_infinite() {
        json!(1.0e6_f64)
    } else if value.is_nan() {
        json!(0.0_f64)
    } else {
        json!(value as f64)
    }
}

fn path_to_forward_slash(path: &std::path::Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Parse the LLM response into a [`SubcarveDecision`]. Expected shape:
/// `{ "should_subcarve": bool, "sub_dirs": ["path", ...], "rationale": "..." }`.
fn parse_subcarve_response(value: &Value) -> Result<SubcarveDecision, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected JSON object, got {}", value_kind(value)))?;

    let should_subcarve = object
        .get("should_subcarve")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "response missing bool `should_subcarve`".to_string())?;

    let sub_dirs: Vec<PathBuf> = object
        .get("sub_dirs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default();

    let rationale = object
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("LLM did not supply a rationale")
        .to_string();

    // Schema-consistency check: should_subcarve=false but sub_dirs
    // non-empty is contradictory; prefer the boolean (no recursion).
    let sub_dirs = if should_subcarve {
        sub_dirs
    } else {
        Vec::new()
    };

    Ok(SubcarveDecision {
        should_subcarve,
        sub_dirs,
        rationale,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AtlasDatabase;
    use crate::ingest::seed_filesystem;
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [7u8; 32],
            ontology_sha: [8u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn build_lib_crate(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{name}\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("lib.rs"), "// lib\n").unwrap();
    }

    fn build_cli_crate(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main(){}\n").unwrap();
    }

    fn db_with_single_crate(
        builder: impl FnOnce(&std::path::Path),
    ) -> (AtlasDatabase, Arc<TestBackend>, TempDir) {
        let tmp = TempDir::new().unwrap();
        builder(tmp.path());
        let backend = Arc::new(TestBackend::with_fingerprint(fingerprint()));
        let backend_dyn: Arc<dyn atlas_llm::LlmBackend> = backend.clone();
        let mut db = AtlasDatabase::new(backend_dyn, tmp.path().to_path_buf(), fingerprint());
        seed_filesystem(&mut db, tmp.path(), false).unwrap();
        (db, backend, tmp)
    }

    // ---------------------------------------------------------------
    // Policy-short-circuit tests — no LLM call must fire.
    // ---------------------------------------------------------------

    #[test]
    fn rust_cli_never_subcarves_regardless_of_signals() {
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_cli_crate(root, "solo-cli"));
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        assert!(!should_subcarve(&db, id.clone()));
        assert!(subcarve_plan(&db, id).is_empty());
        assert_eq!(
            db.llm_cache().call_count(),
            0,
            "RustCli must short-circuit before the backend"
        );
    }

    #[test]
    fn unknown_id_returns_stopped_decision_without_backend_call() {
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        assert!(!should_subcarve(&db, "does-not-exist".into()));
        assert_eq!(db.llm_cache().call_count(), 0);
    }

    #[test]
    fn max_depth_zero_forces_stop_even_on_library_kind() {
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        db.set_max_depth(0);
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        assert!(!should_subcarve(&db, id));
        assert_eq!(db.llm_cache().call_count(), 0);
    }

    // ---------------------------------------------------------------
    // LLM-escalation tests — route via canned TestBackend responses.
    // ---------------------------------------------------------------

    #[test]
    fn rust_library_with_no_hint_asks_llm_and_returns_sub_dirs() {
        let (db, backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let entry_clone = all_components(&db)
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .clone();
        let signals = SubcarveSignals {
            kind: ComponentKind::RustLibrary,
            current_depth: 0,
            max_depth: 8,
            seam_density: 0.0,
            modularity_hint: None,
            cliques_touching: Vec::new(),
            pin_suppressed_children: Vec::new(),
        };
        let inputs = build_subcarve_inputs(&entry_clone, &signals);
        backend.respond(
            PromptId::Subcarve,
            inputs,
            json!({
                "should_subcarve": true,
                "sub_dirs": ["src/auth", "src/billing"],
                "rationale": "two independent sub-systems",
            }),
        );

        let plan = subcarve_plan(&db, id);
        assert_eq!(
            plan.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["src/auth".to_string(), "src/billing".to_string()]
        );
        assert_eq!(db.llm_cache().call_count(), 1);
    }

    #[test]
    fn llm_response_with_should_subcarve_false_yields_empty_plan() {
        let (db, backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let entry_clone = all_components(&db)
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .clone();
        let signals = SubcarveSignals {
            kind: ComponentKind::RustLibrary,
            current_depth: 0,
            max_depth: 8,
            seam_density: 0.0,
            modularity_hint: None,
            cliques_touching: Vec::new(),
            pin_suppressed_children: Vec::new(),
        };
        let inputs = build_subcarve_inputs(&entry_clone, &signals);
        backend.respond(
            PromptId::Subcarve,
            inputs,
            json!({
                "should_subcarve": false,
                "sub_dirs": ["src/ignore_me"],
                "rationale": "library is already the right granularity",
            }),
        );

        let decision = subcarve_decision(&db, id);
        assert!(!decision.should_subcarve);
        assert!(
            decision.sub_dirs.is_empty(),
            "should_subcarve=false forces sub_dirs empty regardless of LLM output"
        );
    }

    #[test]
    fn llm_call_failure_maps_to_stopped_decision_with_rationale() {
        // No canned response registered → TestBackend will error,
        // which maps to a non-panicking Stopped decision.
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let decision = subcarve_decision(&db, id);
        assert!(!decision.should_subcarve);
        assert!(decision.rationale.contains("LLM"), "{}", decision.rationale);
    }

    #[test]
    fn parse_subcarve_response_accepts_minimal_shape() {
        let v = json!({ "should_subcarve": true, "sub_dirs": ["a", "b"] });
        let got = parse_subcarve_response(&v).unwrap();
        assert!(got.should_subcarve);
        assert_eq!(got.sub_dirs.len(), 2);
        assert_eq!(got.rationale, "LLM did not supply a rationale");
    }

    #[test]
    fn parse_subcarve_response_rejects_missing_bool() {
        let v = json!({ "sub_dirs": ["a"] });
        assert!(parse_subcarve_response(&v).is_err());
    }

    #[test]
    fn format_seam_density_maps_infinity_to_large_finite() {
        let n = format_seam_density(f32::INFINITY);
        assert_eq!(n, json!(1.0e6_f64));
    }

    // ---------------------------------------------------------------
    // Depth walk
    // ---------------------------------------------------------------

    fn entry(id: &str, parent: Option<&str>) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: parent.map(String::from),
            kind: "rust-library".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: Vec::new(),
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: String::new(),
            deleted: false,
        }
    }

    #[test]
    fn compute_depth_returns_zero_for_root_and_increments_per_parent() {
        let comps = vec![
            entry("root", None),
            entry("child", Some("root")),
            entry("grandchild", Some("child")),
        ];
        assert_eq!(compute_depth(&comps, "root"), 0);
        assert_eq!(compute_depth(&comps, "child"), 1);
        assert_eq!(compute_depth(&comps, "grandchild"), 2);
    }

    #[test]
    fn compute_depth_unknown_id_returns_zero() {
        let comps = vec![entry("root", None)];
        assert_eq!(compute_depth(&comps, "nope"), 0);
    }

    #[test]
    fn subcarve_prompt_token_coverage_is_bidirectional() {
        // Every `{{TOKEN}}` in subcarve.md must be supplied by
        // build_subcarve_inputs (forward) AND every key
        // build_subcarve_inputs supplies must be referenced by a
        // `{{TOKEN}}` in subcarve.md (inverse). The inverse direction
        // catches the silent-data-drop failure: a builder field with no
        // matching template token is dropped by `prompt::render`,
        // leaving the LLM with no component context for the subcarve
        // decision.
        let entry = ComponentEntry {
            id: "demo".into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: Vec::new(),
            language: None,
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
        let signals = SubcarveSignals {
            kind: ComponentKind::RustLibrary,
            current_depth: 0,
            max_depth: 8,
            seam_density: 0.0,
            modularity_hint: None,
            cliques_touching: Vec::new(),
            pin_suppressed_children: Vec::new(),
        };
        let inputs = build_subcarve_inputs(&entry, &signals);
        let object = inputs.as_object().expect("inputs must be a JSON object");
        let supplied: std::collections::HashSet<String> = object.keys().cloned().collect();
        let referenced: std::collections::HashSet<String> =
            collect_template_tokens(EMBEDDED_SUBCARVE_PROMPT)
                .into_iter()
                .collect();

        for token in &referenced {
            assert!(
                supplied.contains(token),
                "subcarve.md references `{{{{{token}}}}}` but \
                 build_subcarve_inputs does not populate key `{token}`"
            );
        }
        for key in &supplied {
            assert!(
                referenced.contains(key),
                "build_subcarve_inputs supplies key `{key}` but \
                 subcarve.md does not reference `{{{{{key}}}}}` — \
                 the value will be silently dropped by prompt::render, \
                 leaving the LLM without that input"
            );
        }

        let mut tokens = std::collections::BTreeMap::new();
        for (key, value) in object {
            let rendered = match value {
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            tokens.insert(key.clone(), rendered);
        }
        let rendered = atlas_llm::prompt::render(EMBEDDED_SUBCARVE_PROMPT, &tokens)
            .expect("subcarve.md must render with build_subcarve_inputs output");

        assert!(
            rendered.contains("demo"),
            "rendered subcarve prompt must contain component id; \
             got prompt without it (length={})",
            rendered.len()
        );
    }

    /// Extract every `{{TOKEN}}` name referenced in `template`, using
    /// the same grammar as `atlas_llm::prompt::render`: `{{TOKEN}}`
    /// substitutes, `{{{{` and `}}}}` are literal-brace escapes.
    fn collect_template_tokens(template: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut rest = template;
        while !rest.is_empty() {
            if let Some(body) = rest.strip_prefix("{{{{") {
                rest = body;
                continue;
            }
            if let Some(body) = rest.strip_prefix("}}}}") {
                rest = body;
                continue;
            }
            if let Some(body) = rest.strip_prefix("{{") {
                let end = body.find("}}").expect("template must close `{{`");
                tokens.push(body[..end].trim().to_string());
                rest = &body[end + 2..];
                continue;
            }
            let ch = rest.chars().next().unwrap();
            rest = &rest[ch.len_utf8()..];
        }
        tokens
    }
}
