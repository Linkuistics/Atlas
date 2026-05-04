//! Read-only, sandboxed filesystem tools for HTTP-backend tool-use loops.
//!
//! HTTP backends (`AnthropicHttpBackend`, `OpenAiHttpBackend`) need
//! filesystem access in order to service `Stage1Surface` and
//! `Stage2Edges` — without it those prompts ship no per-component file
//! content. Subprocess backends (`claude-code`, `codex`) get filesystem
//! access from the upstream agent's built-in tools; HTTP backends will
//! get it from this module via a tool-use loop the providers' wire
//! formats already support.
//!
//! This file delivers the **tool surface** itself, decoupled from any
//! provider adapter. The provider adapters arrive in a follow-up phase
//! and call `FilesystemTools` impls in their tool-use loops.
//!
//! # Sandboxing
//!
//! [`SandboxedFilesystem`] is rooted at a single workspace path. Every
//! path argument is rejected if it is absolute or contains a `..`
//! segment, and — for paths that exist on disk — re-canonicalised and
//! checked to still live under the workspace root, which catches
//! symlink escapes. Read-only by design: there is no write surface.
//!
//! # Budgets
//!
//! [`ToolBudget`] caps cumulative bytes returned by `read_file` across
//! the whole tool-use call, plus per-call result counts on `glob` and
//! `grep`. Exceeding any cap returns [`ToolError::BudgetExceeded`].
//!
//! # Future work
//!
//! - Provider-specific JSON-Schema descriptors for the four tools live
//!   alongside the provider adapters when those land, not here — the
//!   pure tool surface should stay independent of any wire format.
//! - Lifting this module into a sibling crate (e.g. `linkuistics/llm-tools`)
//!   becomes straightforward once a second consumer exists.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors surfaced by [`FilesystemTools`] implementations.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("path traversal denied: {0}")]
    SandboxEscape(String),

    #[error("tool budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("filesystem error: {0}")]
    Io(String),

    #[error("not found: {0}")]
    NotFound(String),
}

/// Per-call resource caps enforced by [`SandboxedFilesystem`].
///
/// `max_total_bytes_read` is cumulative across every `read_file` call
/// on the same instance — exhausting it during a tool-use loop returns
/// `BudgetExceeded` to the caller, which is expected to surface that
/// as an `LlmError::Invocation` to the engine.
#[derive(Debug, Clone)]
pub struct ToolBudget {
    pub max_total_bytes_read: u64,
    pub max_glob_results: usize,
    pub max_grep_matches: usize,
    pub default_read_max_bytes: usize,
}

