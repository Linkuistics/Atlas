//! L4 tree assembly and rename-match integration.
//!
//! The engine's primary deliverable is the component tree —
//! [`all_components`] walks the full candidate set produced by L2,
//! keeps the ones L3 marks as `is_boundary`, derives parent/child links
//! from the directory hierarchy, and emits a [`Vec<ComponentEntry>`]
//! ready to be serialised as `components.yaml`.
//!
//! Two layers sit on top of the raw classification:
//!
//! - **Overrides**: `overrides.additions` entries bypass L2/L3 and land
//!   in the tree directly; `overrides.pins` may carry
//!   `suppress_children: [id, ...]` lists that prune specific children
//!   from the parent's descendants (sibling-level suppression, not the
//!   node-level `suppress: true` which L3 handles by setting
//!   `is_boundary: false`).
//!
//! - **Rename-match**: on every assembly, the freshly-derived entries
//!   are matched against the prior `components.yaml` via content-SHA
//!   overlap ([`atlas_index::rename_match`]). Matches inherit the prior
//!   id so identifier stability survives directory relocations.
//!   Orphans (prior entries with no live match) are emitted once as
//!   `deleted: true` tombstones; the next clean run filters tombstones
//!   out of the prior list before matching, so they disappear
//!   naturally without needing a side-table sentinel.
//!
//! Acyclicity is enforced here (§4.2: "L4 enforces acyclicity") via
//! DFS on the derived parent/child relation. Later layers assume the
//! tree is a DAG; a violation is a hard engine error, not a panic
//! deep in L5+.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_index::{
    ComponentEntry, ComponentsFile, DocAnchor, OverridesFile, PathSegment, PinValue,
};

use crate::db::{AtlasDatabase, Workspace};
use crate::identifiers::allocate_id;
use crate::l1_queries::{doc_headings, file_tree_sha};
use crate::l2_candidates::candidate_components_at;
use crate::l3_classify::is_component;
use crate::types::Classification;

/// Fatal tree-assembly error. A hard error rather than a soft warning
/// because downstream layers (L5/L6 graphs, L9 projections) assume
/// the tree is a DAG — silently shipping a cycle would produce
/// nonsensical outputs deeper in the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum TreeAssemblyError {
    #[error("component id graph contains a cycle including `{id}`")]
    Cycle { id: String },
}

/// Build the full component tree. The returned vector is sorted by id
/// for deterministic YAML output. Panics on a cycle, matching the
/// design-doc position that acyclicity is a hard invariant (§4.2).
pub fn all_components(db: &AtlasDatabase) -> Arc<Vec<ComponentEntry>> {
    match try_assemble(db) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Fallible form of [`all_components`] for tests that want to assert
/// the acyclicity error is reachable without asking the harness to
/// catch a panic.
pub fn try_assemble(db: &AtlasDatabase) -> Result<Arc<Vec<ComponentEntry>>, TreeAssemblyError> {
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let overrides = workspace.components_overrides(db as &dyn salsa::Database).clone();
    let prior = workspace.prior_components(db as &dyn salsa::Database).clone();

    let live = gather_live_components(db, workspace, &root, &overrides);
    let finalised = resolve_ids_and_tombstones(&prior, &overrides, live);
    enforce_acyclicity(&finalised)?;

    let mut out: Vec<ComponentEntry> = finalised;
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Arc::new(out))
}

/// Parent component id of `id` per the assembled tree, or `None` when
/// `id` is at the root or does not exist.
pub fn component_parent(db: &AtlasDatabase, id: &str) -> Option<String> {
    all_components(db)
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.parent.clone())
}

/// Immediate children of `id` — any component whose `parent` field
/// equals `id`. Returned sorted by id for determinism.
pub fn component_children(db: &AtlasDatabase, id: &str) -> Arc<Vec<String>> {
    let mut out: Vec<String> = all_components(db)
        .iter()
        .filter(|c| c.parent.as_deref() == Some(id))
        .map(|c| c.id.clone())
        .collect();
    out.sort();
    Arc::new(out)
}

/// Path segments of the component with id `id`, or an empty vector if
/// the id does not exist.
pub fn component_path_segments(db: &AtlasDatabase, id: &str) -> Arc<Vec<PathSegment>> {
    let segments = all_components(db)
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.path_segments.clone())
        .unwrap_or_default();
    Arc::new(segments)
}

// ---------------------------------------------------------------------
// Phase 1: gather live components from L2/L3 plus overrides.additions.
// ---------------------------------------------------------------------

