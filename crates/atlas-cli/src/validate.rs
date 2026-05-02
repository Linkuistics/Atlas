//! Static validation for `components.overrides.yaml`.
//!
//! Catches semantic drift that the schema-level deserialiser lets
//! through silently — most importantly, `kind` values that L3 would
//! parse as `None` and degrade to `non-component` without raising
//! (`l3_classify::pins_to_classification`). The validator reports
//! errors (must be fixed) and warnings (suspicious but accepted)
//! without touching the filesystem beyond the YAML it was given.

use std::io::Write;
use std::path::Path;

use atlas_engine::ComponentKind;
use atlas_index::{OverridesFile, PinValue, SubsystemsOverridesFile};

/// Recognised pin field names. Any other field in a pin entry is
/// silently ignored by the engine, so an unrecognised field is almost
/// always a typo (`kid:` for `kind:`, `roles:` for `role:`).
const RECOGNISED_PIN_FIELDS: &[&str] = &[
    "kind",
    "language",
    "build_system",
    "role",
    "suppress",
    "suppress_children",
];

/// One drift finding in a `components.overrides.yaml`. `severity`
/// distinguishes "must fix" from "suspicious but accepted".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub pin_key: Option<String>,
    pub field: Option<String>,
    pub message: String,
    /// Canonical replacement when the issue is a known typo
    /// (e.g., `rust-binary` → `rust-cli`).
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Result of validating an `OverridesFile`. Errors block the
/// pipeline; warnings are printed and analysis continues.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    pub fn has_any(&self) -> bool {
        !self.issues.is_empty()
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
    }
}

/// Validate `overrides` against the canonical kind vocabulary and
/// the recognised set of pin fields.
pub fn validate_overrides(overrides: &OverridesFile) -> ValidationReport {
    let mut report = ValidationReport::default();

    for (pin_key, fields) in &overrides.pins {
        for (field, value) in fields {
            if !RECOGNISED_PIN_FIELDS.contains(&field.as_str()) {
                report.issues.push(ValidationIssue {
                    severity: Severity::Warning,
                    pin_key: Some(pin_key.clone()),
                    field: Some(field.clone()),
                    message: format!("unknown pin field `{field}` will be ignored by the engine"),
                    suggestion: None,
                });
                continue;
            }
            if field == "kind" {
                if let PinValue::Value {
                    value: kind_str, ..
                } = value
                {
                    if ComponentKind::parse(kind_str).is_none() {
                        report.issues.push(ValidationIssue {
                            severity: Severity::Error,
                            pin_key: Some(pin_key.clone()),
                            field: Some("kind".to_string()),
                            message: format!(
                                "kind value `{kind_str}` is not in the canonical \
                                 vocabulary; L3 would silently classify this \
                                 component as `non-component`"
                            ),
                            suggestion: canonical_replacement(kind_str),
                        });
                    }
                }
            }
        }
    }

    for addition in &overrides.additions {
        if ComponentKind::parse(&addition.kind).is_none() {
            report.issues.push(ValidationIssue {
                severity: Severity::Error,
                pin_key: Some(addition.id.clone()),
                field: Some("kind".to_string()),
                message: format!(
                    "addition `{}` has kind `{}` which is not in the canonical \
                     vocabulary",
                    addition.id, addition.kind
                ),
                suggestion: canonical_replacement(&addition.kind),
            });
        }
    }

    report
}

/// Like [`validate_overrides`] but also checks `SubsystemsOverridesFile`
/// for shape-level errors (duplicate ids, empty members). Cross-namespace
/// collision and id-resolution checks happen post-L4 in the engine.
pub fn validate_overrides_with_subsystems(
    overrides: &OverridesFile,
    subsystems: &SubsystemsOverridesFile,
) -> ValidationReport {
    let mut report = validate_overrides(overrides);
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for sub in &subsystems.subsystems {
        if !seen.insert(sub.id.as_str()) {
            report.issues.push(ValidationIssue {
                severity: Severity::Error,
                pin_key: Some(sub.id.clone()),
                field: None,
                message: format!("duplicate subsystem id '{}'", sub.id),
                suggestion: None,
            });
        }
        if sub.members.is_empty() {
            report.issues.push(ValidationIssue {
                severity: Severity::Error,
                pin_key: Some(sub.id.clone()),
                field: Some("members".into()),
                message: format!(
                    "subsystem '{}' has empty members; remove the entry or add at least one glob/id",
                    sub.id
                ),
                suggestion: None,
            });
        }
    }
    report
}

/// Render a `ValidationReport` for human consumption. Each issue
/// gets one line, prefixed by `error:` or `warning:`, with the pin
/// key and field path. A trailing line shows the canonical
/// suggestion when one is known.
pub fn print_report<W: Write>(report: &ValidationReport, path: &Path, out: &mut W) {
    if report.issues.is_empty() {
        let _ = writeln!(out, "{}: ok", path.display());
        return;
    }
    let _ = writeln!(out, "{}:", path.display());
    for issue in &report.issues {
        let prefix = match issue.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let location = match (&issue.pin_key, &issue.field) {
            (Some(k), Some(f)) => format!("pins[{k}].{f}"),
            (Some(k), None) => format!("pins[{k}]"),
            _ => String::new(),
        };
        if location.is_empty() {
            let _ = writeln!(out, "  {prefix}: {}", issue.message);
        } else {
            let _ = writeln!(out, "  {prefix} at {location}: {}", issue.message);
        }
        if let Some(s) = &issue.suggestion {
            let _ = writeln!(out, "    suggestion: use `{s}` instead");
        }
    }
    let n_err = report.errors().count();
    let n_warn = report.warnings().count();
    let _ = writeln!(out, "  ({n_err} error(s), {n_warn} warning(s))");
}

