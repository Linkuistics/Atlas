//! Pass-through wrapper for `atlas_analyzers::lispkit_classifier::LispKitClassifier`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use atlas_analyzers::Analyzer;
use atlas_analyzers::{
    AnalysisContext, AnalyzerResult, LispKitClassificationOutput, LispKitClassifier, Target,
    TargetFile,
};

use crate::tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};

pub struct LispKitClassifyTool;

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
            "Classify a LispKit/R7RS component by reading its *.sld library definition files. \
             Returns kind, evidence fields, rationale and lifecycle roles."
                .into(),
    })
}

#[async_trait]
impl Tool for LispKitClassifyTool {
    fn id(&self) -> &'static str {
        "classify_lispkit_component"
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
        let abs_dir = ctx.workspace_root.join(&component_dir);
        let workspace_root = ctx.workspace_root.clone();

        let output = tokio::task::spawn_blocking(
            move || -> Result<LispKitClassificationOutput, ToolError> {
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
                let analyser = LispKitClassifier::new();
                let ctx = AnalysisContext::deterministic_only();
                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<LispKitClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to LispKitClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<LispKitClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to LispKitClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Declines => {
                        Err(ToolError::Invocation("lispkit classifier declined".into()))
                    }
                    AnalyzerResult::Error(e) => Err(ToolError::Invocation(format!(
                        "lispkit classifier error: {e}"
                    ))),
                }
            },
        )
        .await
        .map_err(|e| ToolError::Invocation(format!("blocking task panicked: {e}")))??;

        let value = serde_json::to_value(&output)
            .map_err(|e| ToolError::Invocation(format!("serialize: {e}")))?;
        let bytes = serde_json::to_vec(&value)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        Ok(ToolResult {
            output: value,
            bytes,
        })
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
    use atlas_analyzers::Analyzer;
    use atlas_analyzers::{AnalysisContext, AnalyzerResult, LispKitClassifier, Target, TargetFile};

    fn make_target(dir: &std::path::Path, sld_name: &str, sld_content: &str) -> Target {
        let bytes = sld_content.as_bytes().to_vec();
        let content_sha = hex_sha256(&bytes);
        Target {
            dir: dir.to_path_buf(),
            languages: BTreeSet::from(["scheme".to_string()]),
            manifests: vec![TargetFile {
                name: sld_name.to_string(),
                relpath: PathBuf::from(sld_name),
                bytes,
                content_sha,
            }],
            top_level_files: vec![sld_name.to_string()],
        }
    }

    #[tokio::test]
    async fn lispkit_classify_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let sld_content =
            "(define-library (mylib)\n  (export foo)\n  (begin (define (foo x) x)))\n";
        std::fs::write(tempdir.path().join("mylib.sld"), sld_content).unwrap();

        let direct: LispKitClassificationOutput = {
            let target = make_target(tempdir.path(), "mylib.sld", sld_content);
            let analyser = LispKitClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<LispKitClassificationOutput>()
                    .unwrap()
                    .clone(),
                other => panic!("expected Confident, got {other:?}"),
            }
        };

        let tool = LispKitClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "sld_files": ["mylib.sld"]
        }));
        let result = tool.invoke(tool_args, &ctx).await.unwrap();
        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn lispkit_classify_tool_missing_file_returns_filesystem_error() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = LispKitClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "sld_files": ["missing.sld"]
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::Filesystem(_))));
    }

    #[tokio::test]
    async fn lispkit_classify_tool_empty_sld_files_returns_invalid_args() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = LispKitClassifyTool;
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
}