impl Default for ToolBudget {
    fn default() -> Self {
        Self {
            max_total_bytes_read: 1024 * 1024,
            max_glob_results: 1000,
            max_grep_matches: 100,
            default_read_max_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadFileOutput {
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobOutput {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepOutput {
    pub matches: Vec<GrepMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DirEntryKind {
    File,
    Dir,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub kind: DirEntryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListDirOutput {
    pub entries: Vec<DirEntryInfo>,
}

/// Read-only filesystem tool surface. Implementations must enforce
/// sandboxing — every path argument must resolve inside the configured
/// root, and `..` / absolute-path / symlink-escape attempts must be
/// rejected with [`ToolError::SandboxEscape`].
pub trait FilesystemTools: Send + Sync {
    fn read_file(
        &self,
        path: &str,
        max_bytes: Option<usize>,
    ) -> Result<ReadFileOutput, ToolError>;

    fn glob(&self, pattern: &str) -> Result<GlobOutput, ToolError>;

    fn grep(
        &self,
        pattern: &str,
        paths: &[String],
        max_matches: Option<usize>,
    ) -> Result<GrepOutput, ToolError>;

    fn list_dir(&self, path: &str) -> Result<ListDirOutput, ToolError>;
}

/// `FilesystemTools` impl rooted at a single canonical workspace
/// directory, with cumulative byte-budget tracking shared across every
/// call on the same instance.
pub struct SandboxedFilesystem {
    root: PathBuf,
    budget: ToolBudget,
    bytes_read: AtomicU64,
}

impl SandboxedFilesystem {
    /// Construct a sandbox rooted at `root`. The root is canonicalised
    /// at construction so subsequent symlink-escape checks compare like
    /// for like.
    pub fn new(root: impl Into<PathBuf>, budget: ToolBudget) -> Result<Self, ToolError> {
        let root = root.into();
        let root = root
            .canonicalize()
            .map_err(|e| ToolError::Io(format!("canonicalize root {}: {e}", root.display())))?;
        Ok(Self {
            root,
            budget,
            bytes_read: AtomicU64::new(0),
        })
    }

    /// Total bytes returned by `read_file` calls on this instance.
    /// Useful for tests and for surfacing budget consumption.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    /// Resolve `rel` against the sandbox root, rejecting traversal
    /// attempts. Existence is **not** required — caller decides whether
    /// missing-path is an error for its operation.
    fn sanitize(&self, rel: &str) -> Result<PathBuf, ToolError> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(ToolError::SandboxEscape(format!(
                "absolute path not allowed: {rel}"
            )));
        }
        for component in rel_path.components() {
            use std::path::Component;
            match component {
                Component::ParentDir => {
                    return Err(ToolError::SandboxEscape(format!(
                        "`..` not allowed: {rel}"
                    )));
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(ToolError::SandboxEscape(format!(
                        "path prefix not allowed: {rel}"
                    )));
                }
                _ => {}
            }
        }
        let joined = self.root.join(rel_path);
        if joined.exists() {
            let canonical = joined.canonicalize().map_err(|e| {
                ToolError::Io(format!("canonicalize {}: {e}", joined.display()))
            })?;
            if !canonical.starts_with(&self.root) {
                return Err(ToolError::SandboxEscape(format!(
                    "symlink escape: {rel}"
                )));
            }
            Ok(canonical)
        } else {
            Ok(joined)
        }
    }
}

impl FilesystemTools for SandboxedFilesystem {
    fn read_file(
        &self,
        path: &str,
        max_bytes: Option<usize>,
    ) -> Result<ReadFileOutput, ToolError> {
        let resolved = self.sanitize(path)?;
        if !resolved.is_file() {
            return Err(ToolError::NotFound(format!(
                "{} is not a file",
                path
            )));
        }
        let prev = self.bytes_read.load(Ordering::Relaxed);
        let remaining_budget = self
            .budget
            .max_total_bytes_read
            .saturating_sub(prev);
        if remaining_budget == 0 {
            return Err(ToolError::BudgetExceeded(format!(
                "max_total_bytes_read ({} bytes) reached",
                self.budget.max_total_bytes_read
            )));
        }
        let user_cap = max_bytes.unwrap_or(self.budget.default_read_max_bytes);
        let effective_cap = std::cmp::min(user_cap as u64, remaining_budget) as usize;

        let bytes = std::fs::read(&resolved)
            .map_err(|e| ToolError::Io(format!("read {}: {e}", path)))?;
        let truncated = bytes.len() > effective_cap;
        let slice = if truncated {
            &bytes[..effective_cap]
        } else {
            &bytes[..]
        };
        self.bytes_read
            .fetch_add(slice.len() as u64, Ordering::Relaxed);
        let content = String::from_utf8_lossy(slice).into_owned();
        Ok(ReadFileOutput { content, truncated })
    }

    fn glob(&self, pattern: &str) -> Result<GlobOutput, ToolError> {
        if pattern.starts_with('/') {
            return Err(ToolError::SandboxEscape(
                "absolute glob pattern not allowed".to_string(),
            ));
        }
        if pattern.split('/').any(|seg| seg == "..") {
            return Err(ToolError::SandboxEscape(
                "`..` not allowed in glob pattern".to_string(),
            ));
        }
        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| {
                ToolError::InvalidArgument(format!("invalid glob `{pattern}`: {e}"))
            })?;
        let matcher = glob.compile_matcher();

        let walker = ignore::WalkBuilder::new(&self.root).build();
        let mut paths = Vec::new();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let abs = entry.path();
            let Ok(rel) = abs.strip_prefix(&self.root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if matcher.is_match(&rel_str) {
                paths.push(rel_str);
                if paths.len() >= self.budget.max_glob_results {
                    break;
                }
            }
        }
        paths.sort();
        Ok(GlobOutput { paths })
    }

    fn grep(
        &self,
        pattern: &str,
        paths: &[String],
        max_matches: Option<usize>,
    ) -> Result<GrepOutput, ToolError> {
        let regex = regex::RegexBuilder::new(pattern)
            .size_limit(1024 * 1024)
            .build()
            .map_err(|e| {
                ToolError::InvalidArgument(format!("invalid regex `{pattern}`: {e}"))
            })?;
        let cap = max_matches.unwrap_or(self.budget.max_grep_matches);

        let mut matches = Vec::new();
        for path_str in paths {
            let resolved = self.sanitize(path_str)?;
            if !resolved.is_file() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&resolved) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (idx, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        path: path_str.clone(),
                        line: idx + 1,
                        text: line.to_string(),
                    });
                    if matches.len() >= cap {
                        return Ok(GrepOutput { matches });
                    }
                }
            }
        }
        Ok(GrepOutput { matches })
    }