/// Map a known-tempting-but-wrong kind slug to the canonical form.
/// Matches the table in `defaults/component-kinds.md` ("Mappings to
/// avoid"). Returns `None` for slugs without a known replacement.
fn canonical_replacement(kind: &str) -> Option<String> {
    let canonical = match kind {
        "rust-binary" => "rust-cli",
        "node-package" => "node-library",
        "python-package" => "python-library",
        "dart-package" => "dart-library",
        "typescript-library" => "node-library",
        "wix-installer" | "msi-installer" | "deb-package" | "dmg" => "installer",
        "shell-script" => "shell-scripts",
        "sql-script" => "sql-scripts",
        "python-cli" => "python-app",
        "python-service" | "node-service" => "service",
        "python-codegen-tool" | "rust-codegen-tool" | "node-codegen-tool" => "codegen-tool",
        _ => return None,
    };
    Some(canonical.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::ComponentEntry;
    use component_ontology::EvidenceGrade;
    use std::collections::BTreeMap;

    fn pin_value(s: &str) -> PinValue {
        PinValue::Value {
            value: s.to_string(),
            reason: None,
        }
    }

    fn overrides_with_pin(id: &str, field: &str, value: &str) -> OverridesFile {
        let mut field_pins = BTreeMap::new();
        field_pins.insert(field.to_string(), pin_value(value));
        let mut pins = BTreeMap::new();
        pins.insert(id.to_string(), field_pins);
        OverridesFile {
            pins,
            ..OverridesFile::default()
        }
    }

    #[test]
    fn clean_file_produces_empty_report() {
        let overrides = overrides_with_pin("crates/foo", "kind", "rust-library");
        let report = validate_overrides(&overrides);
        assert!(!report.has_any());
    }

    #[test]
    fn unknown_kind_is_an_error_with_no_suggestion_for_garbage() {
        let overrides = overrides_with_pin("crates/foo", "kind", "blorp-flim-flam");
        let report = validate_overrides(&overrides);
        assert!(report.has_errors());
        let err = report.errors().next().unwrap();
        assert_eq!(err.field.as_deref(), Some("kind"));
        assert!(err.message.contains("blorp-flim-flam"));
        assert!(err.suggestion.is_none());
    }

    #[test]
    fn rust_binary_typo_suggests_rust_cli() {
        let overrides = overrides_with_pin("crates/foo", "kind", "rust-binary");
        let report = validate_overrides(&overrides);
        let err = report.errors().next().expect("error expected");
        assert_eq!(err.suggestion.as_deref(), Some("rust-cli"));
    }

    #[test]
    fn legacy_node_package_suggests_node_library() {
        let overrides = overrides_with_pin("frontend/foo", "kind", "node-package");
        let report = validate_overrides(&overrides);
        let err = report.errors().next().expect("error expected");
        assert_eq!(err.suggestion.as_deref(), Some("node-library"));
    }

    #[test]
    fn singular_shell_script_suggests_plural() {
        let overrides = overrides_with_pin("scripts", "kind", "shell-script");
        let report = validate_overrides(&overrides);
        let err = report.errors().next().expect("error expected");
        assert_eq!(err.suggestion.as_deref(), Some("shell-scripts"));
    }

    #[test]
    fn unknown_pin_field_is_a_warning_not_an_error() {
        // `kid:` instead of `kind:` — recognisable typo.
        let overrides = overrides_with_pin("crates/foo", "kid", "rust-library");
        let report = validate_overrides(&overrides);
        assert!(!report.has_errors());
        let warn = report.warnings().next().expect("warning expected");
        assert_eq!(warn.field.as_deref(), Some("kid"));
        assert!(warn.message.contains("unknown pin field"));
    }

    #[test]
    fn addition_with_unknown_kind_is_an_error() {
        let mut overrides = OverridesFile::default();
        overrides.additions.push(ComponentEntry {
            id: "wizard".into(),
            parent: None,
            kind: "rust-binary".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: Vec::new(),
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "n".into(),
            deleted: false,
        });
        let report = validate_overrides(&overrides);
        let err = report.errors().next().expect("error expected");
        assert!(err.message.contains("wizard"));
        assert_eq!(err.suggestion.as_deref(), Some("rust-cli"));
    }

    #[test]
    fn language_and_role_pins_are_not_kind_validated() {
        let overrides = overrides_with_pin("crates/foo", "language", "klingon");
        let report = validate_overrides(&overrides);
        // `language` is free-form; no error.
        assert!(!report.has_errors());
    }

    use atlas_index::{SubsystemOverride, SubsystemsOverridesFile};

    fn subsystem_override(id: &str, members: Vec<String>) -> SubsystemOverride {
        SubsystemOverride {
            id: id.into(),
            members,
            role: None,
            lifecycle_roles: vec![],
            rationale: "x".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
        }
    }

    #[test]
    fn validate_subsystems_flags_duplicate_ids() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![
                subsystem_override("auth", vec!["x".into()]),
                subsystem_override("auth", vec!["y".into()]),
            ],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(
            report
                .errors()
                .any(|i| i.message.contains("duplicate subsystem id 'auth'")),
            "expected duplicate-id error, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn validate_subsystems_flags_empty_members() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![subsystem_override("auth", vec![])],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(
            report
                .errors()
                .any(|i| i.message.contains("subsystem 'auth' has empty members")),
            "expected empty-members error, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn validate_subsystems_passes_well_formed_input() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![subsystem_override("auth", vec!["services/auth/*".into()])],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(
            !report.has_errors(),
            "expected no errors, got: {:?}",
            report.issues
        );
    }
}
