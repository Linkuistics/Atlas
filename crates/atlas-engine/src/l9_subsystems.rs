//! L9 subsystem projection — resolves hand-authored subsystem overrides
//! against the live component tree and emits a `SubsystemsFile`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use atlas_index::{
    ComponentEntry, MemberEvidence, SubsystemEntry, SubsystemOverride, SubsystemsFile,
    SUBSYSTEMS_SCHEMA_VERSION,
};
use globset::{Glob, GlobMatcher};

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;

/// Produce `subsystems.yaml` from the workspace input + live components.
/// `generated_at` is left empty; the CLI stamps the wall clock at write
/// time. Salsa-side stable output preserves byte-identity on no-op
/// re-runs.
pub fn subsystems_yaml_snapshot(db: &AtlasDatabase) -> Arc<SubsystemsFile> {
    let ws = db.workspace();
    let overrides = ws.subsystems_overrides(db as &dyn salsa::Database).clone();
    let components = all_components(db);
    let resolved = resolve_subsystems(&overrides.subsystems, &components);
    Arc::new(SubsystemsFile {
        schema_version: SUBSYSTEMS_SCHEMA_VERSION,
        generated_at: String::new(),
        subsystems: resolved,
    })
}

/// Pure resolution helper, factored out so it can be tested without a
/// full `AtlasDatabase`. Inputs: the override list + every non-deleted
/// component. Output: one `SubsystemEntry` per override, with members
/// resolved to component ids.
pub(crate) fn resolve_subsystems(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Vec<SubsystemEntry> {
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();
    let by_id: BTreeMap<&str, &ComponentEntry> = live.iter().map(|c| (c.id.as_str(), *c)).collect();
    overrides
        .iter()
        .map(|sub| resolve_one_subsystem(sub, &live, &by_id))
        .collect()
}

fn resolve_one_subsystem(
    sub: &SubsystemOverride,
    live: &[&ComponentEntry],
    by_id: &BTreeMap<&str, &ComponentEntry>,
) -> SubsystemEntry {
    let mut resolved_ids: BTreeSet<String> = BTreeSet::new();
    let mut evidence: Vec<MemberEvidence> = Vec::new();

    for member in &sub.members {
        if is_glob_form(member) {
            let matcher = match Glob::new(member) {
                Ok(g) => g.compile_matcher(),
                Err(_) => {
                    evidence.push(MemberEvidence {
                        id: String::new(),
                        matched_via: format!("{member} (invalid glob)"),
                    });
                    continue;
                }
            };
            let matches = match_glob(&matcher, live);
            if matches.is_empty() {
                evidence.push(MemberEvidence {
                    id: String::new(),
                    matched_via: format!("{member} (no matches)"),
                });
            } else {
                for c in matches {
                    if resolved_ids.insert(c.id.clone()) {
                        evidence.push(MemberEvidence {
                            id: c.id.clone(),
                            matched_via: member.clone(),
                        });
                    }
                }
            }
        } else if let Some(c) = by_id.get(member.as_str()) {
            if resolved_ids.insert(c.id.clone()) {
                evidence.push(MemberEvidence {
                    id: c.id.clone(),
                    matched_via: "id".into(),
                });
            }
        } else {
            // Unknown id — caller surfaces this as a hard error in the
            // post-L4 validation pass. Record it in evidence so the
            // projection is self-describing even if validation is
            // skipped.
            evidence.push(MemberEvidence {
                id: member.clone(),
                matched_via: "id (no such component)".into(),
            });
        }
    }

    let members: Vec<String> = resolved_ids.into_iter().collect();
    let mut notes: Vec<String> = Vec::new();
    if members.is_empty() {
        notes.push("all members unresolved".into());
    }

    SubsystemEntry {
        id: sub.id.clone(),
        role: sub.role.clone(),
        lifecycle_roles: sub.lifecycle_roles.clone(),
        rationale: sub.rationale.clone(),
        evidence_grade: sub.evidence_grade,
        evidence_fields: sub.evidence_fields.clone(),
        members,
        member_evidence: evidence,
        notes,
    }
}

fn is_glob_form(member: &str) -> bool {
    member.contains('/') || member.contains('*')
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
/// member does not resolve to any component. Hard error in the post-L4
/// validation stage.
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
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: None,
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

    #[test]
    fn empty_input_yields_empty_output() {
        let out = resolve_subsystems(&[], &[]);
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
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].members, vec!["auth-tools"]);
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/auth/*");
    }

    #[test]
    fn id_form_resolves_directly() {
        let comps = vec![comp("identity-core", "libs/identity")];
        let subs = vec![override_with_members("auth", vec!["identity-core".into()])];
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out[0].members, vec!["identity-core"]);
        assert_eq!(out[0].member_evidence[0].matched_via, "id");
    }

    #[test]
    fn glob_with_zero_matches_emits_no_matches_evidence() {
        let comps = vec![comp("storage", "services/storage")];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth/*".into()],
        )];
        let out = resolve_subsystems(&subs, &comps);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
        assert_eq!(
            out[0].member_evidence[0].matched_via,
            "services/auth/* (no matches)"
        );
    }

    #[test]
    fn unknown_id_form_emits_no_such_component_evidence() {
        let subs = vec![override_with_members("auth", vec!["nonexistent".into()])];
        let out = resolve_subsystems(&subs, &[]);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
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
            vec!["services/auth".into(), "auth-service".into()],
        )];
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out[0].members, vec!["auth-service"]);
        // First form ("services/auth") wins; second is a no-op dedupe.
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/auth");
    }

    #[test]
    fn deleted_components_are_skipped() {
        let mut comps = vec![comp("auth-service", "services/auth")];
        comps[0].deleted = true;
        let subs = vec![override_with_members("auth", vec!["auth-service".into()])];
        let out = resolve_subsystems(&subs, &comps);
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
}
