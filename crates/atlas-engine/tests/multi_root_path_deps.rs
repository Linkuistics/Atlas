//! Path-dep root expansion to fixed point (PR-4 of Atlas vNext Phase 1).
//!
//! Five guarantees:
//!
//! 1. **Single-root, no escaping path-deps.** A workspace whose only
//!    Cargo crates are inside the primary root yields `vec![primary]`.
//! 2. **Two-root path-dep.** A primary-root crate with a path-dep
//!    pointing at a sibling tree adds the sibling's crate root as a
//!    peer.
//! 3. **Two-root with workspace at the target.** When the path-dep
//!    target is a workspace member, the walk-up locates the
//!    `[workspace]` manifest and the peer root is the workspace
//!    directory (not the crate directory).
//! 4. **Cycle.** A `crate-a → crate-b → crate-a` cycle terminates
//!    finitely; the function returns a finite `Vec` rather than
//!    looping. (Stderr capture is left to the binary; the test asserts
//!    on the absence of an infinite loop and on the result shape.)
//! 5. **Path-dep that doesn't exist.** A `path = "../missing-crate"`
//!    pointing at a non-existent directory is silently skipped — the
//!    fixed-point walk is intentionally lenient about uncloned
//!    sibling repos.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_engine::{
    all_components, compute_l6_batch_fingerprint, expand_roots, expand_roots_with_warnings,
    seed_filesystem, AtlasDatabase,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::json;
use tempfile::TempDir;

fn write_pyproject_package(dir: &Path, name: &str, path_deps: &[(&str, &str)]) {
    let mut manifest = format!("[tool.poetry]\nname = \"{name}\"\nversion = \"0.1.0\"\n");
    if !path_deps.is_empty() {
        manifest.push_str("\n[tool.poetry.dependencies]\npython = \"^3.11\"\n");
        for (dep_name, dep_path) in path_deps {
            manifest.push_str(&format!("{dep_name} = {{ path = \"{dep_path}\" }}\n"));
        }
    } else {
        manifest.push_str("\n[tool.poetry.dependencies]\npython = \"^3.11\"\n");
    }
    write(&dir.join("pyproject.toml"), &manifest);
    let pkg_dir = dir.join(name.replace('-', "_"));
    write(&pkg_dir.join("__init__.py"), "");
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A self-contained Cargo crate at `dir` with optional path-deps.
fn write_cargo_crate(dir: &Path, name: &str, path_deps: &[(&str, &str)]) {
    let mut manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n"
    );
    if !path_deps.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for (dep_name, dep_path) in path_deps {
            manifest.push_str(&format!("{dep_name} = {{ path = \"{dep_path}\" }}\n"));
        }
    }
    write(&dir.join("Cargo.toml"), &manifest);
    write(&dir.join("src/lib.rs"), "// lib\n");
}

#[test]
fn single_root_no_path_deps() {
    let primary = TempDir::new().unwrap();
    write_cargo_crate(&primary.path().join("crate-a"), "crate-a", &[]);
    write_cargo_crate(&primary.path().join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(primary.path()).expect("expand_roots succeeds");
    let expected = primary.path().canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![expected],
        "single-root fixture must return only the primary root"
    );
}

#[test]
fn two_root_path_dep() {
    // The primary root holds `crate-a` which path-deps `crate-b` in a
    // sibling tree. `crate-b` is a standalone crate (no enclosing
    // workspace); the walk-up returns the crate directory itself.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crate-b")],
    );
    write_cargo_crate(&external.join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let crate_b_canonical = external.join("crate-b").canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, crate_b_canonical],
        "primary then crate-b's directory (no enclosing workspace)"
    );
}

