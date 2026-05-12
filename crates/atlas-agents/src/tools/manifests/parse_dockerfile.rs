//! Pass-through wrapper for `atlas_analyzers::dockerfile_classifier::parse_dockerfile`.
//! Exposes the manifest parser as a `Tool`. No behaviour change.

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::{FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema};

pub struct ParseDockerfileTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "workspace-relative path to a Dockerfile"
                }
            },
            "required": ["path"]
        }),
        description: "Parse a Dockerfile. Returns from_images, copy_directives, labels, envs, \
                      exposed_ports, cmd, and entrypoint."
            .into(),
    })
}

#[async_trait]
impl Tool for ParseDockerfileTool {
    fn id(&self) -> &'static str {
        "parse_dockerfile"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path_str = args
            .0
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing or non-string `path`".into()))?;
        let abs_path =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, path_str)?;

        let output = tokio::task::spawn_blocking({
            let abs_path = abs_path.clone();
            move || -> Result<serde_json::Value, ToolError> {
                let contents = std::fs::read_to_string(&abs_path).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_path.display()))
                })?;
                let shape = atlas_analyzers::dockerfile_classifier::parse_dockerfile(&contents);
                serde_json::to_value(&shape)
                    .map_err(|e| ToolError::Invocation(format!("serialize DockerfileShape: {e}")))
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
        args.0
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| {
                vec![FingerprintInput {
                    path: PathBuf::from(p),
                    sha: [0u8; 32],
                }]
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolArgs, ToolContext};

    #[tokio::test]
    async fn parse_dockerfile_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content = "FROM alpine:3.20\nCOPY src/ /app/src/\nEXPOSE 8080\nCMD [\"./server\"]\n";
        std::fs::write(tempdir.path().join("Dockerfile"), content).unwrap();

        // Direct call
        let direct = atlas_analyzers::dockerfile_classifier::parse_dockerfile(content);
        let expected = serde_json::to_value(&direct).unwrap();

        // Wrapper call
        let tool = ParseDockerfileTool;
        let args = ToolArgs(serde_json::json!({ "path": "Dockerfile" }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, expected);
    }
}
