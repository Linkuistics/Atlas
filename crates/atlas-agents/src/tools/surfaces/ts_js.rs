//! Pass-through wrapper for `atlas_analyzers::ts_js_surface_analyzer::TsJsSurfaceAnalyzer`.
//! Calls `extract_ts_js_surface` directly (the analyser's `analyse` method
//! intentionally returns `Declines`; the engine drives the function directly).
//! Exposes the surface analyser as a `Tool`. No behaviour change.

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use atlas_analyzers::{extract_ts_js_surface, TsJsSourceInputs};

use crate::{
    FingerprintInput as AgentsFingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult,
    ToolSchema,
};

pub struct TsJsSurfaceTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "component_id": {
                    "type": "string",
                    "description": "stable component id used for binding/library-api id namespacing"
                },
                "component_dir": {
                    "type": "string",
                    "description": "workspace-relative path to the component directory"
                },
                "is_typescript": {
                    "type": "boolean",
                    "description": "true for typescript-package, false for javascript-package; drives the language label on the emitted LibraryApi"
                },
                "package_json_path": {
                    "type": "string",
                    "description": "workspace-relative path to package.json (optional; used to resolve main/module/exports entrypoint)"
                }
            },
            "required": ["component_id", "component_dir", "is_typescript"]
        }),
        description: "Extract the TypeScript/JavaScript public-API surface of a component. \
                      Probes the engine's fixed allowlist of well-known entry-point filenames \
                      (src/index.{ts,tsx,js,jsx} + src/main.{ts,tsx,js,jsx}) and returns \
                      bindings and library_apis."
            .into(),
    })
}

/// Fixed allowlist of well-known entry-point filenames, mirroring
/// `crates/atlas-engine/src/l5_surface.rs:439-456`.
const WELL_KNOWN_ENTRY_POINTS: &[&str] = &[
    "src/index.ts",
    "src/index.tsx",
    "src/index.js",
    "src/index.jsx",
    "src/main.ts",
    "src/main.tsx",
    "src/main.js",
    "src/main.jsx",
];

/// Probe the fixed allowlist of well-known entry-point filenames under
/// `absolute_dir`.  Returns `(relative_path, bytes)` pairs for every
/// file that exists; silently skips missing or unreadable files, matching
/// the engine's `file_content(db, &candidate)` returning `None` behaviour.
fn probe_entry_points(absolute_dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut sources = Vec::new();
    for filename in WELL_KNOWN_ENTRY_POINTS {
        if let Ok(bytes) = std::fs::read(absolute_dir.join(filename)) {
            sources.push((PathBuf::from(filename), bytes));
        }
    }
    sources
}

#[async_trait]
impl Tool for TsJsSurfaceTool {
    fn id(&self) -> &'static str {
        "analyse_ts_js_surface"
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
        let is_typescript = args
            .0
            .get("is_typescript")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| ToolError::InvalidArgs("missing or non-bool `is_typescript`".into()))?;
        let package_json_path = args
            .0
            .get("package_json_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        let abs_pkg = package_json_path
            .as_deref()
            .map(|p| crate::tools::path_utils::require_within_root(&ctx.workspace_root, p))
            .transpose()?;

        let output = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
            // Probe the fixed allowlist of well-known entry-point filenames,
            // mirroring l5_surface.rs:439-456. Missing files are silently skipped.
            let sources = probe_entry_points(&abs_dir);

            // Read package.json: use the explicitly provided path if given;
            // otherwise fall back to probing `<component_dir>/package.json`,
            // mirroring l5_surface.rs:458-464. Errors on an explicit path are
            // surfaced; absence of the implicit path is silently ignored.
            let package_json = if let Some(p) = abs_pkg {
                std::fs::read(&p)
                    .map(Some)
                    .map_err(|e| ToolError::Filesystem(format!("read {}: {e}", p.display())))?
            } else {
                std::fs::read(abs_dir.join("package.json")).ok()
            };

            let inputs = TsJsSourceInputs { sources, package_json, is_typescript };
            let surface = extract_ts_js_surface(&component_id, &inputs);

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
        // The source files are discovered at invoke time; record the
        // component dir as a zero-sha sentinel — PR-4's runtime can
        // enumerate and pre-compute actual shas.
        let component_dir = match require_string(args, "component_dir") {
            Ok(d) => d,
            Err(_) => return vec![],
        };
        let mut inputs = vec![AgentsFingerprintInput {
            path: PathBuf::from(&component_dir),
            sha: [0u8; 32],
        }];
        if let Some(p) = args.0.get("package_json_path").and_then(|v| v.as_str()) {
            inputs.push(AgentsFingerprintInput {
                path: PathBuf::from(p),
                sha: [0u8; 32],
            });
        }
        inputs
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
    async fn ts_js_surface_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tempdir.path().join("src")).unwrap();
        let ts_content = "export function greet(name: string): string { return `Hello ${name}`; }\nexport type Greeting = string;\n";
        std::fs::write(tempdir.path().join("src/index.ts"), ts_content).unwrap();

        // Plant a file that is NOT in the allowlist.  A recursive-walk
        // implementation would include it; the fixed-probe must not.
        std::fs::write(
            tempdir.path().join("src/helpers.ts"),
            "export function helper(): void {}\n",
        )
        .unwrap();

        // Direct call — feeds only the allowlist hit (src/index.ts), matching
        // what the engine's fixed-filename probe produces.
        let direct = {
            let inputs = TsJsSourceInputs {
                sources: vec![(
                    PathBuf::from("src/index.ts"),
                    ts_content.as_bytes().to_vec(),
                )],
                package_json: None,
                is_typescript: true,
            };
            extract_ts_js_surface("test-ts-component", &inputs)
        };

        // Wrapper call
        let tool = TsJsSurfaceTool;
        let args = ToolArgs(serde_json::json!({
            "component_id": "test-ts-component",
            "component_dir": "",
            "is_typescript": true
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        // The wrapper's output must match the direct call that only fed
        // src/index.ts.  If the wrapper had walked src/ recursively it would
        // also include src/helpers.ts and produce a different result.
        let expected = serde_json::json!({
            "contracts": serde_json::to_value(&direct.contracts).unwrap(),
            "bindings": serde_json::to_value(&direct.bindings).unwrap(),
            "library_apis": serde_json::to_value(&direct.library_apis).unwrap(),
        });
        assert_eq!(result.output, expected);
    }
}
