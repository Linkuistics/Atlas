//! L9 subsystem projection — resolves hand-authored subsystem overrides
//! against the live component tree and emits a `SubsystemsFile`.
//!
//! ## Precedence (Phase 6 PR-3)
//!
//! Subsystem assignment is now layered: a per-component override file
//! (`<path>/.atlas/components.overrides.yaml` with
//! `field_overrides.subsystem: <id>`) wins over a central
//! `subsystems.overrides.yaml` entry that lists the same component.
//! The rationale is closer-to-source authoring (LLM-spine recast spec
//! §4.1): the per-component file lives next to the component code,
//! while the central file sits at workspace root. Users who want the
//! central definition to win simply edit the central file directly.
//!
//! ## Warning class: `SubsystemOverrideNonExistent`
//!
//! When the central `subsystems.overrides.yaml` lists an id-form
//! `members:` entry that does not resolve to any extant component,
//! [`resolve_subsystems`] emits a warning to the supplied writer and
//! skips the entry. Per-component overrides cannot trigger this
//! warning by construction — the override file is co-located with the
//! component, so the component must exist for the file to be found.
//!
//! The warning is currently emitted via `writeln!(&mut dyn Write, ...)`;
//! Phase 6 PR-4 will refactor that into a structured
//! `collector.emit()` call.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use atlas_index::{
    ComponentEntry, MemberEvidence, SubsystemEntry, SubsystemOverride, SubsystemsFile,
    SUBSYSTEMS_SCHEMA_VERSION,
};
use component_ontology::ComponentId;
use globset::{Glob, GlobMatcher};

use crate::db::AtlasDatabase;
use crate::l4_tree::{all_components, per_component_subsystem_overrides};

/// Audit-trail note attached to a [`SubsystemEntry`] when, after override
/// application, none of its member references resolved to a live
/// component. Centralised so the three call sites that read or write
/// this note (the central-resolution emit in [`resolve_one_subsystem`],
/// the per-component-overlay re-application in [`resolve_subsystems`],
/// and the clear-on-refill branch in the same function) cannot drift.
const NOTE_ALL_UNRESOLVED: &str = "all members unresolved";

/// Append [`NOTE_ALL_UNRESOLVED`] to `entry.notes` iff not already
/// present. Idempotent so the per-component overlay can re-assert the
/// note when a central entry loses its sole member to a higher-priority
/// per-component override.
fn mark_unresolved(entry: &mut SubsystemEntry) {
    if !entry.notes.iter().any(|n| n == NOTE_ALL_UNRESOLVED) {
        entry.notes.push(NOTE_ALL_UNRESOLVED.to_string());
    }
}

/// Drop any prior [`NOTE_ALL_UNRESOLVED`] from `entry.notes`. Used when
/// a previously-empty subsystem entry is refilled by a per-component
/// override so the stale note does not survive into the rendered YAML.
fn clear_unresolved(entry: &mut SubsystemEntry) {
    entry.notes.retain(|n| n != NOTE_ALL_UNRESOLVED);
}

/// Produce `subsystems.yaml` from the workspace input + live components.
/// `generated_at` is left empty; the CLI stamps the wall clock at write
/// time. Salsa-side stable output preserves byte-identity on no-op
/// re-runs.
///
/// Warnings (including the PR-3 `SubsystemOverrideNonExistent` class)
/// are routed to `io::stderr()` by default; callers that want to capture
/// the warning stream should use [`subsystems_yaml_snapshot_with_warnings`].
pub fn subsystems_yaml_snapshot(db: &AtlasDatabase) -> Arc<SubsystemsFile> {
    subsystems_yaml_snapshot_with_warnings(db, &mut io::stderr())
}

/// As [`subsystems_yaml_snapshot`] but routes warnings to `warnings`
/// instead of `io::stderr()`. Used by the CLI pipeline so a session-
/// scoped warning buffer can capture the PR-3
/// `SubsystemOverrideNonExistent` notes without process plumbing,
/// and by tests that need to assert on the warning text.
pub fn subsystems_yaml_snapshot_with_warnings(
    db: &AtlasDatabase,
    warnings: &mut dyn Write,
) -> Arc<SubsystemsFile> {
    let ws = db.workspace();
    let overrides = ws.subsystems_overrides(db as &dyn salsa::Database).clone();
    let components = all_components(db);
    let per_component = per_component_subsystem_overrides(db);
    let resolved = resolve_subsystems(&overrides.subsystems, &components, &per_component, warnings);
    Arc::new(SubsystemsFile {
        schema_version: SUBSYSTEMS_SCHEMA_VERSION,
        generated_at: String::new(),
        subsystems: resolved,
    })
}

