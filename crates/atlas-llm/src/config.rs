use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasConfig {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    pub defaults: OperationConfig,
    #[serde(default)]
    pub operations: OperationsConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationConfig {
    pub model: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OperationsConfig {
    pub classify: Option<OperationConfig>,
    pub subcarve: Option<OperationConfig>,
    pub surface: Option<OperationConfig>,
    pub edges: Option<OperationConfig>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found at {path} — run `atlas init <root>` first")]
    NotFound { path: String },
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse config.yaml: {0}")]
    Parse(String),
    #[error("env var `{name}` is unset (referenced in config.yaml)")]
    EnvVarUnset { name: String },
    #[error("defaults.model is required in config.yaml")]
    MissingDefaultModel,
    #[error("provider `{provider}` is used but not configured in providers:")]
    MissingProviderEntry { provider: String },
    #[error("providers.{provider}.api_key is empty after interpolation")]
    EmptyApiKey { provider: String },
}

pub(crate) fn interpolate_env_vars(s: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(s.len());
    let mut first = true;
    for line in s.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let comment_start = find_yaml_comment_start(line);
        let (active, comment) = line.split_at(comment_start);
        interpolate_segment(active, &mut out)?;
        out.push_str(comment);
    }
    Ok(out)
}

// Returns the index of the first `#` that begins a YAML comment on `line`,
// or `line.len()` if none. A `#` starts a comment when it is at column 0 or
// preceded by ASCII whitespace; this matches YAML's rule for flow-out
// comments and is sufficient for Atlas's config templates. Quoted-string
// edge cases are not modelled — no template ships `#` inside a quoted value.
fn find_yaml_comment_start(line: &str) -> usize {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return i;
        }
    }
    line.len()
}

