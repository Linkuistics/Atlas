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
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_index::{
    ComponentEntry, ComponentFieldOverrides, ComponentsFile, DocAnchor, OverridesFile, PathSegment,
    PinValue, RenameMap,
};
use component_ontology::{ComponentId, LifecycleScope};

use crate::db::{AtlasDatabase, Workspace};
use crate::identifiers::allocate_id;
use crate::identifiers::{slugify_path, slugify_segment};
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

    /// A per-component override file (`<component-path>/.atlas/overrides.yaml`)
    /// declared a pin or addition for a component id outside its
    /// scoping prefix. Per the spec
    /// (`docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md`
    /// §5), per-component override files may only carry entries for
    /// the owning component or its sub-components. Cross-component
    /// pins are always a hard error, never a warning.
    #[error(
        "per-component override file `{file}` declares an entry for `{offending_id}` \
         which is outside its scoping prefix `{owner_prefix}`; per-component overrides \
         may only target the owning component or its sub-components (spec §5)"
    )]
    PerComponentScopeViolation {
        file: String,
        offending_id: String,
        owner_prefix: String,
    },

    /// A per-component override file failed to parse as
    /// [`OverridesFile`]. Malformed override files are a hard error
    /// rather than a soft warning because silently dropping pins
    /// would yield a different component tree without the user
    /// realising it.
    #[error("failed to parse per-component override file `{file}`: {message}")]
    PerComponentParseError { file: String, message: String },

    /// A per-component override file declared `edges_add` or
    /// `edges_suppress`. Per design §5.5, these top-level-only
    /// blocks describe relationships between two components and
    /// therefore have no natural owner at the per-component scope —
    /// they belong only in the primary or peer top-level
    /// `components.overrides.yaml`. Allowing them at the per-
    /// component scope would let a single component's overrides file
    /// silently mutate edges anchored elsewhere in the tree, which
    /// the spec explicitly prohibits.
    #[error(
        "per-component override file `{file}` declares `{kind}`, which is permitted \
         only at the top-level `components.overrides.yaml` (design §5.5); \
         move the entry to the workspace's primary or peer top-level overrides file"
    )]
    EdgesOverridesAtPerComponentScope { file: String, kind: &'static str },
}

/// Map from [`ComponentId`] to `(analyser_id, analyser_version)` as captured
/// during L4 assembly. Used by [`all_component_analyser_identities`] and its
/// internal callers to avoid the verbosity of the nested generic in
/// function signatures.
type AnalyserIdentityMap = BTreeMap<ComponentId, (String, String)>;

/// Build the full component tree. The returned vector is sorted by id
/// for deterministic YAML output. Panics on a cycle, matching the
/// design-doc position that acyclicity is a hard invariant (§4.2).
pub fn all_components(db: &AtlasDatabase) -> Arc<Vec<ComponentEntry>> {
    match try_assemble(db) {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    }
}

/// Return the dispatching analyser's `(analyser_id, analyser_version)` for
/// every live component in the tree, keyed by component id. Identities are
/// captured during L4 assembly — before the `Classification` is discarded
/// — so this is the only place that accurately reflects the `"override"`
/// sentinel for `overrides.additions` entries (which bypass `is_component`
/// entirely and are therefore invisible to callers that re-invoke it post-
/// assembly). Tombstoned components are excluded. Panics on a cycle,
/// matching the invariant in [`all_components`].
pub fn all_component_analyser_identities(db: &AtlasDatabase) -> Arc<AnalyserIdentityMap> {
    match try_assemble_inner(db, &mut io::stderr()) {
        Ok((_, identities, _rename_map)) => identities,
        Err(e) => panic!("{e}"),
    }
}

/// Phase 6 PR-2: return the rename-match-derived `prior_id → final_id`
/// map for the current run. The map is empty under the path-derived id
/// allocator's rename-match-preserves-prior-id behaviour; entries
/// appear only when an explicit_id override (or a future allocator
/// change) caused the live entry to land on a different id than the
/// prior it matched. Used by L5 / L9 / L6 to apply the contract
/// owner-follows rewrite to contract ids and to
/// related-components.yaml edge participants. Panics on a cycle,
/// matching the invariant in [`all_components`].
pub fn rename_map_after_match(db: &AtlasDatabase) -> Arc<RenameMap> {
    match try_assemble_inner(db, &mut io::stderr()) {
        Ok((_, _identities, rename_map)) => rename_map,
        Err(e) => panic!("{e}"),
    }
}

/// Return the merged `OverridesFile` (Phase 3 PR-6) — pins,
/// additions, edges_add, and edges_suppress unioned across the
/// primary + peer + per-component override files discovered by the
/// L4 walk. The returned file's `field_overrides` is left at default
/// (per-component field overrides are applied directly to the
/// component tree at L4 and are not re-projected onto the merged
/// file). Used by L6 to apply `edges_add` / `edges_suppress` to the
/// analyser-discovered edge set.
///
/// Panics on a discovery error or scope violation, matching the
/// invariant in [`all_components`]. Callers that want to recover
/// from those errors should use [`try_assemble`] directly and walk
/// the override files themselves.
pub(crate) fn merged_overrides(db: &AtlasDatabase) -> Arc<OverridesFile> {
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let primary_overrides = workspace
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    match merge_overrides_in_discovery_order(
        std::slice::from_ref(&root),
        &root,
        &primary_overrides,
        &mut io::stderr(),
    ) {
        Ok(merged) => Arc::new(merged.file),
        Err(e) => panic!("{e}"),
    }
}

/// Fallible form of [`all_components`] for tests that want to assert
/// the acyclicity error is reachable without asking the harness to
/// catch a panic.
pub fn try_assemble(db: &AtlasDatabase) -> Result<Arc<Vec<ComponentEntry>>, TreeAssemblyError> {
    try_assemble_with_warnings(db, &mut io::stderr())
}

/// Same as [`try_assemble`] but routes override-conflict warnings to
/// `warnings` instead of stderr. Used by tests that need to assert on
/// the warning text without process plumbing.
pub fn try_assemble_with_warnings(
    db: &AtlasDatabase,
    warnings: &mut dyn Write,
) -> Result<Arc<Vec<ComponentEntry>>, TreeAssemblyError> {
    let (components, _identities, _rename_map) = try_assemble_inner(db, warnings)?;
    Ok(components)
}

/// Phase 6 PR-2: triple of artefacts returned by [`try_assemble_inner`].
/// Bundles the sorted component entries, the per-component analyser
/// identity map, and the rename-match-derived `prior_id → final_id`
/// rename map so callers that need any subset do not run the
/// assembly twice. Factored to a named alias so the function
/// signature passes `clippy::type_complexity`.
type AssemblyArtefacts = (
    Arc<Vec<ComponentEntry>>,
    Arc<AnalyserIdentityMap>,
    Arc<RenameMap>,
);