/// A component as it exists before rename-match — path and
/// classification are known, but the final id and parent id are not yet
/// assigned. `explicit_parent_id` is honoured verbatim when set (used
/// by override-additions that author a cross-tree parent link);
/// otherwise the parent is derived from the directory hierarchy.
struct LiveComponent {
    dir: PathBuf,
    classification: Classification,
    path_segments: Vec<PathSegment>,
    manifests: Vec<PathBuf>,
    doc_anchors: Vec<DocAnchor>,
    provisional_parent_dir: Option<PathBuf>,
    explicit_parent_id: Option<String>,
    /// Explicit id for override-additions entries; `None` for
    /// signal-derived components (which pick an id during allocation).
    explicit_id: Option<String>,
}

fn gather_live_components(
    db: &AtlasDatabase,
    workspace: Workspace,
    root: &Path,
    overrides: &OverridesFile,
) -> Vec<LiveComponent> {
    let candidates = candidate_components_at(db as &dyn salsa::Database, workspace, root.to_path_buf());

    // Confirmed candidates, keyed by dir for quick parent lookup.
    let mut confirmed_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    let mut by_dir: BTreeMap<PathBuf, (Classification, Vec<PathBuf>, Vec<DocAnchor>)> =
        BTreeMap::new();

    for candidate in candidates.iter() {
        let classification = is_component(db, workspace, candidate.dir.clone());
        if !classification.is_boundary {
            continue;
        }

        let manifests = candidate
            .rationale_bundle
            .manifests
            .iter()
            .map(|p| relative_to_root(p, root))
            .collect();
        let doc_anchors: Vec<DocAnchor> = doc_headings(
            db as &dyn salsa::Database,
            workspace,
            candidate.dir.clone(),
        )
        .iter()
        .filter(|h| h.level == 1)
        .map(|h| DocAnchor {
            path: relative_to_root(&h.path, root),
            heading: h.text.clone(),
        })
        .collect();

        confirmed_dirs.insert(candidate.dir.clone());
        by_dir.insert(
            candidate.dir.clone(),
            ((*classification).clone(), manifests, doc_anchors),
        );
    }

    // Compute parent dir for each confirmed candidate. Process from
    // shallowest path first so descendants see their ancestors.
    let sorted_dirs: Vec<PathBuf> = {
        let mut v: Vec<PathBuf> = confirmed_dirs.iter().cloned().collect();
        v.sort_by_key(|d| d.components().count());
        v
    };

    let mut live: Vec<LiveComponent> = Vec::with_capacity(sorted_dirs.len());
    for dir in &sorted_dirs {
        let (classification, manifests, doc_anchors) = by_dir
            .remove(dir)
            .expect("confirmed_dirs is populated from by_dir");
        let parent_dir = nearest_confirmed_ancestor(dir, &confirmed_dirs);
        let tree_sha = file_tree_sha(
            db as &dyn salsa::Database,
            workspace,
            dir.clone(),
        );
        let path_segments = vec![PathSegment {
            path: relative_to_root(dir, root),
            content_sha: hex_encode(&tree_sha),
        }];
        live.push(LiveComponent {
            dir: dir.clone(),
            classification,
            path_segments,
            manifests,
            doc_anchors,
            provisional_parent_dir: parent_dir,
            explicit_parent_id: None,
            explicit_id: None,
        });
    }

    // Overrides.additions: append as explicit-id entries. A pin with
    // `suppress: true` at the addition's id removes it.
    for addition in &overrides.additions {
        if is_suppressed_by_pin(overrides, &addition.id) {
            continue;
        }
        let classification = addition_to_classification(addition);
        let parent_dir = addition
            .path_segments
            .first()
            .map(|seg| absolute_under_root(root, &seg.path))
            .and_then(|abs| nearest_confirmed_ancestor(&abs, &confirmed_dirs));
        let dir = addition
            .path_segments
            .first()
            .map(|seg| absolute_under_root(root, &seg.path))
            .unwrap_or_else(|| root.to_path_buf());
        live.push(LiveComponent {
            dir,
            classification,
            path_segments: addition.path_segments.clone(),
            manifests: addition.manifests.clone(),
            doc_anchors: addition.doc_anchors.clone(),
            provisional_parent_dir: parent_dir,
            explicit_parent_id: addition.parent.clone(),
            explicit_id: Some(addition.id.clone()),
        });
    }

    live
}

