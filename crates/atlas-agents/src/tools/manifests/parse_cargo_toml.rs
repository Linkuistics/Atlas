//! Pass-through wrapper for `atlas_engine::manifest_parse::parse_cargo_toml`.
//! Exposes the manifest parser as a `Tool`. No behaviour change.

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::{FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema};

pub struct ParseCargoTomlTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "workspace-relative path to a Cargo.toml"
                }
            },
            "required": ["path"]
        }),
        description: "Parse a Cargo.toml file. Returns has_lib_section / has_bin_section / \
                      has_workspace_section / has_package_section booleans."
            .into(),
    })
}

#[async_trait]
impl Tool for ParseCargoTomlTool {
    fn id(&self) -> &'static str {
        "parse_cargo_toml"
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
        let abs_path = ctx.workspace_root.join(path_str);
        let contents = tokio::task::spawn_blocking({
            let abs_path = abs_path.clone();
            move || {
                std::fs::read_to_string(&abs_path)
                    .map_err(|e| ToolError::Filesystem(format!("read {}: {e}", abs_path.display())))
            }
        })
        .await
        .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;
        let shape = atlas_engine::manifest_parse::parse_cargo_toml(&contents);
        let output = serde_json::json!({
            "has_lib_section": shape.has_lib_section,
            "has_bin_section": shape.has_bin_section,
            "has_workspace_section": shape.has_workspace_section,
            "has_package_section": shape.has_package_section,
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
    async fn parse_cargo_toml_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content =
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n";
        std::fs::write(tempdir.path().join("Cargo.toml"), content).unwrap();

        // Direct call
        let direct = atlas_engine::manifest_parse::parse_cargo_toml(content);
        let expected = serde_json::json!({
            "has_lib_section": direct.has_lib_section,
            "has_bin_section": direct.has_bin_section,
            "has_workspace_section": direct.has_workspace_section,
            "has_package_section": direct.has_package_section,
        });

        // Wrapper call
        let tool = ParseCargoTomlTool;
        let args = ToolArgs(serde_json::json!({ "path": "Cargo.toml" }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, expected);
    }
}