#[test]
fn two_root_with_workspace_at_target() {
    // The external tree is a Cargo workspace; the path-dep target
    // (crate-b) is a workspace member. The walk-up MUST return the
    // workspace's directory, not the member crate's directory.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crates/crate-b")],
    );
    write(
        &external.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/crate-b\"]\nresolver = \"2\"\n",
    );
    write_cargo_crate(&external.join("crates/crate-b"), "crate-b", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let workspace_canonical = external.canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, workspace_canonical],
        "the peer root is the [workspace] manifest's directory, not the member's"
    );
}

#[test]
fn cycle_terminates_with_warning() {
    // crate-a (primary) path-deps crate-b (external)
    // crate-b path-deps crate-c (back inside primary)
    //
    // The visited-set guard means the walk terminates after enumerating
    // both peer roots once (here just one peer, since crate-c is inside
    // the already-known primary). The test's load-bearing guarantee is
    // that the call returns at all — a cycle bug would loop forever.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external/crate-b")],
    );
    write_cargo_crate(
        &external.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../primary/crate-c")],
    );
    // crate-c lives inside the primary tree; the fixed-point walk
    // notices and skips re-adding the primary as a peer.
    write_cargo_crate(&primary.join("crate-c"), "crate-c", &[]);

    let roots = expand_roots(&primary).expect("expand_roots terminates");
    assert_eq!(
        roots.len(),
        2,
        "primary + external/crate-b only; crate-c stays inside the primary's coverage"
    );
    let primary_canonical = primary.canonicalize().unwrap();
    let external_b_canonical = external.join("crate-b").canonicalize().unwrap();
    assert_eq!(roots[0], primary_canonical);
    assert_eq!(roots[1], external_b_canonical);
}

#[test]
fn path_dep_to_missing_crate_is_skipped() {
    // A manifest may declare a `path = "../sibling"` whose target is
    // not yet checked out (worktree of a multi-repo monorepo, partial
    // git clone). PR-4 treats this as data, not error: the path-dep
    // is skipped and the rest of the walk proceeds.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("missing", "../../external/missing-crate")],
    );
    // crate-b (real) path-deps a real external sibling so the walk
    // still produces a peer; this shows the missing path does not
    // poison the rest of the walk.
    write_cargo_crate(
        &primary.join("crate-b"),
        "crate-b",
        &[("real", "../../external/real-crate")],
    );
    write_cargo_crate(
        &parent.path().join("external/real-crate"),
        "real-crate",
        &[],
    );

    let roots = expand_roots(&primary).expect("expand_roots succeeds despite missing path-dep");

    let primary_canonical = primary.canonicalize().unwrap();
    let real_canonical = parent
        .path()
        .join("external/real-crate")
        .canonicalize()
        .unwrap();
    assert_eq!(roots, vec![primary_canonical, real_canonical]);
}

/// Sanity: path-deps that resolve back inside the primary root (a
/// natural shape for in-tree workspace members referenced by sibling
/// path) are not added as peer roots.
#[test]
fn path_dep_inside_primary_is_not_added_as_peer() {
    let primary = TempDir::new().unwrap();
    write_cargo_crate(
        &primary.path().join("crate-a"),
        "crate-a",
        &[("crate-b", "../crate-b")],
    );
    write_cargo_crate(&primary.path().join("crate-b"), "crate-b", &[]);

    let roots = expand_roots(primary.path()).expect("expand_roots succeeds");
    let expected = primary.path().canonicalize().unwrap();
    assert_eq!(roots, vec![expected]);
}