fn interpolate_segment(s: &str, out: &mut String) -> Result<(), ConfigError> {
    let mut rest = s;
    while let Some(pos) = rest.find("${") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let end = after.find('}').ok_or_else(|| {
            ConfigError::Parse(format!(
                "unclosed '${{' in config.yaml near: {}",
                &rest[pos..rest.len().min(pos + 20)]
            ))
        })?;
        let name = &after[..end];
        let value = std::env::var(name).map_err(|_| ConfigError::EnvVarUnset {
            name: name.to_string(),
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(())
}

impl AtlasConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Err(ConfigError::NotFound {
                path: path.display().to_string(),
            });
        }
        let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let interpolated = interpolate_env_vars(&raw)?;
        let config: AtlasConfig =
            serde_yaml::from_str(&interpolated).map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.defaults.model.is_empty() {
            return Err(ConfigError::MissingDefaultModel);
        }

        let all_models = std::iter::once(&self.defaults.model).chain(
            [
                self.operations.classify.as_ref(),
                self.operations.subcarve.as_ref(),
                self.operations.surface.as_ref(),
                self.operations.edges.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(|op| &op.model),
        );

        const HTTP_PROVIDERS: &[&str] = &["anthropic", "openai"];

        for model in all_models {
            let provider = model.split('/').next().unwrap_or("");
            if HTTP_PROVIDERS.contains(&provider) {
                let entry = self.providers.get(provider).ok_or_else(|| {
                    ConfigError::MissingProviderEntry {
                        provider: provider.to_string(),
                    }
                })?;
                if entry.api_key.is_empty() {
                    return Err(ConfigError::EmptyApiKey {
                        provider: provider.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Return the resolved `OperationConfig` for a given prompt, falling
    /// back to `defaults` when the operation has no explicit entry.
    pub fn resolve_operation(&self, prompt_id: crate::PromptId) -> &OperationConfig {
        let op = match prompt_id {
            crate::PromptId::Classify => self.operations.classify.as_ref(),
            crate::PromptId::Subcarve => self.operations.subcarve.as_ref(),
            crate::PromptId::Stage1Surface => self.operations.surface.as_ref(),
            crate::PromptId::Stage2Edges => self.operations.edges.as_ref(),
        };
        op.unwrap_or(&self.defaults)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_minimal_config() {
        let yaml = r#"
defaults:
  model: "anthropic/claude-haiku-4-5"
"#;
        let config: AtlasConfig = serde_yaml::from_str(yaml).expect("parse ok");
        assert_eq!(config.defaults.model, "anthropic/claude-haiku-4-5");
        assert!(config.providers.is_empty());
        assert!(config.operations.classify.is_none());
    }

    #[test]
    fn round_trips_full_config() {
        let yaml = r#"
providers:
  anthropic:
    api_key: "sk-test"
defaults:
  model: "anthropic/claude-sonnet-4-6"
  params:
    temperature: 0
operations:
  classify:
    model: "anthropic/claude-haiku-4-5"
  surface:
    model: "claude-code/claude-sonnet-4-6"
"#;
        let config: AtlasConfig = serde_yaml::from_str(yaml).expect("parse ok");
        assert_eq!(config.providers["anthropic"].api_key, "sk-test");
        assert_eq!(
            config.operations.classify.as_ref().unwrap().model,
            "anthropic/claude-haiku-4-5"
        );
        assert!(config.operations.subcarve.is_none());
        assert_eq!(
            config.operations.surface.as_ref().unwrap().model,
            "claude-code/claude-sonnet-4-6"
        );
    }

    #[test]
    fn interpolates_env_var() {
        std::env::set_var("_ATLAS_TEST_KEY", "hello");
        let result = interpolate_env_vars("prefix_${_ATLAS_TEST_KEY}_suffix").unwrap();
        assert_eq!(result, "prefix_hello_suffix");
        std::env::remove_var("_ATLAS_TEST_KEY");
    }

    #[test]
    fn passthrough_when_no_placeholders() {
        let result = interpolate_env_vars("no placeholders here").unwrap();
        assert_eq!(result, "no placeholders here");
    }

    #[test]
    fn unset_env_var_is_error() {
        std::env::remove_var("_ATLAS_DEFINITELY_UNSET_XYZ");
        let err = interpolate_env_vars("${_ATLAS_DEFINITELY_UNSET_XYZ}").unwrap_err();
        assert!(
            matches!(err, ConfigError::EnvVarUnset { name } if name == "_ATLAS_DEFINITELY_UNSET_XYZ")
        );
    }

    #[test]
    fn placeholder_in_full_comment_line_is_skipped() {
        std::env::remove_var("_ATLAS_UNSET_IN_COMMENT");
        let yaml = "# example: api_key: ${_ATLAS_UNSET_IN_COMMENT}\nkey: value\n";
        let out = interpolate_env_vars(yaml).expect("comment-only ${} must not be expanded");
        assert_eq!(out, yaml);
    }

    #[test]
    fn placeholder_in_indented_comment_is_skipped() {
        std::env::remove_var("_ATLAS_UNSET_IN_INDENTED_COMMENT");
        let yaml = "providers:\n  #     api_key: ${_ATLAS_UNSET_IN_INDENTED_COMMENT}\n";
        let out = interpolate_env_vars(yaml).expect("indented comment ${} must not be expanded");
        assert_eq!(out, yaml);
    }

    #[test]
    fn placeholder_before_trailing_comment_is_expanded_only_in_value() {
        std::env::set_var("_ATLAS_VALUE_KEY", "real-secret");
        std::env::remove_var("_ATLAS_COMMENT_KEY");
        let yaml = "api_key: ${_ATLAS_VALUE_KEY} # fallback was ${_ATLAS_COMMENT_KEY}\n";
        let out = interpolate_env_vars(yaml).unwrap();
        assert_eq!(
            out,
            "api_key: real-secret # fallback was ${_ATLAS_COMMENT_KEY}\n"
        );
        std::env::remove_var("_ATLAS_VALUE_KEY");
    }

    #[test]
    fn hash_inside_value_without_preceding_whitespace_is_not_a_comment() {
        std::env::set_var("_ATLAS_HASH_VAL", "ok");
        // `foo#${VAR}` has `#` with no preceding whitespace — it is part of
        // the value (e.g. a URL fragment), not a comment, so the placeholder
        // after it must still be expanded.
        let out = interpolate_env_vars("url: foo#${_ATLAS_HASH_VAL}\n").unwrap();
        assert_eq!(out, "url: foo#ok\n");
        std::env::remove_var("_ATLAS_HASH_VAL");
    }

    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(yaml: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{yaml}").unwrap();
        f
    }

    #[test]
    fn load_minimal_valid_config() {
        let f = write_config("defaults:\n  model: \"claude-code/claude-sonnet-4-6\"\n");
        let config = AtlasConfig::load(f.path()).unwrap();
        assert_eq!(config.defaults.model, "claude-code/claude-sonnet-4-6");
    }

    #[test]
    fn load_missing_file_is_not_found_error() {
        let err = AtlasConfig::load(std::path::Path::new("/no/such/file.yaml")).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound { .. }));
    }

    #[test]
    fn load_missing_defaults_model_is_error() {
        let f = write_config("defaults:\n  params:\n    temperature: 0\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn load_rejects_empty_defaults_model() {
        let f = write_config("defaults:\n  model: \"\"\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::MissingDefaultModel));
    }

    #[test]
    fn load_rejects_http_provider_missing_entry() {
        let f = write_config("defaults:\n  model: \"anthropic/claude-haiku-4-5\"\n");
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::MissingProviderEntry { provider } if provider == "anthropic"
        ));
    }

    #[test]
    fn load_rejects_empty_api_key_after_interpolation() {
        std::env::set_var("_ATLAS_TEST_EMPTY_KEY", "");
        let f = write_config(
            "providers:\n  anthropic:\n    api_key: \"${_ATLAS_TEST_EMPTY_KEY}\"\ndefaults:\n  model: \"anthropic/claude-haiku-4-5\"\n",
        );
        let err = AtlasConfig::load(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::EmptyApiKey { .. }));
        std::env::remove_var("_ATLAS_TEST_EMPTY_KEY");
    }

    #[test]
    fn claude_code_provider_needs_no_providers_entry() {
        let f = write_config("defaults:\n  model: \"claude-code/claude-sonnet-4-6\"\n");
        AtlasConfig::load(f.path()).expect("should succeed — claude-code needs no credentials");
    }
}
