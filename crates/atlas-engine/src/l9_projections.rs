//! L9 projections — the three generated YAMLs Atlas writes to disk:
//! `components.yaml`, `external-components.yaml`, `related-components.yaml`.
//!
//! Each projection is a plain function taking `&AtlasDatabase`, matching
//! the existing `all_components` / `all_proposed_edges` pattern. The
//! tree-level Salsa queries they call are memoised; callers that need
//! to hold the result across revisions wrap it in `Arc` themselves.
//!
//! `external_components_yaml_snapshot` reads a separate L1.5 query,
//! [`externals_from_manifests`], which is `#[salsa::tracked]` because
//! its inputs are `(Workspace, dir)` primitives and it does no LLM
//! work — the same contract as the L1 queries in `l1_queries.rs`.
//!
//! The `generated_at` timestamp is intentionally deterministic — the
//! empty string — so YAML output is byte-stable across runs for tests
//! that demand "zero LLM calls → byte-identical outputs". The CLI
//! stamps the real clock value just before writing; the Salsa query
//! result stays stable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use atlas_index::BindingRole;
use atlas_index::{
    CacheFingerprints, ComponentsFile, ExternalEntry, ExternalsFile, ImplementedContract,
    PerComponentFile, RelatedComponentsFile, SurfacesFile, COMPONENTS_SCHEMA_VERSION,
    EXTERNALS_SCHEMA_VERSION, PER_COMPONENT_SCHEMA_VERSION, SURFACES_SCHEMA_VERSION,
};
use component_ontology::{ComponentId, EvidenceGrade, SCHEMA_VERSION as RELATED_SCHEMA_VERSION};
use sha2::{Digest, Sha256};

use crate::contract_canonicalisation::compute_surfaces_fingerprint;
use crate::db::{AtlasDatabase, Workspace};
use crate::l1_queries::manifests_in;
use crate::l4_tree::all_components;
use crate::l5_surface::surface_artefacts_of;
use crate::l6_edges::all_proposed_edges;
use crate::roots::best_root_for;

/// Build the `components.yaml` projection from the live engine state.
/// The `roots` are taken from the workspace input so a caller that
/// seeded the database with absolute paths sees the same absolute
/// paths here.
///
/// `generated_at` is left empty; the CLI stamps the wall clock just
/// before writing. Leaving it empty in the projection keeps the Salsa
/// return value stable across re-runs that changed nothing.
pub fn components_yaml_snapshot(db: &AtlasDatabase) -> Arc<ComponentsFile> {
    let workspace = db.workspace();
    let roots = workspace.roots(db as &dyn salsa::Database).clone();
    let fingerprint = workspace
        .llm_fingerprint(db as &dyn salsa::Database)
        .clone();
    // PR-5: include the analyser-registry sha. The value is the
    // canonical sha256 of the merged AnalyzersFile (see
    // `atlas_analyzers::AnalyzerRegistry::registry_sha`); recording
    // it here makes the fingerprint diffable in the on-disk
    // `components.yaml` and gives downstream tooling a single field
    // to compare across runs.
    let cache_fingerprints = CacheFingerprints {
        ontology_sha: hex_encode(&fingerprint.ontology_sha),
        prompt_shas: BTreeMap::new(),
        model_id: fingerprint.model_id.clone(),
        backend_version: fingerprint.backend_version.clone(),
        analyzer_registry_sha: db.analyzer_registry().registry_sha(),
    };

    let components = all_components(db);
    Arc::new(ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        roots,
        generated_at: String::new(),
        cache_fingerprints,
        components: (*components).clone(),
    })
}

/// Like [`components_yaml_snapshot`] but lets the caller supply the
/// per-prompt SHA map the CLI computes from rendered templates. Kept as
/// a separate entry point so L9 itself does not depend on the prompt
/// rendering path.
pub fn components_yaml_snapshot_with_prompt_shas(
    db: &AtlasDatabase,
    prompt_shas: BTreeMap<String, String>,
) -> Arc<ComponentsFile> {
    let mut file = (*components_yaml_snapshot(db)).clone();
    file.cache_fingerprints.prompt_shas = prompt_shas;
    Arc::new(file)
}

