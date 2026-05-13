//! Audit-prompt template (brainstorm §7.1) + transcript-to-tuples
//! rendering (§7.2) + byte-budgeted truncators.
//!
//! The producer's output + its tool-call trail are folded into a single
//! audit prompt body. The auditor (a *different* provider — see memory
//! `feedback_cross_provider_llm_audit`) returns a fenced ```yaml verdict
//! the runtime deserializes via
//! [`crate::runtime::prompt_examples::extract_yaml_fence`] +
//! `serde_yaml::from_str`.
//!
//! Truncation budgets are conservative initial values (`200` / `400`);
//! PR-5 calibration may surface a need to adjust. The truncation hint
//! (`[N bytes truncated]`) tells the auditor it isn't seeing the full
//! result so the verdict can mention the gap if the truncated bytes
//! mattered.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::runtime::audit::verdict::VerdictKind;
use crate::runtime::audit::Stage;
use crate::runtime::tool_loop_http::{Transcript, TranscriptRecord};

/// Shape the auditor emits inside its single fenced ```yaml block.
/// Distinct from [`crate::runtime::audit::verdict::AuditVerdictOnDisk`]
/// (the on-disk record) — that one carries producer/auditor metadata
/// the runtime fills in; this one is the *LLM-emitted* envelope.
///
/// `reason` is free-form prose (no strict-string adapter) so block
/// scalars and arbitrary phrasing work without YAML implicit-typing
/// rejection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditorEmittedVerdict {
    pub verdict: VerdictKind,
    pub reason: String,
}

/// Byte budget for tool-call argument summaries inside the rendered
/// transcript. Producer's argument JSON is summarised to keep the audit
/// prompt bounded; large args get the truncation hint suffix.
pub const SUMMARISE_ARGS_BUDGET: usize = 200;

/// Byte budget for tool-call result summaries. Larger than args because
/// results (file contents, parsed manifests) carry the bulk of the
/// evidence trail.
pub const SUMMARISE_RESULT_BUDGET: usize = 400;

/// Build the audit prompt body. Inputs:
/// - `producer_provider` / `auditor_provider`: "anthropic" | "openai"
///   strings; embedded verbatim so the auditor knows the cross-provider
///   pairing.
/// - `stage`: which stage the producer was running.
/// - `producer_output_rendered`: the producer's emitted output, already
///   rendered to a string (typically the original fenced YAML body).
/// - `transcript_tuples`: ordered tool-call trail from
///   [`render_transcript_for_audit`].
///
/// The auditor is asked to emit ONE fenced ```yaml block whose body
/// deserializes to `{verdict: <kind>, reason: <prose>}`.
pub fn build_audit_prompt(
    producer_provider: &str,
    auditor_provider: &str,
    stage: Stage,
    producer_output_rendered: &str,
    transcript_tuples: &str,
) -> String {
    // Composed from string slices joined by `\n` so embedded YAML
    // block scalars (the `reason: |` example body) keep their literal
    // indentation. Rust's `\n\` line-continuation escape consumes
    // leading whitespace on the next source line, which would
    // un-indent block-scalar content — `\n`-joined slices sidestep
    // that hazard.
    let lines: Vec<String> = vec![
        format!(
            "You are an auditor for an Atlas agent's output. \
             The producer is a {producer_provider} model; you are a \
             {auditor_provider} model. Your role is to evaluate the \
             producer's *semantic soundness given the evidence trail*, \
             not its coverage (coverage is verified separately by Lane A)."
        ),
        String::new(),
        "# Producer's stage".to_string(),
        stage.as_str().to_string(),
        String::new(),
        "# Producer's output".to_string(),
        producer_output_rendered.to_string(),
        String::new(),
        "# Producer's evidence trail (ordered tool calls + their results)".to_string(),
        transcript_tuples.to_string(),
        String::new(),
        "# Verdict shape".to_string(),
        "Emit ONE fenced YAML block (markdown triple-backtick + 'yaml' tag) \
         in this shape:"
            .to_string(),
        String::new(),
        "```yaml".to_string(),
        "verdict: accept            # one of: accept | request_revision | hard_fail".to_string(),
        "reason: |".to_string(),
        "  One-paragraph rationale. Use a block scalar so multi-sentence prose".to_string(),
        "  reads cleanly. State explicitly which evidence in the producer's".to_string(),
        "  transcript supports or contradicts the producer's output.".to_string(),
        "```".to_string(),
        String::new(),
        "# Verdict rubric".to_string(),
        "- accept: output is consistent with the evidence; reasoning is sound.".to_string(),
        "- request_revision: output has correctable issues — provide the reason".to_string(),
        "  in plain language; the producer will retry with your reason as".to_string(),
        "  additional context.".to_string(),
        "- hard_fail: output is unsalvageable given the evidence; the stage".to_string(),
        "  cannot produce useful output on this target.".to_string(),
        String::new(),
    ];
    lines.join("\n")
}