/// Core assembly implementation used by [`try_assemble_with_warnings`],
/// [`all_component_analyser_identities`], and [`rename_map_after_match`].
/// Returns the sorted component entries, the per-component analyser
/// identity map, and the rename-match-derived `prior_id → final_id`
/// rename map (Phase 6 PR-2) in one pass so callers that need any
/// subset do not run the assembly twice.
fn try_assemble_inner(
    db: &AtlasDatabase,
    warnings: &mut dyn Write,
) -> Result<AssemblyArtefacts, TreeAssemblyError> {
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let primary_overrides = workspace
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    let prior = workspace
        .prior_components(db as &dyn salsa::Database)
        .clone();

    // Per spec §3, the override discovery walk runs in three tiers:
    // primary-root top-level, peer-root top-level (lex-sorted), then
    // per-component (component-id-sorted, simulated here by file path
    // sort because we don't have ids yet — see `walk_per_component`
    // for the rationale). Files seeded onto `Workspace.files` by the
    // CLI's filesystem walk are the source of every per-component
    // file; the root's top-level overrides are injected by the CLI on
    // the workspace input (handled before this file is read) and
    // arrive here as `primary_overrides`.
    let merged = merge_overrides_in_discovery_order(
        std::slice::from_ref(&root),
        &root,
        &primary_overrides,
        warnings,
    )?;
    let merged_overrides = &merged.file;

    let live: Vec<LiveComponent> = gather_live_components(db, workspace, &root, merged_overrides);
    let roots = [root.clone()];
    let (mut finalised, identities, rename_map) =
        resolve_ids_and_tombstones(&prior, merged_overrides, &roots, live);

    // PR-6: apply per-component field overrides on top of the
    // finalised entries. The merge collected `(owning_dir,
    // ComponentFieldOverrides)` pairs at discovery time; we resolve
    // each owning_dir to the allocated `ComponentId` here. Field
    // overrides supersede analyser-emitted values per design §5.5;
    // they are also strictly more specific than primary/peer pins on
    // the same component, so they take effect regardless of pin
    // settings (closest-source-wins, consistent with the existing
    // pin precedence rule).
    apply_per_component_field_overrides(
        &mut finalised,
        &merged.per_component_field_overrides,
        &roots,
        warnings,
    );

    enforce_acyclicity(&finalised)?;

    let mut out: Vec<ComponentEntry> = finalised;
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((Arc::new(out), Arc::new(identities), Arc::new(rename_map)))
}

/// Apply per-component field overrides (PR-6) to the finalised
/// component entries. For each `(owning_dir, ComponentFieldOverrides)`
/// pair collected during the discovery walk, find the entry whose
/// first path segment resolves to `owning_dir` (relative to its
/// owning workspace root), and overwrite the four supported fields:
///
/// - `language` widens to a single-element `BTreeSet<String>` on
///   `ComponentEntry.languages`.
/// - `kind` overwrites `ComponentEntry.kind` directly.
/// - `lifecycle` parses through [`LifecycleScope`] and replaces the
///   analyser-emitted `lifecycle_roles` with the single authored
///   scope. An unparseable lifecycle string is dropped (the analyser
///   value sticks) and a warning is emitted to the same channel that
///   carries override-conflict warnings, so the user sees the typo
///   without the run failing. The design spec leaves the lifecycle
///   vocabulary open and future analysers may add scopes the engine
///   binary has not yet learned about, so a hard error here would be
///   wrong.
/// - `subsystem` does not flow through `ComponentEntry`. PR-3
///   threads it instead through L9 subsystem resolution via
///   [`per_component_subsystem_overrides`]; the per-component file
///   wins over a central `subsystems.overrides.yaml` entry that
///   names the same component.
///
/// Applied unconditionally: if the analyser already emitted the same
/// value, the overwrite is a no-op. If no entry matches `owning_dir`
/// (the directory has no allocated component, e.g. the per-component
/// overrides file is in a not-yet-recognised directory), the entry
/// is silently skipped — the existing scoping check rejected it at
/// discovery time if it was ill-formed.
fn apply_per_component_field_overrides(
    entries: &mut [ComponentEntry],
    by_dir: &BTreeMap<PathBuf, ComponentFieldOverrides>,
    roots: &[PathBuf],
    warnings: &mut dyn Write,
) {
    for (dir, fo) in by_dir {
        // Resolve the directory to its allocated id by matching the
        // first path segment of each finalised entry. The path
        // segment is stored relative to the owning workspace root,
        // so we relativise `dir` against the same root.
        let owning_root = roots.iter().find(|r| dir.starts_with(r));
        let owning_root = match owning_root {
            Some(r) => r.as_path(),
            None => continue,
        };
        let rel = dir.strip_prefix(owning_root).unwrap_or(dir);
        let entry = entries
            .iter_mut()
            .find(|e| e.path_segments.first().map(|s| s.path.as_path()) == Some(rel));
        let Some(entry) = entry else { continue };

        if let Some(language) = &fo.language {
            entry.languages.clear();
            entry.languages.insert(language.clone());
        }
        if let Some(kind) = &fo.kind {
            entry.kind = kind.clone();
        }
        if let Some(lifecycle) = &fo.lifecycle {
            // The lifecycle vocabulary is open (per-analyser
            // contributions land in `LifecycleScope`). An
            // unparseable value indicates either a typo in the
            // user's overrides file or an analyser-shipped scope
            // the engine binary has not yet learned about; the
            // failure mode is "no override applied, analyser value
            // sticks". Emit a warning so the user sees the typo
            // without the run failing.
            if let Some(scope) = LifecycleScope::parse(lifecycle) {
                entry.lifecycle_roles = vec![scope];
            } else {
                let _ = writeln!(
                    warnings,
                    "warning: unrecognised lifecycle scope `{}` in per-component overrides at {} — override not applied",
                    lifecycle,
                    dir.display()
                );
            }
        }
        // `subsystem` has no destination field on `ComponentEntry` —
        // it is consumed downstream by L9 subsystem projection via
        // [`per_component_subsystem_overrides`], which re-runs this
        // resolution step against the merged-override map. We
        // therefore deliberately leave the `subsystem` field alone
        // here.
    }
}

/// Phase 6 PR-3: extract per-component subsystem overrides as a map
/// from `ComponentId` to subsystem name.
///
/// For each per-component override file
/// (`<path>/.atlas/components.overrides.yaml`) discovered during the
/// override-merge walk, if `field_overrides.subsystem` is set, the
/// owning directory is resolved against the live component tree and
/// the resulting `ComponentId` is paired with the authored subsystem
/// name. Components whose per-component file omits the `subsystem`
/// field are absent from the map. Directories that match no live
/// component (e.g. the per-component file sits in a not-yet-recognised
/// directory) are silently skipped — the existing scoping check
/// rejected them at discovery time if they were ill-formed.
///
/// Used by [`crate::l9_subsystems::subsystems_yaml_snapshot`]
/// to overlay per-component assignments on top of the central
/// `subsystems.overrides.yaml`. Per-component overrides take precedence
/// over the central file (LLM-spine recast spec §4.1 — closer-to-source
/// authoring).
///
/// Panics on the same conditions as [`all_components`].
pub fn per_component_subsystem_overrides(db: &AtlasDatabase) -> BTreeMap<ComponentId, String> {
    // TODO(phase6-pr4 or later): consolidate this walk into
    // `apply_per_component_field_overrides` to avoid a third recursive
    // `merge_overrides_in_discovery_order` call per `run_index`. Today the
    // walk is redundant with the one in `try_assemble_inner`; the cleaner
    // fix is to collect the BTreeMap in the same loop where lifecycle /
    // language / kind overrides are already applied.
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let primary_overrides = workspace
        .components_overrides(db as &dyn salsa::Database)
        .clone();
    let merged = match merge_overrides_in_discovery_order(
        std::slice::from_ref(&root),
        &root,
        &primary_overrides,
        &mut io::sink(),
    ) {
        Ok(m) => m,
        Err(e) => panic!("{e}"),
    };

    let components = all_components(db);
    let roots = [root];
    let mut out: BTreeMap<ComponentId, String> = BTreeMap::new();
    for (dir, fo) in &merged.per_component_field_overrides {
        let Some(subsystem_name) = fo.subsystem.as_ref() else {
            continue;
        };
        // Mirror the directory→component-id resolution that
        // `apply_per_component_field_overrides` uses: the first path
        // segment of the entry, relativised against its owning root,
        // identifies the component.
        let owning_root = match roots.iter().find(|r| dir.starts_with(r)) {
            Some(r) => r.as_path(),
            None => continue,
        };
        let rel = dir.strip_prefix(owning_root).unwrap_or(dir);
        let entry = components
            .iter()
            .find(|e| e.path_segments.first().map(|s| s.path.as_path()) == Some(rel));
        if let Some(entry) = entry {
            out.insert(entry.id.clone(), subsystem_name.clone());
        }
    }
    out
}

