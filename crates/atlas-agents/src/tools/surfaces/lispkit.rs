//! Pass-through wrapper for the LispKit subprocess surface analyser.
//!
//! Calls `atlas_analyzers::lispkit_surface_analyzer` functions to locate
//! the `lispkit-analyzer` binary and invoke it via the subprocess proxy.
//! When the binary is not present the wrapper returns a helpful
//! `ToolError::Invocation` pointing at the build command.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use atlas_analyzers::subprocess::SubprocessOutput;
use atlas_analyzers::Analyzer;
use atlas_analyzers::{
    lispkit_subprocess_spec, locate_lispkit_analyzer_binary, AnalysisContext, AnalyzerResult,
    Target, TargetFile,
};

use crate::tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};

pub struct LispKitSurfaceTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "component_dir": {
                    "type": "string",
                    "description": "Workspace-relative path to the component directory."
                },
                "sld_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Workspace-relative paths to *.sld (Scheme Library Definition) files."
                }
            },
            "required": ["component_dir", "sld_files"]
        }),
        description:
            "Extract the public surface (bindings, exports) of a LispKit/R7RS component by \
             spawning the lispkit-analyzer subprocess. \
             Requires `cargo build -p atlas-lispkit-analyzer` to have been run."
                .into(),
    })
}

#[async_trait]
impl Tool for LispKitSurfaceTool {
    fn id(&self) -> &'static str {
        "analyse_lispkit_surface"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let sld_files = require_string_array(&args, "sld_files")?;
        if sld_files.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`sld_files` must contain at least one path".into(),
            ));
        }
        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        // Validate all sld_files paths before moving into the blocking task.
        for rel_path in &sld_files {
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, rel_path)?;
        }
        let workspace_root = ctx.workspace_root.clone();

        let output =
            tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
                let binary_path = locate_lispkit_analyzer_binary().ok_or_else(|| {
                    ToolError::Invocation(
                        "lispkit-analyzer binary not found. \
                             Build it with: cargo build -p atlas-lispkit-analyzer"
                            .into(),
                    )
                })?;

                let mut manifests = Vec::with_capacity(sld_files.len());
                let mut top_level_files = Vec::with_capacity(sld_files.len());
                for rel_path in &sld_files {
                    let abs_path = workspace_root.join(rel_path);
                    let bytes = std::fs::read(&abs_path).map_err(|e| {
                        ToolError::Filesystem(format!("read {}: {e}", abs_path.display()))
                    })?;
                    let content_sha = hex_sha256(&bytes);
                    let name = PathBuf::from(rel_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| rel_path.clone());
                    top_level_files.push(name.clone());
                    manifests.push(TargetFile {
                        name,
                        relpath: PathBuf::from(rel_path),
                        bytes,
                        content_sha,
                    });
                }

                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["scheme".to_string()]),
                    manifests,
                    top_level_files,
                };

                let spec = lispkit_subprocess_spec(binary_path);
                let proxy =
                    atlas_analyzers::cached_lispkit_subprocess_proxy(spec).map_err(|e| {
                        ToolError::Invocation(format!("lispkit-analyzer proxy init failed: {e}"))
                    })?;

                let result = proxy.analyse(&AnalysisContext::deterministic_only(), &target);
                match result {
                    AnalyzerResult::Confident(boxed) => {
                        let out = boxed
                            .as_any()
                            .downcast_ref::<SubprocessOutput>()
                            .ok_or_else(|| {
                                ToolError::Invocation("downcast to SubprocessOutput failed".into())
                            })?;
                        Ok(out.payload.clone())
                    }
                    AnalyzerResult::Graded { output: boxed, .. } => {
                        let out = boxed
                            .as_any()
                            .downcast_ref::<SubprocessOutput>()
                            .ok_or_else(|| {
                                ToolError::Invocation("downcast to SubprocessOutput failed".into())
                            })?;
                        Ok(out.payload.clone())
                    }
                    AnalyzerResult::Declines => {
                        Err(ToolError::Invocation("lispkit-analyzer declined".into()))
                    }
                    AnalyzerResult::Error(e) => Err(ToolError::Invocation(format!(
                        "lispkit-analyzer error: {e}"
                    ))),
                }
            })
            .await
            .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;

        let bytes = serde_json::to_vec(&output)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        Ok(ToolResult { output, bytes })
    }

    fn fingerprint_inputs(&self, args: &ToolArgs) -> Vec<FingerprintInput> {
        require_string_array(args, "sld_files")
            .unwrap_or_default()
            .into_iter()
            .map(|p| FingerprintInput {
                path: PathBuf::from(p),
                sha: [0u8; 32],
            })
            .collect()
    }
}

fn require_string(args: &ToolArgs, field: &str) -> Result<String, ToolError> {
    args.0
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or non-string `{field}`")))
}

fn require_string_array(args: &ToolArgs, field: &str) -> Result<Vec<String>, ToolError> {
    args.0
        .get(field)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    v.as_str().map(|s| s.to_string()).ok_or_else(|| {
                        ToolError::InvalidArgs(format!("`{field}` array element is not a string"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or non-array `{field}`")))?
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lispkit_surface_tool_returns_error_when_binary_missing() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = LispKitSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "sld_files": ["mylib.sld"]
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        match result {
            Ok(_) => {
                // Binary was found and analysis succeeded.
            }
            Err(ToolError::Invocation(msg)) => {
                assert!(
                    msg.contains("analyzer") || msg.contains("not found"),
                    "unexpected invocation error: {msg}"
                );
            }
            Err(ToolError::Filesystem(_)) => {
                // Binary found but sld file missing — also acceptable.
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lispkit_surface_tool_empty_sld_files_returns_invalid_args() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = LispKitSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "sld_files": []
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::InvalidArgs(_))));
    }

    #[tokio::test]
    async fn lispkit_surface_tool_missing_required_arg() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = LispKitSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({ "component_dir": "" }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::InvalidArgs(_))));
    }
}
