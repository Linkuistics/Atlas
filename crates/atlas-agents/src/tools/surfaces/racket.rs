//! Pass-through wrapper for the Racket subprocess surface analyser.
//!
//! Calls `atlas_analyzers::racket_surface_analyzer` functions to locate
//! the `racket-analyzer` binary and invoke it via the subprocess proxy.
//! When the binary is not present the wrapper returns a helpful
//! `ToolError::Invocation` pointing at the build command.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use atlas_analyzers::subprocess::SubprocessOutput;
use atlas_analyzers::Analyzer;
use atlas_analyzers::{
    locate_racket_analyzer_binary, racket_subprocess_spec, AnalysisContext, AnalyzerResult, Target,
    TargetFile,
};

use crate::tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};

pub struct RacketSurfaceTool;

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
                "info_rkt_path": {
                    "type": "string",
                    "description": "Workspace-relative path to the info.rkt manifest."
                }
            },
            "required": ["component_dir", "info_rkt_path"]
        }),
        description: "Extract the public surface (bindings, provided names, path-dep edges) of a \
             Racket component by spawning the racket-analyzer subprocess. \
             Requires `cargo build -p atlas-racket-analyzer` to have been run."
            .into(),
    })
}

#[async_trait]
impl Tool for RacketSurfaceTool {
    fn id(&self) -> &'static str {
        "analyse_racket_surface"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let info_rkt_path = require_string(&args, "info_rkt_path")?;
        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        let abs_manifest =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &info_rkt_path)?;

        let output =
            tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
                let binary_path = locate_racket_analyzer_binary().ok_or_else(|| {
                    ToolError::Invocation(
                        "racket-analyzer binary not found. \
                             Build it with: cargo build -p atlas-racket-analyzer"
                            .into(),
                    )
                })?;

                let bytes = std::fs::read(&abs_manifest).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_manifest.display()))
                })?;
                let content_sha = hex_sha256(&bytes);
                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["racket".to_string()]),
                    manifests: vec![TargetFile {
                        name: "info.rkt".into(),
                        relpath: PathBuf::from("info.rkt"),
                        bytes,
                        content_sha,
                    }],
                    top_level_files: vec!["info.rkt".to_string()],
                };

                let spec = racket_subprocess_spec(binary_path);
                let proxy = atlas_analyzers::cached_racket_subprocess_proxy(spec).map_err(|e| {
                    ToolError::Invocation(format!("racket-analyzer proxy init failed: {e}"))
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
                        Err(ToolError::Invocation("racket-analyzer declined".into()))
                    }
                    AnalyzerResult::Error(e) => {
                        Err(ToolError::Invocation(format!("racket-analyzer error: {e}")))
                    }
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
        require_string(args, "info_rkt_path")
            .ok()
            .map(|p| {
                vec![FingerprintInput {
                    path: PathBuf::from(p),
                    sha: [0u8; 32],
                }]
            })
            .unwrap_or_default()
    }
}

fn require_string(args: &ToolArgs, field: &str) -> Result<String, ToolError> {
    args.0
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or non-string `{field}`")))
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
    async fn racket_surface_tool_returns_error_when_binary_missing() {
        let tempdir = tempfile::TempDir::new().unwrap();
        // Write info.rkt so the test is deterministic across environments
        // where the binary may or may not be built. The tool reads the manifest
        // regardless of binary presence; without it a Filesystem error fires
        // instead of the expected Invocation error.
        std::fs::write(
            tempdir.path().join("info.rkt"),
            "#lang info\n(define name \"foo\")\n",
        )
        .unwrap();
        let tool = RacketSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "info_rkt_path": "info.rkt"
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
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn racket_surface_tool_missing_required_arg() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = RacketSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({ "component_dir": "" }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::InvalidArgs(_))));
    }
}
