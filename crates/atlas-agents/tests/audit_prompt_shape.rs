//! Drift catcher for the PR-4 audit prompt template + transcript
//! renderer. Asserts:
//!
//! - The verdict rubric mentions all three terminal verdict kinds
//!   (`accept`, `request_revision`, `hard_fail`) so the LLM is told
//!   what it may emit.
//! - The single embedded fenced YAML example deserializes to the
//!   `AuditorEmittedVerdict` shape — the prompt's contract is in
//!   lock-step with the parser the runtime feeds the response to.
//! - The transcript renderer truncates large args / results with the
//!   `[N bytes truncated]` hint so the auditor knows the trail is
//!   bounded.
//!
//! Companion to PR-2's `dispatch_prompt_shape.rs` and PR-3's
//! `classify_prompt_shape.rs` / `reduce_prompt_shape.rs` /
//! `project_prompt_shape.rs` — same drift-catcher pattern across the
//! production prompts.

use atlas_agents::runtime::audit::{
    build_audit_prompt, render_transcript_for_audit, AuditorEmittedVerdict, Stage, VerdictKind,
};
use atlas_agents::runtime::prompt_examples::extract_yaml_fence;
use atlas_agents::runtime::tool_loop_http::Transcript;
use serde_json::json;

#[test]
fn audit_prompt_rubric_advertises_three_verdict_kinds() {
    let prompt = build_audit_prompt(
        "anthropic",
        "openai",
        Stage::Classify,
        "<rendered producer output placeholder>",
        "<rendered transcript placeholder>",
    );
    for kind in &["accept", "request_revision", "hard_fail"] {
        assert!(
            prompt.contains(kind),
            "audit-prompt verdict rubric must advertise `{kind}` so the auditor \
             knows the kind is a valid emission"
        );
    }
}

#[test]
fn audit_prompt_embedded_yaml_example_deserializes_to_auditor_verdict() {
    // The prompt embeds exactly ONE fenced ```yaml block (the verdict
    // example). `extract_yaml_fence` enforces the one-fence contract —
    // if a future edit drifts the prose to contain another ```yaml
    // marker, this assertion catches it.
    let prompt = build_audit_prompt("anthropic", "openai", Stage::Reduce, "{}", "(no calls)");
    let body = extract_yaml_fence(&prompt)
        .expect("prompt template must contain exactly one fenced YAML block");
    let parsed: AuditorEmittedVerdict = serde_yaml::from_str(body).expect(
        "embedded YAML example must deserialize to AuditorEmittedVerdict \
         (verdict + reason); if the example shape changes, update the \
         parser at runtime/mod.rs::run_real_audit",
    );
    // The example value is `accept` so the round-trip pins the
    // snake_case label table for VerdictKind too.
    assert_eq!(parsed.verdict, VerdictKind::Accept);
    assert!(
        !parsed.reason.is_empty(),
        "example reason must be non-empty"
    );
}

#[test]
fn audit_prompt_embeds_cross_provider_pairing_labels() {
    // PR-4's whole point: a different provider audits the producer.
    // The prompt body explicitly names both sides so the auditor
    // knows it's the cross-provider partner. A drift that drops these
    // labels silently degrades the cross-provider value proposition.
    let prompt = build_audit_prompt("anthropic", "openai", Stage::Classify, "out", "trail");
    assert!(prompt.contains("anthropic"));
    assert!(prompt.contains("openai"));
}

#[test]
fn transcript_renderer_truncates_long_results_with_byte_hint() {
    let mut t = Transcript::new();
    let huge_payload = "x".repeat(20_000);
    t.push_synthetic_tool_call(
        "read_file",
        json!({"path": "very-long.txt"}),
        json!({"contents": huge_payload}),
    );
    let rendered = render_transcript_for_audit(&t);
    assert!(
        rendered.contains("bytes truncated"),
        "long results must carry the `[N bytes truncated]` hint so the \
         auditor knows the result was clipped"
    );
    // Bounded: the budget for results is 400 bytes + a small hint
    // suffix. The whole rendered transcript should stay well below
    // 1 KB even with very large inputs.
    assert!(
        rendered.len() < 1_500,
        "rendered transcript stays bounded; got len={}",
        rendered.len()
    );
}

#[test]
fn transcript_renderer_handles_empty_transcript() {
    // An empty trail must render as an empty string — the prompt
    // template embeds the (possibly empty) trail verbatim. A trailing
    // newline would visually muddle the auditor's view; the renderer
    // promises a clean empty-string for no records.
    let t = Transcript::new();
    let rendered = render_transcript_for_audit(&t);
    assert_eq!(rendered, "", "empty transcript must render as empty string");
}

#[test]
fn audit_prompt_stage_label_uses_lane_a_as_str_table() {
    // The prompt embeds the stage label via `Stage::as_str` so the
    // wire form matches Lane A's existing label table. Drift here
    // would diverge the auditor's stage-name view from on-disk
    // audit-dir layout (`<audit_dir>/<stage>/<target_id>.yaml`).
    for (stage, label) in [
        (Stage::DispatchSubsystem, "dispatch_subsystem"),
        (Stage::DispatchComponent, "dispatch_component"),
        (Stage::Classify, "classify"),
        (Stage::Surface, "surface"),
        (Stage::Reduce, "reduce"),
        (Stage::Project, "project"),
    ] {
        let prompt = build_audit_prompt("anthropic", "openai", stage, "out", "trail");
        assert!(
            prompt.contains(&format!("\n{label}\n")),
            "stage label `{label}` must appear in prompt body for stage {stage:?}; \
             got prompt:\n{prompt}"
        );
    }
}
