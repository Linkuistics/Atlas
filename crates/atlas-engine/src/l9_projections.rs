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

use atlas_index::{
    CacheFingerprints, ComponentsFile, ExternalEntry, ExternalsFile, RelatedComponentsFile,
    COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION,
};
use component_ontology::{EvidenceGrade, SCHEMA_VERSION as RELATED_SCHEMA_VERSION};
use sha2::{Digest, Sha256};

use crate::db::{AtlasDatabase, Workspace};
use crate::l1_queries::manifests_in;
use crate::l4_tree::all_components;
use crate::l6_edges::all_proposed_edges;

/// Build the `components.yaml` projection from the live engine state.
/// The `root` is taken from the workspace input so a caller that seeded
/// the database with an absolute path sees the same absolute path here.
///
/// `generated_at` is left empty; the CLI stamps the wall clock just
/// before writing. Leaving it empty in the projection keeps the Salsa
/// return value stable across re-runs that changed nothing.
pub fn components_yaml_snapshot(db: &AtlasDatabase) -> Arc<ComponentsFile> {
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let fingerprint = workspace
        .llm_fingerprint(db as &dyn salsa::Database)
        .clone();
    let cache_fingerprints = CacheFingerprints {
        ontology_sha: hex_encode(&fingerprint.ontology_sha),
        prompt_shas: BTreeMap::new(),
        model_id: fingerprint.model_id.clone(),
        backend_version: fingerprint.backend_version.clone(),
    };

    let components = all_components(db);
    Arc::new(ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root,
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

/// Build the `external-components.yaml` projection by walking every
/// manifest under the workspace root and lifting out external package
/// references.
pub fn external_components_yaml_snapshot(db: &AtlasDatabase) -> Arc<ExternalsFile> {
    let workspace = db.workspace();
    let root = workspace.root(db as &dyn salsa::Database).clone();
    let externals = externals_from_manifests(db as &dyn salsa::Database, workspace, root);
    Arc::new(ExternalsFile {
        schema_version: EXTERNALS_SCHEMA_VERSION,
        externals: (*externals).clone(),
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
    let manifests = manifests_in(db, workspace, dir);
    let mut by_id: BTreeMap<String, ExternalEntry> = BTreeMap::new();
    let root = workspace.root(db);

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

        let rel = path_relative(path, root);
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
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for c in all_components(db).iter() {
        if !c.deleted {
            ids.insert(c.id.clone());
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
            root.to_path_buf(),
            fingerprint(),
        );
        seed_filesystem(&mut db, root, false).unwrap();
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
        assert_eq!(file.root, tmp.path());
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
