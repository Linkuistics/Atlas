//! The hand-authored dev-workspace golden must be loadable through
//! atlas-index without schema errors. If this test fails, the golden
//! has drifted against the generated schema and needs a touch-up.

use std::path::PathBuf;

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../goldens/dev-workspace")
}

#[test]
fn dev_workspace_components_golden_loads() {
    let path = goldens_dir().join("components.golden.yaml");
    let file = atlas_index::load_components(&path)
        .expect("dev-workspace components.golden.yaml must parse");
    assert!(!file.components.is_empty(), "golden must not be empty");
    for component in &file.components {
        assert!(
            !component.path_segments.is_empty(),
            "component `{}` lacks path_segments",
            component.id
        );
        assert!(
            !component.rationale.trim().is_empty(),
            "component `{}` has no rationale",
            component.id
        );
    }
}

#[test]
fn dev_workspace_externals_golden_loads() {
    let path = goldens_dir().join("external-components.golden.yaml");
    let _ = atlas_index::load_externals(&path)
        .expect("dev-workspace external-components.golden.yaml must parse");
}

#[test]
fn dev_workspace_related_golden_loads_and_validates() {
    let path = goldens_dir().join("related-components.golden.yaml");
    let file = component_ontology::load(&path)
        .expect("dev-workspace related-components.golden.yaml must parse");
    for edge in &file.edges {
        edge.validate()
            .unwrap_or_else(|e| panic!("edge {:?} failed validation: {e:#}", edge.participants));
    }
}
