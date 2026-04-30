//! Integration tests for L4 tree assembly, rename-match integration,
//! override additions/suppression, and the acyclicity invariant.
//!
//! Each test builds a self-contained temp fixture so L0 inputs are
//! fully controlled. The `TestBackend` has no canned responses — any
//! accidental LLM dispatch fails the test loudly, matching §4.1's
//! deterministic-short-circuit invariant for fixtures built from
//! recognisable manifests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_engine::{
    all_components, component_children, component_parent, component_path_segments, seed_filesystem,
    try_assemble, AtlasDatabase,
};
use atlas_index::{
    AlwaysTrue, ComponentEntry, ComponentsFile, OverridesFile, PathSegment, PinValue,
    COMPONENTS_SCHEMA_VERSION, OVERRIDES_SCHEMA_VERSION,
};
use atlas_llm::{LlmFingerprint, TestBackend};
use component_ontology::EvidenceGrade;
use tempfile::TempDir;

fn default_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn db_without_llm(root: &Path) -> AtlasDatabase {
    let mut db = AtlasDatabase::new(
        Arc::new(TestBackend::new()),
        root.to_path_buf(),
        default_fingerprint(),
    );
    seed_filesystem(&mut db, root, false).expect("seed_filesystem must succeed");
    db
}

fn cargo_lib(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n")
}

fn cargo_bin(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n"
    )
}

fn cargo_workspace(members: &[&str]) -> String {
    let members_str = members
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[workspace]\nresolver = \"2\"\nmembers = [{members_str}]\n")
}

// ---------------------------------------------------------------------
// Core tree assembly
// ---------------------------------------------------------------------

#[test]
fn single_component_project_produces_one_entry_tree() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), cargo_lib("solo")).unwrap();

    let db = db_without_llm(&root);
    let tree = all_components(&db);
    assert_eq!(tree.len(), 1, "expected one entry, got {tree:#?}");
    let entry = &tree[0];
    assert_eq!(entry.kind, "rust-library");
    assert!(entry.parent.is_none());
    assert!(!entry.deleted);
    assert_eq!(entry.path_segments.len(), 1);
    assert!(
        entry.path_segments[0].content_sha.len() == 64,
        "content_sha should be 64 hex chars, got {:?}",
        entry.path_segments[0].content_sha
    );
}

#[test]
fn workspace_with_members_produces_parent_and_children() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        cargo_workspace(&["crates/alpha", "crates/beta"]),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::write(root.join("crates/alpha/Cargo.toml"), cargo_lib("alpha")).unwrap();
    std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a(){}").unwrap();
    std::fs::create_dir_all(root.join("crates/beta/src")).unwrap();
    std::fs::write(root.join("crates/beta/Cargo.toml"), cargo_bin("beta")).unwrap();
    std::fs::write(root.join("crates/beta/src/main.rs"), "fn main(){}").unwrap();

    let db = db_without_llm(&root);
    let tree = all_components(&db);

    // One workspace + two members.
    assert_eq!(tree.len(), 3, "{tree:#?}");

    let workspace = tree
        .iter()
        .find(|c| c.kind == "workspace")
        .expect("workspace present");
    assert!(workspace.parent.is_none());

    let alpha = tree
        .iter()
        .find(|c| c.kind == "rust-library")
        .expect("alpha present");
    assert_eq!(alpha.parent.as_deref(), Some(workspace.id.as_str()));

    let beta = tree
        .iter()
        .find(|c| c.kind == "rust-cli")
        .expect("beta present");
    assert_eq!(beta.parent.as_deref(), Some(workspace.id.as_str()));
}

#[test]
fn component_queries_read_from_assembled_tree() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), cargo_workspace(&["crates/alpha"])).unwrap();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::write(root.join("crates/alpha/Cargo.toml"), cargo_lib("alpha")).unwrap();
    std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a(){}").unwrap();

    let db = db_without_llm(&root);
    let tree = all_components(&db);
    let workspace_id = tree
        .iter()
        .find(|c| c.kind == "workspace")
        .map(|c| c.id.clone())
        .unwrap();
    let alpha_id = tree
        .iter()
        .find(|c| c.kind == "rust-library")
        .map(|c| c.id.clone())
        .unwrap();

    assert_eq!(component_parent(&db, &alpha_id), Some(workspace_id.clone()));
    assert_eq!(component_parent(&db, &workspace_id), None);
    let children = component_children(&db, &workspace_id);
    assert_eq!(children.as_ref(), &vec![alpha_id.clone()]);
    let segments = component_path_segments(&db, &alpha_id);
    assert_eq!(segments.len(), 1);
    assert!(!segments[0].content_sha.is_empty());
}

// ---------------------------------------------------------------------
// Identifier stability across reassembly
// ---------------------------------------------------------------------

