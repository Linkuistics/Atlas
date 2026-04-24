//! L1 enumeration queries — deterministic filters over the L0 file
//! set that form the raw material L2 (candidate generation) reads.
//!
//! Each query is `#[salsa::tracked]`, keyed on the [`Workspace`] input
//! and a directory. Salsa invalidates a result when either the file
//! list changes or a file whose content the query read is mutated;
//! for example, `doc_headings` re-runs when a `README` under `dir`
//! changes but `manifests_in` does not (it only reads each file's
//! path).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::db::{AtlasDatabase, Workspace};
use crate::manifest_patterns::is_manifest_file;

/// One ATX heading lifted from a Markdown-style documentation file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocHeading {
    pub path: PathBuf,
    pub level: u8,
    pub text: String,
}

/// One shebang line (`#!...`) found at the start of a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShebangEntry {
    pub path: PathBuf,
    pub interpreter: String,
}

fn path_is_inside(candidate: &Path, dir: &Path) -> bool {
    // An empty `dir` (used when a caller normalises root to "") means
    // "every path"; otherwise use `starts_with` for a component-wise
    // prefix check.
    dir.as_os_str().is_empty() || candidate.starts_with(dir)
}

/// Paths of all manifest files under `dir`, sorted for determinism.
#[salsa::tracked]
pub fn manifests_in<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<PathBuf>> {
    let files = workspace.files(db);
    let mut out = Vec::new();
    for file in files {
        let path = file.path(db);
        if path_is_inside(path, &dir) && is_manifest_file(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    Arc::new(out)
}

/// Directories containing a `.git` marker under `dir`, sorted.
#[salsa::tracked]
pub fn git_boundaries<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<PathBuf>> {
    let git_dirs = workspace.git_boundary_dirs(db);
    let mut out: Vec<PathBuf> = git_dirs
        .iter()
        .filter(|p| path_is_inside(p, &dir))
        .cloned()
        .collect();
    out.sort();
    Arc::new(out)
}

fn is_readmelike(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // Stem before extension, case-insensitive. `README`, `README.md`,
    // `readme.rst`, `CHANGELOG.md`, `CONTRIBUTING.md` all qualify.
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "README" | "CHANGELOG" | "CONTRIBUTING")
}

/// ATX headings from README/CHANGELOG/CONTRIBUTING-style files under
/// `dir`.  Only lines of the form `^#{1,6}\s+text` at column zero count;
/// headings inside fenced code blocks are ignored.
#[salsa::tracked]
pub fn doc_headings<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<DocHeading>> {
    let files = workspace.files(db);
    let mut out = Vec::new();
    for file in files {
        let path = file.path(db);
        if !path_is_inside(path, &dir) || !is_readmelike(path) {
            continue;
        }
        let bytes = file.bytes(db);
        let Ok(text) = std::str::from_utf8(bytes.as_slice()) else {
            continue;
        };
        let mut in_fence = false;
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            if let Some(heading) = parse_atx_heading(line) {
                out.push(DocHeading {
                    path: path.clone(),
                    level: heading.0,
                    text: heading.1,
                });
            }
        }
    }
    // Within a file, headings are in file order; across files, sort by
    // path to keep output deterministic.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(out)
}

fn parse_atx_heading(line: &str) -> Option<(u8, String)> {
    let bytes = line.as_bytes();
    let mut hashes = 0u8;
    while (hashes as usize) < bytes.len() && bytes[hashes as usize] == b'#' && hashes < 6 {
        hashes += 1;
    }
    if hashes == 0 {
        return None;
    }
    let rest = &line[hashes as usize..];
    // ATX requires at least one space between the hashes and the text.
    let stripped = rest.strip_prefix(' ')?.trim_end();
    // Optional trailing `##...` closing sequence.
    let text = stripped.trim_end_matches('#').trim_end().to_string();
    if text.is_empty() {
        return None;
    }
    Some((hashes, text))
}

/// Files whose first bytes are a shebang (`#!interpreter ...`). The
/// interpreter is captured verbatim after `#!`, trimmed, up to the first
/// newline. Executability is not checked — the shebang alone is a strong
/// enough signal in practice and avoids platform-dependent metadata
/// reads.
#[salsa::tracked]
pub fn shebangs<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> Arc<Vec<ShebangEntry>> {
    let files = workspace.files(db);
    let mut out = Vec::new();
    for file in files {
        let path = file.path(db);
        if !path_is_inside(path, &dir) {
            continue;
        }
        let bytes = file.bytes(db);
        if bytes.len() < 2 || &bytes[..2] != b"#!" {
            continue;
        }
        let rest = &bytes[2..];
        let end = rest
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .unwrap_or(rest.len());
        let Ok(interpreter) = std::str::from_utf8(&rest[..end]) else {
            continue;
        };
        out.push(ShebangEntry {
            path: path.clone(),
            interpreter: interpreter.trim().to_string(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Arc::new(out)
}

/// SHA-256 over the sorted `(path, file_sha)` pairs of every file
/// under `dir`, yielding a stable fingerprint that changes iff any
/// file under `dir` changes content, is added, or is removed.
#[salsa::tracked]
pub fn file_tree_sha<'db>(
    db: &'db dyn salsa::Database,
    workspace: Workspace,
    dir: PathBuf,
) -> [u8; 32] {
    let files = workspace.files(db);
    let mut entries: Vec<(PathBuf, [u8; 32])> = Vec::new();
    for file in files {
        let path = file.path(db);
        if !path_is_inside(path, &dir) {
            continue;
        }
        let bytes = file.bytes(db);
        let file_sha: [u8; 32] = Sha256::digest(bytes.as_slice()).into();
        entries.push((path.clone(), file_sha));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (path, file_sha) in &entries {
        // Length-prefixed framing keeps the digest unambiguous across
        // paths that happen to be prefixes of one another.
        let path_bytes = path.to_string_lossy();
        hasher.update((path_bytes.len() as u64).to_le_bytes());
        hasher.update(path_bytes.as_bytes());
        hasher.update(file_sha);
    }
    hasher.finalize().into()
}

/// Convenience: read a file's bytes by path. This is the operational
/// equivalent of the task-description's `file_content(path)` L0 query.
/// Untracked on purpose — the tracked dependency is on `File::bytes`
/// once a tracked query pulls the handle through.
pub fn file_content(db: &AtlasDatabase, path: &Path) -> Option<Arc<Vec<u8>>> {
    db.file_by_path(path)
        .map(|file| file.bytes(db as &dyn salsa::Database).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_atx_heading_handles_level_and_closing() {
        assert_eq!(parse_atx_heading("# Title"), Some((1, "Title".to_string())));
        assert_eq!(
            parse_atx_heading("### Sub ###"),
            Some((3, "Sub".to_string()))
        );
    }

    #[test]
    fn parse_atx_heading_requires_space_after_hashes() {
        // `#foo` is not ATX per CommonMark.
        assert_eq!(parse_atx_heading("#foo"), None);
    }

    #[test]
    fn parse_atx_heading_ignores_nonhash_lines() {
        assert_eq!(parse_atx_heading("plain text"), None);
        assert_eq!(parse_atx_heading(""), None);
    }

    #[test]
    fn parse_atx_heading_rejects_more_than_six_hashes() {
        // Per CommonMark, opening sequence is 1–6 hashes; `#######`
        // is not a heading.
        assert_eq!(parse_atx_heading("####### Deep"), None);
    }
}
