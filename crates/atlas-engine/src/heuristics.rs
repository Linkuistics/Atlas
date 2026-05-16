//! Non-Cargo deterministic classification rules. Each rule reads a
//! [`Candidate`]'s rationale bundle (plus any manifest contents the
//! caller can supply) and either produces a [`Classification`] — with
//! `evidence_grade: Strong` and an explicit `evidence_fields` list —
//! or declines, passing the candidate on to the LLM fallback.
//!
//! Rules are tabulated as a flat array rather than a chain of
//! if/else: the table is the documentation of the deterministic
//! surface, and adding a new rule is one entry rather than a re-nest.
//!
//! **History:** Cargo classification rules lived here in Phase 1, then
//! moved into the deterministic `atlas_analyzers::cargo_classifier`
//! crate in PR-5. Phase 8 WI-3 retired that deterministic classifier;
//! Rust components now fall through to
//! [`atlas_analyzers::llm_classify::LlmClassifyAnalyzer`]. The L3
//! driver still dispatches the analyser registry first; only when the
//! remaining deterministic analysers (Dockerfile, etc.) all decline
//! does the engine consult the rule table below. The Dockerfile-image
//! rule lives in
//! [`atlas_analyzers::dockerfile_classifier::DockerfileClassifier`].

use std::collections::BTreeSet;
use std::path::Path;

use component_ontology::{EvidenceGrade, LifecycleScope};

use crate::manifest_parse::{parse_package_json, PackageJsonShape};
use crate::types::{Candidate, Classification, ComponentKind};

/// Single-element language set helper. Phase 1 deterministic rules
/// always emit a one-element set; PR-9+ can grow this to a real set
/// when polyglot detection lands.
fn one_lang(lang: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    set.insert(lang.to_string());
    set
}

/// Stable id used for the legacy non-Cargo deterministic rule table.
/// These rules are not yet first-class analysers (PR-9+ will migrate
/// them); the id makes the per-component projection's
/// `analyser_id` field non-empty for components classified by this
/// path.
pub const LEGACY_ANALYSER_ID: &str = "legacy-deterministic-rules";

/// Version paired with [`LEGACY_ANALYSER_ID`].
pub const LEGACY_ANALYSER_VERSION: &str = "1.0.0";

/// Contents of any manifest the classifier may want to inspect. The
/// L3 query loads these before consulting the rule table so rule
/// predicates work on pre-read strings rather than file handles.
#[derive(Debug, Default)]
pub struct ManifestContents<'a> {
    pub cargo_toml: Option<&'a str>,
    pub package_json: Option<&'a str>,
    pub pyproject_toml: Option<&'a str>,
}

/// Try every deterministic rule in order; return the first match.
/// `None` means no rule applied — the caller should fall back to the
/// analyser registry's LLM-classify path.
///
/// The rule table covers the remaining deterministic non-classifier
/// vocabulary (npm, pyproject, bare-git). Cargo rules were retired
/// in Phase 8 WI-3 (the deterministic spine is gone; Rust components
/// fall through to the LLM-classify analyser via the registry's L3
/// dispatch).
pub fn classify_deterministic(
    candidate: &Candidate,
    manifest_contents: &ManifestContents<'_>,
) -> Option<Classification> {
    let package = manifest_contents.package_json.map(parse_package_json);

    for rule in RULES {
        if let Some(classification) = (rule.apply)(candidate, manifest_contents, package.as_ref()) {
            return Some(classification);
        }
    }
    None
}

type RuleFn = fn(
    candidate: &Candidate,
    manifests: &ManifestContents<'_>,
    package: Option<&PackageJsonShape>,
) -> Option<Classification>;

struct Rule {
    /// Human-readable rule name, used only for debugging.
    #[allow(dead_code)]
    name: &'static str,
    apply: RuleFn,
}

const RULES: &[Rule] = &[
    Rule {
        name: "package-json-bin",
        apply: rule_package_json_bin,
    },
    Rule {
        name: "package-json-library",
        apply: rule_package_json_library,
    },
    Rule {
        name: "pyproject-toml",
        apply: rule_pyproject_toml,
    },
    Rule {
        name: "bare-git-no-manifests",
        apply: rule_bare_git_no_manifests,
    },
];