#[test]
fn identifier_stable_when_file_content_changes_but_directory_unchanged() {
    // A component keeps its id across runs when its dir path is the
    // same — no rename-match needed, the primary id allocator returns
    // the same slug.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), cargo_lib("stable")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn a(){}").unwrap();

    let db = db_without_llm(&root);
    let first = all_components(&db);
    let first_id = first[0].id.clone();

    // Modify a file; rebuild a fresh DB because the production seeding
    // path re-reads the filesystem.
    std::fs::write(root.join("src/lib.rs"), "pub fn b(){}").unwrap();
    let db2 = db_without_llm(&root);
    let second = all_components(&db2);
    assert_eq!(second[0].id, first_id);
}

#[test]
fn identifier_preserved_across_directory_rename_via_rename_match() {
    // Simulate: last run produced a component at `src/old-name` with
    // id "old-name". This run's filesystem has the same content at a
    // new path. Rename-match pairs them by content-SHA and the new
    // entry inherits the prior id.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("crates/new-name/src")).unwrap();
    std::fs::write(
        root.join("crates/new-name/Cargo.toml"),
        cargo_lib("new-name"),
    )
    .unwrap();
    std::fs::write(root.join("crates/new-name/src/lib.rs"), "pub fn a(){}").unwrap();

    // First: compute the content_sha the tree assembly would naturally
    // produce for this directory, by running L4 once.
    let db_probe = db_without_llm(&root);
    let probe = all_components(&db_probe);
    let current_sha = probe[0].path_segments[0].content_sha.clone();

    // Now re-run, seeded with a prior ComponentsFile that records the
    // same content_sha at the old path under the id "my-original-id".
    let mut db = db_without_llm(&root);
    let prior = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: root.clone(),
        generated_at: "prior".into(),
        cache_fingerprints: Default::default(),
        components: vec![ComponentEntry {
            id: "my-original-id".into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("crates/old-name"),
                content_sha: current_sha,
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: "prior".into(),
            deleted: false,
        }],
    };
    db.set_prior_components(prior);

    let tree = all_components(&db);
    let live: Vec<&ComponentEntry> = tree.iter().filter(|c| !c.deleted).collect();
    assert_eq!(live.len(), 1, "expected one live component");
    assert_eq!(
        live[0].id, "my-original-id",
        "rename-match should carry the prior id forward"
    );
}

#[test]
fn orphan_prior_component_emitted_as_deleted_tombstone_once() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), cargo_lib("live")).unwrap();

    let mut db = db_without_llm(&root);
    let prior = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: root.clone(),
        generated_at: "prior".into(),
        cache_fingerprints: Default::default(),
        components: vec![ComponentEntry {
            id: "gone".into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("gone"),
                content_sha: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .into(),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: "prior".into(),
            deleted: false,
        }],
    };
    db.set_prior_components(prior.clone());

    let tree = all_components(&db);
    let tomb = tree
        .iter()
        .find(|c| c.id == "gone")
        .expect("tombstone should be present");
    assert!(tomb.deleted, "orphan prior entry should be marked deleted");

    // Second run: prior is this run's output (which contains the
    // tombstone). The tombstone should NOT be re-emitted.
    let tree2_prior = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: root.clone(),
        generated_at: "prior2".into(),
        cache_fingerprints: Default::default(),
        components: tree.as_ref().clone(),
    };
    let mut db2 = db_without_llm(&root);
    db2.set_prior_components(tree2_prior);
    let tree2 = all_components(&db2);
    assert!(
        tree2.iter().all(|c| c.id != "gone"),
        "tombstone should be dropped on the following run; got {tree2:#?}"
    );
}

// ---------------------------------------------------------------------
// Overrides: additions and suppression
// ---------------------------------------------------------------------

#[test]
fn overrides_addition_appears_in_tree_even_without_signals() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("specs/api")).unwrap();
    std::fs::write(root.join("specs/api/.keep"), "").unwrap();

    let mut db = db_without_llm(&root);
    let overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins: BTreeMap::new(),
        additions: vec![ComponentEntry {
            id: "api-spec".into(),
            parent: None,
            kind: "spec".into(),
            lifecycle_roles: vec![],
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("specs/api"),
                content_sha: "0".into(),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: "hand-authored".into(),
            deleted: false,
        }],
    };
    db.set_components_overrides(overrides);

    let tree = all_components(&db);
    assert!(
        tree.iter().any(|c| c.id == "api-spec"),
        "addition should be in tree: {tree:#?}"
    );
}

#[test]
fn suppress_pin_removes_component_from_tree() {
    // A suppressed component's is_boundary is false at L3, so it never
    // reaches the tree. This test documents the L3-L4 contract.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), cargo_workspace(&["crates/alpha"])).unwrap();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::write(root.join("crates/alpha/Cargo.toml"), cargo_lib("alpha")).unwrap();
    std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a(){}").unwrap();

    let mut field_pins = BTreeMap::new();
    field_pins.insert(
        "suppress".to_string(),
        PinValue::Suppress {
            suppress: AlwaysTrue,
        },
    );
    let mut pins = BTreeMap::new();
    pins.insert("crates/alpha".to_string(), field_pins);
    let overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins,
        additions: vec![],
    };

    let mut db = db_without_llm(&root);
    db.set_components_overrides(overrides);

    let tree = all_components(&db);
    assert!(
        !tree.iter().any(|c| c.kind == "rust-library"),
        "suppressed rust-library should not appear: {tree:#?}"
    );
    // Workspace still does.
    assert!(tree.iter().any(|c| c.kind == "workspace"));
}

