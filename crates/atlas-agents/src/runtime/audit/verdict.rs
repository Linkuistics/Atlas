//! On-disk audit verdict artefact (brainstorm §7.4).
//!
//! Atomic via Phase 7 PR-2's
//! [`atlas_engine::atomic_write::atomic_write_pair`] — the verdict YAML
//! and a companion `.audit-transcript` file are written as a pair so a
//! crash between renames is detectable as a half-pair and triggers
//! re-audit rather than silent staleness.
//!
//! Schema:
//!
//! ```yaml
//! agent_id: classify::atlas-engine#i1
//! stage: classify
//! producer:
//!   provider: anthropic
//!   model: claude-opus-4-7
//!   output_sha: "2b91..."
//! auditor:
//!   provider: openai
//!   model: gpt-5-codex
//!   verdict: accept            # accept | request_revision | hard_fail | skipped
//!   reason: "Producer's component_kind 'library' is consistent ..."
//! audit_tokens:
//!   in: 1240
//!   out: 320
//! audited_at: "2026-05-14T14:32:11Z"
//! ```
//!
//! Identity-shaped string fields (`agent_id`, `output_sha`, `model`,
//! `audited_at`) use [`crate::runtime::yaml_strict::deserialize_string_strict`]
//! so YAML implicit-typing (`output_sha: 1.10` → float; `model: NO` →
//! bool) surfaces as a deserialization error rather than a corrupt
//! verdict file. Memory `feedback_yaml_canonical_interchange` discipline.
//!
//! Re-run replay: on agent re-run, the runtime calls
//! [`read_verdict_if_complete`] before invoking the auditor; if the
//! verdict exists and the producer's `output_sha` matches, the auditor
//! is short-circuited and the verdict is replayed. A changed producer
//! output (different sha) → re-audit. A half-pair on disk → re-audit;
//! the orphan file is left in place for forensic inspection (matches
//! Phase 7 PR-2's cache-eviction stance).

use std::path::{Path, PathBuf};

use atlas_engine::atomic_write::atomic_write_pair;
use serde::{Deserialize, Serialize};

use crate::runtime::audit::Stage;
use crate::runtime::yaml_strict::deserialize_string_strict;

/// The on-disk audit-verdict record. Read + write entry points are
/// [`read_verdict_if_complete`] and [`write_verdict_pair`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditVerdictOnDisk {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub agent_id: String,
    pub stage: StageOnDisk,
    pub producer: ProducerMeta,
    pub auditor: AuditorVerdictMeta,
    pub audit_tokens: TokenCounts,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub audited_at: String,
}

/// Producer-side metadata. `output_sha` is the hex-encoded sha256 of
/// the producer's emitted YAML body — re-run replay compares the
/// current producer's output against this value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProducerMeta {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub provider: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub model: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub output_sha: String,
}

/// Auditor-side metadata: which provider/model rendered the verdict,
/// the verdict kind, and the rationale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditorVerdictMeta {
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub provider: String,
    #[serde(deserialize_with = "deserialize_string_strict")]
    pub model: String,
    pub verdict: VerdictKind,
    /// One-paragraph rationale from the auditor. NOT routed through the
    /// strict-string adapter — the auditor's reason is free-form prose
    /// that may legitimately start with `yes`/`no`/etc., which the
    /// strict adapter would reject. Multi-line block scalars are
    /// supported via YAML's `|` syntax.
    pub reason: String,
}

/// Auditor verdict kind. Mirrors the in-memory
/// [`super::lane_b::AuditVerdict`] enum's three terminal arms (the
/// `Degraded` wrapper is *not* persisted — degraded audits write the
/// inner verdict with `auditor.provider` set to the producer's
/// provider, which is how the on-disk shape encodes "single-provider
/// fallback fired").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Accept,
    RequestRevision,
    HardFail,
    Skipped,
}

/// Audit-call token counts. Authoritative (read from the auditor
/// backend's response metadata), not estimated — PR-5's cold-token
/// totals depend on these being the real numbers.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenCounts {
    #[serde(rename = "in")]
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// Newtype around [`Stage`] for the on-disk YAML shape. [`Stage`]
/// itself is a `#[derive(Serialize, Deserialize)]`-less type
/// (Lane A wants the explicit `as_str` mapping), so we route through
/// this thin wrapper that serializes via the snake_case label table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageOnDisk(pub Stage);

impl From<Stage> for StageOnDisk {
    fn from(s: Stage) -> Self {
        Self(s)
    }
}

impl From<StageOnDisk> for Stage {
    fn from(s: StageOnDisk) -> Self {
        s.0
    }
}