fn rule_package_json_bin(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = package?;
    if !shape.has_bin {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::NodeCli,
        languages: one_lang("javascript"),
        build_system: Some("npm".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["package.json:bin".into()],
        rationale: "package.json declares a `bin` field.".into(),
        is_boundary: true,
        analyser_id: LEGACY_ANALYSER_ID.to_string(),
        analyser_version: LEGACY_ANALYSER_VERSION.to_string(),
    })
}

fn rule_package_json_library(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = package?;
    if !(shape.has_main || shape.has_exports) || shape.has_bin {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::NodeLibrary,
        languages: one_lang("javascript"),
        build_system: Some("npm".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["package.json:main|exports".into()],
        rationale: "package.json declares `main` or `exports` with no `bin`.".into(),
        is_boundary: true,
        analyser_id: LEGACY_ANALYSER_ID.to_string(),
        analyser_version: LEGACY_ANALYSER_VERSION.to_string(),
    })
}

fn rule_pyproject_toml(
    _candidate: &Candidate,
    manifests: &ManifestContents<'_>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    manifests.pyproject_toml?;
    Some(Classification {
        kind: ComponentKind::PythonLibrary,
        languages: one_lang("python"),
        build_system: Some("pyproject".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["pyproject.toml".into()],
        rationale: "pyproject.toml present.".into(),
        is_boundary: true,
        analyser_id: LEGACY_ANALYSER_ID.to_string(),
        analyser_version: LEGACY_ANALYSER_VERSION.to_string(),
    })
}

fn rule_bare_git_no_manifests(
    candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let bundle = &candidate.rationale_bundle;
    if !bundle.is_git_root {
        return None;
    }
    if !bundle.manifests.is_empty() {
        return None;
    }
    // A README under this dir counts as a declaration of purpose —
    // let the LLM take a closer look, because a bare-git + README
    // repository might be a spec, docs, or something else interesting.
    let has_readme_near = bundle
        .doc_headings
        .iter()
        .any(|h| is_at_or_directly_under(&h.path, &candidate.dir));
    if has_readme_near {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::NonComponent,
        languages: BTreeSet::new(),
        build_system: None,
        lifecycle_roles: Vec::new(),
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec![".git".into()],
        rationale: "Directory has a .git marker but no manifests and no README declaring purpose."
            .into(),
        is_boundary: false,
        analyser_id: LEGACY_ANALYSER_ID.to_string(),
        analyser_version: LEGACY_ANALYSER_VERSION.to_string(),
    })
}

fn is_at_or_directly_under(file: &Path, dir: &Path) -> bool {
    match file.parent() {
        Some(parent) => parent == dir,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::types::RationaleBundle;

    fn bare_candidate(dir: &str) -> Candidate {
        Candidate {
            dir: PathBuf::from(dir),
            rationale_bundle: RationaleBundle {
                manifests: Vec::new(),
                is_git_root: false,
                doc_headings: Vec::new(),
                shebangs: Vec::new(),
            },
        }
    }

    // Note: Cargo classification rules moved into the deterministic
    // `atlas_analyzers::cargo_classifier` crate in PR-5 and were
    // retired entirely in Phase 8 WI-3. Rust components now classify
    // via the LLM-classify analyser.

    #[test]
    fn bare_git_with_readme_declines_deterministic_rule() {
        // README next to .git is a signal of purpose — decline and
        // let the LLM take over.
        let mut cand = bare_candidate("/repo");
        cand.rationale_bundle.is_git_root = true;
        cand.rationale_bundle
            .doc_headings
            .push(crate::l1_queries::DocHeading {
                path: PathBuf::from("/repo/README.md"),
                level: 1,
                text: "Repo".into(),
            });
        let manifests = ManifestContents::default();
        assert!(classify_deterministic(&cand, &manifests).is_none());
    }

    #[test]
    fn bare_git_without_readme_classifies_non_component() {
        let mut cand = bare_candidate("/repo");
        cand.rationale_bundle.is_git_root = true;
        let manifests = ManifestContents::default();
        let c = classify_deterministic(&cand, &manifests).unwrap();
        assert_eq!(c.kind, ComponentKind::NonComponent);
        assert!(!c.is_boundary);
    }
}