/// Parent component id of `id` per the assembled tree, or `None` when
/// `id` is at the root or does not exist.
pub fn component_parent(db: &AtlasDatabase, id: &ComponentId) -> Option<ComponentId> {
    all_components(db)
        .iter()
        .find(|c| &c.id == id)
        .and_then(|c| c.parent.clone())
}

/// Immediate children of `id` — any component whose `parent` field
/// equals `id`. Returned sorted by id for determinism.
pub fn component_children(db: &AtlasDatabase, id: &ComponentId) -> Arc<Vec<ComponentId>> {
    let mut out: Vec<ComponentId> = all_components(db)
        .iter()
        .filter(|c| c.parent.as_ref() == Some(id))
        .map(|c| c.id.clone())
        .collect();
    out.sort();
    Arc::new(out)
}

/// Path segments of the component with id `id`, or an empty vector if
/// the id does not exist.
pub fn component_path_segments(db: &AtlasDatabase, id: &ComponentId) -> Arc<Vec<PathSegment>> {
    let segments = all_components(db)
        .iter()
        .find(|c| &c.id == id)
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
    explicit_parent_id: Option<ComponentId>,
    /// Explicit id for override-additions entries; `None` for
    /// signal-derived components (which pick an id during allocation).
    explicit_id: Option<ComponentId>,
    /// The workspace root that owns this component's directory. Stored
    /// so id allocation and parent-derivation reason about the right
    /// root in the multi-root case (a component under
    /// `/ravel-lite/...` allocates its id under the `ravel-lite`
    /// namespace; a peer-root component under `/atlas-contracts/...`
    /// gets `atlas-contracts/...`). Equal to `dir`-trimmed-to-root for
    /// signal-derived components; equal to the matching root for
    /// override-addition entries (with absolute path resolution).
    /// Reserved for future per-root id-namespace prefixing — Phase 1
    /// keeps id allocation root-agnostic, so the field is not yet
    /// consumed.
    #[allow(dead_code)]
    owning_root: PathBuf,
}