/// Build a per-component `<component-path>/.atlas/component.yaml`
/// projection for a single component. The output carries the
/// component's `ComponentEntry` (identical to its slot in the
/// top-level `components.yaml`) plus an envelope of analyser
/// identity, fingerprint, and pointers to the co-located
/// `surfaces.yaml` / `overrides.yaml` files.
///
/// **Envelope fields (PR-7 wiring):**
///
/// - `fingerprint` is the surfaces fingerprint computed by
///   [`surfaces_yaml_snapshot`] (which uses the schema-derived
///   canonicaliser per spec §2.2 over the component's `SurfacesFile`).
///   Per the design §6.2 invariant, this is the value other
///   components' L6 cache keys cite. PR-11 wires the participant-
///   surface contribution in L6.
/// - `analyser_id` / `analyser_version`: PR-6 used the static
///   `"l3-driver"` / [`L3_DRIVER_VERSION`] placeholders. PR-7's
///   plan note allows keeping these placeholders if per-analyser
///   identity cannot be plumbed without disproportionate refactoring
///   of `l3_classify.rs`. We keep the placeholders here because the
///   L3 dispatcher returns a single `Classification` not labelled
///   with the originating analyser; threading that through the L3
///   adapter would require either tagging the dispatch outcome
///   (changes to the analyser crate's API) or a per-analyser shim
///   in the engine. Both approaches are larger than the PR's scope;
///   the placeholder is preserved with a `DONE_WITH_CONCERNS`
///   notation in the PR-7 final report.
/// - `surfaces_path` and `overrides_path` are the relative
///   filenames `surfaces.yaml` / `overrides.yaml` — pointers within
///   the component's own `.atlas/` directory.
///
/// Errors when `component_id` does not resolve to any component in
/// the live tree.
pub fn per_component_yaml_snapshot(
    db: &AtlasDatabase,
    component_id: &ComponentId,
) -> anyhow::Result<Arc<PerComponentFile>> {
    let components = components_yaml_snapshot(db);
    let entry = components
        .components
        .iter()
        .find(|c| &c.id == component_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "component id `{}` not found in the live component tree",
                component_id.as_str()
            )
        })?
        .clone();

    // PR-7: replace the entry-yaml-sha placeholder with the real
    // surfaces fingerprint (design §6.2). Other components' L6 cache
    // keys cite this value; PR-11 makes that wiring load-bearing.
    let surfaces = surfaces_yaml_snapshot(db, component_id)?;
    let fingerprint = surfaces.fingerprint.clone();

    Ok(Arc::new(PerComponentFile {
        schema_version: PER_COMPONENT_SCHEMA_VERSION,
        component: entry,
        surfaces_path: PathBuf::from("surfaces.yaml"),
        overrides_path: Some(PathBuf::from("overrides.yaml")),
        analyser_id: "l3-driver".to_string(),
        analyser_version: L3_DRIVER_VERSION.to_string(),
        fingerprint,
    }))
}

