//! L5 surface-extraction integration tests for the shell-script / Makefile
//! LLM-fallback analyser (Atlas vNext Phase 2 PR-12).
//!
//! The tests drive `surface_artefacts_of` end-to-end through the in-process
//! shell-surface extractor wired by PR-12's `l5_surface.rs` branch.
//!
//! ## Acceptance criteria (§4 PR-12)
//!
//! - Integration test: `deploy.sh` with `function deploy()` → `shell-script`
//!   component with one binding.
//! - Integration test: `Makefile` with `build:` and `clean:` →
//!   `makefile-orchestration` component (target extraction in scope;
//!   LLM-derived purpose `Confidence::Graded`).
//!
//! Because `Makefile` and `.sh` files are not in the manifest-patterns table
//! (the engine's L2 heuristic would not discover bare shell dirs as component
//! candidates without a manifest), the tests inject the component via
//! `overrides.additions`. A configurable stub backend returns the expected
//! classification kind so the L3 pass and the addition agree.

use std::sync::Arc;

use atlas_engine::{all_components, seed_filesystem, surface_artefacts_of, AtlasDatabase};
use atlas_index::{ComponentEntry, OverridesFile, PathSegment};
use atlas_llm::{LlmBackend, LlmError, LlmFingerprint, LlmRequest, PromptId};
use component_ontology::{ComponentId, EvidenceGrade};
use serde_json::json;
use tempfile::TempDir;

// ── Configurable stub backend ─────────────────────────────────────────────────

/// A backend that returns a specified `kind` for `Classify` prompts.
struct KindBackend {
    fingerprint: LlmFingerprint,
    /// Kind string to return from `Classify` prompts.
    classify_kind: String,
}

impl KindBackend {
    fn new(classify_kind: &str) -> Self {
        KindBackend {
            fingerprint: LlmFingerprint {
                template_sha: [10u8; 32],
                ontology_sha: [11u8; 32],
                model_id: "test-shell-backend".into(),
                backend_version: "0".into(),
            },
            classify_kind: classify_kind.to_string(),
        }
    }
}