/// Render `transcript`'s tool-call trail as ordered numbered tuples:
///
/// ```text
/// 1. tool: read_file
///    args: {"path":"crates/atlas-cli/Cargo.toml"}
///    result: {"contents":"[package]\nname = \"atlas-cli\"\n..."}
/// 2. tool: parse_cargo_toml
///    ...
/// ```
///
/// Args + results are byte-budget-summarised. Non-`ToolResult` records
/// (`AssistantTurn`, `McpEvent`) are skipped — they're not part of the
/// evidence trail the auditor cares about. An empty transcript renders
/// as the empty string; the caller's prompt template embeds this so an
/// empty trail surfaces explicitly to the auditor.
pub fn render_transcript_for_audit(transcript: &Transcript) -> String {
    let mut out = String::new();
    let mut idx = 0usize;
    for record in transcript.records() {
        if let TranscriptRecord::ToolResult {
            tool_name,
            args,
            output,
            ..
        } = record
        {
            idx += 1;
            writeln!(out, "{idx}. tool: {tool_name}").ok();
            writeln!(
                out,
                "   args: {}",
                summarise_value(args, SUMMARISE_ARGS_BUDGET)
            )
            .ok();
            writeln!(
                out,
                "   result: {}",
                summarise_value(output, SUMMARISE_RESULT_BUDGET)
            )
            .ok();
        }
    }
    out
}