fn gather_live_components(
    db: &AtlasDatabase,
    workspace: Workspace,
    root: &Path,
    overrides: &OverridesFile,
) -> Vec<LiveComponent> {
    let candidates =
        candidate_components_at(db as &dyn salsa::Database, workspace, root.to_path_buf());

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
        let doc_anchors: Vec<DocAnchor> =
            doc_headings(db as &dyn salsa::Database, workspace, candidate.dir.clone())
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
        let tree_sha = file_tree_sha(db as &dyn salsa::Database, workspace, dir.clone());
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
            owning_root: root.to_path_buf(),
        });
    }

    // Overrides.additions: append as explicit-id entries scoped to
    // this root. The `gather_live_components` driver is called once
    // per root, and additions that resolve to a path *inside* this
    // root are owned by it; additions that resolve elsewhere are
    // skipped here and handled when the loop reaches their owning
    // root. A pin with `suppress: true` at the addition's id removes
    // it.
    for addition in &overrides.additions {
        if is_suppressed_by_pin(overrides, &addition.id) {
            continue;
        }
        let abs_dir = addition
            .path_segments
            .first()
            .map(|seg| absolute_under_root(root, &seg.path))
            .unwrap_or_else(|| root.to_path_buf());
        // Skip additions whose first path segment doesn't fall under
        // *this* root — they'll be picked up on a different
        // per-root pass.
        if !abs_dir.starts_with(root) {
            continue;
        }
        let classification = addition_to_classification(addition);
        let parent_dir = nearest_confirmed_ancestor(&abs_dir, &confirmed_dirs);
        live.push(LiveComponent {
            dir: abs_dir,
            classification,
            path_segments: addition.path_segments.clone(),
            manifests: addition.manifests.clone(),
            doc_anchors: addition.doc_anchors.clone(),
            provisional_parent_dir: parent_dir,
            explicit_parent_id: addition.parent.clone(),
            explicit_id: Some(addition.id.clone()),
            owning_root: root.to_path_buf(),
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
    roots: &[PathBuf],
    live: Vec<LiveComponent>,
) -> (Vec<ComponentEntry>, AnalyserIdentityMap, RenameMap) {
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
            id: ComponentId::parse(&format!("provisional-{i}"))
                .expect("provisional-N is a valid ComponentId"),
            parent: None,
            kind: lc.classification.kind.as_str().into(),
            lifecycle_roles: lc.classification.lifecycle_roles.clone(),
            languages: lc.classification.languages.clone(),
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
    // `roots` is reserved for cross-root id-namespace prefixing in a
    // future PR. Phase 1 leaves identifier allocation root-agnostic
    // because `identifiers::allocate_id` already uses the directory
    // basename + parent-id chain — multi-root naturally yields
    // distinct ids when the per-root basename sets disjoin.
    let _ = roots;

    let match_out = atlas_index::rename_match(atlas_index::RenameMatchInput::new(
        &prior_live,
        &live_entries,
    ));

    // Build a map: live index → matched prior id (if any).
    let mut live_to_prior_id: HashMap<usize, ComponentId> = HashMap::new();
    for (prior_idx, live_idx) in &match_out.matches {
        live_to_prior_id.insert(*live_idx, prior_live[*prior_idx].id.clone());
    }

    // Allocate ids. Iterate in the same order as `live` so a
    // LiveComponent at index i aligns with its provisional entry.
    // Process shallow-first so parents get ids before children.
    let mut order: Vec<usize> = (0..live.len()).collect();
    order.sort_by_key(|i| live[*i].dir.components().count());

    let mut allocated_ids: Vec<Option<ComponentId>> = vec![None; live.len()];
    let mut existing_ids: HashSet<ComponentId> = HashSet::new();

    for &i in &order {
        let lc = &live[i];
        let id = if let Some(explicit) = &lc.explicit_id {
            explicit.clone()
        } else if let Some(prior_id) = live_to_prior_id.get(&i) {
            prior_id.clone()
        } else {
            let parent_id: Option<ComponentId> = lc
                .provisional_parent_dir
                .as_ref()
                .and_then(|p| dir_to_live_index(p, &live))
                .and_then(|idx| allocated_ids[idx].clone());
            allocate_id(&lc.dir, parent_id.as_ref(), &existing_ids)
        };
        existing_ids.insert(id.clone());
        allocated_ids[i] = Some(id);
    }

    // Now build the final ComponentEntry list.
    let mut out: Vec<ComponentEntry> = Vec::new();
    // Identity map: component-id → (analyser_id, analyser_version). Captured
    // here while `live[i].classification` is still in scope; once
    // `LiveComponent` is dropped the identity information is gone. The map is
    // the authoritative source for `lookup_analyser_identity` in L9 — it
    // correctly records `"override"` for `overrides.additions` entries that
    // bypassed `is_component` entirely.
    let mut identities: AnalyserIdentityMap = BTreeMap::new();
    let mut id_by_dir: BTreeMap<PathBuf, ComponentId> = BTreeMap::new();
    for i in 0..live.len() {
        id_by_dir.insert(
            live[i].dir.clone(),
            allocated_ids[i]
                .clone()
                .expect("every live component has an allocated id"),
        );
    }
    for i in 0..live.len() {
        let lc = &live[i];
        let id = allocated_ids[i]
            .clone()
            .expect("every live component has an allocated id");
        let parent = lc.explicit_parent_id.clone().or_else(|| {
            lc.provisional_parent_dir
                .as_ref()
                .and_then(|p| id_by_dir.get(p).cloned())
        });
        identities.insert(
            id.clone(),
            (
                lc.classification.analyser_id.clone(),
                lc.classification.analyser_version.clone(),
            ),
        );
        out.push(ComponentEntry {
            id,
            parent,
            kind: lc.classification.kind.as_str().into(),
            lifecycle_roles: lc.classification.lifecycle_roles.clone(),
            languages: lc.classification.languages.clone(),
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
        for id in &suppressed {
            identities.remove(id);
        }
    }

    // Orphan tombstones. Tombstones are not live components, so they have no
    // identity entry in the map — `lookup_analyser_identity` already handles
    // the not-found case with a graceful fallback.
    for prior_idx in &match_out.orphans {
        let mut tomb = prior_live[*prior_idx].clone();
        tomb.deleted = true;
        out.push(tomb);
    }

    // Phase 6 PR-2: build the rename map (prior_id → final allocated id)
    // for every rename-match that produced a *different* final id. The
    // common case under the path-derived id allocator is the identity
    // map — rename-match preserves the prior id, so the entry is
    // filtered out at construction. Entries appear only when an
    // explicit_id override (or a future allocator change) caused the
    // live entry to land on a different id than the prior it matched.
    // The map drives the owner-follows rewrite on contract ids and on
    // related-components.yaml edge participants downstream.
    let mut rename_map: RenameMap = RenameMap::new();
    for (prior_idx, live_idx) in &match_out.matches {
        let prior_id = prior_live[*prior_idx].id.clone();
        let final_id = allocated_ids[*live_idx]
            .clone()
            .expect("every live component has an allocated id");
        if prior_id != final_id {
            rename_map.insert(prior_id, final_id);
        }
    }

    (out, identities, rename_map)
}

fn dir_to_live_index(dir: &Path, live: &[LiveComponent]) -> Option<usize> {
    live.iter().position(|lc| lc.dir == dir)
}

fn collect_suppressed_children(
    components: &[ComponentEntry],
    overrides: &OverridesFile,
) -> HashSet<ComponentId> {
    let mut out: HashSet<ComponentId> = HashSet::new();
    for (key, pins) in &overrides.pins {
        if let Some(PinValue::SuppressChildren { suppress_children }) =
            pins.get("suppress_children")
        {
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

fn is_suppressed_by_pin(overrides: &OverridesFile, id: &ComponentId) -> bool {
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
        languages: addition.languages.clone(),
        build_system: addition.build_system.clone(),
        lifecycle_roles: addition.lifecycle_roles.clone(),
        role: addition.role.clone(),
        evidence_grade: addition.evidence_grade,
        evidence_fields: addition.evidence_fields.clone(),
        rationale: addition.rationale.clone(),
        is_boundary: true,
        // `overrides.additions` entries bypass the analyser registry
        // entirely; the override sentinel records that provenance so
        // the per-component projection can distinguish hand-authored
        // entries from analyser verdicts.
        analyser_id: crate::l3_classify::OVERRIDE_ANALYSER_ID.to_string(),
        analyser_version: crate::l3_classify::OVERRIDE_ANALYSER_VERSION.to_string(),
    }
}

// ---------------------------------------------------------------------
// Override-merge discovery walk (PR-6 / spec §3-§5).
// ---------------------------------------------------------------------

/// Result of the merged-discovery walk (Phase 3 PR-6).
///
/// Carries both the merged `OverridesFile` (pins, additions,
/// edges_add, edges_suppress unioned across every discovered file)
/// AND the per-component `field_overrides` keyed by the owning
/// component directory. The directory→id mapping is finalised after
/// L4 id allocation, so the merge cannot pre-resolve ids itself.
#[derive(Debug, Clone)]
pub(crate) struct MergedOverrides {
    /// Pins, additions, edges_add, edges_suppress merged across
    /// every discovered overrides file.
    pub(crate) file: OverridesFile,
    /// Per-component field overrides, keyed by the directory that
    /// owns the per-component override file (i.e. the parent of
    /// `<dir>/.atlas/overrides.yaml`). Resolved to a `ComponentId`
    /// post-id-allocation by the caller. The map preserves
    /// insertion order via `BTreeMap` (path-sorted).
    pub(crate) per_component_field_overrides: BTreeMap<PathBuf, ComponentFieldOverrides>,
}

/// Source category of an override file, used by the conflict-warning
/// emitter to label the lines per spec §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverrideSource {
    Primary,
    Peer,
    PerComponent,
}

impl OverrideSource {
    fn label(self) -> &'static str {
        match self {
            OverrideSource::Primary => "primary",
            OverrideSource::Peer => "peer",
            OverrideSource::PerComponent => "per-component",
        }
    }
}

/// One discovered override file.
struct DiscoveredOverride {
    /// Display path (canonicalised when possible) used for warning
    /// emission and error messages.
    display_path: String,
    /// Source category — drives both the warning label and §5
    /// scoping rules (the scoping check applies only to per-component).
    source: OverrideSource,
    /// Implied owner-id prefix (per spec §5). `None` for top-level
    /// files (no scoping check). For per-component files, both the
    /// path-only and the root-prefixed form are recorded so the
    /// check matches multi-root id derivations where the root
    /// basename becomes the id namespace. Phase 1 validates this at
    /// discovery time (see [`validate_per_component_scope`]); this
    /// field is retained on the struct for forward-compatibility
    /// with Phase 2 (`--strict-overrides` may want to re-validate
    /// against the assembled tree, where the post-assembly id form
    /// is known).
    #[allow(dead_code)]
    scoping_prefixes: Option<Vec<String>>,
    /// Owning component directory, set only for per-component files
    /// (`Some(<parent of .atlas/>)`). `None` for primary/peer
    /// top-level files. PR-6 uses this to resolve `field_overrides`
    /// onto the post-allocation id of the directory.
    owning_dir: Option<PathBuf>,
    /// Parsed contents.
    file: OverridesFile,
}

/// Discover every override file (primary, peer, per-component),
/// validate per-component scoping, and merge the lot in discovery
/// order with last-writer-wins semantics. Conflicts on the same
/// `(component_id, key)` tuple emit warnings to `warnings`.
///
/// Per-component override files are discovered via a direct
/// filesystem walk (see [`find_per_component_overrides_under`])
/// rather than through `Workspace.files` — the seed walk excludes
/// `.atlas/` directories, so engine-side discovery cannot rely on
/// them being in the file index. The function therefore does not
/// need a database handle; future PRs that want Salsa-tracked
/// override discovery should add a parallel tracked-query path.
fn merge_overrides_in_discovery_order(
    roots: &[PathBuf],
    primary_root: &Path,
    primary_overrides: &OverridesFile,
    warnings: &mut dyn Write,
) -> Result<MergedOverrides, TreeAssemblyError> {
    let mut discovered: Vec<DiscoveredOverride> = Vec::new();

    // Tier 1: primary-root top-level. The CLI populates this from
    // `<primary-root>/.atlas/components.overrides.yaml` and installs
    // it on the Workspace input; we receive it as `primary_overrides`.
    // Empty (default-constructed) primary overrides are still
    // recorded with a synthesised display path so any warning naming
    // them prints a sensible file name.
    let primary_display = primary_root
        .join(".atlas")
        .join("components.overrides.yaml")
        .display()
        .to_string();
    discovered.push(DiscoveredOverride {
        display_path: primary_display,
        source: OverrideSource::Primary,
        scoping_prefixes: None,
        owning_dir: None,
        file: primary_overrides.clone(),
    });

    // Tier 2: peer-root top-level files, lex-sorted by canonical
    // absolute path (spec §3 step 2).
    let mut peer_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    for root in roots.iter().skip(1) {
        let candidate = root.join(".atlas").join("components.overrides.yaml");
        if candidate.exists() {
            let canonical = std::fs::canonicalize(&candidate).unwrap_or(candidate.clone());
            peer_files.push((canonical, candidate));
        }
    }
    peer_files.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, path) in peer_files {
        let parsed = read_overrides_file(&path)?;
        discovered.push(DiscoveredOverride {
            display_path: path.display().to_string(),
            source: OverrideSource::Peer,
            scoping_prefixes: None,
            owning_dir: None,
            file: parsed,
        });
    }

    // Tier 3: per-component files. Walk each root's filesystem
    // directly looking for paths matching `<dir>/.atlas/overrides.yaml`
    // (NOT `components.overrides.yaml`, which is the top-level
    // form). Files inside `.atlas/` directories are intentionally
    // **not** seeded onto `Workspace.files` (`ingest::seed_filesystem`
    // skips them) so that PR-6's per-component writers don't cause
    // their own outputs to feed back into L0; the override discovery
    // walks the filesystem directly to side-step that exclusion.
    //
    // Walk is keyed off the file path, not off `all_components` ids,
    // because the override merge runs before tree assembly. The
    // per-component scoping check uses the file's directory location
    // to derive the implied owner-id prefix; cross-component pins
    // are a hard error (spec §5).
    let mut per_component_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    for root in roots {
        for path in find_per_component_overrides_under(root) {
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            per_component_files.push((canonical, path));
        }
    }
    // Sort by component-id-equivalent ordering (spec §3 step 3):
    // we don't have ids yet, so order by canonical file path —
    // strictly monotonic in directory depth + lexicographic
    // basename. For the §3 test obligations the only requirement is
    // determinism and "later wins", which path-sort satisfies for
    // sibling components and for ancestor/descendant pairs. Dedup
    // by canonical path so a peer root visited transitively via
    // path-deps does not contribute the same file twice.
    per_component_files.sort_by(|a, b| a.0.cmp(&b.0));
    per_component_files.dedup_by(|a, b| a.0 == b.0);
    for (_, path) in per_component_files {
        let parsed = read_overrides_file(&path)?;
        let prefixes = derive_scoping_prefixes(&path, roots);
        // Validate scoping eagerly so the error names the offending
        // file the moment we see the violation (rather than after
        // every other override is merged).
        validate_per_component_scope(&path, &parsed, &prefixes)?;
        // The owning component directory is the parent of `.atlas/`,
        // i.e. two ancestors up from `overrides.yaml`. Used post-merge
        // to apply `field_overrides` onto the directory's allocated
        // id.
        let owning_dir = path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        discovered.push(DiscoveredOverride {
            display_path: path.display().to_string(),
            source: OverrideSource::PerComponent,
            scoping_prefixes: Some(prefixes),
            owning_dir,
            file: parsed,
        });
    }

    Ok(merge_discovered_overrides(&discovered, warnings))
}

/// True iff `path` is a per-component overrides file
/// (`<dir>/.atlas/overrides.yaml`), as opposed to the top-level form
/// (`<dir>/.atlas/components.overrides.yaml`). Retained for the
/// recursive walker below; the engine itself now discovers via a
/// direct filesystem walk because `Workspace.files` no longer
/// includes `.atlas/` contents (see `ingest::seed_filesystem`).
#[allow(dead_code)]
fn is_per_component_overrides_path(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) != Some("overrides.yaml") {
        return false;
    }
    let parent = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    parent.file_name().and_then(|n| n.to_str()) == Some(".atlas")
}

/// Walk `root` looking for `<dir>/.atlas/overrides.yaml` files and
/// return their absolute paths. The walker descends into hidden
/// directories (it has to: `.atlas/` IS hidden) but skips `.git/`
/// and `target/` as a courtesy on large repos. The CLI ingests
/// every other file under `root` via `seed_filesystem`, but
/// `seed_filesystem` skips `.atlas/` entirely (its contents are
/// tool-owned and would otherwise feed L0 from the previous run's
/// outputs); the override walk side-steps that exclusion.
fn find_per_component_overrides_under(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk_for_overrides(root, &mut out);
    out
}

fn walk_for_overrides(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip noisy build artefacts and version-control dirs.
            // `.atlas/` is descended specifically to find the
            // overrides file inside it.
            if matches!(name, ".git" | "target" | "node_modules") {
                continue;
            }
            if name == ".atlas" {
                let candidate = path.join("overrides.yaml");
                if candidate.is_file() {
                    out.push(candidate);
                }
                // Don't descend deeper into `.atlas/` — its only
                // override file lives directly inside.
                continue;
            }
            walk_for_overrides(&path, out);
        }
    }
}

