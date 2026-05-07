//! PR-9 integration tests: deterministic composition edges
//! (`bundled-into`, `deployed-with`) derived from Dockerfile `COPY`
//! directives.
//!
//! Each test builds a self-contained temp fixture. The `TestBackend`
//! is wired but no Stage 2 responses are canned — composition edges
//! must flow from the deterministic Dockerfile path even when the
//! LLM is silent. The accept-no-canned-responses default fails on
//! any unexpected LLM call (matching the §4.1 deterministic
//! short-circuit).

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{
    all_components, all_proposed_edges, related_components_yaml_snapshot, seed_filesystem,
    AtlasDatabase,
};
use atlas_llm::{LlmFingerprint, TestBackend};
use component_ontology::{EdgeKind, LifecycleScope};
use tempfile::TempDir;

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [9u8; 32],
        ontology_sha: [9u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn db_no_llm(root: &Path) -> AtlasDatabase {
    let mut db = AtlasDatabase::new(
        Arc::new(TestBackend::new()),
        vec![root.to_path_buf()],
        fingerprint(),
    );
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem succeeds");
    db
}

fn write_cargo_bin(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main(){}\n").unwrap();
}

fn write_cargo_lib(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\nname = \"{name}\"\npath = \"src/lib.rs\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), "pub fn x(){}\n").unwrap();
}

fn write_dockerfile(root: &Path, dir_under_root: &str, body: &str) {
    let dir = root.join(dir_under_root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Dockerfile"), body).unwrap();
}

/// Locate a single component whose id ends with the given suffix
/// (after the last `/`). Useful for assertions that don't care about
/// the root-prefix variant of the id.
fn find_component_id_by_leaf(db: &AtlasDatabase, leaf: &str) -> String {
    let components = all_components(db);
    let matches: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted)
        .filter(|c| {
            let id = c.id.as_str();
            match id.rfind('/') {
                Some(idx) => &id[idx + 1..] == leaf,
                None => id == leaf,
            }
        })
        .map(|c| c.id.as_str().to_string())
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one component with leaf `{leaf}`, got {matches:?}\n\
         all components: {:?}",
        components
            .iter()
            .map(|c| (c.id.as_str(), c.kind.as_str()))
            .collect::<Vec<_>>()
    );
    matches.into_iter().next().unwrap()
}

// ---------------------------------------------------------------------
// Acceptance: bundled-into edge from a single COPY of a build-output
// binary into a Dockerfile.
// ---------------------------------------------------------------------

#[test]
fn dockerfile_copy_of_build_output_emits_bundled_into_edge() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Source crate that produces the binary.
    write_cargo_bin(root, "billing-core");
    // Dockerfile under deploy/billing — gets classified as
    // docker-image by the Dockerfile classifier.
    write_dockerfile(
        root,
        "deploy/billing",
        "FROM debian:bookworm-slim\nCOPY target/release/billing-core /usr/local/bin/\n",
    );

    let db = db_no_llm(root);

    let billing_core_id = find_component_id_by_leaf(&db, "billing-core");
    // The docker-image component's leaf is whatever L4 slugifies the
    // dir basename to — for `deploy/billing` that's `billing`. The
    // brief's "billing-image" naming is the spec's worked example;
    // the L4 id is path-derived from the dir basename. Assert by
    // resolving via `kind: docker-image`.
    let docker_id = {
        let components = all_components(&db);
        let candidates: Vec<String> = components
            .iter()
            .filter(|c| !c.deleted && c.kind == "docker-image")
            .map(|c| c.id.as_str().to_string())
            .collect();
        assert_eq!(
            candidates.len(),
            1,
            "expected exactly one docker-image component, got {candidates:?}"
        );
        candidates.into_iter().next().unwrap()
    };

    let edges = all_proposed_edges(&db);
    let bundled: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        1,
        "expected exactly one bundled-into edge, got {edges:?}"
    );
    let edge = bundled[0];
    assert_eq!(edge.lifecycle, LifecycleScope::Deploy);
    assert_eq!(
        edge.participants,
        vec![billing_core_id.clone(), docker_id.clone()],
        "bundled-into participants must be [source, image]"
    );
    assert!(
        edge.evidence_fields
            .iter()
            .any(|f| f.starts_with("Dockerfile:COPY:target/release/billing-core")),
        "expected COPY:target/release/billing-core evidence, got {:?}",
        edge.evidence_fields
    );
}

// ---------------------------------------------------------------------
// Acceptance: deployed-with edge between two binaries bundled into the
// same image. Symmetric, lex-sorted participants.
// ---------------------------------------------------------------------