    fn list_dir(&self, path: &str) -> Result<ListDirOutput, ToolError> {
        let resolved = self.sanitize(path)?;
        if !resolved.is_dir() {
            return Err(ToolError::NotFound(format!(
                "{} is not a directory",
                path
            )));
        }
        let read = std::fs::read_dir(&resolved)
            .map_err(|e| ToolError::Io(format!("read_dir {}: {e}", path)))?;
        let mut entries = Vec::new();
        for entry in read {
            let entry = entry
                .map_err(|e| ToolError::Io(format!("read_dir entry {}: {e}", path)))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = match entry.file_type() {
                Ok(ft) if ft.is_file() => DirEntryKind::File,
                Ok(ft) if ft.is_dir() => DirEntryKind::Dir,
                _ => DirEntryKind::Other,
            };
            entries.push(DirEntryInfo { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ListDirOutput { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fs_with_layout() -> (TempDir, SandboxedFilesystem) {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("readme.md"), b"hello\nworld\nfoo bar\n").unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(
            tmp.path().join("src/main.rs"),
            b"fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        fs::write(tmp.path().join("src/lib.rs"), b"pub fn greet() {}\n").unwrap();
        let sb = SandboxedFilesystem::new(tmp.path(), ToolBudget::default()).unwrap();
        (tmp, sb)
    }

    #[test]
    fn sanitize_rejects_absolute_paths() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.read_file("/etc/passwd", None).unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[test]
    fn sanitize_rejects_parent_dir() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.read_file("../etc/passwd", None).unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[test]
    fn sanitize_rejects_parent_dir_in_middle() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.read_file("src/../../etc/passwd", None).unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn sanitize_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret"), b"top secret\n").unwrap();
        let inside = TempDir::new().unwrap();
        symlink(
            outside.path().join("secret"),
            inside.path().join("escape"),
        )
        .unwrap();
        let sb = SandboxedFilesystem::new(inside.path(), ToolBudget::default()).unwrap();
        let err = sb.read_file("escape", None).unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[test]
    fn read_file_returns_full_content_when_under_cap() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.read_file("readme.md", None).unwrap();
        assert_eq!(out.content, "hello\nworld\nfoo bar\n");
        assert!(!out.truncated);
    }

    #[test]
    fn read_file_truncates_at_max_bytes() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.read_file("readme.md", Some(5)).unwrap();
        assert_eq!(out.content, "hello");
        assert!(out.truncated);
    }

    #[test]
    fn read_file_missing_is_not_found() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.read_file("does/not/exist.txt", None).unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn read_file_charges_cumulative_budget() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a"), b"aaaaaaaaaa").unwrap();
        fs::write(tmp.path().join("b"), b"bbbbbbbbbb").unwrap();
        let budget = ToolBudget {
            max_total_bytes_read: 12,
            ..ToolBudget::default()
        };
        let sb = SandboxedFilesystem::new(tmp.path(), budget).unwrap();

        let first = sb.read_file("a", None).unwrap();
        assert_eq!(first.content, "aaaaaaaaaa");
        assert!(!first.truncated);
        assert_eq!(sb.bytes_read(), 10);

        let second = sb.read_file("b", None).unwrap();
        assert_eq!(second.content, "bb");
        assert!(second.truncated);
        assert_eq!(sb.bytes_read(), 12);

        let err = sb.read_file("a", None).unwrap_err();
        assert!(matches!(err, ToolError::BudgetExceeded(_)), "got {err:?}");
    }

    #[test]
    fn list_dir_returns_sorted_entries() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.list_dir("").unwrap();
        let names: Vec<&str> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["readme.md", "src"]);
        assert_eq!(out.entries[0].kind, DirEntryKind::File);
        assert_eq!(out.entries[1].kind, DirEntryKind::Dir);
    }

