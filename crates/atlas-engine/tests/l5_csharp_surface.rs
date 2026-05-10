//! L5 surface-extraction integration tests for the C# subprocess
//! analyser (Atlas vNext Phase 2 PR-6).
//!
//! These tests drive `surface_artefacts_of` end-to-end through the
//! actual `csharp-analyzer` subprocess transport. `cargo test --workspace`
//! builds the csharp-analyzer binary into `target/<profile>/`; the
//! engine resolves it at runtime via
//! [`atlas_analyzers::locate_csharp_analyzer_binary`].
//!
//! The tests skip themselves if the binary cannot be located — defensive
//! against running outside a cargo target tree.

use std::path::Path;
use std::sync::Arc;

use atlas_engine::testing::LenientBackend;
use atlas_engine::{
    all_components, seed_filesystem, surface_artefacts_of, AtlasDatabase, ComponentKind,
};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use serde_json::json;
use tempfile::TempDir;

fn default_fingerprint() -> LlmFingerprint {
    LlmFingerprint {
        template_sha: [0u8; 32],
        ontology_sha: [0u8; 32],
        model_id: "test-backend".into(),
        backend_version: "0".into(),
    }
}

fn lenient_classify() -> serde_json::Value {
    json!({
        "kind": "csharp-project",
        "language": "csharp",
        "evidence_grade": "strong",
        "evidence_fields": [],
        "rationale": "stub",
        "is_boundary": true,
    })
}

fn build_db_lenient(root: &Path) -> AtlasDatabase {
    let backend: Arc<dyn LlmBackend> =
        LenientBackend::with_classify(default_fingerprint(), lenient_classify());
    let mut db = AtlasDatabase::new(backend, root.to_path_buf(), default_fingerprint());
    seed_filesystem(&mut db, &[root.to_path_buf()], false).expect("seed_filesystem must succeed");
    db
}

fn skip_if_binary_missing() -> bool {
    if atlas_analyzers::locate_csharp_analyzer_binary().is_none() {
        eprintln!("skipping: csharp-analyzer binary not located in target/");
        return true;
    }
    false
}

/// Write a minimal C# project fixture at `root`:
/// - `MyApp.csproj`
/// - `Program.cs` (with `public class Program`)
/// - `Models/User.cs` (with `public class User` in `App.Models` namespace)
fn write_csharp_project_fixture(root: &Path) {
    std::fs::write(
        root.join("MyApp.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("Program.cs"),
        r#"namespace App {
    public class Program {
        public static void Main(string[] args) {}
    }
}
"#,
    )
    .unwrap();

    std::fs::create_dir_all(root.join("Models")).unwrap();
    std::fs::write(
        root.join("Models/User.cs"),
        r#"namespace App.Models {
    public class User {
        public string Name { get; set; }
        public int Age { get; set; }
    }
}
"#,
    )
    .unwrap();
}

// ── Acceptance criterion 1: classify + surface ────────────────────────────────

#[test]
fn csharp_project_classifies_as_csharp_project_kind() {
    // §4 PR-6 acceptance criterion: a `*.csproj` + `Program.cs` +
    // `Models/User.cs` fixture is classified `csharp-project` at L3
    // with no LLM call.
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    write_csharp_project_fixture(&root);

    let db = build_db_lenient(&root);
    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted)
        .expect("fixture must produce a component");
    assert_eq!(
        comp.kind,
        ComponentKind::CsharpProject.as_str(),
        "fixture must classify as csharp-project, got {}",
        comp.kind
    );
}

#[test]
fn csharp_project_surface_lists_program_and_user_bindings() {
    // §4 PR-6 acceptance criterion: after L5 the surfaces include
    // `Program` and `Models.User` (via namespace-derived module_path).
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    write_csharp_project_fixture(&root);

    let db = build_db_lenient(&root);
    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");

    let artefacts = surface_artefacts_of(&db, comp.id.clone());
    let symbols: Vec<&str> = artefacts
        .bindings
        .iter()
        .map(|b| b.symbol.as_str())
        .collect();

    assert!(
        symbols.contains(&"Program"),
        "expected `Program` in bindings, got: {symbols:?}"
    );
    assert!(
        symbols.contains(&"User"),
        "expected `User` in bindings, got: {symbols:?}"
    );

    // §4 PR-6: module_path is derived from namespace.
    let user = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "User")
        .expect("User binding present");
    assert_eq!(
        user.module_path,
        vec!["App", "Models"],
        "User.module_path must be [\"App\", \"Models\"], got {:?}",
        user.module_path
    );

    let program = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Program")
        .expect("Program binding present");
    assert_eq!(
        program.module_path,
        vec!["App"],
        "Program.module_path must be [\"App\"], got {:?}",
        program.module_path
    );
    // Both have explicit public visibility.
    assert!(
        matches!(&user.visibility, atlas_index::Visibility::Explicit { keyword } if keyword == "public"),
        "User must have Visibility::Explicit{{\"public\"}}, got {:?}",
        user.visibility
    );
}