/// Byte-budgeted JSON-value summariser. Serialises `value` to its
/// canonical JSON form, then either passes it through unchanged (when
/// it fits the budget) or truncates with the standard hint suffix
/// `... [N bytes truncated]`. UTF-8-boundary-safe: truncates at the
/// nearest char boundary <= byte_budget rather than slicing into a
/// multibyte codepoint.
pub fn summarise_value(value: &serde_json::Value, byte_budget: usize) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    if raw.len() <= byte_budget {
        return raw;
    }
    let mut cut = byte_budget;
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    let truncated_bytes = raw.len() - cut;
    format!("{}... [{truncated_bytes} bytes truncated]", &raw[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn audit_prompt_contains_all_three_verdict_kinds_in_rubric() {
        let prompt = build_audit_prompt(
            "anthropic",
            "openai",
            Stage::Classify,
            "<rendered producer output>",
            "<rendered transcript tuples>",
        );
        for kind in &["accept", "request_revision", "hard_fail"] {
            assert!(
                prompt.contains(kind),
                "rubric must mention `{kind}` verdict kind; got prompt:\n{prompt}"
            );
        }
    }

    #[test]
    fn audit_prompt_embeds_cross_provider_pairing() {
        let prompt = build_audit_prompt("anthropic", "openai", Stage::Reduce, "out", "transcript");
        assert!(prompt.contains("producer is a anthropic"));
        assert!(prompt.contains("you are a openai"));
    }

    #[test]
    fn audit_prompt_embeds_stage_label() {
        let prompt = build_audit_prompt("anthropic", "openai", Stage::Project, "out", "trail");
        assert!(prompt.contains("project"), "stage label must be present");
    }

    #[test]
    fn render_transcript_emits_numbered_tuples() {
        let mut t = Transcript::new();
        t.push_synthetic_tool_call(
            "read_file",
            json!({"path": "Cargo.toml"}),
            json!({"contents": "[package]\nname = \"atlas\""}),
        );
        t.push_synthetic_tool_call(
            "parse_cargo_toml",
            json!({"path": "Cargo.toml"}),
            json!({"package_name": "atlas"}),
        );
        let rendered = render_transcript_for_audit(&t);
        assert!(rendered.contains("1. tool: read_file"));
        assert!(rendered.contains("2. tool: parse_cargo_toml"));
        assert!(rendered.contains("args: "));
        assert!(rendered.contains("result: "));
    }

    #[test]
    fn render_transcript_truncates_long_result() {
        let mut t = Transcript::new();
        let huge = "x".repeat(10_000);
        t.push_synthetic_tool_call(
            "read_file",
            json!({"path": "huge.txt"}),
            json!({"contents": huge}),
        );
        let rendered = render_transcript_for_audit(&t);
        assert!(
            rendered.contains("bytes truncated"),
            "long results must carry the truncation hint; got len={}",
            rendered.len()
        );
        assert!(
            rendered.len() < 1_500,
            "rendered transcript stays bounded; got len={}",
            rendered.len()
        );
    }

    #[test]
    fn render_transcript_truncates_long_args() {
        let mut t = Transcript::new();
        let bulky_args = json!({"path": "x".repeat(5_000)});
        t.push_synthetic_tool_call("grep", bulky_args, json!({"hits": 0}));
        let rendered = render_transcript_for_audit(&t);
        assert!(rendered.contains("bytes truncated"));
    }

    #[test]
    fn render_empty_transcript_yields_empty_string() {
        let t = Transcript::new();
        assert_eq!(render_transcript_for_audit(&t), "");
    }

    #[test]
    fn summarise_value_honours_byte_budget_exactly_when_under() {
        let small = json!({"k": "v"});
        let out = summarise_value(&small, 200);
        // Compact serialisation: {"k":"v"} = 9 bytes; well under budget,
        // passes through unchanged.
        assert_eq!(out, r#"{"k":"v"}"#);
    }

    #[test]
    fn summarise_value_truncates_when_over_budget() {
        let big = json!({"x": "a".repeat(500)});
        let out = summarise_value(&big, 200);
        assert!(out.len() > 200, "truncation suffix adds length");
        assert!(out.starts_with(r#"{"x":""#));
        assert!(out.contains("bytes truncated"));
    }

    #[test]
    fn summarise_value_truncates_at_char_boundary() {
        // Multi-byte UTF-8: each `é` is 2 bytes. If the budget falls
        // mid-codepoint, the summariser must back off to the prior
        // boundary rather than slicing into the codepoint (which would
        // panic in `&raw[..cut]`).
        let multibyte = json!({"x": "é".repeat(150)}); // ~302 bytes plus envelope
                                                       // Force cut to potentially land mid-codepoint.
        let out = summarise_value(&multibyte, 199);
        assert!(out.contains("bytes truncated"));
        // No panic = char-boundary handling works.
    }

    #[test]
    fn assistant_turn_records_are_not_rendered() {
        // Only ToolResult records belong in the evidence trail. An
        // assistant turn (the model's reasoning text) shouldn't leak
        // into the auditor's view — that would risk the auditor
        // anchoring on the producer's stated reasoning rather than
        // judging the evidence independently.
        let mut t = Transcript::new();
        t.record_assistant_turn(&json!({"role": "assistant", "content": "thinking..."}));
        t.push_synthetic_tool_call("read_file", json!({"path": "x"}), json!({"contents": "y"}));
        let rendered = render_transcript_for_audit(&t);
        assert!(!rendered.contains("thinking"));
        assert!(rendered.contains("read_file"));
        // The single tool call should be numbered `1`, not `2` — assistant
        // turns don't consume the index either.
        assert!(rendered.starts_with("1. tool: read_file"));
    }
}
