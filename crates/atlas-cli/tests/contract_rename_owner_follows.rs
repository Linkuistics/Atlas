//! Phase 6 PR-2: contract rename-match owner-follows.
//!
//! When a component is renamed (path moves; rename-match maps
//! `prior_id A -> new_id B`), contracts owned by `A` follow to `B`:
//! their id-prefix rewrites from `A/...` to `B/...`, and edges in
//! related-components.yaml have their participants updated.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use atlas_cli::progress::{make_stderr_reporter, ProgressMode};
use atlas_cli::{run_index, IndexConfig};
use atlas_engine::testing::LenientBackend;
use atlas_index::{ComponentsFile, SurfacesFile};
use atlas_llm::LlmFingerprint;
use component_ontology::{EdgeKind, RelatedComponentsFile};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [42u8; 32],
        ontology_sha: [43u8; 32],
        model_id: "phase6-pr2-test-backend".into(),
        backend_version: "v-phase6-pr2".into(),
    }
}

fn base_config(root: &Path) -> IndexConfig {
    let mut config = IndexConfig::new(root.to_path_buf());
    config.output_dir = root.join(".atlas");
    config.respect_gitignore = false;
    config.fingerprint_override = Some(fingerprint());
    config
}

fn run_atlas_index(root: &Path) {
    let config = base_config(root);
    let backend: Arc<dyn atlas_llm::LlmBackend> = LenientBackend::new(fingerprint());
    run_index(
        &config,
        backend,
        None,
        make_stderr_reporter(ProgressMode::Never, None),
    )
    .expect("run_index must succeed");
}

fn read_surfaces_yaml(path: &Path) -> SurfacesFile {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("expected surfaces.yaml at {}: {e}", path.display()));
    serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {} as SurfacesFile: {e}", path.display()))
}

fn read_related_components_yaml(path: &Path) -> RelatedComponentsFile {
    let bytes = fs::read(path).unwrap_or_else(|e| {
        panic!(
            "expected related-components.yaml at {}: {e}",
            path.display()
        )
    });
    serde_yaml::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "failed to parse {} as RelatedComponentsFile: {e}",
            path.display()
        )
    })
}

fn read_components_yaml(path: &Path) -> ComponentsFile {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("expected components.yaml at {}: {e}", path.display()));
    serde_yaml::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {} as ComponentsFile: {e}", path.display()))
}

#[test]
fn contract_owner_follows_component_rename() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Run 1: workspace has `original-name` containing contract `C1`.
    fs::create_dir_all(root.join("original-name/src")).unwrap();
    fs::write(
        root.join("original-name/Cargo.toml"),
        "[package]\n\
         name = \"original-name\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("original-name/src/lib.rs"),
        "#[derive(serde::Serialize, serde::Deserialize)]\n\
         pub struct C1 { pub x: i32 }\n",
    )
    .unwrap();

    run_atlas_index(root);

    let surfaces_pre = read_surfaces_yaml(&root.join("original-name/.atlas/cache/surfaces.yaml"));
    let contract_id_pre = surfaces_pre
        .contracts_defined
        .first()
        .expect("contract defined")
        .id
        .clone();
    assert!(
        contract_id_pre.starts_with("original-name/"),
        "expected owner-prefix; got {contract_id_pre}"
    );

    // Rename the component directory. Cargo.toml's `name` field stays
    // the same -- rename-match's signal is path-segment content overlap,
    // not the package name.
    fs::rename(root.join("original-name"), root.join("renamed-component")).unwrap();

    run_atlas_index(root);

    // Assertion 0: the renamed component is alive under
    // `renamed-component/` in this run's components.yaml. (The
    // `original-name` entry survives as a tombstone in the same file --
    // this is the regular tombstone behaviour, not the owner-follows
    // mechanism under test.)
    let components = read_components_yaml(&root.join(".atlas/cache/components.yaml"));
    let live_ids: Vec<String> = components
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str().to_string())
        .collect();
    assert!(
        live_ids.iter().any(|id| id == "renamed-component"),
        "expected `renamed-component` in live ids; got {live_ids:?}"
    );

    // Assertion 1: surfaces.yaml under renamed-component contains the
    // same contract but with the new owner-prefix.
    let surfaces_post =
        read_surfaces_yaml(&root.join("renamed-component/.atlas/cache/surfaces.yaml"));
    let contract_id_post = surfaces_post
        .contracts_defined
        .first()
        .expect("contract defined")
        .id
        .clone();
    assert!(
        contract_id_post.starts_with("renamed-component/"),
        "expected new owner-prefix; got {contract_id_post}; live ids: {live_ids:?}"
    );

    // Assertion 2: related-components.yaml's defines-contract edge has
    // the new participant id, and no edge references the stale
    // pre-rename contract id.
    let related = read_related_components_yaml(&root.join(".atlas/cache/related-components.yaml"));
    let defines: Vec<&component_ontology::Edge> = related
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DefinesContract)
        .collect();
    let participants: Vec<&str> = defines
        .iter()
        .flat_map(|e| e.participants.iter().map(String::as_str))
        .collect();
    assert!(
        participants.contains(&contract_id_post.as_str()),
        "expected defines-contract edge to reference new contract id; got participants={participants:?}"
    );
    assert!(
        !participants.contains(&contract_id_pre.as_str()),
        "expected no edge to reference stale pre-rename contract id"
    );
}
