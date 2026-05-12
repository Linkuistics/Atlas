//! On-disk path layout for the persistent cache.
//!
//! Layout: `<root>/cache/<stage>/<sha>.blob`. Stage directory names are
//! the lowercase form (`l1`, `l2`, …, `l9`), matching the kebab-case
//! `Stage` serialisation used in `analyzers.yaml` (atlas-contracts
//! design §6.6).
//!
//! Atomic write: the cache writer ([`super::PersistentCache::put`])
//! delegates to the canonical [`crate::atomic_write::atomic_write`]
//! helper (Phase 4 PR-4 collapsed a previously-duplicated tempfile +
//! rename routine that lived in this module). A kill -9 mid write
//! either leaves the target absent or, very rarely, a stray
//! `.tmp.<pid>.<rand>` sibling — which never matches `<sha>.blob`
//! and is therefore unreachable from `get()`.
//!
//! Read: `fs::read` of the target path, with `NotFound` mapped to
//! `Ok(None)`. Other I/O errors propagate.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use atlas_index::Stage;

use super::Sha256Hex;

/// Subdirectory under the cache root holding stage-keyed blobs.
/// `<root>/cache/<stage>/<sha>.blob`.
pub(crate) const CACHE_DIRNAME: &str = "cache";

/// Lowercase stage directory name. Matches the `Stage` kebab-case
/// serialisation used in `analyzers.yaml`.
pub(crate) fn stage_dir_name(stage: Stage) -> &'static str {
    match stage {
        Stage::L1 => "l1",
        Stage::L2 => "l2",
        Stage::L3 => "l3",
        Stage::L4 => "l4",
        Stage::L5 => "l5",
        Stage::L6 => "l6",
        Stage::L7 => "l7",
        Stage::L8 => "l8",
        Stage::L9 => "l9",
    }
}

/// All nine stages, in declaration order. Used by GC to enumerate the
/// per-stage subdirectories of the cache root.
pub(crate) const ALL_STAGES: [Stage; 9] = [
    Stage::L1,
    Stage::L2,
    Stage::L3,
    Stage::L4,
    Stage::L5,
    Stage::L6,
    Stage::L7,
    Stage::L8,
    Stage::L9,
];

/// Canonical blob suffix. The GC pass uses this to distinguish
/// fully-written cache entries from stray `.tmp*` files left behind
/// by an interrupted atomic write.
pub(crate) const BLOB_SUFFIX: &str = ".blob";

/// Resolve the absolute on-disk path for `(stage, fingerprint)`.
pub(crate) fn blob_path(root: &Path, stage: Stage, fingerprint: &Sha256Hex) -> PathBuf {
    root.join(CACHE_DIRNAME)
        .join(stage_dir_name(stage))
        .join(format!("{fingerprint}{BLOB_SUFFIX}"))
}

/// Resolve the per-stage directory under the cache root.
pub(crate) fn stage_dir_path(root: &Path, stage: Stage) -> PathBuf {
    root.join(CACHE_DIRNAME).join(stage_dir_name(stage))
}

/// Read `target` if present. `NotFound` maps to `Ok(None)`; other
/// I/O errors propagate so the caller can distinguish "miss" from
/// "filesystem broken".
pub(crate) fn read_if_exists(target: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(target) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading cache blob {}", target.display())),
    }
}

/// Parse a directory entry filename of the form `<sha>.blob`. Returns
/// `Some(sha)` for valid blob filenames; `None` for anything else
/// (including stray `.tmp*` files left by an interrupted atomic
/// write — those are unreachable from `get()` and ignored by GC).
pub(crate) fn fingerprint_from_blob_filename(name: &str) -> Option<&str> {
    name.strip_suffix(BLOB_SUFFIX)
}

// ---------- Phase 7 PR-2: agents (transcript) cache layout ------------
//
// Layout: `<root>/cache/agents/<stage>/<sha>.transcript` +
// `<root>/cache/agents/<stage>/<sha>.output`. The `agents` subdirectory
// segment isolates the multi-shot transcript cache from the single-shot
// L3/L5/L6 cache (`<root>/cache/<stage>/<sha>.blob`) so they cannot
// collide on filesystem layout even though the `<sha>` namespaces are
// independent.
//
// See LLM-spine recast §6.1 (cache key shape) and §6.4 (atomic-pair
// write semantics, materialised via `atomic_write::atomic_write_pair`).

/// `<root>/cache/agents/` — root of the multi-shot transcript cache.
/// Distinct from `<root>/cache/<stage>/` (single-shot blobs) so a future
/// GC pass can sweep one independently of the other.
pub(crate) const AGENTS_DIRNAME: &str = "agents";

