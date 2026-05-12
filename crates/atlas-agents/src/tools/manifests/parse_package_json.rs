//! Pass-through wrapper for `atlas_engine::manifest_parse::parse_package_json`.
//! Exposes the manifest parser as a `Tool`. No behaviour change.

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::{FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema};

pub struct ParsePackageJsonTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "workspace-relative path to a package.json"
                }
            },
            "required": ["path"]
        }),
        description:
            "Parse a package.json file. Returns has_main / has_exports / has_bin booleans.".into(),
    })
}

#[async_trait]
impl Tool for ParsePackageJsonTool {
    fn id(&self) -> &'static str {
        "parse_package_json"
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
        let contents = tokio::task::spawn_blocking({
            let abs_path = abs_path.clone();
            move || {
                std::fs::read_to_string(&abs_path)
                    .map_err(|e| ToolError::Filesystem(format!("read {}: {e}", abs_path.display())))
            }
        })
        .await
        .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;
        let shape = atlas_engine::manifest_parse::parse_package_json(&contents);
        let output = serde_json::json!({
            "has_main": shape.has_main,
            "has_exports": shape.has_exports,
            "has_bin": shape.has_bin,
        });
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
    async fn parse_package_json_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content = r#"{"name":"example","version":"1.0.0","main":"index.js"}"#;
        std::fs::write(tempdir.path().join("package.json"), content).unwrap();

        // Direct call
        let direct = atlas_engine::manifest_parse::parse_package_json(content);
        let expected = serde_json::json!({
            "has_main": direct.has_main,
            "has_exports": direct.has_exports,
            "has_bin": direct.has_bin,
        });

        // Wrapper call
        let tool = ParsePackageJsonTool;
        let args = ToolArgs(serde_json::json!({ "path": "package.json" }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, expected);
    }
}