/// True cross-tree cycle: crate-b in `external-1` path-deps crate-c
/// in `external-2`, and crate-c path-deps crate-b. The fixed-point
/// walk discovers external-1 (via the primary's path-dep) and
/// external-2 (via crate-b's), and on revisiting external-2's
/// path-dep to crate-b the visited-set guard short-circuits — no
/// stack overflow, no infinite loop. The expected discovery order is
/// `[primary, external-1, external-2]`.
#[test]
fn cross_tree_cycle_terminates() {
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let ext1 = parent.path().join("external-1");
    let ext2 = parent.path().join("external-2");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../external-1/crate-b")],
    );
    write_cargo_crate(
        &ext1.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../external-2/crate-c")],
    );
    write_cargo_crate(
        &ext2.join("crate-c"),
        "crate-c",
        &[("crate-b", "../../external-1/crate-b")],
    );

    let roots = expand_roots(&primary).expect("expand_roots terminates on cross-tree cycle");
    assert_eq!(roots.len(), 3, "primary + two peers, no infinite loop");

    let primary_canonical = primary.canonicalize().unwrap();
    let ext1_canonical = ext1.join("crate-b").canonicalize().unwrap();
    let ext2_canonical = ext2.join("crate-c").canonicalize().unwrap();
    assert_eq!(roots[0], primary_canonical);
    assert_eq!(roots[1], ext1_canonical);
    assert_eq!(roots[2], ext2_canonical);
}

/// Cycle between two true peer roots (both outside the primary):
/// peer-a path-deps a crate inside peer-b, peer-b path-deps a crate
/// inside peer-a. The walk discovers both peers, and on revisiting
/// the second peer's path-dep back to the first the visited-set
/// guard fires — emitting a `warning: path-dep cycle detected …`
/// line on the supplied warnings writer. The test pins the exact
/// warning string so a future regression in the cycle wording (or
/// the silent loss of the warning entirely) is caught.
#[test]
fn cycle_between_two_peers_emits_warning() {
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let peer_a = parent.path().join("peer-a");
    let peer_b = parent.path().join("peer-b");

    // Primary path-deps into peer-a's crate; peer-a's crate
    // path-deps into peer-b's crate; peer-b's crate path-deps back
    // into peer-a. The trip back into peer-a triggers the cycle
    // warning — peer-a is already visited.
    write_cargo_crate(
        &primary.join("crate-p"),
        "crate-p",
        &[("crate-a", "../../peer-a/crate-a")],
    );
    write_cargo_crate(
        &peer_a.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../peer-b/crate-b")],
    );
    write_cargo_crate(
        &peer_b.join("crate-b"),
        "crate-b",
        &[("crate-a", "../../peer-a/crate-a")],
    );

    let mut buf: Vec<u8> = Vec::new();
    let roots = expand_roots_with_warnings(&primary, &mut buf)
        .expect("expand_roots_with_warnings terminates on peer-peer cycle");
    assert_eq!(
        roots.len(),
        3,
        "primary + peer-a/crate-a + peer-b/crate-b — no extra roots from the cycle"
    );

    let stderr = String::from_utf8(buf).expect("warnings are utf-8");
    assert!(
        stderr.contains("warning: path-dep cycle detected"),
        "expected cycle warning, got: {stderr:?}"
    );
    let peer_a_canonical = peer_a.join("crate-a").canonicalize().unwrap();
    assert!(
        stderr.contains(&peer_a_canonical.display().to_string()),
        "warning should name peer-a's discovered root, got: {stderr:?}"
    );
}

