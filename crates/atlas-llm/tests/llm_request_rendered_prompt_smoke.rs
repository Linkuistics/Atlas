//! WI-1 bypass smoke: a request constructed via
//! `LlmRequest::from_rendered` must succeed against any backend even
//! when `prompts_dir` is empty. A request constructed via
//! `LlmRequest::from_template` carries the prompt id through; backends
//! read the template from `prompts_dir` as before.
//!
//! The invariant under test is in the request shape, not the wire
//! shape, so this file exercises constructor behaviour directly. The
//! end-to-end "rendered request survives empty prompts_dir against an
//! HTTP backend" assertion is covered by
//! `crates/atlas-cli/tests/agent_runtime_http_smoke.rs`'s
//! `agent_runtime_http_smoke_completes_with_config_loaded_from_env`,
//! which WI-1 un-ignores.

use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use serde_json::json;

#[test]
fn rendered_prompt_request_constructs_without_prompts_dir() {
    let req = LlmRequest::from_rendered(
        "You are a test agent. Reply with: ok.".to_string(),
        ResponseSchema::accept_any(),
    );
    assert!(req.rendered_prompt.is_some());
    assert!(req.prompt_template.is_none());
    assert_eq!(
        req.rendered_prompt.as_deref().unwrap(),
        "You are a test agent. Reply with: ok."
    );
}

#[test]
fn templated_request_carries_prompt_id() {
    let req = LlmRequest::from_template(
        PromptId::Classify,
        json!({"COMPONENT_KINDS": "[]", "LIFECYCLE_SCOPES": "[]"}),
        ResponseSchema::accept_any(),
    );
    assert!(req.prompt_template.is_some());
    assert!(req.rendered_prompt.is_none());
    assert_eq!(req.prompt_template, Some(PromptId::Classify));
}
