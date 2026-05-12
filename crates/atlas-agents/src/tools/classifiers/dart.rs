//! Pass-through wrapper for `atlas_analyzers::dart_classifier::DartClassifier`.
//!
//! Exposes the Dart/Flutter L3 classifier as a `Tool` so the LLM-spine
//! runtime can invoke it directly. No new logic — pure pass-through.

use async_trait::async_trait;
use atlas_analyzers::{
    AnalysisContext, Analyzer, AnalyzerResult, DartClassificationOutput, DartClassifier, Target,
    TargetFile,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::{FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema};

/// Pass-through tool wrapping [`DartClassifier`].
pub struct DartClassifyTool;

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
                "manifest_path": {
                    "type": "string",
                    "description": "workspace-relative path to pubspec.yaml"
                }
            },
            "required": ["component_dir", "manifest_path"]
        }),
        description: "Classify a Dart or Flutter component. Returns kind (dart-package or \
             flutter-package), build system (pub), evidence fields, and rationale. \
             Pass `pubspec.yaml` as `manifest_path`."
            .into(),
    })
}

#[async_trait]
impl Tool for DartClassifyTool {
    fn id(&self) -> &'static str {
        "classify_dart_component"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let manifest_path = require_string(&args, "manifest_path")?;

        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        let abs_manifest =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &manifest_path)?;

        let manifest_basename = std::path::Path::new(&manifest_path)
            .file_name()
            .ok_or_else(|| ToolError::InvalidArgs("manifest_path has no basename".into()))?
            .to_string_lossy()
            .into_owned();

        let output =
            tokio::task::spawn_blocking(move || -> Result<DartClassificationOutput, ToolError> {
                let bytes = std::fs::read(&abs_manifest).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_manifest.display()))
                })?;
                let content_sha = hex_sha256(&bytes);

                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["dart".to_string()]),
                    manifests: vec![TargetFile {
                        name: manifest_basename,
                        relpath: PathBuf::from(&manifest_path),
                        bytes,
                        content_sha,
                    }],
                    top_level_files: vec![],
                };

                let analyser = DartClassifier::new();
                let ctx = AnalysisContext::deterministic_only();

                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<DartClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| ToolError::Invocation("downcast failed".into())),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<DartClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| ToolError::Invocation("downcast failed".into())),
                    AnalyzerResult::Declines => Err(ToolError::Invocation(
                        "dart classifier declined: no pubspec.yaml found".into(),
                    )),
                    AnalyzerResult::Error(e) => {
                        Err(ToolError::Invocation(format!("classifier error: {e}")))
                    }
                }
            })
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
        require_string(args, "manifest_path")
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
    use atlas_analyzers::{AnalysisContext, Analyzer, AnalyzerResult, Target, TargetFile};
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn dart_classify_tool_matches_direct_call_dart_package() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let manifest_body = "name: my_dart_lib\nversion: 0.1.0\ndescription: A Dart library.\n";
        std::fs::write(tempdir.path().join("pubspec.yaml"), manifest_body).unwrap();

        // Direct call.
        let bytes = manifest_body.as_bytes().to_vec();
        let content_sha = hex_sha256(&bytes);
        let target = Target {
            dir: tempdir.path().to_path_buf(),
            languages: BTreeSet::from(["dart".to_string()]),
            manifests: vec![TargetFile {
                name: "pubspec.yaml".into(),
                relpath: PathBuf::from("pubspec.yaml"),
                bytes,
                content_sha,
            }],
            top_level_files: vec![],
        };
        let direct: DartClassificationOutput =
            match DartClassifier::new().analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<DartClassificationOutput>()
                    .cloned()
                    .unwrap(),
                other => panic!("expected Confident, got {other:?}"),
            };

        // Wrapper call.
        let tool = DartClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "manifest_path": "pubspec.yaml"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn dart_classify_tool_returns_error_when_manifest_missing() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let tool = DartClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "manifest_path": "pubspec.yaml"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await;
        assert!(
            matches!(result, Err(ToolError::Filesystem(_))),
            "expected Filesystem error, got {result:?}"
        );
    }
}
