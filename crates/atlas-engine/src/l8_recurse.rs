//! L8 sub-carve decision — per-component "should we recurse, and if so,
//! into which sub-directories?".
//!
//! The policy table in [`crate::subcarve_policy`] resolves most cases
//! deterministically. For genuinely ambiguous ones — a Rust library
//! with no modularity hint, say — the engine maps over the component's
//! immediate sub-directories, calling [`crate::l3_classify::is_component`]
//! on each (the "map" step), then deterministically reduces the
//! verdicts into a [`SubcarveDecision`] consumed by [`crate::fixedpoint`].
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
//!
//! ## Map/reduce shape
//!
//! Each L8 decision now produces N small per-subdir Classify calls
//! instead of one large Subcarve call. The advantages:
//!
//! - **Cache reuse.** Two crates with similar layouts share Classify
//!   cache entries on identical sub-dir signal shapes; today's single
//!   Subcarve prompt was so large that no two prompts ever cache-shared.
//! - **HTTP routing.** The map step routes through the same `Classify`
//!   prompt id as L3 itself, so an HTTP backend (e.g. `anthropic/...`)
//!   handles both seamlessly via [`atlas_llm::BackendRouter`].
//! - **No prompt drift.** A single classification prompt, not two.
//!
//! `defaults/prompts/subcarve.md` is retained in the prompt corpus so
//! its `template_sha` keeps contributing to the run-wide fingerprint
//! during the transition. Slated for deletion after one shipped
//! release cycle.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_index::{ComponentEntry, PathSegment, PinValue};

use crate::db::{AtlasDatabase, Workspace};
use crate::l4_tree::all_components;
use crate::l7_structural::{cliques, modularity_hint, seam_density, Clique};
use crate::subcarve_policy::{decide, PolicyDecision, SubcarveSignals};
use crate::types::ComponentKind;

/// Min-k for clique search feeding [`SubcarveSignals::cliques_touching`].
/// 3 matches the design §4.1 wording ("triangles of mutual reference are
/// a strong coupling signal"); a K2 clique is just an edge and would
/// flood the signal with noise.
const CLIQUES_TOUCHING_MIN_K: u32 = 3;

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
            map_reduce_subcarve(db, entry, &signals)
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

/// Map step: enumerate the component's immediate sub-directories,
/// filter those the user has pin-suppressed, classify each via
/// [`crate::l3_classify::is_component`], and aggregate the boundary
/// verdicts into a [`SubcarveDecision`].
///
/// Verdicts win over the engine's structural hints: when
/// `signals.modularity_hint` disagrees with the verdicts, the verdicts
/// stand and the disagreement is logged in the rationale. The hint is
/// a prior, not a gate.
///
/// Returns a stopped decision when there are no eligible sub-dirs at
/// all — this avoids spinning the LLM on leaf-like layouts where the
/// component's source tree is a single directory of files.
fn map_reduce_subcarve(
    db: &AtlasDatabase,
    entry: &ComponentEntry,
    signals: &SubcarveSignals,
) -> SubcarveDecision {
    let workspace = db.workspace();
    let workspace_root = workspace.root(db as &dyn salsa::Database).clone();

    let suppressed = pin_suppressed_keys(&signals.pin_suppressed_children);
    let candidates: Vec<PathBuf> =
        enumerate_immediate_subdirs(db, workspace, &workspace_root, &entry.path_segments)
            .into_iter()
            .filter(|abs_dir| !is_pin_suppressed(abs_dir, &suppressed))
            .collect();

    if candidates.is_empty() {
        return SubcarveDecision::stopped("no eligible immediate sub-directories");
    }

    let mut sub_dirs: Vec<PathBuf> = Vec::new();
    let mut accepted_summaries: Vec<String> = Vec::new();
    let mut rejected = 0usize;
    for abs_dir in &candidates {
        let classification = crate::l3_classify::is_component(db, workspace, abs_dir.clone());
        if classification.is_boundary {
            let rel = abs_dir.strip_prefix(&workspace_root).unwrap_or(abs_dir);
            sub_dirs.push(rel.to_path_buf());
            accepted_summaries.push(format!(
                "{} ({})",
                path_to_forward_slash(rel),
                classification.kind.as_str()
            ));
        } else {
            rejected += 1;
        }
    }

    let should_subcarve = !sub_dirs.is_empty();
    let rationale = build_rationale(
        should_subcarve,
        sub_dirs.len(),
        rejected,
        candidates.len(),
        &accepted_summaries,
        signals.modularity_hint.is_some(),
    );

    SubcarveDecision {
        should_subcarve,
        sub_dirs,
        rationale,
    }
}

/// Build the set of pin-suppressed name keys, expanded to cover both
/// the raw form and the slugified form so `crates/Atlas-Engine` and
/// `atlas-engine` both match a single user pin.
fn pin_suppressed_keys(suppressed: &[String]) -> BTreeSet<String> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for name in suppressed {
        keys.insert(name.clone());
        if let Some(slug) = crate::identifiers::slugify_segment(name) {
            keys.insert(slug);
        }
    }
    keys
}

fn is_pin_suppressed(abs_dir: &Path, suppressed: &BTreeSet<String>) -> bool {
    let basename = match abs_dir.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };
    if suppressed.contains(basename) {
        return true;
    }
    if let Some(slug) = crate::identifiers::slugify_segment(basename) {
        if suppressed.contains(&slug) {
            return true;
        }
    }
    false
}

