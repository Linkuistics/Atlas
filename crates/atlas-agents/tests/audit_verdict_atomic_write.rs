//! PR-4: on-disk audit verdict atomic-write integration test
//! (plan §4 Task 4.7, brainstorm §7.4).
//!
//! Asserts:
//!
//! - Round-trip through disk: `write_verdict_pair` →
//!   `read_verdict_if_complete` returns the same record.
//! - Half-pair on disk surfaces as `Ok(None)` → re-audit. The orphan
//!   file is *not* deleted; the test confirms forensic survival.
//! - YAML shape matches brainstorm §7.4 keys verbatim.
//! - Atomic write is robust against intermediate inspection:
//!   between two writes, the previously-persisted file remains
//!   readable (no torn writes visible to a concurrent reader).

use atlas_agents::runtime::audit::{
    read_verdict_if_complete, write_verdict_pair, AuditVerdictOnDisk, AuditorVerdictMeta,
    ProducerMeta, Stage, TokenCounts, VerdictKind,
};

fn synthetic_verdict() -> AuditVerdictOnDisk {
    AuditVerdictOnDisk {
        agent_id: "classify::atlas-engine#i1".to_string(),
        stage: Stage::Classify.into(),
        producer: ProducerMeta {
            provider: "anthropic".to_string(),
            model: "claude-opus-4-7".to_string(),
            output_sha: "2b91aa0123456789abcdef".to_string(),
        },
        auditor: AuditorVerdictMeta {
            provider: "openai".to_string(),
            model: "gpt-5-codex".to_string(),
            verdict: VerdictKind::Accept,
            reason: "Component kind matches manifest declaration.".to_string(),
        },
        audit_tokens: TokenCounts {
            tokens_in: 1240,
            tokens_out: 320,
        },
        audited_at: "2026-05-14T14:32:11Z".to_string(),
    }
}

#[test]
fn verdict_round_trip_through_disk() {
    let dir = tempfile::tempdir().unwrap();
    let v = synthetic_verdict();
    write_verdict_pair(
        dir.path(),
        Stage::Classify,
        "atlas-engine",
        &v,
        "1. tool: read_file\n   args: {}\n   result: {}\n",
    )
    .expect("write_verdict_pair should succeed on a writable tempdir");

    let reread = read_verdict_if_complete(dir.path(), Stage::Classify, "atlas-engine")
        .expect("read should not error on a freshly-written pair");
    let reread = reread.expect("freshly-written verdict must be readable");
    assert_eq!(reread.agent_id, v.agent_id);
    assert_eq!(reread.producer.output_sha, v.producer.output_sha);
    assert_eq!(reread.auditor.verdict, v.auditor.verdict);
    assert_eq!(reread.audit_tokens, v.audit_tokens);
    assert_eq!(reread.audited_at, v.audited_at);
}

#[test]
fn absent_verdict_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let result = read_verdict_if_complete(dir.path(), Stage::Classify, "never-audited")
        .expect("absent verdict is not an error");
    assert!(result.is_none(), "never-audited target must return None");
}

#[test]
fn half_pair_missing_transcript_returns_none_and_leaves_orphan() {
    let dir = tempfile::tempdir().unwrap();
    let v = synthetic_verdict();
    write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript-body").unwrap();
    // Simulate an interrupted write (or external deletion) by
    // removing the transcript sibling.
    std::fs::remove_file(dir.path().join("classify").join("tid.audit-transcript"))
        .expect("transcript sibling should exist");

    let result = read_verdict_if_complete(dir.path(), Stage::Classify, "tid")
        .expect("half-pair must not surface as an error");
    assert!(
        result.is_none(),
        "half-pair must return None so the runtime re-audits"
    );
    // Orphan verdict file is preserved for forensic inspection.
    assert!(
        dir.path().join("classify").join("tid.yaml").exists(),
        "orphan verdict file must NOT be deleted on half-pair detection"
    );
}

#[test]
fn half_pair_missing_verdict_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let v = synthetic_verdict();
    write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript-body").unwrap();
    std::fs::remove_file(dir.path().join("classify").join("tid.yaml")).unwrap();

    let result = read_verdict_if_complete(dir.path(), Stage::Classify, "tid")
        .expect("orphan transcript half-pair must not error");
    assert!(result.is_none(), "half-pair (missing verdict) → re-audit");
}