/// Convenience: a path-dep through a chain of intermediate peers
/// works (a → b → c, all in different trees) and produces all three
/// roots in discovery order.
#[test]
fn transitive_path_deps_reach_all_peers() {
    let parent = TempDir::new().unwrap();
    let primary: PathBuf = parent.path().join("primary");
    let mid = parent.path().join("mid");
    let far = parent.path().join("far");

    write_cargo_crate(
        &primary.join("crate-a"),
        "crate-a",
        &[("crate-b", "../../mid/crate-b")],
    );
    write_cargo_crate(
        &mid.join("crate-b"),
        "crate-b",
        &[("crate-c", "../../far/crate-c")],
    );
    write_cargo_crate(&far.join("crate-c"), "crate-c", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let mid_canonical = mid.join("crate-b").canonicalize().unwrap();
    let far_canonical = far.join("crate-c").canonicalize().unwrap();
    assert_eq!(roots, vec![primary_canonical, mid_canonical, far_canonical]);
}

#[test]
fn pyproject_path_dep_expands_to_peer_root() {
    // Phase 2 PR-3 acceptance criterion: a Python component
    // path-dep'd via `pyproject.toml`'s `[tool.poetry.dependencies]`
    // from another root contributes its surface to the consumer's L6
    // cache key (echoes Phase 1 PR-12's cross-tree pattern with
    // Python in place of Rust).
    //
    // Fixture shape: `primary/consumer/` is a pyproject-shaped
    // package whose Poetry dependencies include a sibling tree's
    // `external/lib_a/` via `path = "../../external/lib_a"`. The
    // expanded-roots result must list both the primary and the
    // external-tree project root.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write_pyproject_package(
        &primary.join("consumer"),
        "consumer",
        &[("lib_a", "../../external/lib_a")],
    );
    write_pyproject_package(&external.join("lib_a"), "lib_a", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let lib_a_canonical = external.join("lib_a").canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, lib_a_canonical],
        "primary then lib_a's directory (no enclosing workspace)"
    );
}

#[test]
fn pyproject_dev_dependency_path_dep_expands_to_peer_root() {
    // The `[tool.poetry.dev-dependencies]` table is a peer of the
    // primary `dependencies` table for path-dep recognition; the
    // walker must consult both.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write(
        &primary.join("consumer/pyproject.toml"),
        "[tool.poetry]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n\
         [tool.poetry.dev-dependencies]\n\
         test_helpers = { path = \"../../external/test_helpers\" }\n",
    );
    write(&primary.join("consumer/consumer/__init__.py"), "");
    write_pyproject_package(&external.join("test_helpers"), "test_helpers", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");
    let primary_canonical = primary.canonicalize().unwrap();
    let helpers_canonical = external.join("test_helpers").canonicalize().unwrap();
    assert_eq!(roots, vec![primary_canonical, helpers_canonical]);
}

#[test]
fn pyproject_uv_sources_path_dep_expands_to_peer_root() {
    // PEP-621 projects layered with uv use `[tool.uv.sources]` to
    // declare path-deps. The walker must recognise this convention
    // alongside the Poetry one.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    write(
        &primary.join("consumer/pyproject.toml"),
        "[project]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n\
         [tool.uv.sources]\n\
         lib_b = { path = \"../../external/lib_b\" }\n",
    );
    write(&primary.join("consumer/consumer/__init__.py"), "");
    write_pyproject_package(&external.join("lib_b"), "lib_b", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");
    let primary_canonical = primary.canonicalize().unwrap();
    let lib_b_canonical = external.join("lib_b").canonicalize().unwrap();
    assert_eq!(roots, vec![primary_canonical, lib_b_canonical]);
}

// ---------------------------------------------------------------------
// §4 PR-3 acceptance criterion 5 (F4): a Python component path-dep'd
// via `pyproject.toml` from another root contributes its surface to
// the consumer's L6 cache key. The bare-`expand_roots` test above
// proves the *prerequisite* (path-dep is discovered as a peer root);
// the test below proves the *contribution* — mutating the lib's
// public surface changes the consumer's L6 fingerprint.
// ---------------------------------------------------------------------

/// Lenient stub backend: returns minimal valid responses for every
/// prompt so the L3 + L5 path completes during fingerprint capture.
/// Mirrors the LenientBackend pattern used in PR-3's other tests.
struct LenientBackend {
    fingerprint: LlmFingerprint,
}

fn lenient_fp() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

impl LlmBackend for LenientBackend {
    fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": "python-package",
                "language": "python",
                "evidence_grade": "strong",
                "evidence_fields": [],
                "rationale": "stub",
                "is_boundary": true,
            }),
            PromptId::Stage1Surface => json!({ "purpose": "stub", "notes": "" }),
            PromptId::Stage2Edges => json!([]),
            PromptId::Subcarve => json!({
                "should_subcarve": false,
                "sub_dirs": [],
                "rationale": "policy declined",
            }),
        })
    }

    fn fingerprint(&self) -> LlmFingerprint {
        self.fingerprint.clone()
    }
}