impl Serialize for StageOnDisk {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ser.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for StageOnDisk {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = deserialize_string_strict(de)?;
        let stage = match s.as_str() {
            "dispatch_subsystem" => Stage::DispatchSubsystem,
            "dispatch_component" => Stage::DispatchComponent,
            "classify" => Stage::Classify,
            "surface" => Stage::Surface,
            "reduce" => Stage::Reduce,
            "project" => Stage::Project,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "unknown stage label `{other}` (expected one of: \
                     dispatch_subsystem, dispatch_component, classify, \
                     surface, reduce, project)"
                )))
            }
        };
        Ok(Self(stage))
    }
}

/// Write the verdict + audit-transcript pair atomically. The verdict
/// lands at `<audit_dir>/<stage>/<target_id>.yaml`; the transcript at
/// `<audit_dir>/<stage>/<target_id>.audit-transcript`. Both renames
/// happen via Phase 7 PR-2's `atomic_write_pair` — a crash between the
/// two renames leaves a detectable half-pair (see
/// [`read_verdict_if_complete`]).
pub fn write_verdict_pair(
    audit_dir: &Path,
    stage: Stage,
    target_id: &str,
    verdict: &AuditVerdictOnDisk,
    transcript_text: &str,
) -> Result<(), VerdictWriteError> {
    let stage_dir = audit_dir.join(stage.as_str());
    let verdict_path = stage_dir.join(format!("{target_id}.yaml"));
    let transcript_path = stage_dir.join(format!("{target_id}.audit-transcript"));

    let verdict_yaml = serde_yaml::to_string(verdict)?;

    atomic_write_pair(
        &verdict_path,
        verdict_yaml.as_bytes(),
        &transcript_path,
        transcript_text.as_bytes(),
    )
    .map_err(|source| VerdictWriteError::Io {
        path: verdict_path,
        source,
    })?;
    Ok(())
}

/// Read a previously-written verdict, if one exists. Returns `Ok(None)`
/// when neither file is present (never audited), when only one of the
/// pair is present (half-pair — interrupted write; re-audit), or when
/// the verdict YAML fails to deserialize (stale or hand-edited;
/// re-audit). The half-pair orphan is *not* deleted — leaves it
/// forensically inspectable; user can `rm` manually.
pub fn read_verdict_if_complete(
    audit_dir: &Path,
    stage: Stage,
    target_id: &str,
) -> Result<Option<AuditVerdictOnDisk>, VerdictWriteError> {
    let stage_dir = audit_dir.join(stage.as_str());
    let verdict_path = stage_dir.join(format!("{target_id}.yaml"));
    let transcript_path = stage_dir.join(format!("{target_id}.audit-transcript"));

    let verdict_exists = verdict_path.exists();
    let transcript_exists = transcript_path.exists();

    if !verdict_exists && !transcript_exists {
        return Ok(None);
    }
    if verdict_exists != transcript_exists {
        tracing::warn!(
            verdict_path = ?verdict_path,
            transcript_path = ?transcript_path,
            "audit verdict half-pair detected; treating as cache miss \
             (re-audit will overwrite the orphan)"
        );
        return Ok(None);
    }

    let bytes = std::fs::read_to_string(&verdict_path).map_err(|source| VerdictWriteError::Io {
        path: verdict_path.clone(),
        source,
    })?;
    Ok(Some(serde_yaml::from_str(&bytes)?))
}

