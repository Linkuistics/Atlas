//! Pass-through wrapper for `atlas_analyzers::cargo_classifier::CargoClassifier`.
//! Exposes the L3 classifier as a `Tool`. No behaviour change.

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use atlas_analyzers::{
    cargo_classifier::CargoClassificationOutput, AnalysisContext, Analyzer, AnalyzerResult,
    CargoClassifier, Target, TargetFile,
};

use crate::{
    FingerprintInput as AgentsFingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult,
    ToolSchema,
};

pub struct CargoClassifyTool;

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
                "cargo_toml_path": {
                    "type": "string",
                    "description": "workspace-relative path to the Cargo.toml manifest"
                }
            },
            "required": ["component_dir", "cargo_toml_path"]
        }),
        description: "Classify a Rust/Cargo component by reading its Cargo.toml. Returns kind \
                      (workspace / rust-cli / rust-library) plus evidence fields."
            .into(),
    })
}

#[async_trait]
impl Tool for CargoClassifyTool {
    fn id(&self) -> &'static str {
        "classify_cargo_component"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn json_schema(&self) -> &ToolSchema {
        schema()
    }

    async fn invoke(&self, args: ToolArgs, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let component_dir = require_string(&args, "component_dir")?;
        let cargo_toml_path = require_string(&args, "cargo_toml_path")?;
        let abs_dir =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &component_dir)?;
        let abs_manifest =
            crate::tools::path_utils::require_within_root(&ctx.workspace_root, &cargo_toml_path)?;

        let output =
            tokio::task::spawn_blocking(move || -> Result<CargoClassificationOutput, ToolError> {
                let bytes = std::fs::read(&abs_manifest).map_err(|e| {
                    ToolError::Filesystem(format!("read {}: {e}", abs_manifest.display()))
                })?;
                let content_sha = hex_sha256(&bytes);
                let target = Target {
                    dir: abs_dir,
                    languages: BTreeSet::from(["rust".to_string()]),
                    manifests: vec![TargetFile {
                        name: "Cargo.toml".into(),
                        relpath: PathBuf::from("Cargo.toml"),
                        bytes,
                        content_sha,
                    }],
                    top_level_files: vec![],
                };
                let analyser = CargoClassifier::new();
                let ctx = AnalysisContext::deterministic_only();
                match analyser.analyse(&ctx, &target) {
                    AnalyzerResult::Confident(boxed) => boxed
                        .as_any()
                        .downcast_ref::<CargoClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to CargoClassificationOutput failed".into(),
                            )
                        }),
                    AnalyzerResult::Graded { output: boxed, .. } => boxed
                        .as_any()
                        .downcast_ref::<CargoClassificationOutput>()
                        .cloned()
                        .ok_or_else(|| {
                            ToolError::Invocation(
                                "downcast to CargoClassificationOutput failed".into(),
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
        require_string(args, "cargo_toml_path")
            .ok()
            .map(|p| {
                vec![AgentsFingerprintInput {
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
    use crate::{ToolArgs, ToolContext};

    #[tokio::test]
    async fn cargo_classify_tool_matches_direct_call_rust_library() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content = "[package]\nname = \"foo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n";
        std::fs::write(tempdir.path().join("Cargo.toml"), content).unwrap();

        // Direct call
        let direct = {
            let bytes = std::fs::read(tempdir.path().join("Cargo.toml")).unwrap();
            let target = Target {
                dir: tempdir.path().to_path_buf(),
                languages: BTreeSet::from(["rust".to_string()]),
                manifests: vec![TargetFile {
                    name: "Cargo.toml".into(),
                    relpath: PathBuf::from("Cargo.toml"),
                    bytes,
                    content_sha: "ignored".into(),
                }],
                top_level_files: vec![],
            };
            let analyser = CargoClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<CargoClassificationOutput>()
                    .unwrap()
                    .clone(),
                other => panic!("expected Confident, got {other:?}"),
            }
        };

        // Wrapper call
        let tool = CargoClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "cargo_toml_path": "Cargo.toml"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn cargo_classify_tool_matches_direct_call_rust_cli() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content = "[package]\nname = \"foo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[[bin]]\nname = \"foo\"\npath = \"src/main.rs\"\n";
        std::fs::write(tempdir.path().join("Cargo.toml"), content).unwrap();

        // Direct call
        let direct = {
            let bytes = std::fs::read(tempdir.path().join("Cargo.toml")).unwrap();
            let target = Target {
                dir: tempdir.path().to_path_buf(),
                languages: BTreeSet::from(["rust".to_string()]),
                manifests: vec![TargetFile {
                    name: "Cargo.toml".into(),
                    relpath: PathBuf::from("Cargo.toml"),
                    bytes,
                    content_sha: "ignored".into(),
                }],
                top_level_files: vec![],
            };
            let analyser = CargoClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<CargoClassificationOutput>()
                    .unwrap()
                    .clone(),
                other => panic!("expected Confident, got {other:?}"),
            }
        };

        // Wrapper call
        let tool = CargoClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "cargo_toml_path": "Cargo.toml"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }

    #[tokio::test]
    async fn cargo_classify_tool_matches_direct_call() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let content = "[workspace]\nmembers = []\n";
        std::fs::write(tempdir.path().join("Cargo.toml"), content).unwrap();

        // Direct call
        let direct = {
            let bytes = std::fs::read(tempdir.path().join("Cargo.toml")).unwrap();
            let target = Target {
                dir: tempdir.path().to_path_buf(),
                languages: BTreeSet::from(["rust".to_string()]),
                manifests: vec![TargetFile {
                    name: "Cargo.toml".into(),
                    relpath: PathBuf::from("Cargo.toml"),
                    bytes,
                    content_sha: "ignored".into(),
                }],
                top_level_files: vec![],
            };
            let analyser = CargoClassifier::new();
            match analyser.analyse(&AnalysisContext::deterministic_only(), &target) {
                AnalyzerResult::Confident(boxed) => boxed
                    .as_any()
                    .downcast_ref::<CargoClassificationOutput>()
                    .unwrap()
                    .clone(),
                _ => panic!("expected Confident"),
            }
        };

        // Wrapper call
        let tool = CargoClassifyTool;
        let args = ToolArgs(serde_json::json!({
            "component_dir": "",
            "cargo_toml_path": "Cargo.toml"
        }));
        let ctx = ToolContext {
            workspace_root: tempdir.path().to_path_buf(),
        };
        let result = tool.invoke(args, &ctx).await.unwrap();

        assert_eq!(result.output, serde_json::to_value(&direct).unwrap());
    }
}