/// Derive the implied owner-id prefix(es) for a per-component
/// override file at `<root>/<rel-path>/.atlas/overrides.yaml`.
///
/// Returns one or two candidate prefixes. The first is the
/// path-derived form (slugified relative path under the owning
/// root); the second, when applicable, is the root-prefixed form
/// (`<root-basename-slug>/<rel-path-slug>`) which corresponds to the
/// usual id derivation when the root itself is a component.
///
/// Either form is valid for the §5 scoping check: an entry id is
/// in-scope iff it matches *any* prefix as `id == prefix` or
/// `id.starts_with(format!("{prefix}/"))`. Both forms are checked
/// because Phase 1 cannot distinguish at override-merge time
/// (before `all_components` runs) which form the actual id will
/// take. The check is conservative: a pin that matches neither
/// form is rejected as the spec demands.
fn derive_scoping_prefixes(file_path: &Path, roots: &[PathBuf]) -> Vec<String> {
    // Component dir is the parent of `.atlas/`, i.e. two levels up
    // from `overrides.yaml`. (Layout: `<dir>/.atlas/overrides.yaml`.)
    let dir = file_path
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new(""));

    // Find the owning root (the root that contains dir, if any).
    let owning_root = roots.iter().find(|r| dir.starts_with(r.as_path()));

    let rel_path = match owning_root {
        Some(root) => dir.strip_prefix(root).unwrap_or(dir),
        None => dir,
    };
    let path_slug = slugify_path(rel_path);

    let mut out: Vec<String> = Vec::new();
    if !path_slug.is_empty() {
        out.push(path_slug.clone());
    }
    if let Some(root) = owning_root {
        if let Some(root_basename) = root
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(slugify_segment)
        {
            let prefixed = if path_slug.is_empty() {
                root_basename
            } else {
                format!("{root_basename}/{path_slug}")
            };
            if !out.contains(&prefixed) {
                out.push(prefixed);
            }
        }
    }
    out
}