#[test]
fn suppress_children_pin_removes_specific_child_ids() {
    // Workspace has two members; a suppress_children pin on the
    // workspace id names one of the allocated child ids, which L4
    // strips from the output.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(
        root.join("Cargo.toml"),
        cargo_workspace(&["crates/alpha", "crates/beta"]),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::write(root.join("crates/alpha/Cargo.toml"), cargo_lib("alpha")).unwrap();
    std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a(){}").unwrap();
    std::fs::create_dir_all(root.join("crates/beta/src")).unwrap();
    std::fs::write(root.join("crates/beta/Cargo.toml"), cargo_lib("beta")).unwrap();
    std::fs::write(root.join("crates/beta/src/lib.rs"), "pub fn b(){}").unwrap();

    // First: run without pins to discover what ids get allocated.
    let db_probe = db_without_llm(&root);
    let probe = all_components(&db_probe);
    let workspace_id = probe
        .iter()
        .find(|c| c.kind == "workspace")
        .unwrap()
        .id
        .clone();
    let beta_id = probe
        .iter()
        .find(|c| c.kind == "rust-library" && c.path_segments[0].path.ends_with("beta"))
        .unwrap()
        .id
        .clone();

    // Now install a suppress_children pin on the workspace id, listing
    // beta's id.
    let mut field_pins = BTreeMap::new();
    field_pins.insert(
        "suppress_children".to_string(),
        PinValue::SuppressChildren {
            suppress_children: vec![beta_id.clone()],
        },
    );
    let mut pins = BTreeMap::new();
    pins.insert(workspace_id.clone(), field_pins);
    let overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins,
        additions: vec![],
    };
    let mut db = db_without_llm(&root);
    db.set_components_overrides(overrides);

    let tree = all_components(&db);
    assert!(
        !tree.iter().any(|c| c.id == beta_id),
        "suppress_children should drop beta: {tree:#?}"
    );
    assert!(
        tree.iter().any(|c| c.id == workspace_id),
        "workspace itself remains"
    );
    // alpha stays.
    assert!(tree
        .iter()
        .any(|c| c.kind == "rust-library" && c.path_segments[0].path.ends_with("alpha")));
}

// ---------------------------------------------------------------------
// Acyclicity invariant
// ---------------------------------------------------------------------

#[test]
fn assembled_tree_round_trips_through_components_file_yaml() {
    // Exit criterion: the vector L4 produces must round-trip through
    // atlas-index's ComponentsFile save/load without loss.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("Cargo.toml"), cargo_workspace(&["crates/alpha"])).unwrap();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::write(root.join("crates/alpha/Cargo.toml"), cargo_lib("alpha")).unwrap();
    std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a(){}").unwrap();

    let db = db_without_llm(&root);
    let tree = all_components(&db);

    let file = ComponentsFile {
        schema_version: COMPONENTS_SCHEMA_VERSION,
        root: root.clone(),
        generated_at: "test".into(),
        cache_fingerprints: Default::default(),
        components: tree.as_ref().clone(),
    };
    let yaml = serde_yaml::to_string(&file).expect("serialise");
    let parsed: ComponentsFile = serde_yaml::from_str(&yaml).expect("deserialise");
    assert_eq!(parsed, file);
}

#[test]
fn cycle_in_additions_triggers_hard_error_not_infinite_loop() {
    // Two additions that each declare the other as their parent —
    // L4 surfaces the cycle via `try_assemble`'s Err arm rather than
    // looping forever or silently shipping a nonsensical tree.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a/.keep"), "").unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(root.join("b/.keep"), "").unwrap();

    let comp_a = ComponentEntry {
        id: "comp-a".into(),
        parent: Some("comp-b".into()),
        kind: "spec".into(),
        lifecycle_roles: vec![],
        language: None,
        build_system: None,
        role: None,
        path_segments: vec![PathSegment {
            path: PathBuf::from("a"),
            content_sha: "0".into(),
        }],
        manifests: vec![],
        doc_anchors: vec![],
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec![],
        rationale: "a".into(),
        deleted: false,
    };
    let comp_b = ComponentEntry {
        id: "comp-b".into(),
        parent: Some("comp-a".into()),
        path_segments: vec![PathSegment {
            path: PathBuf::from("b"),
            content_sha: "0".into(),
        }],
        ..comp_a.clone()
    };
    let overrides = OverridesFile {
        schema_version: OVERRIDES_SCHEMA_VERSION,
        pins: BTreeMap::new(),
        additions: vec![comp_a, comp_b],
    };

    let mut db = db_without_llm(&root);
    db.set_components_overrides(overrides);

    let res = try_assemble(&db);
    assert!(
        res.is_err(),
        "cyclic additions must surface a TreeAssemblyError, got {res:?}"
    );
}