#[test]
fn dockerfile_bundling_two_binaries_emits_deployed_with_edge() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_cargo_bin(root, "billing-core");
    write_cargo_bin(root, "billing-admin");
    write_dockerfile(
        root,
        "deploy/billing",
        "FROM debian:bookworm-slim\n\
         COPY target/release/billing-core /usr/local/bin/\n\
         COPY target/release/billing-admin /usr/local/bin/\n",
    );

    let db = db_no_llm(root);

    let billing_core_id = find_component_id_by_leaf(&db, "billing-core");
    let billing_admin_id = find_component_id_by_leaf(&db, "billing-admin");
    let mut sorted_pair = vec![billing_core_id.clone(), billing_admin_id.clone()];
    sorted_pair.sort();

    let edges = all_proposed_edges(&db);

    // Two bundled-into edges (one per binary).
    let bundled: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        2,
        "expected two bundled-into edges, got {bundled:?}"
    );

    // Exactly one deployed-with edge between the two source crates.
    let deployed_with: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DeployedWith)
        .collect();
    assert_eq!(
        deployed_with.len(),
        1,
        "expected exactly one deployed-with edge, got {deployed_with:?}"
    );
    let edge = deployed_with[0];
    assert_eq!(edge.lifecycle, LifecycleScope::Runtime);
    assert_eq!(
        edge.participants, sorted_pair,
        "deployed-with participants must be lex-sorted"
    );
    assert!(
        edge.evidence_fields
            .iter()
            .any(|f| f == "Dockerfile:bundles-both"),
        "expected `Dockerfile:bundles-both` evidence, got {:?}",
        edge.evidence_fields
    );
}

// ---------------------------------------------------------------------
// Negative: a fixture without Dockerfiles emits no composition edges.
// The LLM batch is also empty (the TestBackend has no canned
// responses; an unmatched call would error out). Here we assert the
// composition-edge path itself is silent.
// ---------------------------------------------------------------------

#[test]
fn fixture_without_dockerfiles_emits_no_composition_edges() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_cargo_lib(root, "alpha");
    write_cargo_lib(root, "beta");

    let db = db_no_llm(root);

    let edges = all_proposed_edges(&db);
    let composition: Vec<_> = edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::BundledInto | EdgeKind::DeployedWith))
        .collect();
    assert!(
        composition.is_empty(),
        "no Dockerfiles → no composition edges, got {composition:?}"
    );
}

// ---------------------------------------------------------------------
// Defensive: a Dockerfile whose COPY is `--from=<stage>` (intra-image
// stage copy) does NOT create a `bundled-into` edge against the host
// repo. Only repo-source copies are eligible.
// ---------------------------------------------------------------------

#[test]
fn copy_from_stage_directives_are_skipped() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_cargo_bin(root, "atlas-cli");
    // Multi-stage Dockerfile: the *second* COPY is `--from=builder`,
    // referring to the `builder` stage's filesystem (NOT a host path).
    // Only the first COPY (a host-relative path) should produce an
    // edge.
    write_dockerfile(
        root,
        "deploy/atlas",
        "FROM rust:1.79 AS builder\n\
         COPY target/release/atlas-cli /usr/local/bin/atlas-cli\n\
         FROM debian:bookworm-slim\n\
         COPY --from=builder /usr/local/bin/atlas-cli /usr/local/bin/atlas-cli\n",
    );

    let db = db_no_llm(root);

    let cli_id = find_component_id_by_leaf(&db, "atlas-cli");
    let edges = all_proposed_edges(&db);
    let bundled: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        1,
        "expected exactly one bundled-into edge (the --from=builder COPY is skipped), got {bundled:?}"
    );
    assert!(
        bundled[0].participants.contains(&cli_id),
        "expected atlas-cli as a participant, got {:?}",
        bundled[0].participants
    );
}

// ---------------------------------------------------------------------
// Lex order: `related_components_yaml_snapshot` sorts edges by
// (kind, lifecycle, participants).
// ---------------------------------------------------------------------

#[test]
fn related_components_snapshot_orders_edges_lexicographically() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_cargo_bin(root, "billing-core");
    write_cargo_bin(root, "billing-admin");
    write_dockerfile(
        root,
        "deploy/billing",
        "FROM debian:bookworm-slim\n\
         COPY target/release/billing-core /usr/local/bin/\n\
         COPY target/release/billing-admin /usr/local/bin/\n",
    );

    let db = db_no_llm(root);
    let file = related_components_yaml_snapshot(&db);

    // Edges' canonical-key strings should be in lex order.
    let keys: Vec<(String, String, Vec<String>)> = file
        .edges
        .iter()
        .map(|e| {
            let k = e.canonical_key();
            (k.0.as_str().to_string(), k.1.as_str().to_string(), k.2)
        })
        .collect();
    let sorted = {
        let mut copy = keys.clone();
        copy.sort();
        copy
    };
    assert_eq!(
        keys, sorted,
        "edges must be emitted in lex order on (kind, lifecycle, participants)"
    );

    // Sanity: bundled-into precedes deployed-with alphabetically
    // (`bundled-into` < `deployed-with`).
    let kinds: Vec<&str> = file.edges.iter().map(|e| e.kind.as_str()).collect();
    let bundled_idx = kinds.iter().position(|k| *k == "bundled-into");
    let deployed_idx = kinds.iter().position(|k| *k == "deployed-with");
    if let (Some(b), Some(d)) = (bundled_idx, deployed_idx) {
        assert!(
            b < d,
            "bundled-into must precede deployed-with in lex order: kinds={:?}",
            kinds
        );
    }
}
