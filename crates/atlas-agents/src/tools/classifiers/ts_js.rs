//! Pass-through wrapper for `atlas_analyzers::ts_js_classifier::TsJsClassifier`.
//! Exposes the L3 classifier as a `Tool`. No behaviour change.

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use atlas_analyzers::{
    ts_js_classifier::TsJsClassificationOutput, AnalysisContext, Analyzer, AnalyzerResult, Target,
    TargetFile, TsJsClassifier,
};

use crate::{
    FingerprintInput as AgentsFingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult,
    ToolSchema,
};

pub struct TsJsClassifyTool;

static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();

fn schema() -> &'static ToolSchema {
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "component_dir": {
                    "type": "string",
                    "description": "workspace-relative path to the candidate directory"
                },
                "package_json_path": {
                    "type": "string",
                    "description": "workspace-relative path to the package.json manifest"
                },
                "tsconfig_json_path": {
                    "type": "string",
                    "description": "workspace-relative path to tsconfig.json (optional; omit for JavaScript-only packages)"
                }
            },
            "required": ["component_dir", "package_json_path"]
        }),
        description: "Classify a TypeScript or JavaScript package. Returns kind \
                      (typescript-package / javascript-package) plus evidence fields."
            .into(),
    })
}

#[async_trait]
impl Tool for TsJsClassifyTool {
    fn id(&self) -> &'static str {
        "classify_ts_js_component"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let package_json_path = require_string(&args, "package_json_path")?;
        let tsconfig_path = args
            .0
            .get("tsconfig_json_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let abs_dir = ctx.workspace_root.join(&component_dir);
        let abs_pkg = ctx.workspace_root.join(&package_json_path);
        let abs_tsconfig = tsconfig_path.as_deref().map(|p| ctx.workspace_root.join(p));

        let output =
            tokio::task::spawn_blocking(move || -> Result<TsJsClassificationOutput, ToolError> {
                let pkg_bytes = std::fs::read(&abs_pkg).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_pkg.display()))
                })?;
                let pkg_sha = hex_sha256(&pkg_bytes);

                let mut manifests = vec![TargetFile {
                    name: "package.json".into(),
                    relpath: PathBuf::from("package.json"),
                    bytes: pkg_bytes,
                    content_sha: pkg_sha,
                }];

                if let Some(ref abs_ts) = abs_tsconfig {
                    let ts_bytes = std::fs::read(abs_ts).map_err(|e| {
                        ToolError::Filesystem(format!("read {}: {e}", abs_ts.display()))
                    })?;
                    let ts_sha = hex_sha256(&ts_bytes);
                    manifests.push(TargetFile {
                        name: "tsconfig.json".into(),
                        relpath: PathBuf::from("tsconfig.json"),
                        bytes: ts_bytes,
                        content_sha: ts_sha,
                    });
                }

                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["typescript".to_string()]),
                    manifests,
                    top_level_files: vec![],
                };

                let analyser = TsJsClassifier::new();
                let ctx = AnalysisContext::deterministic_only();
                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<TsJsClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to TsJsClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<TsJsClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to TsJsClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Declines => {
                        Err(ToolError::Invocation("classifier declined".into()))
                    }
                    AnalyzerResult::Error(e) => {
                        Err(ToolError::Invocation(format!("classifier error: {e}")))
                    }
                }
            })
            .await
            .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;

        let value = serde_json::to_value(&output)
            .map_err(|e| ToolError::Invocation(format!("serialize output: {e}")))?;
        let bytes = serde_json::to_vec(&value)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        Ok(ToolResult {
            output: value,
            bytes,
        })
    }

    fn fingerprint_inputs(&self, args: &ToolArgs) -> Vec<AgentsFingerprintInput> {
        let mut inputs = Vec::new();
        if let Ok(p) = require_string(args, "package_json_path") {
            inputs.push(AgentsFingerprintInput {
                path: PathBuf::from(p),
                sha: [0u8; 32],
            });
        }
        if let Some(p) = args.0.get("tsconfig_json_path").and_then(|v| v.as_str()) {
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
    use crate::{ToolArgs, ToolContext};

    #[tokio::test]
    async fn ts_js_classify_tool_matches_direct_call_typescript() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let pkg_content = r#"{"name":"my-app","version":"1.0.0"}"#;
        let ts_content = r#"{"compilerOptions":{"target":"ES2020"}}"#;
        std::fs::write(tempdir.path().join("package.json"), pkg_content).unwrap();
        std::fs::write(tempdir.path().join("tsconfig.json"), ts_content).unwrap();

        // Direct call
        let direct = {
            let pkg_bytes = pkg_content.as_bytes().to_vec();
            let ts_bytes = ts_content.as_bytes().to_vec();
            let target = Target {
                dir: tempdir.path().to_path_buf(),
                languages: BTreeSet::from(["typescript".to_string()]),
                manifests: vec![
                    TargetFile {
                        name: "package.json".into(),
                        relpath: PathBuf::from("package.json"),
                        bytes: pkg_bytes,
                        content_sha: "ignored".into(),
                    },
                    TargetFile {
                        name: "tsconfig.json".into(),
                        relpath: PathBuf::from("tsconfig.json"),
                        bytes: ts_bytes,
                        content_sha: "ignored".into(),
                    },
                ],
                top_level_files: vec![],
            };
            let analyser = TsJsClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<TsJsClassificationOutput>()
                    .unwrap()
                    .clone(),
                _ => panic!("expected Confident"),
            }
        };

        // Wrapper call
        let tool = TsJsClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "package_json_path": "package.json",
            "tsconfig_json_path": "tsconfig.json"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }
}