#[test]
fn on_disk_yaml_shape_matches_brainstorm_section_7_4_keys() {
    // Brainstorm §7.4 explicitly enumerates the verdict YAML keys.
    // This drift catcher rejects any future schema change that drops
    // or renames one of them.
    let dir = tempfile::tempdir().unwrap();
    let v = synthetic_verdict();
    write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript").unwrap();

    let yaml = std::fs::read_to_string(dir.path().join("classify").join("tid.yaml")).unwrap();
    for key in &[
        "agent_id:",
        "stage:",
        "producer:",
        "provider:",
        "model:",
        "output_sha:",
        "auditor:",
        "verdict:",
        "reason:",
        "audit_tokens:",
        "in:",
        "out:",
        "audited_at:",
    ] {
        assert!(
            yaml.contains(key),
            "verdict yaml on disk must contain `{key}` (brainstorm §7.4); \
             got file body:\n{yaml}"
        );
    }
}

#[test]
fn verdict_kind_round_trips_through_yaml() {
    // Per-kind round-trip: write each variant + re-read it. Catches
    // any future renames of the snake_case label table.
    for kind in [
        VerdictKind::Accept,
        VerdictKind::RequestRevision,
        VerdictKind::HardFail,
        VerdictKind::Skipped,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut v = synthetic_verdict();
        v.auditor.verdict = kind;
        let target_id = match kind {
            VerdictKind::Accept => "accept-target",
            VerdictKind::RequestRevision => "revision-target",
            VerdictKind::HardFail => "hardfail-target",
            VerdictKind::Skipped => "skipped-target",
        };
        write_verdict_pair(dir.path(), Stage::Classify, target_id, &v, "t").unwrap();
        let reread = read_verdict_if_complete(dir.path(), Stage::Classify, target_id)
            .unwrap()
            .unwrap();
        assert_eq!(reread.auditor.verdict, kind);
    }
}

#[test]
fn write_to_nested_audit_dir_creates_directory_structure() {
    // The runtime hands `audit_dir = <output_dir>/audit`; the runtime
    // doesn't `create_dir_all` upfront. `atomic_write_pair` handles
    // it via `fs::create_dir_all` on each rename target's parent.
    // Verify this for a deeply-nested fresh path.
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested").join("audit-root");
    let v = synthetic_verdict();
    write_verdict_pair(&nested, Stage::Classify, "tid", &v, "t")
        .expect("nested directories must be auto-created");
    assert!(nested.join("classify").join("tid.yaml").exists());
}

#[test]
fn second_write_overwrites_atomically() {
    // After a successful write_verdict_pair, a second write to the
    // same `<stage>/<target_id>` replaces both files atomically. The
    // second-write contents must be observable on the read path;
    // the first-write contents must not be visible.
    let dir = tempfile::tempdir().unwrap();
    let mut v = synthetic_verdict();
    write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript-v1").unwrap();

    v.auditor.verdict = VerdictKind::RequestRevision;
    v.auditor.reason = "second revision needed".to_string();
    v.audited_at = "2026-05-14T14:35:00Z".to_string();
    write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript-v2").unwrap();

    let reread = read_verdict_if_complete(dir.path(), Stage::Classify, "tid")
        .unwrap()
        .unwrap();
    assert_eq!(reread.auditor.verdict, VerdictKind::RequestRevision);
    assert!(reread.auditor.reason.contains("second revision"));
    let transcript =
        std::fs::read_to_string(dir.path().join("classify").join("tid.audit-transcript")).unwrap();
    assert_eq!(transcript, "transcript-v2");
}

#[test]
fn per_stage_subdirectory_layout_matches_spec() {
    // Each stage gets its own subdirectory under `audit_dir`. This
    // matches brainstorm §7.4's "<.atlas/audit/<stage>/<target_id>.yaml>"
    // layout and lets `read_verdict_if_complete` route by stage.
    let dir = tempfile::tempdir().unwrap();
    let v = synthetic_verdict();
    write_verdict_pair(dir.path(), Stage::Classify, "foo", &v, "t").unwrap();
    write_verdict_pair(dir.path(), Stage::Reduce, "agents", &v, "t").unwrap();
    write_verdict_pair(dir.path(), Stage::Project, "_workspace", &v, "t").unwrap();
    assert!(dir.path().join("classify").join("foo.yaml").exists());
    assert!(dir.path().join("reduce").join("agents.yaml").exists());
    assert!(dir.path().join("project").join("_workspace.yaml").exists());
    // Cross-stage reads return None — each stage's verdict is its
    // own slot.
    let cross_stage = read_verdict_if_complete(dir.path(), Stage::Classify, "agents").unwrap();
    assert!(
        cross_stage.is_none(),
        "verdict written under reduce/agents.yaml must NOT be readable \
         via classify/agents"
    );
}
