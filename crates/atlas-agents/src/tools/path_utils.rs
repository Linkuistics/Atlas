//! Path validation helpers for tool wrappers (Phase 7).
//!
//! Tools accept LLM-supplied paths that must be confined to the
//! `ToolContext::workspace_root` (per `tool.rs` doc). `PathBuf::join`
//! happily absorbs `..` components and absolute paths replace the
//! base entirely — so a lexical check before joining is required.

use crate::{ToolArgs, ToolError};
use std::path::{Path, PathBuf};

/// Resolve a user-supplied relative path against `workspace_root`,
/// rejecting absolute paths and any `..` component.
///
/// Returns the absolute path on success. The check is lexical: no
/// filesystem round-trip, no symlink resolution. This is deliberate
/// — `canonicalize` requires the path to exist (failing spuriously
/// for paths the caller intends to read soon) and resolves symlinks
/// (potentially leaking host-path information into error messages).
pub fn require_within_root(workspace_root: &Path, user_path: &str) -> Result<PathBuf, ToolError> {
    let p = Path::new(user_path);
    if p.is_absolute() {
        return Err(ToolError::InvalidArgs(format!(
            "absolute path not allowed: {user_path}"
        )));
    }
    for component in p.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(ToolError::InvalidArgs(format!(
                "path contains `..`: {user_path}"
            )));
        }
    }
    Ok(workspace_root.join(p))
}

/// Read a required string field from `ToolArgs` and validate that the
/// resulting path stays under `workspace_root`. Convenience over
/// `require_string` + `require_within_root` for the common pattern.
pub fn require_path_arg(
    args: &ToolArgs,
    field: &str,
    workspace_root: &Path,
) -> Result<PathBuf, ToolError> {
    let s = args
        .0
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or non-string `{field}`")))?;
    require_within_root(workspace_root, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn require_within_root_accepts_relative_path() {
        let root = PathBuf::from("/work");
        let resolved = require_within_root(&root, "src/lib.rs").unwrap();
        assert_eq!(resolved, PathBuf::from("/work/src/lib.rs"));
    }

    #[test]
    fn require_within_root_rejects_parent_dir_component() {
        let root = PathBuf::from("/work");
        let err = require_within_root(&root, "../etc/passwd").unwrap_err();
        match err {
            ToolError::InvalidArgs(msg) => assert!(msg.contains("..")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[test]
    fn require_within_root_rejects_embedded_parent_dir() {
        let root = PathBuf::from("/work");
        let err = require_within_root(&root, "src/../../etc").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn require_within_root_rejects_absolute_path() {
        let root = PathBuf::from("/work");
        let err = require_within_root(&root, "/etc/passwd").unwrap_err();
        match err {
            ToolError::InvalidArgs(msg) => assert!(msg.contains("absolute")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[test]
    fn require_within_root_accepts_current_dir_component() {
        // `./foo` is fine — `Component::CurDir` is not `ParentDir`.
        let root = PathBuf::from("/work");
        let resolved = require_within_root(&root, "./src/lib.rs").unwrap();
        assert_eq!(resolved, PathBuf::from("/work/./src/lib.rs"));
    }
}
