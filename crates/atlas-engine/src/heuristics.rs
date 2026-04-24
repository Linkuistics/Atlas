//! Deterministic classification rules. Each rule reads a
//! [`Candidate`]'s rationale bundle (plus any manifest contents the
//! caller can supply) and either produces a [`Classification`] — with
//! `evidence_grade: Strong` and an explicit `evidence_fields` list —
//! or declines, passing the candidate on to the LLM fallback.
//!
//! Rules are tabulated as a flat array rather than a chain of
//! if/else: the table is the documentation of the deterministic
//! surface, and adding a new rule is one entry rather than a re-nest.

use std::path::Path;

use component_ontology::{EvidenceGrade, LifecycleScope};

use crate::manifest_parse::{parse_cargo_toml, parse_package_json, CargoTomlShape, PackageJsonShape};
use crate::types::{Candidate, Classification, ComponentKind};

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
/// LLM classifier.
pub fn classify_deterministic(
    candidate: &Candidate,
    manifest_contents: &ManifestContents<'_>,
) -> Option<Classification> {
    let cargo = manifest_contents.cargo_toml.map(parse_cargo_toml);
    let package = manifest_contents.package_json.map(parse_package_json);

    for rule in RULES {
        if let Some(classification) =
            (rule.apply)(candidate, manifest_contents, cargo.as_ref(), package.as_ref())
        {
            return Some(classification);
        }
    }
    None
}

type RuleFn = fn(
    candidate: &Candidate,
    manifests: &ManifestContents<'_>,
    cargo: Option<&CargoTomlShape>,
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
        name: "cargo-workspace",
        apply: rule_cargo_workspace,
    },
    Rule {
        name: "cargo-bin",
        apply: rule_cargo_bin,
    },
    Rule {
        name: "cargo-lib",
        apply: rule_cargo_lib,
    },
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

fn rule_cargo_workspace(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    cargo: Option<&CargoTomlShape>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = cargo?;
    if !shape.has_workspace_section {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::Workspace,
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        lifecycle_roles: vec![LifecycleScope::Build],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml:[workspace]".into()],
        rationale: "Cargo.toml declares a [workspace] section.".into(),
        is_boundary: true,
    })
}

fn rule_cargo_bin(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    cargo: Option<&CargoTomlShape>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = cargo?;
    if !shape.has_bin_section || shape.has_workspace_section {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::RustCli,
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml:[[bin]]".into()],
        rationale: "Cargo.toml declares a [[bin]] section.".into(),
        is_boundary: true,
    })
}

fn rule_cargo_lib(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    cargo: Option<&CargoTomlShape>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = cargo?;
    if !shape.has_lib_section || shape.has_bin_section || shape.has_workspace_section {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::RustLibrary,
        language: Some("rust".into()),
        build_system: Some("cargo".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["Cargo.toml:[lib]".into()],
        rationale: "Cargo.toml declares a [lib] section with no [[bin]].".into(),
        is_boundary: true,
    })
}

fn rule_package_json_bin(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    _cargo: Option<&CargoTomlShape>,
    package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = package?;
    if !shape.has_bin {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::NodeCli,
        language: Some("javascript".into()),
        build_system: Some("npm".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["package.json:bin".into()],
        rationale: "package.json declares a `bin` field.".into(),
        is_boundary: true,
    })
}

fn rule_package_json_library(
    _candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    _cargo: Option<&CargoTomlShape>,
    package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    let shape = package?;
    if !(shape.has_main || shape.has_exports) || shape.has_bin {
        return None;
    }
    Some(Classification {
        kind: ComponentKind::NodePackage,
        language: Some("javascript".into()),
        build_system: Some("npm".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["package.json:main|exports".into()],
        rationale: "package.json declares `main` or `exports` with no `bin`.".into(),
        is_boundary: true,
    })
}

fn rule_pyproject_toml(
    _candidate: &Candidate,
    manifests: &ManifestContents<'_>,
    _cargo: Option<&CargoTomlShape>,
    _package: Option<&PackageJsonShape>,
) -> Option<Classification> {
    manifests.pyproject_toml?;
    Some(Classification {
        kind: ComponentKind::PythonPackage,
        language: Some("python".into()),
        build_system: Some("pyproject".into()),
        lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec!["pyproject.toml".into()],
        rationale: "pyproject.toml present.".into(),
        is_boundary: true,
    })
}

fn rule_bare_git_no_manifests(
    candidate: &Candidate,
    _manifests: &ManifestContents<'_>,
    _cargo: Option<&CargoTomlShape>,
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
        language: None,
        build_system: None,
        lifecycle_roles: Vec::new(),
        role: None,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: vec![".git".into()],
        rationale: "Directory has a .git marker but no manifests and no README declaring purpose."
            .into(),
        is_boundary: false,
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

    #[test]
    fn cargo_lib_rule_fires_on_lib_only_manifest() {
        let contents = "[package]\nname = \"x\"\n[lib]\npath = \"src/lib.rs\"\n";
        let manifests = ManifestContents {
            cargo_toml: Some(contents),
            ..Default::default()
        };
        let c = classify_deterministic(&bare_candidate("."), &manifests).unwrap();
        assert_eq!(c.kind, ComponentKind::RustLibrary);
    }

    #[test]
    fn cargo_bin_rule_fires_on_bin_section() {
        let contents = "[package]\nname = \"x\"\n[[bin]]\nname = \"tool\"\n";
        let manifests = ManifestContents {
            cargo_toml: Some(contents),
            ..Default::default()
        };
        let c = classify_deterministic(&bare_candidate("."), &manifests).unwrap();
        assert_eq!(c.kind, ComponentKind::RustCli);
    }

    #[test]
    fn cargo_workspace_rule_wins_over_lib() {
        let contents = "[workspace]\nmembers = [\"a\"]\n[lib]\n";
        let manifests = ManifestContents {
            cargo_toml: Some(contents),
            ..Default::default()
        };
        let c = classify_deterministic(&bare_candidate("."), &manifests).unwrap();
        assert_eq!(c.kind, ComponentKind::Workspace);
    }

    #[test]
    fn bare_git_with_readme_declines_deterministic_rule() {
        // README next to .git is a signal of purpose — decline and
        // let the LLM take over.
        let mut cand = bare_candidate("/repo");
        cand.rationale_bundle.is_git_root = true;
        cand.rationale_bundle.doc_headings.push(crate::l1_queries::DocHeading {
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