/// Build the per-component `<component-path>/.atlas/surfaces.yaml`
/// projection. The output is an [`atlas_index::SurfacesFile`] (PR-1)
/// populated from the deterministic Rust-surface analyser
/// (`atlas-analyzers::extract_rust_surface`, PR-7) over the
/// component's `src/lib.rs` and `src/main.rs`.
///
/// Schema:
///
/// - `schema_version: 1` (Phase 1; Phase 2 bumps with the AST
///   canonicaliser).
/// - `component_id`: the component's id.
/// - `fingerprint`: the schema-derived sha of the canonical
///   serialisation of the file with `fingerprint` zeroed
///   ([`compute_surfaces_fingerprint`]; spec §2.2 algorithm).
/// - `contracts_defined`: code-derived `data-format` contracts the
///   analyser found.
/// - `contracts_implemented`: one entry per `contracts_defined` —
///   the defining component is also the implementing component for
///   its own contracts (`role: defining-binding`).
/// - `contracts_consumed`: empty in PR-7. PR-8 derives consume edges
///   from the LLM Stage 2 prompt; PR-9 derives them from cross-
///   component code references. Phase 1 ships this field as an
///   always-empty placeholder so the YAML shape is complete.
/// - `library_apis`: at most one Rust library API.
///
/// Errors when the component id does not resolve.
pub fn surfaces_yaml_snapshot(
    db: &AtlasDatabase,
    component_id: &ComponentId,
) -> anyhow::Result<Arc<SurfacesFile>> {
    let components = all_components(db);
    if !components
        .iter()
        .any(|c| &c.id == component_id && !c.deleted)
    {
        return Err(anyhow::anyhow!(
            "component id `{}` not found in the live component tree",
            component_id.as_str()
        ));
    }

    let artefacts = surface_artefacts_of(db, component_id.clone());
    let contracts_defined = artefacts.contracts.clone();
    // For each defined contract the same component is also its
    // defining-binding implementer (design §6.3 worked example).
    let contracts_implemented: Vec<ImplementedContract> = contracts_defined
        .iter()
        .map(|c| ImplementedContract {
            contract_id: c.id.clone(),
            role: BindingRole::DefiningBinding,
            binding: c.definition_binding.clone(),
        })
        .collect();
    let library_apis = artefacts.library_apis.clone();

    let mut file = SurfacesFile {
        schema_version: SURFACES_SCHEMA_VERSION,
        component_id: component_id.clone(),
        fingerprint: String::new(),
        contracts_defined,
        contracts_implemented,
        contracts_consumed: Vec::new(),
        library_apis,
    };
    // Validate every library_api before serialising (PR-1 status
    // note). If validation fails (it shouldn't — the analyser
    // constructs them with `kind: LibraryApi`), surface the error to
    // the caller rather than emitting a corrupt file.
    for api in &file.library_apis {
        api.validate().map_err(|e| {
            anyhow::anyhow!(
                "library_api `{}` failed validation before surfaces.yaml emission: {e}",
                api.id
            )
        })?;
    }
    file.fingerprint = compute_surfaces_fingerprint(&file);
    Ok(Arc::new(file))
}

/// Stable analyser version string recorded in per-component records
/// produced before per-analyser identity is plumbed through L3
/// (PR-7). The string is independent of `crate::version()` so a Cargo
/// publish bump does not invalidate every PR-6 era per-component
/// fingerprint.
pub const L3_DRIVER_VERSION: &str = "1.0.0";

/// Build the `external-components.yaml` projection by walking every
/// manifest under the workspace root and lifting out external package
/// references.
pub fn external_components_yaml_snapshot(db: &AtlasDatabase) -> Arc<ExternalsFile> {
    let workspace = db.workspace();
    let roots = workspace.roots(db as &dyn salsa::Database).clone();
    // Per-root externals are unioned by id so a crate that appears in
    // two roots is reported once with both manifests in
    // `discovered_from`. Sources accumulate in a `BTreeSet` keyed by
    // path string to avoid the O(n²) `.any` scan per push; the final
    // sorted Vec is built at output time. The set's lex ordering is
    // deterministic, so the resulting YAML is byte-stable.
    let mut by_id: BTreeMap<String, ExternalEntry> = BTreeMap::new();
    let mut sources_by_id: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for root in &roots {
        let per_root =
            externals_from_manifests(db as &dyn salsa::Database, workspace, root.clone());
        for entry in per_root.iter() {
            by_id
                .entry(entry.id.clone())
                .or_insert_with(|| entry.clone());
            let bucket = sources_by_id.entry(entry.id.clone()).or_default();
            for source in &entry.discovered_from {
                bucket.insert(source.clone());
            }
        }
    }
    let mut externals: Vec<ExternalEntry> = by_id
        .into_iter()
        .map(|(id, mut entry)| {
            entry.discovered_from = sources_by_id
                .remove(&id)
                .map(|set| set.into_iter().collect())
                .unwrap_or_default();
            entry
        })
        .collect();
    externals.sort_by(|a, b| a.id.cmp(&b.id));
    Arc::new(ExternalsFile {
        schema_version: EXTERNALS_SCHEMA_VERSION,
        externals,
    })
}