impl LlmBackend for KindBackend {
    fn call(&self, req: &LlmRequest) -> Result<serde_json::Value, LlmError> {
        Ok(match req.prompt_template {
            PromptId::Classify => json!({
                "kind": self.classify_kind,
                "language": if self.classify_kind == "makefile-orchestration" { "makefile" } else { "shell" },
                "evidence_grade": "medium",
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a `ComponentEntry` for injection via `overrides.additions`.
fn shell_addition(id: &str, kind: &str, rel_path: &str) -> ComponentEntry {
    ComponentEntry {
        id: ComponentId::parse(id).unwrap(),
        parent: None,
        kind: kind.to_string(),
        lifecycle_roles: Vec::new(),
        languages: {
            let mut s = std::collections::BTreeSet::new();
            if kind == "makefile-orchestration" {
                s.insert("makefile".to_string());
            } else {
                s.insert("shell".to_string());
            }
            s
        },
        build_system: None,
        role: None,
        path_segments: vec![PathSegment {
            path: std::path::PathBuf::from(rel_path),
            content_sha: "0".repeat(64),
        }],
        manifests: Vec::new(),
        doc_anchors: Vec::new(),
        evidence_grade: EvidenceGrade::Medium,
        evidence_fields: vec![],
        rationale: "PR-12 integration test fixture".into(),
        deleted: false,
    }
}

/// Build a database with the given additions and a backend that returns
/// `classify_kind` for every `Classify` prompt. The backend and additions
/// must agree on the kind so the L3-classified and additions-sourced
/// entries carry the same kind string.
fn db_with_additions(
    tmp: &TempDir,
    additions: Vec<ComponentEntry>,
    classify_kind: &str,
) -> AtlasDatabase {
    let fp = LlmFingerprint {
        template_sha: [10u8; 32],
        ontology_sha: [11u8; 32],
        model_id: "test-shell-backend".into(),
        backend_version: "0".into(),
    };
    let backend: Arc<dyn LlmBackend> = Arc::new(KindBackend::new(classify_kind));
    let mut db = AtlasDatabase::new(backend, tmp.path().to_path_buf(), fp);
    db.set_components_overrides(OverridesFile {
        additions,
        ..OverridesFile::default()
    });
    seed_filesystem(&mut db, &[tmp.path().to_path_buf()], false).unwrap();
    db
}

// ── Acceptance criterion 1: deploy.sh with function deploy() ─────────────────

/// Lay out a minimal shell-script fixture:
///
/// ```
/// scripts/
///   deploy.sh
/// ```
fn write_shell_fixture(tmp: &TempDir) {
    let scripts_dir = tmp.path().join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();
    std::fs::write(
        scripts_dir.join("deploy.sh"),
        "#!/bin/bash\n\nfunction deploy() {\n  echo \"deploying...\"\n}\n\ndeploy \"$@\"\n",
    )
    .unwrap();
}

#[test]
fn deploy_sh_with_function_deploy_produces_shell_script_component_with_one_binding() {
    let tmp = TempDir::new().unwrap();
    write_shell_fixture(&tmp);

    // Inject the component via additions AND configure the backend to
    // agree on the kind (so L3 classification and additions both say
    // "shell-script").
    let db = db_with_additions(
        &tmp,
        vec![shell_addition("scripts", "shell-script", "scripts")],
        "shell-script",
    );

    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted && c.id.as_str() == "scripts" && c.kind == "shell-script")
        .expect("fixture must produce a `scripts` component with kind=shell-script");

    assert_eq!(comp.kind, "shell-script");

    let artefacts = surface_artefacts_of(&db, comp.id.clone());

    // The shell surface extractor should extract the `deploy` function.
    assert_eq!(
        artefacts.bindings.len(),
        1,
        "deploy.sh has one function definition; expected one binding, got {:?}",
        artefacts
            .bindings
            .iter()
            .map(|b| &b.symbol)
            .collect::<Vec<_>>()
    );

    let binding = &artefacts.bindings[0];
    assert_eq!(binding.symbol, "deploy");
    assert_eq!(binding.language, "shell");
    assert_eq!(
        binding.visibility,
        atlas_index::Visibility::Conventional,
        "shell bindings must use Visibility::Conventional"
    );
    assert_eq!(
        binding.attributes.get(atlas_analyzers::ATTR_SHELL_FUNCTION),
        Some(&serde_yaml::Value::Bool(true)),
        "binding must have attributes.shell_function: true"
    );

    // One LibraryApi should be present (non-empty binding set).
    assert_eq!(artefacts.library_apis.len(), 1);
    assert_eq!(artefacts.library_apis[0].pub_items.len(), 1);
    assert_eq!(artefacts.library_apis[0].pub_items[0].name, "deploy");
}

// ── Acceptance criterion 2: Makefile with build and clean targets ─────────────

/// Lay out a minimal Makefile fixture:
///
/// ```
/// makefiles/
///   Makefile
/// ```
fn write_makefile_fixture(tmp: &TempDir) {
    let dir = tmp.path().join("makefiles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Makefile"),
        ".PHONY: build clean\n\nbuild:\n\tcargo build\n\nclean:\n\tcargo clean\n",
    )
    .unwrap();
}

#[test]
fn makefile_with_build_and_clean_produces_makefile_orchestration_component() {
    let tmp = TempDir::new().unwrap();
    write_makefile_fixture(&tmp);

    // Backend and additions both say "makefile-orchestration".
    let db = db_with_additions(
        &tmp,
        vec![shell_addition(
            "makefiles",
            "makefile-orchestration",
            "makefiles",
        )],
        "makefile-orchestration",
    );

    let components = all_components(&db);
    let comp = components
        .iter()
        .find(|c| !c.deleted && c.id.as_str() == "makefiles" && c.kind == "makefile-orchestration")
        .expect("fixture must produce a `makefiles` component with kind=makefile-orchestration");

    assert_eq!(comp.kind, "makefile-orchestration");

    let artefacts = surface_artefacts_of(&db, comp.id.clone());

    // The Makefile has `build:` and `clean:` targets → 2 bindings.
    let symbols: Vec<&str> = artefacts
        .bindings
        .iter()
        .map(|b| b.symbol.as_str())
        .collect();
    assert!(
        symbols.contains(&"build"),
        "expected `build` target in bindings; got {symbols:?}"
    );
    assert!(
        symbols.contains(&"clean"),
        "expected `clean` target in bindings; got {symbols:?}"
    );

    // All Makefile bindings must be language=makefile, Conventional, shell_function=true.
    for b in &artefacts.bindings {
        assert_eq!(b.language, "makefile");
        assert_eq!(b.visibility, atlas_index::Visibility::Conventional);
        assert_eq!(
            b.attributes.get(atlas_analyzers::ATTR_SHELL_FUNCTION),
            Some(&serde_yaml::Value::Bool(true))
        );
    }

    // Both targets are phony → each binding should carry `phony: true`.
    for b in &artefacts.bindings {
        assert_eq!(
            b.attributes.get("phony"),
            Some(&serde_yaml::Value::Bool(true)),
            "target `{}` is declared .PHONY but lacks `phony: true` attribute",
            b.symbol
        );
    }

    // Library API should be present.
    assert_eq!(artefacts.library_apis.len(), 1);
    assert_eq!(artefacts.library_apis[0].language, "makefile");
}

// ── Additional: component_kinds vocab round-trips ─────────────────────────────

#[test]
fn component_kind_shell_script_parses_and_round_trips() {
    use atlas_engine::ComponentKind;
    let parsed = ComponentKind::parse("shell-script").expect("must parse");
    assert_eq!(parsed.as_str(), "shell-script");
}

#[test]
fn component_kind_makefile_orchestration_parses_and_round_trips() {
    use atlas_engine::ComponentKind;
    let parsed = ComponentKind::parse("makefile-orchestration").expect("must parse");
    assert_eq!(parsed.as_str(), "makefile-orchestration");
}
