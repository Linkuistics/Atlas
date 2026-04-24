//! On-disk persistence of the engine's LLM response cache.
//!
//! Stored as JSON at `<output_dir>/llm-cache.json`. Schema-versioned
//! so an out-of-date snapshot is ignored rather than mis-loaded. The
//! cache is a pure optimisation — if the file is missing or rejected,
//! the run still succeeds; it just makes fresh LLM calls.
//!
//! Load at pipeline start, save at pipeline end (on success, not on
//! `--dry-run` or budget exhaustion — a dry-run reflects candidate
//! state, not state we want to preserve).

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use atlas_engine::{LlmCacheKey, LlmResponseCache};
use atlas_llm::{LlmFingerprint, PromptId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Cache-file schema version. Bumped when the serialised shape
/// changes; a prior-version snapshot is discarded on load.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,
    entries: Vec<CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    fingerprint: CachedFingerprint,
    prompt: String,
    /// Canonical JSON string of the original `LlmRequest.inputs`.
    inputs: String,
    response: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedFingerprint {
    template_sha: String,
    ontology_sha: String,
    model_id: String,
    backend_version: String,
}

/// Load the on-disk cache (if any) and seed `target` with its entries.
/// Returns the number of entries loaded. A missing or malformed file
/// is ignored — the caller proceeds with an empty cache.
pub fn load_into(path: &Path, target: &LlmResponseCache) -> usize {
    let Ok(text) = fs::read_to_string(path) else {
        return 0;
    };
    let Ok(file) = serde_json::from_str::<CacheFile>(&text) else {
        return 0;
    };
    if file.schema_version != CACHE_SCHEMA_VERSION {
        return 0;
    }
    let mut loaded = 0usize;
    for entry in file.entries {
        let Some(prompt) = parse_prompt_id(&entry.prompt) else {
            continue;
        };
        let Some(fingerprint) = hydrate_fingerprint(&entry.fingerprint) else {
            continue;
        };
        let key = LlmCacheKey {
            fingerprint,
            prompt,
            inputs: entry.inputs,
        };
        target.seed(key, Arc::new(entry.response));
        loaded += 1;
    }
    loaded
}

/// Serialise every entry in `source` to `path` as JSON, using an
/// atomic temp-file-then-rename write so a crash mid-save cannot
/// leave a partial cache file.
pub fn save_from(path: &Path, source: &LlmResponseCache) -> io::Result<()> {
    let entries: Vec<CacheEntry> = source
        .entries_snapshot()
        .into_iter()
        .map(|(key, value)| CacheEntry {
            fingerprint: CachedFingerprint {
                template_sha: hex_encode(&key.fingerprint.template_sha),
                ontology_sha: hex_encode(&key.fingerprint.ontology_sha),
                model_id: key.fingerprint.model_id.clone(),
                backend_version: key.fingerprint.backend_version.clone(),
            },
            prompt: prompt_id_string(key.prompt).to_string(),
            inputs: key.inputs.clone(),
            response: (*value).clone(),
        })
        .collect();

    // Deterministic ordering for diffable cache files.
    let mut entries = entries;
    entries.sort_by(|a, b| {
        a.prompt
            .cmp(&b.prompt)
            .then_with(|| a.fingerprint.model_id.cmp(&b.fingerprint.model_id))
            .then_with(|| a.inputs.cmp(&b.inputs))
    });

    let file = CacheFile {
        schema_version: CACHE_SCHEMA_VERSION,
        entries,
    };
    let bytes = serde_json::to_vec_pretty(&file)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "cache path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache path has no file name"))?
        .to_string_lossy()
        .into_owned();
    let tmp = parent.join(format!(".{name}.tmp"));
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)
}

fn parse_prompt_id(s: &str) -> Option<PromptId> {
    Some(match s {
        "classify" => PromptId::Classify,
        "subcarve" => PromptId::Subcarve,
        "stage1-surface" => PromptId::Stage1Surface,
        "stage2-edges" => PromptId::Stage2Edges,
        _ => return None,
    })
}

fn prompt_id_string(id: PromptId) -> &'static str {
    match id {
        PromptId::Classify => "classify",
        PromptId::Subcarve => "subcarve",
        PromptId::Stage1Surface => "stage1-surface",
        PromptId::Stage2Edges => "stage2-edges",
    }
}

fn hydrate_fingerprint(raw: &CachedFingerprint) -> Option<LlmFingerprint> {
    Some(LlmFingerprint {
        template_sha: hex_decode_32(&raw.template_sha)?,
        ontology_sha: hex_decode_32(&raw.ontology_sha)?,
        model_id: raw.model_id.clone(),
        backend_version: raw.backend_version.clone(),
    })
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = from_hex_digit(chunk[0])?;
        let lo = from_hex_digit(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn from_hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_engine::{LlmCacheKey, LlmResponseCache};
    use atlas_llm::{LlmFingerprint, PromptId};
    use serde_json::json;
    use tempfile::TempDir;

    fn sample_fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [1u8; 32],
            ontology_sha: [2u8; 32],
            model_id: "m".into(),
            backend_version: "v".into(),
        }
    }

    fn sample_key() -> LlmCacheKey {
        LlmCacheKey {
            fingerprint: sample_fingerprint(),
            prompt: PromptId::Stage1Surface,
            inputs: r#"{"a":1}"#.into(),
        }
    }

    #[test]
    fn hex_encode_then_decode_round_trips() {
        let bytes = [0u8, 1, 2, 254, 255, 128, 42, 100, 200, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230, 240, 250, 5, 6];
        let encoded = hex_encode(&bytes);
        let decoded = hex_decode_32(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn save_then_load_preserves_entries() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("llm-cache.json");
        let source = LlmResponseCache::new();
        source.seed(sample_key(), Arc::new(json!({"purpose": "hi"})));
        save_from(&path, &source).unwrap();

        let target = LlmResponseCache::new();
        let loaded = load_into(&path, &target);
        assert_eq!(loaded, 1);
        assert_eq!(target.entries_snapshot().len(), 1);
    }

    #[test]
    fn load_returns_zero_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        let target = LlmResponseCache::new();
        assert_eq!(load_into(&path, &target), 0);
    }

    #[test]
    fn load_rejects_mismatched_schema_version() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cache.json");
        std::fs::write(&path, r#"{"schema_version": 99, "entries": []}"#).unwrap();
        let target = LlmResponseCache::new();
        assert_eq!(load_into(&path, &target), 0);
    }

    #[test]
    fn load_rejects_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cache.json");
        std::fs::write(&path, "not json at all").unwrap();
        let target = LlmResponseCache::new();
        assert_eq!(load_into(&path, &target), 0);
    }

    #[test]
    fn save_never_leaves_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cache.json");
        let source = LlmResponseCache::new();
        source.seed(sample_key(), Arc::new(json!({"x": 1})));
        save_from(&path, &source).unwrap();
        let leftover: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftover.is_empty(), "expected no temp file, got {leftover:?}");
    }
}