/// Build the `related-components.yaml` projection from the L6 batch.
/// Edges are already canonicalised by L6; dedup is re-applied here
/// against `canonical_key` so any caller-side manipulation that adds
/// duplicates gets collapsed, and every surviving edge is re-validated.
pub fn related_components_yaml_snapshot(db: &AtlasDatabase) -> Arc<RelatedComponentsFile> {
    let edges = all_proposed_edges(db);
    let mut file = RelatedComponentsFile {
        schema_version: RELATED_SCHEMA_VERSION,
        edges: Vec::new(),
    };
    for edge in edges.iter() {
        let _ = file.add_edge(edge.clone());
    }
    Arc::new(file)
}

/// Walk every manifest under `dir` and collect the external packages
/// they reference. Currently supports `Cargo.toml` dependencies and
/// `package.json` dependencies; unrecognised manifest shapes contribute
/// nothing rather than erroring — an unparseable Cargo.toml is already
/// degraded to "no facts" by [`crate::manifest_parse`], and externals
/// discovery should not be stricter than classification.
#[salsa::tracked]
pub fn externals_from_manifests<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<ExternalEntry>> {
    let manifests = manifests_in(db, workspace, dir.clone());
    let mut by_id: BTreeMap<String, ExternalEntry> = BTreeMap::new();
    // Multi-root: relativise manifest paths against the root they live
    // under (the longest matching prefix among `workspace.roots()`),
    // falling back to the query's `dir` if no root matches.
    let roots = workspace.roots(db);

    for path in manifests.iter() {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(file_handle) = workspace
            .files(db)
            .iter()
            .find(|f| f.path(db) == path)
            .copied()
        else {
            continue;
        };
        let bytes = file_handle.bytes(db);
        let Ok(text) = std::str::from_utf8(bytes.as_slice()) else {
            continue;
        };

        let owning_root = best_root_for(roots, path).unwrap_or(dir.as_path());
        let rel = path_relative(path, owning_root);
        let rel_str = rel.to_string_lossy().into_owned();

        match name {
            "Cargo.toml" => collect_cargo_externals(text, &rel_str, &mut by_id),
            "package.json" => collect_npm_externals(text, &rel_str, &mut by_id),
            _ => {}
        }
    }

    let mut externals: Vec<ExternalEntry> = by_id.into_values().collect();
    for entry in &mut externals {
        entry.discovered_from.sort();
        entry.discovered_from.dedup();
    }
    externals.sort_by(|a, b| a.id.cmp(&b.id));
    Arc::new(externals)
}

fn collect_cargo_externals(
    contents: &str,
    manifest_rel: &str,
    by_id: &mut BTreeMap<String, ExternalEntry>,
) {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return;
    };

    let tables = [
        table.get("dependencies"),
        table.get("dev-dependencies"),
        table.get("build-dependencies"),
    ];
    for block in tables.into_iter().flatten() {
        let Some(deps) = block.as_table() else {
            continue;
        };
        for (name, spec) in deps {
            if is_path_dependency(spec) {
                continue;
            }
            register_external(
                by_id,
                format!("crate:{name}"),
                "external",
                Some("rust"),
                purl_for_cargo(name, spec),
                manifest_rel,
            );
        }
    }
}

fn is_path_dependency(spec: &toml::Value) -> bool {
    match spec {
        toml::Value::Table(t) => t.contains_key("path"),
        _ => false,
    }
}

fn purl_for_cargo(name: &str, spec: &toml::Value) -> Option<String> {
    let version = match spec {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    };
    version.map(|v| format!("pkg:cargo/{name}@{v}"))
}