/// Compute the consumer's L6 fingerprint over the live component set
/// against the configured roots. Wraps `compute_l6_batch_fingerprint`
/// with the engine's surrounding setup (lenient backend, seeded
/// filesystem). Returns the 64-char hex fingerprint that the
/// production `all_proposed_edges` would feed to the L6 cache layer.
fn consumer_l6_fingerprint(roots: &[PathBuf]) -> String {
    let backend: Arc<dyn LlmBackend> = Arc::new(LenientBackend {
        fingerprint: lenient_fp(),
    });
    let mut db = AtlasDatabase::new(backend, roots.to_vec(), lenient_fp());
    seed_filesystem(&mut db, roots, false).expect("seed_filesystem must succeed");

    // Materialise the live component set in `all_proposed_edges` shape.
    let components = all_components(&db);
    let live: Vec<&atlas_index::ComponentEntry> =
        components.iter().filter(|c| !c.deleted).collect();

    // Mirror `all_proposed_edges`'s fingerprint construction: the
    // surfaces-derived participant_surface_sha contributions are what
    // the test cares about. Use stable stub values for the
    // prompt/registry/llm_fp slots so any fingerprint *change*
    // observed across runs comes from the surface contribution alone.
    let prompt_sha = "stub-prompt-sha".to_string();
    let registry_sha = "stub-registry-sha".to_string();
    let llm_fp = lenient_fp();
    compute_l6_batch_fingerprint(&db, &live, &prompt_sha, &registry_sha, &llm_fp)
}

#[test]
fn pyproject_path_dep_lib_surface_contributes_to_consumer_l6_fingerprint() {
    // §4 PR-3 acceptance criterion 5 (full): a Python component
    // path-dep'd via `pyproject.toml`'s `[tool.poetry.dependencies]`
    // from another root contributes its surface to the consumer's L6
    // cache key.
    //
    // Shape: primary root `primary/consumer/` is a python package
    // whose `pyproject.toml` declares a path-dep on `external/lib_a/`
    // (a sibling root). `lib_a/` exposes a public binding `foo`. We:
    //
    // 1. Capture the consumer's L6 fingerprint with `lib_a` as-is.
    // 2. Mutate `lib_a`'s source to add a *new* public function
    //    `bar` (changing its surface).
    // 3. Capture the consumer's L6 fingerprint again.
    // 4. Assert the two fingerprints differ.
    //
    // A regression where `lib_a`'s surface failed to flow into the
    // consumer's L6 cache key (e.g. peer-root surfaces are not
    // included in `live`, or `surfaces_yaml_snapshot` short-circuits
    // for cross-root components) would produce equal fingerprints
    // and fail this test.
    if atlas_analyzers::locate_python_analyzer_binary().is_none() {
        eprintln!("skipping: python-analyzer binary not located in target/");
        return;
    }

    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    // Consumer with a path-dep on lib_a.
    write(
        &primary.join("consumer/pyproject.toml"),
        "[tool.poetry]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n\
         [tool.poetry.dependencies]\n\
         python = \"^3.11\"\n\
         lib_a = { path = \"../../external/lib_a\" }\n",
    );
    write(&primary.join("consumer/consumer/__init__.py"), "");
    write(
        &primary.join("consumer/consumer/main.py"),
        "def consumer_entry():\n    return 1\n",
    );

    // lib_a — pyproject + a single public binding `foo`.
    write(
        &external.join("lib_a/pyproject.toml"),
        "[tool.poetry]\nname = \"lib_a\"\nversion = \"0.1.0\"\n",
    );
    let lib_pkg_init = external.join("lib_a/lib_a/__init__.py");
    write(&lib_pkg_init, "");
    let lib_source = external.join("lib_a/lib_a/mod.py");
    write(&lib_source, "def foo():\n    return 1\n");

    // Step 1: discover both roots via the path-dep walker, then
    // capture the consumer's L6 fingerprint with `lib_a` as-is.
    let roots = expand_roots(&primary).expect("expand_roots succeeds");
    assert_eq!(
        roots.len(),
        2,
        "fixture must expand to consumer + lib_a roots; got {roots:?}"
    );
    let fp_before = consumer_l6_fingerprint(&roots);

    // Step 2: mutate lib_a's source to add a new public binding.
    write(
        &lib_source,
        "def foo():\n    return 1\n\n\
         def bar():\n    return 2\n",
    );

    // Step 3: re-capture the consumer's L6 fingerprint.
    let fp_after = consumer_l6_fingerprint(&roots);

    // Step 4: the consumer's L6 cache key must reflect the lib's
    // surface change.
    assert_ne!(
        fp_before, fp_after,
        "lib_a's surface mutation must invalidate the consumer's L6 \
         fingerprint (the lib's bindings flow into the consumer's L6 \
         cache key via participant_surface_sha)"
    );
}

