//! Pass-through wrapper for `atlas_analyzers::compose_classifier::ComposeClassifier`.
//!
//! Wraps the CLASSIFIER (kind + evidence + rationale). Distinct from
//! PR-3a's `parse_compose` manifest-parser wrapper under `manifests/`
//! which exposes the raw structural `ComposeShape`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use atlas_analyzers::compose_classifier::{ComposeClassificationOutput, ComposeClassifier};
use atlas_analyzers::Analyzer;
use atlas_analyzers::{AnalysisContext, AnalyzerResult, Target, TargetFile};

use crate::tool::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};

pub struct ComposeClassifyTool;

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
                "compose_file_path": {
                    "type": "string",
                    "description": "Workspace-relative path to the compose file \
                                    (e.g. docker-compose.yml, compose.yaml)."
                }
            },
            "required": ["component_dir", "compose_file_path"]
        }),
        description: "Classify a Docker Compose component by reading its compose file. \
             Returns kind (`compose-orchestration`), service count, evidence fields, \
             rationale and the extracted ComposeShape."
            .into(),
    })
}

#[async_trait]
impl Tool for ComposeClassifyTool {
    fn id(&self) -> &'static str {
        "classify_compose_component"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let compose_file_path = require_string(&args, "compose_file_path")?;
        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        let abs_manifest =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &compose_file_path)?;

        let output = tokio::task::spawn_blocking(
            move || -> Result<ComposeClassificationOutput, ToolError> {
                let bytes = std::fs::read(&abs_manifest).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_manifest.display()))
                })?;
                let content_sha = hex_sha256(&bytes);
                let filename = PathBuf::from(&compose_file_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| compose_file_path.clone());

                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::new(),
                    manifests: vec![TargetFile {
                        name: filename.clone(),
                        relpath: PathBuf::from(&compose_file_path),
                        bytes,
                        content_sha,
                    }],
                    top_level_files: vec![filename],
                };
                let analyser = ComposeClassifier::new();
                let ctx = AnalysisContext::deterministic_only();
                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<ComposeClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to ComposeClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<ComposeClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to ComposeClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Declines => Err(ToolError::Invocation(
                        "compose classifier declined (no services declared)".into(),
                    )),
                    AnalyzerResult::Error(e) => Err(ToolError::Invocation(format!(
                        "compose classifier error: {e}"
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
        require_string(args, "compose_file_path")
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
    use atlas_analyzers::compose_classifier::ComposeClassifier;
    use atlas_analyzers::Analyzer;
    use atlas_analyzers::{AnalysisContext, AnalyzerResult, Target, TargetFile};

    const TWO_SERVICE_COMPOSE: &str = r#"
version: "3"
services:
  web:
    image: "myrepo/web:1"
  db:
    image: "postgres:15"
"#;

    fn make_target(dir: &std::path::Path, filename: &str, content: &str) -> Target {
        let bytes = content.as_bytes().to_vec();
        let content_sha = hex_sha256(&bytes);
        Target {
            dir: dir.to_path_buf(),
            languages: BTreeSet::new(),
            manifests: vec![TargetFile {
                name: filename.to_string(),
                relpath: PathBuf::from(filename),
                bytes,
                content_sha,
            }],
            top_level_files: vec![filename.to_string()],
        }
    }

    #[tokio::test]
    async fn compose_classify_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tempdir.path().join("docker-compose.yml"),
            TWO_SERVICE_COMPOSE,
        )
        .unwrap();

        let direct: ComposeClassificationOutput = {
            let target = make_target(tempdir.path(), "docker-compose.yml", TWO_SERVICE_COMPOSE);
            let analyser = ComposeClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<ComposeClassificationOutput>()
                    .unwrap()
                    .clone(),
                other => panic!("expected Confident, got {other:?}"),
            }
        };

        let tool = ComposeClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "compose_file_path": "docker-compose.yml"
        }));
        let result = tool.invoke(tool_args, &ctx).await.unwrap();
        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn compose_classify_tool_missing_file_returns_filesystem_error() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = ComposeClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "compose_file_path": "docker-compose.yml"
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::Filesystem(_))));
    }

    #[tokio::test]
    async fn compose_classify_tool_empty_services_returns_invocation_error() {
        let tempdir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tempdir.path().join("docker-compose.yml"),
            "version: '3'\nservices: {}\n",
        )
        .unwrap();
        let tool = ComposeClassifyTool;
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let tool_args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "compose_file_path": "docker-compose.yml"
        }));
        let result = tool.invoke(tool_args, &ctx).await;
        assert!(matches!(result, Err(ToolError::Invocation(_))));
    }
}
