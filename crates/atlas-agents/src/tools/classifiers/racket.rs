//! Pass-through wrapper for `atlas_analyzers::racket_classifier::RacketClassifier`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use atlas_analyzers::Analyzer;
use atlas_analyzers::{
    AnalysisContext, AnalyzerResult, RacketClassificationOutput, RacketClassifier, Target,
    TargetFile,
};

use crate::tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};

pub struct RacketClassifyTool;

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
        description: "Classify a Racket component by reading its info.rkt. \
             Returns kind, evidence fields, rationale and lifecycle roles."
            .into(),
    })
}

#[async_trait]
impl Tool for RacketClassifyTool {
    fn id(&self) -> &'static str {
        "classify_racket_component"
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
        let abs_dir = ctx.workspace_root.join(&component_dir);
        let abs_manifest = ctx.workspace_root.join(&info_rkt_path);

        let output = tokio::task::spawn_blocking(
            move || -> Result<RacketClassificationOutput, ToolError> {
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
                let analyser = RacketClassifier::new();
                let ctx = AnalysisContext::deterministic_only();
                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<RacketClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to RacketClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<RacketClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to RacketClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Declines => {
                        Err(ToolError::Invocation("racket classifier declined".into()))
                    }
                    AnalyzerResult::Error(e) => Err(ToolError::Invocation(format!(
                        "racket classifier error: {e}"
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
    use atlas_analyzers::Analyzer;
    use atlas_analyzers::{AnalysisContext, AnalyzerResult, RacketClassifier, Target, TargetFile};

    fn make_target(dir: &std::path::Path, info_rkt_content: &str) -> Target {
        let bytes = info_rkt_content.as_bytes().to_vec();
        let content_sha = hex_sha256(&bytes);
        Target {
            dir: dir.to_path_buf(),
            languages: BTreeSet::from(["racket".to_string()]),
            manifests: vec![TargetFile {
                name: "info.rkt".into(),
                relpath: PathBuf::from("info.rkt"),
                bytes,
                content_sha,
            }],
            top_level_files: vec!["info.rkt".to_string()],
        }
    }

    #[tokio::test]
    async fn racket_classify_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let info_rkt = "#lang info\n(define name \"my-pkg\")\n(define version \"1.0\")\n";
        std::fs::write(tempdir.path().join("info.rkt"), info_rkt).unwrap();

        let direct: RacketClassificationOutput = {
            let target = make_target(tempdir.path(), info_rkt);
            let analyser = RacketClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<RacketClassificationOutput>()
                    .unwrap()
                    .clone(),
                other => panic!("expected Confident, got {other:?}"),
            }
        };

        let tool = RacketClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "info_rkt_path": "info.rkt"
        }));
        let result = tool.invoke(tool_args, &ctx).await.unwrap();
        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn racket_classify_tool_missing_manifest_returns_filesystem_error() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = RacketClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "info_rkt_path": "info.rkt"
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::Filesystem(_))));
    }

    #[tokio::test]
    async fn racket_classify_tool_missing_arg_returns_invalid_args() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = RacketClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({ "component_dir": "" }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::InvalidArgs(_))));
    }
}
