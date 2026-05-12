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
                    "description": "workspace-relative path to the component directory (src/ subdirectory is walked for .ts/.tsx/.js/.jsx source files)"
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
                      Parses src/**/*.ts/tsx/js/jsx with swc and returns bindings and library_apis."
            .into(),
    })
}

/// Source file extensions recognised by the TS/JS surface analyser.
const TS_JS_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "mjs", "cjs", "jsx"];

fn is_ts_js_source(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| TS_JS_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

/// Walk a directory recursively and collect all TS/JS source files as
/// `(relative_path, bytes)` pairs.  `relative_to` is used to strip the
/// component directory prefix so paths are component-relative.
fn collect_sources(
    dir: &std::path::Path,
    relative_to: &std::path::Path,
) -> Result<Vec<(PathBuf, Vec<u8>)>, ToolError> {
    let mut sources = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        // Directory may not exist (e.g. no `src/` yet). Return empty.
        return Ok(sources);
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources.extend(collect_sources(&path, relative_to)?);
        } else if is_ts_js_source(&path) {
            let rel = path
                .strip_prefix(relative_to)
                .unwrap_or(&path)
                .to_path_buf();
            let bytes = std::fs::read(&path)
                .map_err(|e| ToolError::Filesystem(format!("read {}: {e}", path.display())))?;
            sources.push((rel, bytes));
        }
    }
    // Sort deterministically so the output is stable across runs.
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(sources)
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

        let abs_dir = ctx.workspace_root.join(&component_dir);
        let abs_pkg = package_json_path
            .as_deref()
            .map(|p| ctx.workspace_root.join(p));

        let output = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ToolError> {
            // Walk the src/ subdirectory for source files; if it doesn't
            // exist, fall back to the component dir itself.
            let src_dir = abs_dir.join("src");
            let walk_root = if src_dir.is_dir() { src_dir.clone() } else { abs_dir.clone() };
            let sources = collect_sources(&walk_root, &abs_dir)?;

            let package_json = abs_pkg
                .map(|p| std::fs::read(&p).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", p.display()))
                }))
                .transpose()?;

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

        // Direct call — mirrors what the wrapper does
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

        let expected = serde_json::json!({
            "contracts": serde_json::to_value(&direct.contracts).unwrap(),
            "bindings": serde_json::to_value(&direct.bindings).unwrap(),
            "library_apis": serde_json::to_value(&direct.library_apis).unwrap(),
        });
        assert_eq!(result.output, expected);
    }
}