    #[test]
    fn list_dir_subdir() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.list_dir("src").unwrap();
        let names: Vec<&str> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["lib.rs", "main.rs"]);
    }

    #[test]
    fn list_dir_missing_is_not_found() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.list_dir("nope").unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn list_dir_on_file_is_not_found() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.list_dir("readme.md").unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "got {err:?}");
    }

    #[test]
    fn glob_matches_extension() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.glob("src/*.rs").unwrap();
        assert_eq!(out.paths, vec!["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn glob_double_star_matches_recursively() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb.glob("**/*.rs").unwrap();
        assert_eq!(out.paths, vec!["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn glob_rejects_absolute_pattern() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.glob("/etc/*").unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[test]
    fn glob_rejects_parent_dir_pattern() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.glob("../*").unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }

    #[test]
    fn glob_invalid_pattern_is_invalid_argument() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb.glob("[unclosed").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgument(_)), "got {err:?}");
    }

    #[test]
    fn glob_caps_at_max_results() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(tmp.path().join(format!("f{i}.txt")), b"x").unwrap();
        }
        let budget = ToolBudget {
            max_glob_results: 3,
            ..ToolBudget::default()
        };
        let sb = SandboxedFilesystem::new(tmp.path(), budget).unwrap();
        let out = sb.glob("*.txt").unwrap();
        assert_eq!(out.paths.len(), 3);
    }

    #[test]
    fn grep_finds_literal_match() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb
            .grep("foo", &["readme.md".to_string()], None)
            .unwrap();
        assert_eq!(out.matches.len(), 1);
        assert_eq!(out.matches[0].path, "readme.md");
        assert_eq!(out.matches[0].line, 3);
        assert_eq!(out.matches[0].text, "foo bar");
    }

    #[test]
    fn grep_finds_regex_match() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb
            .grep(r"fn \w+", &["src/main.rs".to_string()], None)
            .unwrap();
        assert_eq!(out.matches.len(), 1);
        assert!(out.matches[0].text.contains("fn main"));
    }

    #[test]
    fn grep_searches_multiple_paths() {
        let (_tmp, sb) = fs_with_layout();
        let out = sb
            .grep(
                "fn",
                &["src/main.rs".to_string(), "src/lib.rs".to_string()],
                None,
            )
            .unwrap();
        assert_eq!(out.matches.len(), 2);
    }

    #[test]
    fn grep_caps_at_max_matches() {
        let tmp = TempDir::new().unwrap();
        let mut content = String::new();
        for _ in 0..20 {
            content.push_str("hit\n");
        }
        fs::write(tmp.path().join("many"), content.as_bytes()).unwrap();
        let sb = SandboxedFilesystem::new(tmp.path(), ToolBudget::default()).unwrap();
        let out = sb
            .grep("hit", &["many".to_string()], Some(3))
            .unwrap();
        assert_eq!(out.matches.len(), 3);
    }

    #[test]
    fn grep_invalid_regex_is_invalid_argument() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb
            .grep("(unclosed", &["readme.md".to_string()], None)
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgument(_)), "got {err:?}");
    }

    #[test]
    fn grep_rejects_path_traversal() {
        let (_tmp, sb) = fs_with_layout();
        let err = sb
            .grep("hit", &["../../etc/passwd".to_string()], None)
            .unwrap_err();
        assert!(matches!(err, ToolError::SandboxEscape(_)), "got {err:?}");
    }
}
