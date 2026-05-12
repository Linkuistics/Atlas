//! Pass-through wrapper for `atlas_analyzers::rust_surface_analyzer::RustSurfaceAnalyzer`.
//! Calls `extract_rust_surface` directly (the analyser's `analyse` method
//! intentionally returns `Declines`; the engine drives the function directly).
//! Exposes the surface analyser as a `Tool`. No behaviour change.

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use atlas_analyzers::{extract_rust_surface, RustSourceInputs};

use crate::{
    FingerprintInput as AgentsFingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult,
    ToolSchema,
};

pub struct RustSurfaceTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "component_id": {
                    "type": "string",
                    "description": "stable component id used for binding/contract id namespacing (e.g. my-crate)"
                },
                "component_dir": {
                    "type": "string",
                    "description": "workspace-relative path to the component directory (used to locate src/lib.rs and src/main.rs)"
                }
            },
            "required": ["component_id", "component_dir"]
        }),
        description: "Extract the Rust public-API surface of a component. Parses src/lib.rs and \
                      src/main.rs with syn and returns contracts, bindings, and library_apis."
            .into(),
    })
}

#[async_trait]
impl Tool for RustSurfaceTool {
    fn id(&self) -> &'static str {
        "analyse_rust_surface"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_id = require_string(&args, "component_id")?;
        let component_dir = require_string(&args, "component_dir")?;
        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;

        let output = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
            let candidates = [
                ("src/lib.rs", abs_dir.join("src/lib.rs")),
                ("src/main.rs", abs_dir.join("src/main.rs")),
            ];

            let mut sources: Vec<(PathBuf, Vec<u8>)> = Vec::new();
            for (rel, abs) in &candidates {
                match std::fs::read(abs) {
                    Ok(bytes) => sources.push((PathBuf::from(rel), bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Not every crate has both files; skip silently.
                    }
                    Err(e) => {
                        return Err(ToolError::Filesystem(format!(
                            "read {}: {e}",
                            abs.display()
                        )));
                    }
                }
            }

            let inputs = RustSourceInputs { sources };
            let surface = extract_rust_surface(&component_id, &inputs);

            let value = serde_json::json!({
                "contracts": serde_json::to_value(&surface.contracts)
                    .map_err(|e| ToolError::Invocation(format!("serialize contracts: {e}")))?,
                "bindings": serde_json::to_value(&surface.bindings)
                    .map_err(|e| ToolError::Invocation(format!("serialize bindings: {e}")))?,
                "library_apis": serde_json::to_value(&surface.library_apis)
                    .map_err(|e| ToolError::Invocation(format!("serialize library_apis: {e}")))?,
            });
            Ok(value)
        })
        .await
        .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;

        let bytes = serde_json::to_vec(&output)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        Ok(ToolResult { output, bytes })
    }

    fn fingerprint_inputs(&self, args: &ToolArgs) -> Vec<AgentsFingerprintInput> {
        // Record the two source files as fingerprint inputs with zero-shas.
        // PR-4's runtime pre-computes actual shas before invoke.
        let component_dir = match require_string(args, "component_dir") {
            Ok(d) => d,
            Err(_) => return vec![],
        };
        vec![
            AgentsFingerprintInput {
                path: PathBuf::from(&component_dir).join("src/lib.rs"),
                sha: [0u8; 32],
            },
            AgentsFingerprintInput {
                path: PathBuf::from(&component_dir).join("src/main.rs"),
                sha: [0u8; 32],
            },
        ]
    }
}

fn require_string(args: &ToolArgs, field: &str) -> Result<String, ToolError> {
    args.0
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or non-string `{field}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolArgs, ToolContext};

    #[tokio::test]
    async fn rust_surface_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tempdir.path().join("src")).unwrap();
        let lib_content = "#[derive(serde::Serialize, serde::Deserialize)]\npub struct Foo { pub x: u32 }\n\npub fn bar() {}\n";
        std::fs::write(tempdir.path().join("src/lib.rs"), lib_content).unwrap();

        // Direct call
        let direct = {
            let inputs = RustSourceInputs {
                sources: vec![(PathBuf::from("src/lib.rs"), lib_content.as_bytes().to_vec())],
            };
            extract_rust_surface("test-component", &inputs)
        };

        // Wrapper call
        let tool = RustSurfaceTool;
        let args = ToolArgs(serde_json::json!({
            "component_id": "test-component",
            "component_dir": ""
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        let expected = serde_json::json!({
            "contracts": serde_json::to_value(&direct.contracts).unwrap(),
            "bindings": serde_json::to_value(&direct.bindings).unwrap(),
            "library_apis": serde_json::to_value(&direct.library_apis).unwrap(),
        });
        assert_eq!(result.output, expected);
    }
}