/// Enumerate the immediate sub-directories of every path segment
/// owned by the component. A directory counts as "immediate" when it
/// is the first path component strictly inside one of the component's
/// segments AND has at least one registered file underneath (so
/// gitignored or otherwise unindexed dirs disappear automatically).
///
/// Directories that ARE another owned segment are filtered out: a
/// component's own carved-out children must not be re-proposed.
fn enumerate_immediate_subdirs(
    db: &AtlasDatabase,
    workspace: Workspace,
    workspace_root: &Path,
    path_segments: &[PathSegment],
) -> Vec<PathBuf> {
    let dyn_db: &dyn salsa::Database = db;
    let owned_dirs: BTreeSet<PathBuf> = path_segments
        .iter()
        .map(|seg| absolutise(workspace_root, &seg.path))
        .collect();

    let mut immediate: BTreeSet<PathBuf> = BTreeSet::new();
    let mut consider = |descendant: &Path| {
        for owned_dir in &owned_dirs {
            let rel = match descendant.strip_prefix(owned_dir) {
                Ok(r) if !r.as_os_str().is_empty() => r,
                _ => continue,
            };
            if let Some(first) = rel.components().next() {
                let abs = owned_dir.join(first.as_os_str());
                if &abs != owned_dir {
                    immediate.insert(abs);
                }
            }
        }
    };

    for file in workspace.files(dyn_db).iter() {
        if let Some(parent) = file.path(dyn_db).parent() {
            consider(parent);
        }
    }
    for git_dir in workspace.git_boundary_dirs(dyn_db).iter() {
        consider(git_dir);
    }

    immediate.retain(|d| !owned_dirs.contains(d));
    immediate.into_iter().collect()
}

fn absolutise(workspace_root: &Path, segment_path: &Path) -> PathBuf {
    if segment_path.is_absolute() {
        segment_path.to_path_buf()
    } else {
        workspace_root.join(segment_path)
    }
}

fn build_rationale(
    should_subcarve: bool,
    accepted: usize,
    rejected: usize,
    total: usize,
    accepted_summaries: &[String],
    modularity_hint_present: bool,
) -> String {
    let mut parts = vec![format!(
        "L3 verdicts across {total} immediate sub-dir(s): {accepted} component(s), {rejected} non-component(s)"
    )];
    if !accepted_summaries.is_empty() {
        parts.push(format!("kept: {}", accepted_summaries.join(", ")));
    }
    if modularity_hint_present && !should_subcarve {
        parts.push("note: modularity_hint suggested a partition but no sub-dir verdict was a boundary".to_string());
    }
    parts.join("; ")
}

fn path_to_forward_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
    // Policy short-circuits — never enters the map step.
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
    }

    #[test]
    fn unknown_id_returns_stopped_decision() {
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        assert!(!should_subcarve(&db, "does-not-exist".into()));
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
    }

    // ---------------------------------------------------------------
    // Map/reduce flow
    // ---------------------------------------------------------------

    #[test]
    fn library_with_no_immediate_subdir_yields_no_subcarve() {
        // `lib/` has exactly one immediate sub-dir (`src`); the L3
        // map step on `src` errors against the empty TestBackend and
        // defaults to non-boundary, so the reduce yields no carve.
        let (db, _backend, _tmp) = db_with_single_crate(|root| build_lib_crate(root, "lib"));
        let id = all_components(&db)
            .iter()
            .find(|c| !c.deleted)
            .unwrap()
            .id
            .clone();
        let decision = subcarve_decision(&db, id);
        assert!(!decision.should_subcarve);
        assert!(decision.sub_dirs.is_empty());
        assert!(
            decision.rationale.contains("L3 verdicts"),
            "rationale should record map-step verdicts; got: {}",
            decision.rationale
        );
    }

    // ---------------------------------------------------------------
    // Helpers
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

    // ---------------------------------------------------------------
    // Pin-suppression helper unit tests
    // ---------------------------------------------------------------

    #[test]
    fn pin_suppressed_keys_includes_raw_and_slug_forms() {
        let keys = pin_suppressed_keys(&["Atlas-Engine".to_string()]);
        assert!(keys.contains("Atlas-Engine"));
        assert!(
            keys.contains("atlas-engine"),
            "slugified form must also be present so a pin written in \
             slug form matches a raw mixed-case directory and vice versa"
        );
    }

    #[test]
    fn is_pin_suppressed_matches_basename_and_slug() {
        let mut suppressed = BTreeSet::new();
        suppressed.insert("scripts".to_string());
        assert!(is_pin_suppressed(Path::new("/ws/lib/scripts"), &suppressed));
        assert!(!is_pin_suppressed(
            Path::new("/ws/lib/src"),
            &suppressed
        ));
    }

    #[test]
    fn build_rationale_summarises_zero_carve() {
        let r = build_rationale(false, 0, 3, 3, &[], false);
        assert!(r.contains("0 component"), "{r}");
        assert!(r.contains("3 non-component"), "{r}");
    }

    #[test]
    fn build_rationale_lists_accepted_subdirs_when_carving() {
        let r = build_rationale(
            true,
            2,
            1,
            3,
            &["src/auth (rust-library)".to_string(), "src/billing (rust-library)".to_string()],
            false,
        );
        assert!(r.contains("kept:"));
        assert!(r.contains("src/auth"));
        assert!(r.contains("src/billing"));
    }
}
