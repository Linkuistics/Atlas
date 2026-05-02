use std::path::PathBuf;

use serde_json::{json, Value};

use crate::claude_code::{extract_tokens, prompt_template_filename, validate_response};
use crate::stream_parse::strip_json_fence;
use crate::{LlmBackend, LlmError, LlmFingerprint, LlmRequest};

pub struct AnthropicHttpBackend {
    model_id: String,
    api_key: String,
    params: Value,
    prompts_dir: PathBuf,
    template_sha: [u8; 32],
    ontology_sha: [u8; 32],
    client: reqwest::blocking::Client,
}

impl AnthropicHttpBackend {
    pub fn new(
        model_id: impl Into<String>,
        api_key: impl Into<String>,
        params: Value,
        prompts_dir: impl Into<PathBuf>,
        template_sha: [u8; 32],
        ontology_sha: [u8; 32],
    ) -> Self {
        Self {
            model_id: model_id.into(),
            api_key: api_key.into(),
            params,
            prompts_dir: prompts_dir.into(),
            template_sha,
            ontology_sha,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn render_request(&self, req: &LlmRequest) -> Result<String, LlmError> {
        let path = self
            .prompts_dir
            .join(prompt_template_filename(req.prompt_template));
        let template = std::fs::read_to_string(&path)
            .map_err(|e| LlmError::Invocation(format!("failed to read {:?}: {e}", path)))?;
        let tokens = extract_tokens(&req.inputs)?;
        crate::prompt::render(&template, &tokens)
    }
}

pub(crate) fn parse_anthropic_response(
    resp: &Value,
    schema: &crate::ResponseSchema,
) -> Result<Value, LlmError> {
    let text = resp
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Parse("Anthropic response missing content[0].text".to_string()))?;
    let stripped = strip_json_fence(text);
    let value: Value = serde_json::from_str(stripped)
        .map_err(|e| LlmError::Parse(format!("Anthropic response is not valid JSON: {e}")))?;
    validate_response(&value, schema)?;
    Ok(value)
}

impl LlmBackend for AnthropicHttpBackend {
    fn call(&self, req: &LlmRequest) -> Result<Value, LlmError> {
        let rendered = self.render_request(req)?;

        let max_tokens = self
            .params
            .get("max_tokens")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                LlmError::Setup(
                    "params.max_tokens is required for the `anthropic` provider — \
                 add it to the operation's `params:` block in .atlas/config.yaml"
                        .to_string(),
                )
            })?;

        let mut body = json!({
            "model": self.model_id,
            "max_tokens": max_tokens,
            "messages": [{ "role": "user", "content": rendered }]
        });
        if let (Some(body_obj), Some(params_obj)) = (body.as_object_mut(), self.params.as_object())
        {
            for (k, v) in params_obj {
                if k != "max_tokens" {
                    body_obj.insert(k.clone(), v.clone());
                }
            }
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .map_err(|e| LlmError::Invocation(format!("Anthropic HTTP request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().unwrap_or_default();
            return Err(LlmError::Invocation(format!(
                "Anthropic API returned {status}: {}",
                &body_text[..body_text.len().min(200)]
            )));
        }

        let resp_json: Value = response.json().map_err(|e| {
            LlmError::Parse(format!("failed to parse Anthropic response body: {e}"))
        })?;
        parse_anthropic_response(&resp_json, &req.schema)
    }

    fn fingerprint(&self) -> LlmFingerprint {
        LlmFingerprint {
            template_sha: self.template_sha,
            ontology_sha: self.ontology_sha,
            model_id: self.model_id.clone(),
            backend_version: format!("anthropic-http/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResponseSchema;

    #[test]
    fn parses_anthropic_response_with_json_fence() {
        let resp = json!({
            "content": [{ "type": "text", "text": "```json\n{\"is_component\": true}\n```" }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_anthropic_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], true);
    }

    #[test]
    fn parses_anthropic_response_bare_json() {
        let resp = json!({
            "content": [{ "type": "text", "text": "{\"is_component\": false}" }]
        });
        let schema = ResponseSchema::accept_any();
        let value = parse_anthropic_response(&resp, &schema).unwrap();
        assert_eq!(value["is_component"], false);
    }

    #[test]
    fn missing_content_is_parse_error() {
        let resp = json!({ "model": "claude-haiku-4-5" });
        let schema = ResponseSchema::accept_any();
        let err = parse_anthropic_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn non_json_content_is_parse_error() {
        let resp = json!({
            "content": [{ "type": "text", "text": "not json at all" }]
        });
        let schema = ResponseSchema::accept_any();
        let err = parse_anthropic_response(&resp, &schema).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }
}