fn validate_per_component_scope(
    file_path: &Path,
    file: &OverridesFile,
    prefixes: &[String],
) -> Result<(), TreeAssemblyError> {
    // Design §5.5: `edges_add` / `edges_suppress` are top-level-only.
    // A per-component file may not author edges because edges
    // describe relationships between two components and therefore
    // have no natural single-component owner; allowing them here
    // would let one component's overrides silently mutate edges
    // anchored elsewhere in the tree. Reject before the merge ever
    // sees the file so the error message names the offending file
    // directly.
    if !file.edges_add.is_empty() {
        return Err(TreeAssemblyError::EdgesOverridesAtPerComponentScope {
            file: file_path.display().to_string(),
            kind: "edges_add",
        });
    }
    if !file.edges_suppress.is_empty() {
        return Err(TreeAssemblyError::EdgesOverridesAtPerComponentScope {
            file: file_path.display().to_string(),
            kind: "edges_suppress",
        });
    }

    let mut all_ids: Vec<&ComponentId> = Vec::new();
    for id in file.pins.keys() {
        all_ids.push(id);
    }
    for addition in &file.additions {
        all_ids.push(&addition.id);
    }
    for (key, pins) in &file.pins {
        if let Some(PinValue::SuppressChildren { suppress_children }) =
            pins.get("suppress_children")
        {
            // The pin key already counts. The suppress_children list
            // names children — also under the scoping prefix.
            let _ = key;
            for child in suppress_children {
                all_ids.push(child);
            }
        }
    }

    for id in all_ids {
        if !id_in_scope(id.as_str(), prefixes) {
            return Err(TreeAssemblyError::PerComponentScopeViolation {
                file: file_path.display().to_string(),
                offending_id: id.as_str().to_string(),
                owner_prefix: prefixes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "<root>".to_string()),
            });
        }
    }
    Ok(())
}

fn id_in_scope(id: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        // No prefix means the file is at the workspace root with no
        // relative path (degenerate case the discovery walk
        // shouldn't produce, but a defensive accept is preferable to
        // a false-positive scope violation that the user can't fix).
        return true;
    }
    prefixes.iter().any(|p| {
        id == p.as_str()
            || (id.len() > p.len() && id.starts_with(p) && id.as_bytes()[p.len()] == b'/')
    })
}

fn read_overrides_file(path: &Path) -> Result<OverridesFile, TreeAssemblyError> {
    let bytes = std::fs::read(path).map_err(|e| TreeAssemblyError::PerComponentParseError {
        file: path.display().to_string(),
        message: format!("read failed: {e}"),
    })?;
    let parsed: OverridesFile =
        serde_yaml::from_slice(&bytes).map_err(|e| TreeAssemblyError::PerComponentParseError {
            file: path.display().to_string(),
            message: e.to_string(),
        })?;
    Ok(parsed)
}

/// Materialise the merged override set (last-writer-wins) and emit a
/// warning for every conflict on the same `(component_id, key)`
/// tuple.
fn merge_discovered_overrides(
    discovered: &[DiscoveredOverride],
    warnings: &mut dyn Write,
) -> MergedOverrides {
    // Pin merge: track the *winning* source for each (id, key) tuple
    // and the chain of prior sources for warning emission.
    type PinKey = (ComponentId, String);
    let mut pin_winner: BTreeMap<PinKey, (PinValue, &DiscoveredOverride)> = BTreeMap::new();
    let mut pin_history: BTreeMap<PinKey, Vec<(&DiscoveredOverride, PinValue)>> = BTreeMap::new();

    // Additions merge: id-keyed, last-writer-wins. Suppressions go
    // through the existing `Suppress` pin form, which is handled by
    // the pin merge above; an `additions` block is a structural
    // create.
    let mut addition_winner: BTreeMap<ComponentId, (ComponentEntry, &DiscoveredOverride)> =
        BTreeMap::new();
    let mut addition_history: BTreeMap<ComponentId, Vec<(&DiscoveredOverride, ComponentEntry)>> =
        BTreeMap::new();

    for d in discovered {
        for (id, pins) in &d.file.pins {
            for (key, value) in pins {
                let pk: PinKey = (id.clone(), key.clone());
                pin_history
                    .entry(pk.clone())
                    .or_default()
                    .push((d, value.clone()));
                pin_winner.insert(pk, (value.clone(), d));
            }
        }
        for addition in &d.file.additions {
            addition_history
                .entry(addition.id.clone())
                .or_default()
                .push((d, addition.clone()));
            addition_winner.insert(addition.id.clone(), (addition.clone(), d));
        }
    }

    // Emit warnings: any (id, key) tuple with more than one
    // *contributing* source emits a warning naming each contributor.
    for ((id, key), history) in &pin_history {
        // Only emit when a real conflict exists — i.e. two
        // contributors with different values. Two contributors with
        // identical values is a redundant declaration, not a
        // conflict.
        let distinct_values: BTreeSet<String> = history
            .iter()
            .map(|(_, v)| serde_yaml::to_string(v).unwrap_or_default())
            .collect();
        if history.len() < 2 || distinct_values.len() < 2 {
            continue;
        }
        let winner = pin_winner.get(&(id.clone(), key.clone())).expect("winner");
        let _ = writeln!(
            warnings,
            "warning: override conflict on ({}, {}):",
            id.as_str(),
            key
        );
        for (d, v) in history {
            let _ = writeln!(
                warnings,
                "  {:<13} {}: {}",
                d.source.label(),
                d.display_path,
                pin_value_short_form(v)
            );
        }
        let _ = writeln!(
            warnings,
            "  resolved value: {}  ({} wins by discovery order)",
            pin_value_short_form(&winner.0),
            winner.1.source.label()
        );
    }

    // Build the merged file.
    let mut merged = OverridesFile::default();
    for ((id, key), (value, _)) in pin_winner {
        merged.pins.entry(id).or_default().insert(key, value);
    }
    let mut additions: Vec<ComponentEntry> =
        addition_winner.into_values().map(|(e, _)| e).collect();
    additions.sort_by(|a, b| a.id.cmp(&b.id));
    merged.additions = additions;

    // PR-6: union `edges_add` / `edges_suppress` across every
    // discovered file. Discovery order (primary → peer → per-
    // component) controls insertion order; the engine's L6 stage
    // applies edges_add first then subtracts edges_suppress, so two
    // entries with the same `(kind, from, to)` triple — one in `add`
    // and one in `suppress` — semantically resolve to "edge dropped"
    // regardless of the per-file order.
    for d in discovered {
        merged.edges_add.extend(d.file.edges_add.iter().cloned());
        merged
            .edges_suppress
            .extend(d.file.edges_suppress.iter().cloned());
    }

    // PR-6: collect per-component field overrides keyed by the
    // owning directory. Top-level (primary/peer) files cannot
    // contribute field overrides because their YAML targets the
    // workspace as a whole, not a single component — those
    // contributions are silently ignored here. Per-component files
    // contribute one entry per file; if two per-component files
    // somehow point at the same dir (vendored peer + canonical
    // copy), the later file wins (path-sorted discovery order
    // determines "later").
    let mut per_component_field_overrides: BTreeMap<PathBuf, ComponentFieldOverrides> =
        BTreeMap::new();
    for d in discovered {
        if d.source != OverrideSource::PerComponent {
            continue;
        }
        if d.file.field_overrides.is_empty() {
            continue;
        }
        if let Some(dir) = &d.owning_dir {
            per_component_field_overrides.insert(dir.clone(), d.file.field_overrides.clone());
        }
    }

    MergedOverrides {
        file: merged,
        per_component_field_overrides,
    }
}

