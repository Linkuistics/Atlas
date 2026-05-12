//! Pass-through wrapper for the C# subprocess surface analyser.
//!
//! Wraps `atlas_analyzers::cached_csharp_subprocess_proxy` in a `Tool`. The
//! subprocess payload is forwarded verbatim as the `ToolResult` output. When
//! the `csharp-analyzer` binary is not found in the cargo target tree, returns
//! a descriptive `ToolError::Invocation`.

use async_trait::async_trait;
use atlas_analyzers::{
    cached_csharp_subprocess_proxy, csharp_subprocess_spec, locate_csharp_analyzer_binary,
    AnalysisContext, Analyzer, AnalyzerResult, SubprocessOutput, Target, TargetFile,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::{FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema};

/// Pass-through tool wrapping the C# surface subprocess analyser.
pub struct CsharpSurfaceTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "component_dir": {
                    "type": "string",
                    "description": "workspace-relative path to the C# component directory"
                },
                "manifest_path": {
                    "type": "string",
                    "description": "optional workspace-relative path to *.csproj or *.sln manifest"
                }
            },
            "required": ["component_dir"]
        }),
        description:
            "Extract the C# surface (public types, methods, project references) from a C# \
             component via the csharp-analyzer subprocess. Returns the raw analyser JSON payload. \
             Requires the csharp-analyzer binary to be built (`cargo build -p atlas-csharp-analyzer`)."
                .into(),
    })
}

#[async_trait]
impl Tool for CsharpSurfaceTool {
    fn id(&self) -> &'static str {
        "extract_csharp_surface"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let manifest_path = args
            .0
            .get("manifest_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let abs_dir = ctx.workspace_root.join(&component_dir);
        let workspace_root = ctx.workspace_root.clone();

        let output =
            tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
                // Locate the binary; return a descriptive error when absent.
                let binary_path = locate_csharp_analyzer_binary().ok_or_else(|| {
                    ToolError::Invocation(
                        "csharp-analyzer binary not found in cargo target tree; \
                     run `cargo build -p atlas-csharp-analyzer` first"
                            .into(),
                    )
                })?;

                let spec = csharp_subprocess_spec(binary_path);
                let proxy = cached_csharp_subprocess_proxy(spec).map_err(|e| {
                    ToolError::Invocation(format!("failed to construct csharp-analyzer proxy: {e}"))
                })?;

                // Build the Target. Load the manifest bytes if a path was given.
                let mut manifests = Vec::new();
                if let Some(ref rel_path) = manifest_path {
                    let abs_manifest = workspace_root.join(rel_path);
                    if let Ok(bytes) = std::fs::read(&abs_manifest) {
                        let content_sha = hex_sha256(&bytes);
                        let basename = std::path::Path::new(rel_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| rel_path.clone());
                        manifests.push(TargetFile {
                            name: basename,
                            relpath: PathBuf::from(rel_path),
                            bytes,
                            content_sha,
                        });
                    }
                }

                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["csharp".to_string()]),
                    manifests,
                    top_level_files: vec![],
                };

                let ctx = AnalysisContext::deterministic_only();
                match proxy.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => {
                        let sub = boxed
                            .as_any()
                            .downcast_ref::<SubprocessOutput>()
                            .ok_or_else(|| {
                                ToolError::Invocation("downcast to SubprocessOutput failed".into())
                            })?;
                        Ok(sub.payload.clone())
                    }
                    AnalyzerResult::Graded { output: boxed, .. } => {
                        let sub = boxed
                            .as_any()
                            .downcast_ref::<SubprocessOutput>()
                            .ok_or_else(|| {
                                ToolError::Invocation("downcast to SubprocessOutput failed".into())
                            })?;
                        Ok(sub.payload.clone())
                    }
                    AnalyzerResult::Declines => Err(ToolError::Invocation(
                        "csharp-analyzer declined this target".into(),
                    )),
                    AnalyzerResult::Error(e) => {
                        Err(ToolError::Invocation(format!("csharp-analyzer error: {e}")))
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

    fn fingerprint_inputs(&self, _args: &ToolArgs) -> Vec<FingerprintInput> {
        vec![]
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
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When the binary is not built, the wrapper must return a descriptive
    /// `ToolError::Invocation` rather than panicking or returning an opaque
    /// error. If the binary IS built (full workspace build), the invocation
    /// succeeds — both outcomes are acceptable.
    #[tokio::test]
    async fn csharp_surface_tool_either_succeeds_or_returns_helpful_error() {
        let tempdir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tempdir.path().join("MyApp.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\" />\n",
        )
        .unwrap();

        let tool = CsharpSurfaceTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool
            .invoke(ToolArgs(serde_json::json!({ "component_dir": "" })), &ctx)
            .await;

        match result {
            Ok(_) => { /* binary was built — happy path */ }
            Err(ToolError::Invocation(msg)) => {
                assert!(
                    msg.contains("csharp-analyzer")
                        || msg.contains("not found")
                        || msg.contains("proxy"),
                    "error message should mention the analyzer: {msg}"
                );
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }
}
