//! PR-11 integration tests: deterministic composition edges
//! (`bundled-into`, `deployed-with`) derived from Docker Compose files.
//!
//! Each test builds a self-contained temp fixture. The `TestBackend` is
//! wired but no Stage 2 responses are canned — compose edges must flow
//! from the deterministic Compose path even when the LLM is silent.
//!
//! ## Acceptance criteria covered
//!
//! - AC-1: `docker-compose.yml` with `image:` services →
//!   `compose-orchestration` component, `bundled-into` from
//!   external-component (or local docker-image) to orchestration,
//!   `deployed-with` between service sources.
//! - AC-2: compose file with `build:` declarations resolves to local
//!   Dockerfile-derived components.
//! - AC-3: multiple compose files in one workspace → separate
//!   orchestration components, separate edge sets.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::{
    all_components, all_proposed_edges, composition_edges_from_compose, seed_filesystem,
    AtlasDatabase,
};
use atlas_llm::{LlmFingerprint, TestBackend};
use component_ontology::{EdgeKind, LifecycleScope};
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [11u8; 32],
        ontology_sha: [11u8; 32],
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

fn write_compose(root: &Path, dir_under_root: &str, filename: &str, body: &str) {
    let dir = root.join(dir_under_root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(filename), body).unwrap();
}

fn write_dockerfile_dir(root: &Path, dir_under_root: &str) {
    let dir = root.join(dir_under_root);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Dockerfile"), "FROM alpine:3.20\nCOPY . /app\n").unwrap();
}

/// Find the unique component whose kind matches and return its id.
fn find_component_by_kind(db: &AtlasDatabase, kind: &str) -> Vec<String> {
    let components = all_components(db);
    components
        .iter()
        .filter(|c| !c.deleted && c.kind == kind)
        .map(|c| c.id.as_str().to_string())
        .collect()
}

// ─── AC-1: image: services → compose-orchestration + edges ───────────────────

/// Minimal acceptance case: two services declared with `image:`, no local
/// docker-image components. Both image sources are external.
///
/// Expected:
/// - One `compose-orchestration` component.
/// - Two `bundled-into` edges (one per image source → orchestration).
/// - One `deployed-with` edge between the two external sources.
#[test]
fn compose_image_services_emit_orchestration_and_edges() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    write_compose(
        root,
        "deploy",
        "docker-compose.yml",
        r#"
version: "3"
services:
  web:
    image: "myrepo/web:1"
  db:
    image: "postgres:15"
"#,
    );

    let db = db_no_llm(root);

    // Verify one compose-orchestration component was created.
    let orchestrations = find_component_by_kind(&db, "compose-orchestration");
    assert_eq!(
        orchestrations.len(),
        1,
        "expected exactly one compose-orchestration component, got: {orchestrations:?}"
    );
    let orch_id = &orchestrations[0];

    // Verify edges.
    let edges = all_proposed_edges(&db);

    let bundled: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        2,
        "expected two bundled-into edges (web + db → orchestration), got: {bundled:?}"
    );
    // Both bundled-into edges target the orchestration component.
    for e in &bundled {
        assert_eq!(
            e.lifecycle,
            LifecycleScope::Deploy,
            "bundled-into lifecycle must be deploy"
        );
        assert_eq!(
            &e.participants[1], orch_id,
            "second participant must be the orchestration component"
        );
        // Source ids are external-component ids.
        assert!(
            e.participants[0].starts_with("external-"),
            "image-only services produce external-... source ids, got: {}",
            e.participants[0]
        );
    }

    // Exactly one deployed-with between the two external sources.
    let deployed: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DeployedWith)
        .collect();
    assert_eq!(
        deployed.len(),
        1,
        "expected one deployed-with edge, got: {deployed:?}"
    );
    let dw = deployed[0];
    assert_eq!(
        dw.lifecycle,
        LifecycleScope::Deploy,
        "deployed-with lifecycle must be deploy"
    );
    // Participants are lex-sorted (symmetric kind).
    let mut sorted = dw.participants.clone();
    sorted.sort();
    assert_eq!(
        dw.participants, sorted,
        "deployed-with participants must be lex-sorted"
    );
}

// ─── AC-2: build: → local Dockerfile-derived component ───────────────────────