/// Suffix for the LLM-side transcript blob (the full request/response
/// transcript of an agent invocation). Forensic value: the transcript
/// stays debuggable side-by-side with the output even if the output is
/// corrupt — see `atomic_write::atomic_write_pair` docs.
pub(crate) const TRANSCRIPT_SUFFIX: &str = ".transcript";

/// Suffix for the engine-facing output blob (the structured result the
/// caller consumes). Paired with `<sha>.transcript`; the two move
/// together via `atomic_write_pair`.
pub(crate) const OUTPUT_SUFFIX: &str = ".output";

/// Resolve the transcript-cache file path:
/// `<root>/cache/agents/<stage>/<sha>.transcript`.
pub(crate) fn agents_transcript_path(
    root: &Path,
    stage: Stage,
    fingerprint: &Sha256Hex,
) -> PathBuf {
    root.join(CACHE_DIRNAME)
        .join(AGENTS_DIRNAME)
        .join(stage_dir_name(stage))
        .join(format!("{fingerprint}{TRANSCRIPT_SUFFIX}"))
}

/// Resolve the transcript-cache output path:
/// `<root>/cache/agents/<stage>/<sha>.output`.
pub(crate) fn agents_output_path(root: &Path, stage: Stage, fingerprint: &Sha256Hex) -> PathBuf {
    root.join(CACHE_DIRNAME)
        .join(AGENTS_DIRNAME)
        .join(stage_dir_name(stage))
        .join(format!("{fingerprint}{OUTPUT_SUFFIX}"))
}

/// Resolve the per-stage directory under the transcript-cache root.
#[allow(dead_code)] // wired into PR-4+ when the runtime GCs orphan entries.
pub(crate) fn agents_stage_dir_path(root: &Path, stage: Stage) -> PathBuf {
    root.join(CACHE_DIRNAME)
        .join(AGENTS_DIRNAME)
        .join(stage_dir_name(stage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_dir_name_matches_kebab_lowercase() {
        // The kebab-case lowercase form is the normative wire form for
        // `analyzers.yaml`; the cache layout reuses it so a future
        // `<stage>` lookup based on serde::Serialize<Stage> stays in
        // sync.
        for (s, want) in [
            (Stage::L1, "l1"),
            (Stage::L2, "l2"),
            (Stage::L3, "l3"),
            (Stage::L4, "l4"),
            (Stage::L5, "l5"),
            (Stage::L6, "l6"),
            (Stage::L7, "l7"),
            (Stage::L8, "l8"),
            (Stage::L9, "l9"),
        ] {
            assert_eq!(stage_dir_name(s), want);
        }
    }

    #[test]
    fn blob_path_has_three_components_under_root() {
        let p = blob_path(Path::new("/r"), Stage::L3, &"deadbeef".to_string());
        assert_eq!(p, PathBuf::from("/r/cache/l3/deadbeef.blob"));
    }

    #[test]
    fn fingerprint_from_blob_filename_strips_suffix_only() {
        assert_eq!(fingerprint_from_blob_filename("abc.blob"), Some("abc"));
        assert_eq!(fingerprint_from_blob_filename("abc.tmp1234"), None);
        assert_eq!(fingerprint_from_blob_filename("abc"), None);
    }

    #[test]
    fn agents_paths_have_four_components_under_root() {
        let r = Path::new("/r");
        let sha = "deadbeef".to_string();
        assert_eq!(
            agents_transcript_path(r, Stage::L3, &sha),
            PathBuf::from("/r/cache/agents/l3/deadbeef.transcript")
        );
        assert_eq!(
            agents_output_path(r, Stage::L3, &sha),
            PathBuf::from("/r/cache/agents/l3/deadbeef.output")
        );
    }

    #[test]
    fn agents_layout_isolated_from_single_shot_layout() {
        // The single-shot cache lives at `<root>/cache/<stage>/`; the
        // multi-shot transcript cache lives at
        // `<root>/cache/agents/<stage>/`. The `agents` segment is the
        // load-bearing isolation so the two caches cannot collide on
        // a filesystem layout. Lock it down here.
        let r = Path::new("/r");
        let sha = "deadbeef".to_string();
        let blob = blob_path(r, Stage::L3, &sha);
        let transcript = agents_transcript_path(r, Stage::L3, &sha);
        assert!(blob.starts_with("/r/cache/l3"));
        assert!(transcript.starts_with("/r/cache/agents/l3"));
    }
}