/// Pure resolution helper, factored out so it can be tested without a
/// full `AtlasDatabase`. Inputs:
///
/// - `overrides` — the central `subsystems.overrides.yaml` entries.
/// - `components` — every component in the live tree (deleted ones are
///   filtered internally).
/// - `per_component_overrides` — map from component id to the subsystem
///   name authored in `<path>/.atlas/components.overrides.yaml`'s
///   `field_overrides.subsystem` block.
/// - `warnings` — writer for the PR-3 `SubsystemOverrideNonExistent`
///   warning class, emitted when a central `members:` entry references
///   a non-existent component id.
///
/// Output: one `SubsystemEntry` per defined subsystem, with members
/// resolved to component ids. Empty subsystems (every member dropped
/// after override application) are removed from the output.
///
/// Precedence rule (PR-3): per-component overrides win over central
/// entries. A component that the central file places in subsystem `B`
/// and that its per-component file places in subsystem `A` ends up in
/// `A` only; `B` no longer references it.
pub(crate) fn resolve_subsystems(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
    per_component_overrides: &BTreeMap<ComponentId, String>,
    warnings: &mut dyn Write,
) -> Vec<SubsystemEntry> {
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();
    let by_id: BTreeMap<&str, &ComponentEntry> = live.iter().map(|c| (c.id.as_str(), *c)).collect();

    // 1. Resolve central overrides (lower precedence).
    let mut resolved: Vec<SubsystemEntry> = overrides
        .iter()
        .map(|sub| resolve_one_subsystem(sub, &live, &by_id, warnings))
        .collect();

    // 2. Apply per-component overrides on top — closer-to-source wins.
    //    For each (component_id, subsystem_name) pair:
    //      a) drop the component from any other subsystem that contains it,
    //      b) ensure the named subsystem contains it (creating the
    //         subsystem entry from scratch if no central entry exists).
    for (component_id, subsystem_name) in per_component_overrides {
        // a) Remove the component (and its evidence) from every OTHER
        //    subsystem. A central entry that loses its sole member to
        //    a per-component override remains in the output as an
        //    empty entry with the existing "all members unresolved"
        //    note re-applied (so YAML readers see why beta is empty).
        for entry in &mut resolved {
            if entry.id == *subsystem_name {
                continue;
            }
            let had_member = entry.members.iter().any(|m| m == component_id);
            entry.members.retain(|m| m != component_id);
            entry
                .member_evidence
                .retain(|e| e.id != component_id.as_str());
            if had_member && entry.members.is_empty() {
                mark_unresolved(entry);
            }
        }

        // b) Add the component to the named subsystem. Create the
        //    entry if no central definition exists.
        if let Some(target) = resolved.iter_mut().find(|s| s.id == *subsystem_name) {
            if !target.members.contains(component_id) {
                target.members.push(component_id.clone());
                target.member_evidence.push(MemberEvidence {
                    id: component_id.as_str().to_string(),
                    matched_via: "per-component override".into(),
                });
                // Sort to preserve the determinism guarantee on `members`.
                target.members.sort();
            }
            // A subsystem that was previously empty-then-refilled
            // should drop the stale "all members unresolved" note.
            clear_unresolved(target);
        } else {
            resolved.push(SubsystemEntry {
                id: subsystem_name.clone(),
                role: None,
                lifecycle_roles: vec![],
                rationale: String::new(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
                members: vec![component_id.clone()],
                member_evidence: vec![MemberEvidence {
                    id: component_id.as_str().to_string(),
                    matched_via: "per-component override".into(),
                }],
                notes: vec![],
            });
        }
    }

    resolved
}

fn resolve_one_subsystem(
    sub: &SubsystemOverride,
    live: &[&ComponentEntry],
    by_id: &BTreeMap<&str, &ComponentEntry>,
    warnings: &mut dyn Write,
) -> SubsystemEntry {
    let mut resolved_ids: BTreeSet<ComponentId> = BTreeSet::new();
    let mut evidence: Vec<MemberEvidence> = Vec::new();

    for member in &sub.members {
        if is_glob_form(member) {
            let matcher = match Glob::new(member) {
                Ok(g) => g.compile_matcher(),
                Err(_) => {
                    // Record the failure faithfully: id carries the
                    // source member string, matched_via tags the
                    // failure mode. MemberEvidence.id is a String
                    // precisely so this audit trail can survive
                    // unresolved members.
                    evidence.push(MemberEvidence {
                        id: member.clone(),
                        matched_via: format!("{member} (invalid glob)"),
                    });
                    continue;
                }
            };
            let matches = match_glob(&matcher, live);
            if matches.is_empty() {
                evidence.push(MemberEvidence {
                    id: member.clone(),
                    matched_via: format!("{member} (no matches)"),
                });
            } else {
                for c in matches {
                    if resolved_ids.insert(c.id.clone()) {
                        evidence.push(MemberEvidence {
                            id: c.id.as_str().to_string(),
                            matched_via: member.clone(),
                        });
                    }
                }
            }
        } else if let Some(c) = by_id.get(member.as_str()) {
            if resolved_ids.insert(c.id.clone()) {
                evidence.push(MemberEvidence {
                    id: c.id.as_str().to_string(),
                    matched_via: "id".into(),
                });
            }
        } else {
            // PR-3: SubsystemOverrideNonExistent warning class. A
            // central `members:` entry that names an id-form component
            // which does not exist in the workspace used to be a hard
            // error in the post-L4 validation pass; PR-3 downgrades it
            // to a warning so the run can complete (Phase 6 plan
            // §4 PR-3, plus the warning-channel discipline described
            // in this module's top-level docstring). Phase 6 PR-4 will
            // route this through a structured warning collector and add
            // a `--strict-overrides` flag that turns it back into an
            // error.
            let _ = writeln!(
                warnings,
                "warning: subsystems.overrides.yaml references component `{}` in subsystem `{}` but no such component exists in the workspace — override entry does not apply (no extant component)",
                member, sub.id
            );
            // Preserve the audit-trail entry in evidence so the YAML is
            // still self-describing even when the warning is unseen.
            evidence.push(MemberEvidence {
                id: member.clone(),
                matched_via: "id (no such component)".into(),
            });
        }
    }

    let members: Vec<ComponentId> = resolved_ids.into_iter().collect();
    let mut entry = SubsystemEntry {
        id: sub.id.clone(),
        role: sub.role.clone(),
        lifecycle_roles: sub.lifecycle_roles.clone(),
        rationale: sub.rationale.clone(),
        evidence_grade: sub.evidence_grade,
        evidence_fields: sub.evidence_fields.clone(),
        members,
        member_evidence: evidence,
        notes: Vec::new(),
    };
    if entry.members.is_empty() {
        mark_unresolved(&mut entry);
    }
    entry
}

/// A member entry is a glob iff it contains a glob metacharacter
/// (`*`, `?`, or `[`). Component ids are now path-shaped (segments
/// joined by `/`), so the slash alone no longer disambiguates.
fn is_glob_form(member: &str) -> bool {
    member.contains('*') || member.contains('?') || member.contains('[')
}

fn match_glob<'a>(
    matcher: &GlobMatcher,
    live: &'a [&'a ComponentEntry],
) -> Vec<&'a ComponentEntry> {
    live.iter()
        .copied()
        .filter(|c| {
            c.path_segments
                .iter()
                .any(|seg| matcher.is_match(Path::new(&seg.path)))
        })
        .collect()
}