fn pin_value_short_form(v: &PinValue) -> String {
    match v {
        PinValue::Suppress { .. } => "true".to_string(),
        PinValue::SuppressChildren { suppress_children } => {
            let inner: Vec<String> = suppress_children
                .iter()
                .map(|c| c.as_str().to_string())
                .collect();
            format!("[{}]", inner.join(", "))
        }
        PinValue::Value { value, reason } => match reason {
            Some(r) => format!("{value} (reason: {r})"),
            None => value.clone(),
        },
    }
}

// ---------------------------------------------------------------------
// Phase 3: acyclicity.
// ---------------------------------------------------------------------

fn enforce_acyclicity(components: &[ComponentEntry]) -> Result<(), TreeAssemblyError> {
    let parent_by_id: HashMap<&ComponentId, Option<&ComponentId>> = components
        .iter()
        .map(|c| (&c.id, c.parent.as_ref()))
        .collect();

    for entry in components {
        let mut seen: HashSet<&ComponentId> = HashSet::new();
        let mut cursor: Option<&ComponentId> = Some(&entry.id);
        while let Some(id) = cursor {
            if !seen.insert(id) {
                return Err(TreeAssemblyError::Cycle {
                    id: entry.id.as_str().to_string(),
                });
            }
            cursor = match parent_by_id.get(id) {
                Some(Some(parent)) => Some(*parent),
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
            id: ComponentId::parse(id).unwrap(),
            parent: parent.map(|p| ComponentId::parse(p).unwrap()),
            kind: "spec".into(),
            lifecycle_roles: vec![],
            languages: std::collections::BTreeSet::new(),
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
            other => panic!("expected Cycle, got {other:?}"),
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

    // -----------------------------------------------------------------
    // PR-6 override-merge discovery tests (spec §3-§6).
    // -----------------------------------------------------------------

    use atlas_index::OVERRIDES_SCHEMA_VERSION;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn primary_overrides_with_pin(component: &str, key: &str, value: &str) -> OverridesFile {
        let mut pins: BTreeMap<ComponentId, BTreeMap<String, PinValue>> = BTreeMap::new();
        let mut inner: BTreeMap<String, PinValue> = BTreeMap::new();
        inner.insert(
            key.into(),
            PinValue::Value {
                value: value.into(),
                reason: None,
            },
        );
        pins.insert(ComponentId::parse(component).unwrap(), inner);
        OverridesFile {
            schema_version: OVERRIDES_SCHEMA_VERSION,
            pins,
            ..OverridesFile::default()
        }
    }

    fn write_overrides(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Two pins on the same `(component_id, key)` tuple — one in the
    /// primary-root top-level file, one in a per-component file —
    /// resolve to the per-component value (last writer wins).
    #[test]
    fn discovery_order_per_component_overrides_top_level() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let primary = primary_overrides_with_pin("billing-core", "kind", "rust-library");
        write_overrides(
            &root.join("billing-core/.atlas/overrides.yaml"),
            "schema_version: 1\npins:\n  billing-core:\n    kind:\n      value: docker-image\n",
        );

        let mut warnings: Vec<u8> = Vec::new();
        let merged = merge_overrides_in_discovery_order(
            &[root.to_path_buf()],
            root,
            &primary,
            &mut warnings,
        )
        .expect("merge succeeds");

        let pin = merged
            .file
            .pins
            .get(&ComponentId::parse("billing-core").unwrap())
            .and_then(|m| m.get("kind"))
            .expect("kind pin present");
        match pin {
            PinValue::Value { value, .. } => assert_eq!(value, "docker-image"),
            other => panic!("expected Value pin, got {other:?}"),
        }
    }

    /// A per-component override file at `<root>/<owner>/.atlas/overrides.yaml`
    /// declaring a pin for an unrelated component id is rejected with
    /// a clear error naming the file and the offending id.
    #[test]
    fn per_component_scope_violation_is_hard_error() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_overrides(
            &root.join("atlas-index/.atlas/overrides.yaml"),
            "schema_version: 1\npins:\n  component-ontology:\n    kind:\n      value: rust-library\n",
        );

        let primary = OverridesFile::default();
        let mut warnings: Vec<u8> = Vec::new();
        let err = merge_overrides_in_discovery_order(
            &[root.to_path_buf()],
            root,
            &primary,
            &mut warnings,
        )
        .expect_err("scope violation must be rejected");
        match err {
            TreeAssemblyError::PerComponentScopeViolation {
                file, offending_id, ..
            } => {
                assert!(
                    file.contains("atlas-index/.atlas/overrides.yaml"),
                    "file message must name the offending file: {file}"
                );
                assert_eq!(offending_id, "component-ontology");
            }
            other => panic!("expected PerComponentScopeViolation, got {other:?}"),
        }
    }

    /// Design §5.5: `edges_add` is top-level-only. A per-component
    /// `overrides.yaml` declaring an `edges_add` entry must be
    /// rejected with `EdgesOverridesAtPerComponentScope` before the
    /// merge consumes it, regardless of whether the participants
    /// fall inside the file's scoping prefix. The error message must
    /// name both the offending file and the offending kind so the
    /// user knows which entry to move.
    #[test]
    fn per_component_edges_add_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_overrides(
            &root.join("billing-core/.atlas/overrides.yaml"),
            "schema_version: 1\nedges_add:\n  - kind: depends-on\n    from: billing-core\n    to: billing-core/api\n    reason: manual\n",
        );

        let primary = OverridesFile::default();
        let mut warnings: Vec<u8> = Vec::new();
        let err = merge_overrides_in_discovery_order(
            &[root.to_path_buf()],
            root,
            &primary,
            &mut warnings,
        )
        .expect_err("edges_add at per-component scope must be rejected");
        match err {
            TreeAssemblyError::EdgesOverridesAtPerComponentScope { file, kind } => {
                assert!(
                    file.contains("billing-core/.atlas/overrides.yaml"),
                    "error must name the offending file: {file}"
                );
                assert_eq!(kind, "edges_add");
            }
            other => panic!("expected EdgesOverridesAtPerComponentScope, got {other:?}"),
        }
    }

    /// Companion to `per_component_edges_add_is_rejected`: the
    /// `edges_suppress` block is rejected with the same error variant
    /// when authored at per-component scope.
    #[test]
    fn per_component_edges_suppress_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_overrides(
            &root.join("billing-core/.atlas/overrides.yaml"),
            "schema_version: 1\nedges_suppress:\n  - kind: depends-on\n    from: billing-core\n    to: billing-core/api\n    reason: bogus\n",
        );

        let primary = OverridesFile::default();
        let mut warnings: Vec<u8> = Vec::new();
        let err = merge_overrides_in_discovery_order(
            &[root.to_path_buf()],
            root,
            &primary,
            &mut warnings,
        )
        .expect_err("edges_suppress at per-component scope must be rejected");
        match err {
            TreeAssemblyError::EdgesOverridesAtPerComponentScope { file, kind } => {
                assert!(
                    file.contains("billing-core/.atlas/overrides.yaml"),
                    "error must name the offending file: {file}"
                );
                assert_eq!(kind, "edges_suppress");
            }
            other => panic!("expected EdgesOverridesAtPerComponentScope, got {other:?}"),
        }
    }

    /// Fix 3 regression guard: a per-component `field_overrides`
    /// block with a `lifecycle:` value that does not parse as a
    /// known `LifecycleScope` emits a warning to the warnings
    /// channel and leaves the analyser-emitted lifecycle in place.
    /// The unparseable case is a soft failure (typo or future-
    /// scope), not a panic, but it must surface to the user.
    #[test]
    fn unparseable_lifecycle_emits_warning_and_skips_override() {
        use atlas_index::ComponentFieldOverrides;
        use std::path::PathBuf;

        // Build a minimal `ComponentEntry` that already has a
        // populated lifecycle, then run `apply_per_component_field_overrides`
        // with a typo'd lifecycle. The warning must mention the
        // offending value AND the directory; the entry's
        // `lifecycle_roles` must be untouched.
        let dir = PathBuf::from("/tmp/atlas-test/billing-core");
        let mut entries = vec![ComponentEntry {
            id: ComponentId::parse("billing-core").unwrap(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            languages: BTreeSet::new(),
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("billing-core"),
                content_sha: String::new(),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: String::new(),
            deleted: false,
        }];

        let mut by_dir: BTreeMap<PathBuf, ComponentFieldOverrides> = BTreeMap::new();
        by_dir.insert(
            dir.clone(),
            ComponentFieldOverrides {
                language: None,
                kind: None,
                lifecycle: Some("definitely-not-a-real-scope".into()),
                subsystem: None,
            },
        );
        let roots = vec![PathBuf::from("/tmp/atlas-test")];

        let mut warnings: Vec<u8> = Vec::new();
        apply_per_component_field_overrides(&mut entries, &by_dir, &roots, &mut warnings);

        // The analyser-emitted lifecycle is preserved.
        assert_eq!(
            entries[0].lifecycle_roles,
            vec![LifecycleScope::Runtime],
            "unparseable lifecycle override must not clobber the analyser value"
        );

        let text = String::from_utf8(warnings).unwrap();
        assert!(
            text.contains("definitely-not-a-real-scope"),
            "warning must echo the offending value: {text}"
        );
        assert!(
            text.contains("billing-core"),
            "warning must reference the owning directory: {text}"
        );
        assert!(
            text.contains("override not applied"),
            "warning must state the consequence: {text}"
        );
    }

    /// Two peer-root top-level files with conflicting pins on the
    /// same `(component_id, key)` tuple resolve to the value of the
    /// peer whose canonical path sorts later (spec §3 step 2: peers
    /// are merged in lex order, last writer wins).
    #[test]
    fn peer_roots_resolve_in_lex_order() {
        let tmp = TempDir::new().unwrap();
        let peer_alpha = tmp.path().join("alpha-root");
        let peer_zulu = tmp.path().join("zulu-root");
        write_overrides(
            &peer_alpha.join(".atlas/components.overrides.yaml"),
            "schema_version: 1\npins:\n  shared-id:\n    kind:\n      value: from-alpha\n",
        );
        write_overrides(
            &peer_zulu.join(".atlas/components.overrides.yaml"),
            "schema_version: 1\npins:\n  shared-id:\n    kind:\n      value: from-zulu\n",
        );
        let primary_root = tmp.path().join("primary-root");
        std::fs::create_dir_all(&primary_root).unwrap();

        let primary = OverridesFile::default();
        let roots = vec![primary_root.clone(), peer_alpha.clone(), peer_zulu.clone()];
        let mut warnings: Vec<u8> = Vec::new();
        let merged =
            merge_overrides_in_discovery_order(&roots, &primary_root, &primary, &mut warnings)
                .expect("merge succeeds");

        let pin = merged
            .file
            .pins
            .get(&ComponentId::parse("shared-id").unwrap())
            .and_then(|m| m.get("kind"))
            .expect("kind pin present");
        match pin {
            PinValue::Value { value, .. } => assert_eq!(value, "from-zulu"),
            other => panic!("expected Value pin, got {other:?}"),
        }
        let warning_text = String::from_utf8(warnings).unwrap();
        assert!(
            warning_text.contains("override conflict on (shared-id, kind)"),
            "expected conflict warning, got: {warning_text}"
        );
    }

    /// A primary/per-component conflict on the same `(component_id,
    /// key)` tuple emits a warning to stderr but does NOT fail the
    /// run (spec §6: warning-only behaviour for Phase 1).
    #[test]
    fn primary_per_component_conflict_emits_warning_but_does_not_fail() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let primary = primary_overrides_with_pin("billing-core", "kind", "rust-library");
        write_overrides(
            &root.join("billing-core/.atlas/overrides.yaml"),
            "schema_version: 1\npins:\n  billing-core:\n    kind:\n      value: docker-image\n",
        );

        let mut warnings: Vec<u8> = Vec::new();
        let result = merge_overrides_in_discovery_order(
            &[root.to_path_buf()],
            root,
            &primary,
            &mut warnings,
        );
        assert!(
            result.is_ok(),
            "warning-only conflict must not fail the run; got {result:?}"
        );
        let text = String::from_utf8(warnings).unwrap();
        assert!(
            text.contains("override conflict on (billing-core, kind)"),
            "expected conflict warning header in: {text}"
        );
        assert!(
            text.contains("primary"),
            "warning must label the primary source: {text}"
        );
        assert!(
            text.contains("per-component"),
            "warning must label the per-component source: {text}"
        );
        assert!(
            text.contains("resolved value: docker-image"),
            "warning must announce the resolved value: {text}"
        );
        assert!(
            text.contains("per-component wins by discovery order"),
            "warning must explain the resolution: {text}"
        );
    }

    /// The implied owner-id derivation under a peer root with a
    /// non-trivial basename produces both the path-only form and the
    /// root-prefixed form. The scoping check accepts entries that
    /// match either.
    #[test]
    fn derive_scoping_prefixes_includes_root_namespace() {
        let tmp = TempDir::new().unwrap();
        let peer = tmp.path().join("atlas-contracts");
        std::fs::create_dir_all(peer.join("crates/atlas-index/.atlas")).unwrap();
        let file = peer.join("crates/atlas-index/.atlas/overrides.yaml");
        let prefixes = derive_scoping_prefixes(&file, std::slice::from_ref(&peer));
        assert!(
            prefixes.contains(&"crates/atlas-index".to_string()),
            "expected path-only form, got {prefixes:?}"
        );
        assert!(
            prefixes.contains(&"atlas-contracts/crates/atlas-index".to_string()),
            "expected root-prefixed form, got {prefixes:?}"
        );

        // Both forms accept exact-match.
        assert!(id_in_scope("crates/atlas-index", &prefixes));
        assert!(id_in_scope("atlas-contracts/crates/atlas-index", &prefixes));
        // Both accept descendant.
        assert!(id_in_scope("crates/atlas-index/sub", &prefixes));
        assert!(id_in_scope(
            "atlas-contracts/crates/atlas-index/sub",
            &prefixes
        ));
        // Neither accepts a sibling.
        assert!(!id_in_scope("crates/component-ontology", &prefixes));
        assert!(!id_in_scope(
            "atlas-contracts/component-ontology",
            &prefixes
        ));
    }

    #[test]
    fn id_in_scope_is_strict_about_prefix_boundary() {
        let prefixes = vec!["billing".to_string()];
        // Exact match.
        assert!(id_in_scope("billing", &prefixes));
        // Slash boundary.
        assert!(id_in_scope("billing/core", &prefixes));
        // No accidental prefix-as-substring matches.
        assert!(!id_in_scope("billing-core", &prefixes));
        assert!(!id_in_scope("billings", &prefixes));
    }
}