// ── §4 PR-6 acceptance criterion: csproj <ProjectReference> path-dep ─────────

/// Write a minimal C# project at `dir`:
/// - `<name>.csproj` with optional `<ProjectReference>` entries.
/// - `<name>.cs` with a single `public class`.
fn write_csharp_project(dir: &Path, name: &str, project_refs: &[&str]) {
    let mut item_group = String::new();
    if !project_refs.is_empty() {
        item_group.push_str("  <ItemGroup>\n");
        for r in project_refs {
            item_group.push_str(&format!("    <ProjectReference Include=\"{r}\" />\n"));
        }
        item_group.push_str("  </ItemGroup>\n");
    }
    let csproj = format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  \
         <PropertyGroup>\n    \
         <TargetFramework>net8.0</TargetFramework>\n  \
         </PropertyGroup>\n\
         {item_group}\
         </Project>\n"
    );
    write(&dir.join(format!("{name}.csproj")), &csproj);
    let cs = format!("namespace {name} {{\n    public class {name}Class {{}}\n}}\n");
    write(&dir.join(format!("{name}.cs")), &cs);
}

#[test]
fn csproj_path_dep_expands_to_peer_root() {
    // §4 PR-6 acceptance criterion (F2): a C# consumer csproj that
    // declares `<ProjectReference Include="../lib_a/lib_a.csproj">` must
    // cause `expand_roots` to discover the `lib_a` directory as a peer
    // root — mirroring the Python `pyproject_path_dep_expands_to_peer_root`
    // pattern from PR-3.
    //
    // <PackageReference> is intentionally excluded from path-dep
    // recognition: NuGet packages resolve through the NuGet feed, not as
    // local source trees. Only <ProjectReference> carries a real relative
    // filesystem path and participates in the workspace-local walk.
    let parent = TempDir::new().unwrap();
    let primary = parent.path().join("primary");
    let external = parent.path().join("external");

    // Consumer in the primary tree references lib_a in the external tree.
    write_csharp_project(
        &primary.join("consumer"),
        "consumer",
        &["../../external/lib_a/lib_a.csproj"],
    );
    // lib_a lives in a separate tree with no outgoing ProjectReferences.
    write_csharp_project(&external.join("lib_a"), "lib_a", &[]);

    let roots = expand_roots(&primary).expect("expand_roots succeeds");

    let primary_canonical = primary.canonicalize().unwrap();
    let lib_a_canonical = external.join("lib_a").canonicalize().unwrap();
    assert_eq!(
        roots,
        vec![primary_canonical, lib_a_canonical],
        "primary then lib_a's directory (no enclosing sln); got {roots:?}"
    );
}