/// A compose file whose services use `build:` instead of `image:`. The
/// build context is a sibling dir that contains a `Dockerfile`. Atlas
/// classifies that dir as `docker-image` and should emit a `bundled-into`
/// from that component to the compose-orchestration.
#[test]
fn compose_build_service_emits_bundled_into_from_local_dockerfile_component() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Service build contexts: two sibling dirs, each with a Dockerfile.
    write_dockerfile_dir(root, "services/web");
    write_dockerfile_dir(root, "services/worker");

    // Compose file that builds both services.
    write_compose(
        root,
        "deploy",
        "docker-compose.yml",
        r#"
services:
  web:
    build: ../services/web
  worker:
    build: ../services/worker
"#,
    );

    let db = db_no_llm(root);

    let orchestrations = find_component_by_kind(&db, "compose-orchestration");
    assert_eq!(
        orchestrations.len(),
        1,
        "expected exactly one compose-orchestration, got: {orchestrations:?}"
    );
    let orch_id = &orchestrations[0];

    // The build service dirs should be classified as docker-image
    // (each has a Dockerfile).
    let docker_images = find_component_by_kind(&db, "docker-image");
    assert_eq!(
        docker_images.len(),
        2,
        "expected two docker-image components (web + worker), got: {docker_images:?}"
    );

    let compose_edges = composition_edges_from_compose(&db);

    let bundled: Vec<_> = compose_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        2,
        "expected two bundled-into edges (web + worker → orchestration), got: {bundled:?}"
    );
    for e in &bundled {
        assert_eq!(
            &e.participants[1], orch_id,
            "bundled-into second participant must be orchestration"
        );
        // Sources must be local docker-image component ids.
        assert!(
            docker_images.contains(&e.participants[0]),
            "build: service source must be a local docker-image component, got: {}",
            e.participants[0]
        );
    }

    // deployed-with between the two build sources.
    let deployed: Vec<_> = compose_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DeployedWith)
        .collect();
    assert_eq!(
        deployed.len(),
        1,
        "expected one deployed-with edge, got: {deployed:?}"
    );
}

// ─── AC-3: multiple compose files → separate orchestrations ──────────────────

/// Two compose files in different sub-directories. Each should produce an
/// independent `compose-orchestration` component and its own edge set.
#[test]
fn multiple_compose_files_produce_separate_orchestrations() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Two independent compose files, each in its own subdir.
    write_compose(
        root,
        "stack-a",
        "docker-compose.yml",
        r#"
services:
  api:
    image: "myrepo/api:1"
  cache:
    image: "redis:7"
"#,
    );

    write_compose(
        root,
        "stack-b",
        "docker-compose.yml",
        r#"
services:
  worker:
    image: "myrepo/worker:1"
  queue:
    image: "rabbitmq:3"
"#,
    );

    let db = db_no_llm(root);

    let orchestrations = find_component_by_kind(&db, "compose-orchestration");
    assert_eq!(
        orchestrations.len(),
        2,
        "expected two separate compose-orchestration components (one per file), \
         got: {orchestrations:?}"
    );

    let edges = all_proposed_edges(&db);

    // Four bundled-into edges total (2 per orchestration × 2 files).
    let bundled: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        4,
        "expected four bundled-into edges (2 services × 2 compose files), got: {bundled:?}"
    );

    // Two deployed-with edges total (one pair per compose file).
    let deployed: Vec<_> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DeployedWith)
        .collect();
    assert_eq!(
        deployed.len(),
        2,
        "expected two deployed-with edges (one per compose file), got: {deployed:?}"
    );

    // Each orchestration component appears as a participant in exactly
    // two bundled-into edges.
    for orch_id in &orchestrations {
        let count = bundled
            .iter()
            .filter(|e| e.participants.contains(orch_id))
            .count();
        assert_eq!(
            count, 2,
            "each orchestration must appear in exactly two bundled-into edges; \
             `{orch_id}` appeared in {count}"
        );
    }
}

// ─── AC-4: no compose files → no compose edges ───────────────────────────────

#[test]
fn workspace_without_compose_files_emits_no_compose_edges() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    // Rust-only workspace with no compose files.
    let lib_dir = root.join("mylib");
    std::fs::create_dir_all(lib_dir.join("src")).unwrap();
    std::fs::write(
        lib_dir.join("Cargo.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\npath = \"src/lib.rs\"\n",
    )
    .unwrap();
    std::fs::write(lib_dir.join("src/lib.rs"), "pub fn x(){}\n").unwrap();

    let db = db_no_llm(root);

    let compose_edges = composition_edges_from_compose(&db);
    assert!(
        compose_edges.is_empty(),
        "no compose files → no compose edges, got: {compose_edges:?}"
    );
}

// ─── AC-5: compose file with only one service → no deployed-with ─────────────

#[test]
fn single_service_compose_emits_no_deployed_with() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    write_compose(
        root,
        "deploy",
        "docker-compose.yml",
        r#"
services:
  web:
    image: "myrepo/web:1"
"#,
    );

    let db = db_no_llm(root);

    let compose_edges = composition_edges_from_compose(&db);

    // One bundled-into (the single service → orchestration).
    let bundled: Vec<_> = compose_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::BundledInto)
        .collect();
    assert_eq!(
        bundled.len(),
        1,
        "expected exactly one bundled-into edge for single-service compose, \
         got: {bundled:?}"
    );

    // No deployed-with (need at least two services).
    let deployed: Vec<_> = compose_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DeployedWith)
        .collect();
    assert!(
        deployed.is_empty(),
        "single service → no deployed-with edges, got: {deployed:?}"
    );
}