fn nearest_confirmed_ancestor(dir: &Path, confirmed: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    let mut cursor = dir.parent();
    while let Some(p) = cursor {
        if confirmed.contains(p) {
            return Some(p.to_path_buf());
        }
        cursor = p.parent();
    }
    None
}

// ---------------------------------------------------------------------
// Phase 2: rename-match and final id allocation.
// ---------------------------------------------------------------------

fn resolve_ids_and_tombstones(
    prior: &ComponentsFile,
    overrides: &OverridesFile,
    live: Vec<LiveComponent>,
) -> Vec<ComponentEntry> {
    // Filter prior to live (non-deleted) entries — tombstones must not
    // feed back into rename-match, or they'd re-emit indefinitely.
    let prior_live: Vec<ComponentEntry> = prior
        .components
        .iter()
        .filter(|c| !c.deleted)
        .cloned()
        .collect();

    // Build a ComponentEntry-shaped view of each LiveComponent for the
    // matcher. The matcher only needs `path_segments`, but feeding it
    // the full shape keeps the call site readable.
    let live_entries: Vec<ComponentEntry> = live
        .iter()
        .enumerate()
        .map(|(i, lc)| ComponentEntry {
            id: format!("provisional-{i}"),
            parent: None,
            kind: lc.classification.kind.as_str().into(),
            lifecycle_roles: lc.classification.lifecycle_roles.clone(),
            language: lc.classification.language.clone(),
            build_system: lc.classification.build_system.clone(),
            role: lc.classification.role.clone(),
            path_segments: lc.path_segments.clone(),
            manifests: lc.manifests.clone(),
            doc_anchors: lc.doc_anchors.clone(),
            evidence_grade: lc.classification.evidence_grade,
            evidence_fields: lc.classification.evidence_fields.clone(),
            rationale: lc.classification.rationale.clone(),
            deleted: false,
        })
        .collect();

    let match_out = atlas_index::rename_match(atlas_index::RenameMatchInput::new(
        &prior_live,
        &live_entries,
    ));

    // Build a map: live index → matched prior id (if any).
    let mut live_to_prior_id: HashMap<usize, String> = HashMap::new();
    for (prior_idx, live_idx) in &match_out.matches {
        live_to_prior_id.insert(*live_idx, prior_live[*prior_idx].id.clone());
    }

    // Allocate ids. Iterate in the same order as `live` so a
    // LiveComponent at index i aligns with its provisional entry.
    // Process shallow-first so parents get ids before children.
    let mut order: Vec<usize> = (0..live.len()).collect();
    order.sort_by_key(|i| live[*i].dir.components().count());

    let mut allocated_ids: Vec<String> = vec![String::new(); live.len()];
    let mut existing_ids: HashSet<String> = HashSet::new();

    for &i in &order {
        let lc = &live[i];
        let id = if let Some(explicit) = &lc.explicit_id {
            explicit.clone()
        } else if let Some(prior_id) = live_to_prior_id.get(&i) {
            prior_id.clone()
        } else {
            let parent_id = lc
                .provisional_parent_dir
                .as_ref()
                .and_then(|p| dir_to_live_index(p, &live))
                .map(|idx| allocated_ids[idx].clone());
            allocate_id(&lc.dir, parent_id.as_deref(), &existing_ids)
        };
        existing_ids.insert(id.clone());
        allocated_ids[i] = id;
    }

    // Now build the final ComponentEntry list.
    let mut out: Vec<ComponentEntry> = Vec::new();
    let mut id_by_dir: HashMap<PathBuf, String> = HashMap::new();
    for i in 0..live.len() {
        id_by_dir.insert(live[i].dir.clone(), allocated_ids[i].clone());
    }
    for i in 0..live.len() {
        let lc = &live[i];
        let id = allocated_ids[i].clone();
        let parent = lc.explicit_parent_id.clone().or_else(|| {
            lc.provisional_parent_dir
                .as_ref()
                .and_then(|p| id_by_dir.get(p).cloned())
        });
        out.push(ComponentEntry {
            id,
            parent,
            kind: lc.classification.kind.as_str().into(),
            lifecycle_roles: lc.classification.lifecycle_roles.clone(),
            language: lc.classification.language.clone(),
            build_system: lc.classification.build_system.clone(),
            role: lc.classification.role.clone(),
            path_segments: lc.path_segments.clone(),
            manifests: lc.manifests.clone(),
            doc_anchors: lc.doc_anchors.clone(),
            evidence_grade: lc.classification.evidence_grade,
            evidence_fields: lc.classification.evidence_fields.clone(),
            rationale: lc.classification.rationale.clone(),
            deleted: false,
        });
    }

    // Apply suppress_children pins — remove any live component whose id
    // appears in an ancestor's suppress_children list. Walk after id
    // allocation so the ids are final.
    let suppressed = collect_suppressed_children(&out, overrides);
    if !suppressed.is_empty() {
        out.retain(|c| !suppressed.contains(&c.id));
    }

    // Orphan tombstones.
    for prior_idx in &match_out.orphans {
        let mut tomb = prior_live[*prior_idx].clone();
        tomb.deleted = true;
        out.push(tomb);
    }

    out
}