/// Returns the sorted set of subsystem ids that collide with component ids.
/// Hard error in the post-L4 validation stage.
pub fn check_subsystem_namespace(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Result<(), Vec<String>> {
    let component_ids: BTreeSet<&str> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    let subsystem_ids: BTreeSet<&str> = overrides.iter().map(|s| s.id.as_str()).collect();
    let mut collisions: Vec<String> = component_ids
        .intersection(&subsystem_ids)
        .map(|s| (*s).to_string())
        .collect();
    if collisions.is_empty() {
        Ok(())
    } else {
        collisions.sort();
        Err(collisions)
    }
}

/// Returns the sorted `<subsystem-id>/<member-id>` pairs whose id-form
/// member does not resolve to any component.
///
/// Phase 6 PR-3 demoted the runtime use of this check from a hard
/// error to a soft warning emitted directly by [`resolve_subsystems`];
/// the function itself is retained for the future
/// `--strict-overrides` flag (Phase 6 PR-4) and for direct
/// programmatic use by embedders that want pre-flight validation.
pub fn check_subsystem_id_members(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Result<(), Vec<String>> {
    let component_ids: BTreeSet<&str> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    let mut bad: Vec<String> = Vec::new();
    for sub in overrides {
        for member in &sub.members {
            if !is_glob_form(member) && !component_ids.contains(member.as_str()) {
                bad.push(format!("{}/{}", sub.id, member));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        bad.sort();
        Err(bad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::PathSegment;
    use component_ontology::EvidenceGrade;
    use std::path::PathBuf;

    fn comp(id: &str, path: &str) -> ComponentEntry {
        ComponentEntry {
            id: ComponentId::parse(id).unwrap(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            languages: std::collections::BTreeSet::new(),
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from(path),
                content_sha: "0".repeat(64),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: "x".into(),
            deleted: false,
        }
    }

    fn override_with_members(id: &str, members: Vec<String>) -> SubsystemOverride {
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

    /// Convenience: call `resolve_subsystems` with an empty per-component
    /// overlay and a sink that discards warnings, mirroring the pre-PR-3
    /// signature for the suite of existing pure-resolution tests.
    fn resolve(
        overrides: &[SubsystemOverride],
        components: &[ComponentEntry],
    ) -> Vec<SubsystemEntry> {
        let per_component: BTreeMap<ComponentId, String> = BTreeMap::new();
        let mut sink: Vec<u8> = Vec::new();
        resolve_subsystems(overrides, components, &per_component, &mut sink)
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let out = resolve(&[], &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn glob_resolves_against_path_segments() {
        let comps = vec![
            comp("auth-service", "services/auth"),
            comp("auth-tools", "services/auth/tools"),
            comp("storage", "services/storage"),
        ];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth/*".into()],
        )];
        let out = resolve(&subs, &comps);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].members,
            vec![ComponentId::parse("auth-tools").unwrap()]
        );
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/auth/*");
    }

    #[test]
    fn id_form_resolves_directly() {
        let comps = vec![comp("identity-core", "libs/identity")];
        let subs = vec![override_with_members("auth", vec!["identity-core".into()])];
        let out = resolve(&subs, &comps);
        assert_eq!(
            out[0].members,
            vec![ComponentId::parse("identity-core").unwrap()]
        );
        assert_eq!(out[0].member_evidence[0].matched_via, "id");
    }

    #[test]
    fn multi_segment_component_id_resolves_as_id_not_glob() {
        // Regression: `is_glob_form` once treated any `/` as a glob, which
        // misclassified path-shaped component ids (e.g. `atlas/atlas-cli`)
        // as glob expressions and routed them to the glob matcher. Globs
        // are now disambiguated by the presence of `*`, `?`, or `[`, so
        // a multi-segment id resolves through `by_id`.
        let comps = vec![comp("atlas/atlas-cli", "crates/atlas-cli")];
        let subs = vec![override_with_members(
            "atlas",
            vec!["atlas/atlas-cli".into()],
        )];
        let out = resolve(&subs, &comps);
        assert_eq!(
            out[0].members,
            vec![ComponentId::parse("atlas/atlas-cli").unwrap()]
        );
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].id, "atlas/atlas-cli");
        assert_eq!(out[0].member_evidence[0].matched_via, "id");
        assert!(out[0].notes.is_empty());
    }

    #[test]
    fn glob_with_zero_matches_emits_evidence_with_no_matches() {
        let comps = vec![comp("storage", "services/storage")];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth/*".into()],
        )];
        let out = resolve(&subs, &comps);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].id, "services/auth/*");
        assert_eq!(
            out[0].member_evidence[0].matched_via,
            "services/auth/* (no matches)"
        );
    }

    #[test]
    fn unknown_id_form_emits_no_such_component_evidence() {
        let subs = vec![override_with_members("auth", vec!["nonexistent".into()])];
        let out = resolve(&subs, &[]);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].id, "nonexistent");
        assert_eq!(
            out[0].member_evidence[0].matched_via,
            "id (no such component)"
        );
    }

    #[test]
    fn duplicate_glob_matches_dedupe_in_members_but_keep_evidence_first_form() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/*".into(), "auth-service".into()],
        )];
        let out = resolve(&subs, &comps);
        assert_eq!(
            out[0].members,
            vec![ComponentId::parse("auth-service").unwrap()]
        );
        // First form ("services/*") wins; second is a no-op dedupe.
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/*");
    }

    #[test]
    fn deleted_components_are_skipped() {
        let mut comps = vec![comp("auth-service", "services/auth")];
        comps[0].deleted = true;
        let subs = vec![override_with_members("auth", vec!["auth-service".into()])];
        let out = resolve(&subs, &comps);
        assert!(out[0].members.is_empty());
    }

    #[test]
    fn collision_check_passes_when_disjoint() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members("auth", vec![])];
        let result = check_subsystem_namespace(&subs, &comps);
        assert!(result.is_ok());
    }

    #[test]
    fn collision_check_reports_id_clash() {
        let comps = vec![comp("auth", "services/auth")];
        let subs = vec![override_with_members("auth", vec![])];
        let err = check_subsystem_namespace(&subs, &comps).unwrap_err();
        assert_eq!(err, vec!["auth"]);
    }

    #[test]
    fn collision_check_reports_unknown_id_form_member() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members("auth", vec!["nonexistent".into()])];
        let err = check_subsystem_id_members(&subs, &comps).unwrap_err();
        assert_eq!(err, vec!["auth/nonexistent".to_string()]);
    }

    #[test]
    fn collision_check_id_member_present_passes() {
        let comps = vec![comp("identity-core", "libs/identity")];
        let subs = vec![override_with_members("auth", vec!["identity-core".into()])];
        assert!(check_subsystem_id_members(&subs, &comps).is_ok());
    }

    // -----------------------------------------------------------------
    // Phase 6 PR-3: per-component subsystem overlay precedence rule.
    // -----------------------------------------------------------------

    #[test]
    fn per_component_subsystem_override_wins_over_central() {
        // Closer-to-source authoring: a per-component
        // `<path>/.atlas/components.overrides.yaml` with
        // `field_overrides.subsystem: alpha` displaces a central
        // `subsystems.overrides.yaml` entry that places the same
        // component in subsystem `beta`. The component lives in
        // `alpha`; `beta` no longer references it.
        let comps = vec![comp("comp-a", "comps/a")];
        let central = vec![override_with_members("beta", vec!["comp-a".into()])];
        let mut per_component: BTreeMap<ComponentId, String> = BTreeMap::new();
        per_component.insert(ComponentId::parse("comp-a").unwrap(), "alpha".into());
        let mut warnings: Vec<u8> = Vec::new();

        let resolved = resolve_subsystems(&central, &comps, &per_component, &mut warnings);

        let alpha = resolved
            .iter()
            .find(|s| s.id == "alpha")
            .expect("alpha subsystem must exist after per-component overlay");
        assert!(
            alpha
                .members
                .iter()
                .any(|m| m == &ComponentId::parse("comp-a").unwrap()),
            "expected comp-a in alpha; got members={:?}",
            alpha.members
        );
        let beta = resolved.iter().find(|s| s.id == "beta");
        if let Some(beta) = beta {
            assert!(
                !beta
                    .members
                    .iter()
                    .any(|m| m == &ComponentId::parse("comp-a").unwrap()),
                "expected comp-a NOT in beta after per-component overlay; got beta members={:?}",
                beta.members
            );
        }
    }

    #[test]
    fn central_referencing_nonexistent_component_emits_warning() {
        // PR-3 demotes the id-form-not-found case to a warning. The
        // resolution still produces a `SubsystemEntry` (with no
        // members; the existing `"all members unresolved"` note
        // remains), and the warning channel carries a human-readable
        // line naming the offending id and its subsystem.
        let comps: Vec<ComponentEntry> = vec![];
        let central = vec![override_with_members("gamma", vec!["missing-comp".into()])];
        let per_component: BTreeMap<ComponentId, String> = BTreeMap::new();
        let mut warnings: Vec<u8> = Vec::new();

        let _ = resolve_subsystems(&central, &comps, &per_component, &mut warnings);

        let text = String::from_utf8(warnings).unwrap();
        assert!(
            text.contains("missing-comp"),
            "warning must echo the missing component id: {text}"
        );
        assert!(
            text.contains("gamma"),
            "warning must reference the subsystem id: {text}"
        );
        assert!(
            text.contains("no extant"),
            "warning must indicate non-existence (substring `no extant`): {text}"
        );
    }

    #[test]
    fn per_component_override_creates_subsystem_when_central_silent() {
        // No central entry, just a per-component override. The
        // overlay creates a fresh `SubsystemEntry` for the named
        // subsystem and tags the evidence as `per-component override`.
        let comps = vec![comp("comp-z", "comps/z")];
        let central: Vec<SubsystemOverride> = vec![];
        let mut per_component: BTreeMap<ComponentId, String> = BTreeMap::new();
        per_component.insert(ComponentId::parse("comp-z").unwrap(), "zeta".into());
        let mut warnings: Vec<u8> = Vec::new();

        let resolved = resolve_subsystems(&central, &comps, &per_component, &mut warnings);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "zeta");
        assert_eq!(
            resolved[0].members,
            vec![ComponentId::parse("comp-z").unwrap()]
        );
        assert_eq!(
            resolved[0].member_evidence[0].matched_via,
            "per-component override"
        );
    }
}