// ── Acceptance criterion 2: attribute capture ─────────────────────────────────

#[test]
fn csharp_serializable_attribute_appears_in_surface() {
    // §4 PR-6 acceptance criterion: `[Serializable]` attribute on a
    // class → `attributes.cs_attributes: ["Serializable"]`.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    std::fs::write(root.join("Data.csproj"), "<Project />\n").unwrap();
    std::fs::write(
        root.join("Data.cs"),
        r#"namespace Acme {
    [Serializable]
    public class Data {
        public int Value { get; set; }
    }
}
"#,
    )
    .unwrap();

    let db = build_db_lenient(&root);
    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");

    let artefacts = surface_artefacts_of(&db, comp.id.clone());
    let data = artefacts
        .bindings
        .iter()
        .find(|b| b.symbol == "Data")
        .expect("Data binding present");

    let cs_attrs = data
        .attributes
        .get("cs_attributes")
        .expect("cs_attributes must be present on [Serializable] class");
    let names: Vec<String> = cs_attrs
        .as_sequence()
        .expect("cs_attributes must be a sequence")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        names.contains(&"Serializable".to_string()),
        "expected Serializable in cs_attributes, got: {names:?}"
    );
}

// ── Acceptance criterion 3: internal exclusion ────────────────────────────────

#[test]
fn csharp_internal_class_excluded_from_surface() {
    // §4 PR-6 acceptance criterion: `internal class Foo` is NOT in
    // surface; `public class Foo` is.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    std::fs::write(root.join("Vis.csproj"), "<Project />\n").unwrap();
    std::fs::write(
        root.join("Vis.cs"),
        r#"namespace Acme {
    public class PublicClass {}
    internal class InternalClass {}
}
"#,
    )
    .unwrap();

    let db = build_db_lenient(&root);
    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");

    let artefacts = surface_artefacts_of(&db, comp.id.clone());
    let symbols: Vec<&str> = artefacts
        .bindings
        .iter()
        .map(|b| b.symbol.as_str())
        .collect();

    assert!(
        symbols.contains(&"PublicClass"),
        "PublicClass must appear in surface, got: {symbols:?}"
    );
    assert!(
        !symbols.contains(&"InternalClass"),
        "InternalClass must NOT appear in surface, got: {symbols:?}"
    );
}

// ── Acceptance criterion 4: sln → multi-component ────────────────────────────

#[test]
fn csharp_sln_fixture_classifies_as_csharp_solution() {
    // §4 PR-6 acceptance criterion: `*.sln` referencing two `*.csproj`
    // files → a component classified as `csharp-solution`.
    // (Full multi-component walk is beyond this test's scope — the
    // acceptance criterion that counts is that the sln boundary is
    // recognised at L3.)
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    std::fs::write(
        root.join("MySolution.sln"),
        "# VS solution file\nProject(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \
         \"Core\", \"Core\\Core.csproj\"\nEndProject\n",
    )
    .unwrap();

    let db = build_db_lenient(&root);
    let components = all_components(&db);
    let sln_comp = components
        .iter()
        .find(|c| !c.deleted && c.kind == "csharp-solution");
    assert!(
        sln_comp.is_some(),
        "expected a csharp-solution component, found: {:?}",
        components.iter().map(|c| &c.kind).collect::<Vec<_>>()
    );
}

// ── Acceptance criterion 4 (full): sln with two csproj → two surfaces ────────