/// Failure modes for verdict persistence. `Io` carries the offending
/// path so callers can include it in the error chain.
#[derive(Debug, thiserror::Error)]
pub enum VerdictWriteError {
    #[error("filesystem error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("yaml (de)serialization: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn synthetic_verdict() -> AuditVerdictOnDisk {
        AuditVerdictOnDisk {
            agent_id: "classify::atlas-engine#i1".to_string(),
            stage: StageOnDisk(Stage::Classify),
            producer: ProducerMeta {
                provider: "anthropic".to_string(),
                model: "claude-opus-4-7".to_string(),
                output_sha: "2b91aa".to_string(),
            },
            auditor: AuditorVerdictMeta {
                provider: "openai".to_string(),
                model: "gpt-5-codex".to_string(),
                verdict: VerdictKind::Accept,
                reason: "Producer's component_kind matches the manifest.".to_string(),
            },
            audit_tokens: TokenCounts {
                tokens_in: 1240,
                tokens_out: 320,
            },
            audited_at: "2026-05-14T14:32:11Z".to_string(),
        }
    }

    #[test]
    fn round_trip_through_disk() {
        let dir = TempDir::new().unwrap();
        let v = synthetic_verdict();
        write_verdict_pair(
            dir.path(),
            Stage::Classify,
            "atlas-engine",
            &v,
            "transcript",
        )
        .unwrap();
        let reread = read_verdict_if_complete(dir.path(), Stage::Classify, "atlas-engine").unwrap();
        let reread = reread.expect("verdict should be present after write");
        assert_eq!(reread.agent_id, v.agent_id);
        assert_eq!(reread.producer.output_sha, v.producer.output_sha);
        assert_eq!(reread.auditor.verdict, v.auditor.verdict);
        assert_eq!(reread.audit_tokens, v.audit_tokens);
    }

    #[test]
    fn absent_verdict_returns_none() {
        let dir = TempDir::new().unwrap();
        let result =
            read_verdict_if_complete(dir.path(), Stage::Classify, "never-audited").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn half_pair_missing_transcript_returns_none() {
        let dir = TempDir::new().unwrap();
        let v = synthetic_verdict();
        write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript").unwrap();
        std::fs::remove_file(dir.path().join("classify").join("tid.audit-transcript")).unwrap();
        let result = read_verdict_if_complete(dir.path(), Stage::Classify, "tid").unwrap();
        assert!(
            result.is_none(),
            "half-pair (missing transcript) must surface as None to trigger re-audit"
        );
        // Orphan verdict file is left in place for forensic inspection.
        assert!(dir.path().join("classify").join("tid.yaml").exists());
    }

    #[test]
    fn half_pair_missing_verdict_returns_none() {
        let dir = TempDir::new().unwrap();
        let v = synthetic_verdict();
        write_verdict_pair(dir.path(), Stage::Classify, "tid", &v, "transcript").unwrap();
        std::fs::remove_file(dir.path().join("classify").join("tid.yaml")).unwrap();
        let result = read_verdict_if_complete(dir.path(), Stage::Classify, "tid").unwrap();
        assert!(
            result.is_none(),
            "half-pair (missing verdict) must surface as None"
        );
    }

    #[test]
    fn on_disk_yaml_shape_matches_brainstorm_section_7_4() {
        // Brainstorm §7.4 names the keys explicitly. Verify the
        // serialized form carries every key verbatim.
        let v = synthetic_verdict();
        let yaml = serde_yaml::to_string(&v).unwrap();
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
                "verdict yaml must contain key `{key}` (brainstorm §7.4); got:\n{yaml}"
            );
        }
    }

    #[test]
    fn stage_round_trips_via_label_table() {
        for stage in [
            Stage::DispatchSubsystem,
            Stage::DispatchComponent,
            Stage::Classify,
            Stage::Surface,
            Stage::Reduce,
            Stage::Project,
        ] {
            let on_disk = StageOnDisk(stage);
            let yaml = serde_yaml::to_string(&on_disk).unwrap();
            let parsed: StageOnDisk = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed.0, stage);
        }
    }

    #[test]
    fn unknown_stage_label_rejects_with_actionable_error() {
        let bad = "not_a_stage\n";
        let err = serde_yaml::from_str::<StageOnDisk>(bad).unwrap_err();
        assert!(err.to_string().contains("unknown stage label"));
    }

    #[test]
    fn verdict_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_yaml::to_string(&VerdictKind::Accept).unwrap().trim(),
            "accept"
        );
        assert_eq!(
            serde_yaml::to_string(&VerdictKind::RequestRevision)
                .unwrap()
                .trim(),
            "request_revision"
        );
        assert_eq!(
            serde_yaml::to_string(&VerdictKind::HardFail)
                .unwrap()
                .trim(),
            "hard_fail"
        );
        assert_eq!(
            serde_yaml::to_string(&VerdictKind::Skipped).unwrap().trim(),
            "skipped"
        );
    }

    #[test]
    fn strict_string_adapter_rejects_coerced_output_sha() {
        // Norway-problem hazard for `output_sha: NO` (YAML 1.1 →
        // bool) or `output_sha: 1.10` (→ float). YAML 1.2 + the
        // strict adapter together: bool/null/number variants reject;
        // unquoted `NO` stays a string (YAML 1.2 doesn't coerce it).
        let bad_bool = "\
agent_id: foo
stage: classify
producer:
  provider: anthropic
  model: claude-opus-4-7
  output_sha: true
auditor:
  provider: openai
  model: gpt-5-codex
  verdict: accept
  reason: text
audit_tokens:
  in: 0
  out: 0
audited_at: '2026-05-14T00:00:00Z'
";
        let err = serde_yaml::from_str::<AuditVerdictOnDisk>(bad_bool).unwrap_err();
        assert!(
            err.to_string().contains("Norway-problem")
                || err.to_string().contains("quote the value"),
            "strict-string adapter must reject bool coercion in output_sha; got: {err}"
        );
    }
}