fn collect_npm_externals(
    contents: &str,
    manifest_rel: &str,
    by_id: &mut BTreeMap<String, ExternalEntry>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(contents) else {
        return;
    };
    let Some(obj) = value.as_object() else {
        return;
    };
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        let Some(deps) = obj.get(key).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, spec) in deps {
            let version = spec.as_str();
            register_external(
                by_id,
                format!("npm:{name}"),
                "external",
                Some("javascript"),
                version.map(|v| format!("pkg:npm/{name}@{v}")),
                manifest_rel,
            );
        }
    }
}

fn register_external(
    by_id: &mut BTreeMap<String, ExternalEntry>,
    id: String,
    kind: &str,
    language: Option<&str>,
    purl: Option<String>,
    manifest_rel: &str,
) {
    let entry = by_id.entry(id.clone()).or_insert_with(|| ExternalEntry {
        id,
        kind: kind.to_string(),
        language: language.map(String::from),
        purl,
        homepage: None,
        url: None,
        discovered_from: Vec::new(),
        evidence_grade: EvidenceGrade::Strong,
    });
    if !entry.discovered_from.iter().any(|d| d == manifest_rel) {
        entry.discovered_from.push(manifest_rel.to_string());
    }
}

fn path_relative(path: &std::path::Path, root: &std::path::Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// SHA-256 of the rendered prompt template body, returned as a
/// lowercase hex string suitable for
/// [`CacheFingerprints::prompt_shas`]. The CLI calls this once per
/// prompt id after rendering the template with the ontology-derived
/// tokens; the result lands in `components.yaml` so a subsequent run
/// can detect prompt drift without re-rendering.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Stable list of the four Atlas prompt ids paired with their id
/// strings, for drivers that need to iterate every prompt when
/// computing prompt SHAs.
pub const PROMPT_ID_STRINGS: &[&str] = &["classify", "subcarve", "stage1-surface", "stage2-edges"];

/// Subset of the component-id set that the CLI considers "present in
/// output files" — the union of live internal components and the
/// externals collected from manifests. Used by the evaluation harness
/// (future task) to check edge-participant existence, but exposed here
/// because it's a natural L9 projection.
pub fn known_component_ids(db: &AtlasDatabase) -> Arc<BTreeSet<String>> {
    // Union of internal component ids and external `crate:serde`-style
    // ids. Externals share the participant namespace with components in
    // edges but are not valid `ComponentId`s, so the union is on the
    // string form.
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for c in all_components(db).iter() {
        if !c.deleted {
            ids.insert(c.id.as_str().to_string());
        }
    }
    for e in external_components_yaml_snapshot(db).externals.iter() {
        ids.insert(e.id.clone());
    }
    Arc::new(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::seed_filesystem;
    use atlas_llm::{LlmFingerprint, TestBackend};
    use std::path::Path;
    use tempfile::TempDir;

    fn fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [3u8; 32],
            ontology_sha: [4u8; 32],
            model_id: "test-backend".into(),
            backend_version: "v0".into(),
        }
    }

    fn db_no_llm(root: &Path) -> AtlasDatabase {
        let mut db = AtlasDatabase::new(
            Arc::new(TestBackend::new()),
            vec![root.to_path_buf()],
            fingerprint(),
        );
        seed_filesystem(&mut db, &[root.to_path_buf()], false).unwrap();
        db
    }

    fn write_cargo_lib(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nserde = \"1\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
    }

    #[test]
    fn components_snapshot_round_trips_with_fingerprint_data() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "alpha");
        let db = db_no_llm(tmp.path());

        let file = components_yaml_snapshot(&db);
        assert_eq!(file.schema_version, COMPONENTS_SCHEMA_VERSION);
        assert_eq!(file.roots, vec![tmp.path().to_path_buf()]);
        assert!(file.cache_fingerprints.ontology_sha.len() == 64);
        assert_eq!(file.cache_fingerprints.model_id, "test-backend");
        assert!(
            file.components.iter().any(|c| c.kind == "rust-library"),
            "expected alpha to be classified as rust-library, got {:?}",
            file.components
        );
    }

    #[test]
    fn components_snapshot_prompt_shas_injected_when_supplied() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "beta");
        let db = db_no_llm(tmp.path());

        let mut shas: BTreeMap<String, String> = BTreeMap::new();
        shas.insert("classify".into(), "abc".into());
        let file = components_yaml_snapshot_with_prompt_shas(&db, shas);
        assert_eq!(
            file.cache_fingerprints.prompt_shas.get("classify").unwrap(),
            "abc"
        );
    }

    #[test]
    fn externals_snapshot_lifts_cargo_deps() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "gamma");
        let db = db_no_llm(tmp.path());

        let externals = external_components_yaml_snapshot(&db);
        assert_eq!(externals.schema_version, EXTERNALS_SCHEMA_VERSION);
        let ids: Vec<&str> = externals.externals.iter().map(|e| e.id.as_str()).collect();
        assert!(
            ids.contains(&"crate:serde"),
            "expected crate:serde in {ids:?}"
        );
        let serde = externals
            .externals
            .iter()
            .find(|e| e.id == "crate:serde")
            .unwrap();
        assert_eq!(serde.language.as_deref(), Some("rust"));
        assert_eq!(serde.purl.as_deref(), Some("pkg:cargo/serde@1"));
    }

    #[test]
    fn externals_snapshot_skips_path_dependencies() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("delta");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"delta\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[dependencies]\nsibling = { path = \"../sibling\" }\nserde = \"1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
        let db = db_no_llm(tmp.path());

        let externals = external_components_yaml_snapshot(&db);
        let ids: Vec<&str> = externals.externals.iter().map(|e| e.id.as_str()).collect();
        assert!(
            !ids.contains(&"crate:sibling"),
            "path dependency must not appear as external: got {ids:?}"
        );
        assert!(ids.contains(&"crate:serde"));
    }

    #[test]
    fn externals_snapshot_handles_npm_dependencies() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("epsilon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"epsilon","main":"index.js","dependencies":{"lodash":"^4.17.0"}}"#,
        )
        .unwrap();
        let db = db_no_llm(tmp.path());

        let externals = external_components_yaml_snapshot(&db);
        let lodash = externals
            .externals
            .iter()
            .find(|e| e.id == "npm:lodash")
            .expect("lodash external not found");
        assert_eq!(lodash.language.as_deref(), Some("javascript"));
        assert_eq!(lodash.purl.as_deref(), Some("pkg:npm/lodash@^4.17.0"));
    }

    #[test]
    fn externals_deduplicated_across_multiple_manifests() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "crate-a");
        write_cargo_lib(tmp.path(), "crate-b");
        let db = db_no_llm(tmp.path());

        let externals = external_components_yaml_snapshot(&db);
        let serde = externals
            .externals
            .iter()
            .find(|e| e.id == "crate:serde")
            .unwrap();
        assert_eq!(
            serde.discovered_from.len(),
            2,
            "crate:serde must list both manifests, got {:?}",
            serde.discovered_from
        );
    }

    #[test]
    fn related_components_snapshot_defaults_empty_when_single_component() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "solo");
        let db = db_no_llm(tmp.path());

        let file = related_components_yaml_snapshot(&db);
        assert_eq!(file.schema_version, RELATED_SCHEMA_VERSION);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn known_component_ids_is_union_of_internal_and_external() {
        let tmp = TempDir::new().unwrap();
        write_cargo_lib(tmp.path(), "zeta");
        let db = db_no_llm(tmp.path());

        let ids = known_component_ids(&db);
        assert!(ids.contains("crate:serde"));
        // One of the internal ids should also be present; don't pin
        // the exact id because slug allocation depends on directory
        // basename rules.
        assert!(
            ids.len() >= 2,
            "expected at least one internal + one external, got {ids:?}"
        );
    }

    #[test]
    fn sha256_hex_is_deterministic_and_64_chars() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }
}