fn dir_to_live_index(dir: &Path, live: &[LiveComponent]) -> Option<usize> {
    live.iter().position(|lc| lc.dir == dir)
}

fn collect_suppressed_children(
    components: &[ComponentEntry],
    overrides: &OverridesFile,
) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    for (key, pins) in &overrides.pins {
        if let Some(PinValue::SuppressChildren { suppress_children }) = pins.get("suppress_children") {
            // The pin key is a component id; we simply collect its
            // suppress_children list. The list values are child ids.
            if components.iter().any(|c| &c.id == key) {
                for child in suppress_children {
                    out.insert(child.clone());
                }
            }
        }
    }
    out
}

fn is_suppressed_by_pin(overrides: &OverridesFile, id: &str) -> bool {
    overrides
        .pins
        .get(id)
        .and_then(|pins| pins.get("suppress"))
        .map(|v| matches!(v, PinValue::Suppress { .. }))
        .unwrap_or(false)
}

fn addition_to_classification(addition: &ComponentEntry) -> Classification {
    use crate::types::ComponentKind;
    let kind = ComponentKind::parse(&addition.kind).unwrap_or(ComponentKind::NonComponent);
    Classification {
        kind,
        language: addition.language.clone(),
        build_system: addition.build_system.clone(),
        lifecycle_roles: addition.lifecycle_roles.clone(),
        role: addition.role.clone(),
        evidence_grade: addition.evidence_grade,
        evidence_fields: addition.evidence_fields.clone(),
        rationale: addition.rationale.clone(),
        is_boundary: true,
    }
}

// ---------------------------------------------------------------------
// Phase 3: acyclicity.
// ---------------------------------------------------------------------

fn enforce_acyclicity(components: &[ComponentEntry]) -> Result<(), TreeAssemblyError> {
    let parent_by_id: HashMap<&str, Option<&str>> = components
        .iter()
        .map(|c| (c.id.as_str(), c.parent.as_deref()))
        .collect();

    for entry in components {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut cursor: Option<&str> = Some(entry.id.as_str());
        while let Some(id) = cursor {
            if !seen.insert(id) {
                return Err(TreeAssemblyError::Cycle { id: entry.id.clone() });
            }
            cursor = match parent_by_id.get(id) {
                Some(Some(parent)) => Some(parent),
                _ => None,
            };
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------

fn relative_to_root(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn absolute_under_root(root: &Path, relative: &Path) -> PathBuf {
    if relative.is_absolute() {
        relative.to_path_buf()
    } else {
        root.join(relative)
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_with_parent(id: &str, parent: Option<&str>) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: parent.map(String::from),
            kind: "spec".into(),
            lifecycle_roles: vec![],
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: String::new(),
            deleted: false,
        }
    }

    #[test]
    fn enforce_acyclicity_accepts_tree() {
        let entries = vec![
            entry_with_parent("root", None),
            entry_with_parent("child", Some("root")),
            entry_with_parent("grandchild", Some("child")),
        ];
        assert!(enforce_acyclicity(&entries).is_ok());
    }

    #[test]
    fn enforce_acyclicity_rejects_self_parent() {
        let entries = vec![entry_with_parent("self", Some("self"))];
        let err = enforce_acyclicity(&entries).unwrap_err();
        match err {
            TreeAssemblyError::Cycle { id } => assert_eq!(id, "self"),
        }
    }

    #[test]
    fn enforce_acyclicity_rejects_two_cycle() {
        let entries = vec![
            entry_with_parent("a", Some("b")),
            entry_with_parent("b", Some("a")),
        ];
        assert!(enforce_acyclicity(&entries).is_err());
    }

    #[test]
    fn enforce_acyclicity_rejects_three_cycle() {
        let entries = vec![
            entry_with_parent("a", Some("b")),
            entry_with_parent("b", Some("c")),
            entry_with_parent("c", Some("a")),
        ];
        assert!(enforce_acyclicity(&entries).is_err());
    }
}