#[test]
fn csharp_sln_with_two_csproj_drives_surfaces_for_both_child_components() {
    // §4 PR-6 acceptance criterion (closure): `MySolution.sln` lists
    // TWO `*.csproj` entries → two distinct non-deleted components both
    // classified as `csharp-project`, and `surface_artefacts_of` on
    // each returns non-empty bindings.
    if skip_if_binary_missing() {
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    // Solution file referencing two projects.
    std::fs::write(
        root.join("MySolution.sln"),
        "# VS solution\n\
         Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \
         \"Core\", \"Core\\Core.csproj\", \"{11111111-0000-0000-0000-000000000001}\"\n\
         EndProject\n\
         Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \
         \"App\", \"App\\App.csproj\", \"{11111111-0000-0000-0000-000000000002}\"\n\
         EndProject\n",
    )
    .unwrap();

    // Core project: one public class.
    std::fs::create_dir_all(root.join("Core")).unwrap();
    std::fs::write(
        root.join("Core/Core.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  \
         <PropertyGroup>\n    \
         <TargetFramework>net8.0</TargetFramework>\n  \
         </PropertyGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Core/CoreService.cs"),
        "namespace Core {\n    public class CoreService {}\n}\n",
    )
    .unwrap();

    // App project: one public class.
    std::fs::create_dir_all(root.join("App")).unwrap();
    std::fs::write(
        root.join("App/App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  \
         <PropertyGroup>\n    \
         <OutputType>Exe</OutputType>\n    \
         <TargetFramework>net8.0</TargetFramework>\n  \
         </PropertyGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("App/AppMain.cs"),
        "namespace App {\n    public class AppMain {}\n}\n",
    )
    .unwrap();

    let db = build_db_lenient(&root);
    let components = all_components(&db);

    // (a) Exactly two non-deleted csharp-project components (beyond the
    //     solution itself, which may or may not be produced as a separate
    //     component depending on subcarve policy).
    let project_comps: Vec<_> = components
        .iter()
        .filter(|c| !c.deleted && c.kind == "csharp-project")
        .collect();
    assert_eq!(
        project_comps.len(),
        2,
        "expected exactly 2 non-deleted csharp-project components, found: {:?}",
        components
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| (&c.kind, &c.id))
            .collect::<Vec<_>>()
    );

    // (b) Each child component must return non-empty bindings from L5.
    for comp in &project_comps {
        let artefacts = surface_artefacts_of(&db, comp.id.clone());
        assert!(
            !artefacts.bindings.is_empty(),
            "component id={} (kind={}) must have non-empty bindings from L5 surface",
            comp.id,
            comp.kind,
        );
    }
}

// ── L3 no-LLM classification ──────────────────────────────────────────────────

#[test]
fn csharp_project_classifies_without_llm_call() {
    // §4 PR-6 acceptance criterion: `*.csproj` is classified at L3
    // by the `csharp-classifier` with NO LLM call.
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ClassifyCountingBackend {
        fingerprint: LlmFingerprint,
        classify_calls: Arc<AtomicUsize>,
    }

    impl LlmBackend for ClassifyCountingBackend {
        fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
            match req.prompt_template {
                PromptId::Classify => {
                    self.classify_calls.fetch_add(1, Ordering::SeqCst);
                    Err(LlmError::TestBackendMiss(
                        "Classify prompt must NOT fire for a *.csproj fixture (§4 PR-6: \
                         csharp-classifier provides deterministic L3 verdict)"
                            .to_string(),
                    ))
                }
                PromptId::Stage1Surface => Ok(json!({ "purpose": "stub", "notes": "" })),
                PromptId::Stage2Edges => Ok(json!([])),
                PromptId::Subcarve => Ok(json!({
                    "should_subcarve": false,
                    "sub_dirs": [],
                    "rationale": "policy declined",
                })),
            }
        }
        fn fingerprint(&self) -> LlmFingerprint {
            self.fingerprint.clone()
        }
    }

    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    std::fs::write(root.join("MyApp.csproj"), "<Project />\n").unwrap();

    let classify_calls = Arc::new(AtomicUsize::new(0));
    let backend: Arc<dyn LlmBackend> = Arc::new(ClassifyCountingBackend {
        fingerprint: default_fingerprint(),
        classify_calls: classify_calls.clone(),
    });
    let mut db = AtlasDatabase::new(backend, root.clone(), default_fingerprint());
    seed_filesystem(&mut db, std::slice::from_ref(&root), false)
        .expect("seed_filesystem must succeed");

    let comp = all_components(&db)
        .iter()
        .find(|c| !c.deleted)
        .cloned()
        .expect("fixture produces a component");

    assert_eq!(
        comp.kind,
        ComponentKind::CsharpProject.as_str(),
        "csharp-classifier must deterministically produce csharp-project, got {}",
        comp.kind
    );
    assert_eq!(
        classify_calls.load(Ordering::SeqCst),
        0,
        "Classify LLM prompt fired {} time(s); csharp-classifier must short-circuit at L3",
        classify_calls.load(Ordering::SeqCst)
    );
}

// ── csproj path-dep extraction ────────────────────────────────────────────────

#[test]
fn csharp_project_surface_artefacts_include_project_reference() {
    // §4 PR-6 acceptance criterion: `<ProjectReference>` in *.csproj
    // → `project_references` in `CsharpSurfaceOutput` (available via
    // the in-process library).
    use atlas_csharp_analyzer::{extract_csharp_surface, CsharpSourceInputs};
    use std::path::PathBuf;

    let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <ProjectReference Include="../Shared/Shared.csproj" />
  </ItemGroup>
</Project>
"#;

    let inputs = CsharpSourceInputs {
        sources: vec![],
        csproj: Some(csproj.as_bytes().to_vec()),
        csproj_name: Some("MyApp.csproj".into()),
    };
    let out = extract_csharp_surface("test/comp", &inputs);
    assert_eq!(
        out.project_references,
        vec![PathBuf::from("../Shared/Shared.csproj")]
    );
}
