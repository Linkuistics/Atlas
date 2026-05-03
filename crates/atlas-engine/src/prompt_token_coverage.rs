//! Unified bidirectional prompt/builder token-coverage check.
//!
//! For every prompt template in `defaults/prompts/`, asserts:
//!
//! 1. **Forward direction** — every `{{TOKEN}}` referenced in the
//!    template is supplied by the layer's input builder. Missing
//!    tokens cause `atlas_llm::prompt::render` to error with
//!    `TemplateSyntax`; absorbed by callers, this previously caused
//!    silent L3 LLM-fallback degradation (2026-05-02 incident).
//!
//! 2. **Inverse direction** — every key the input builder supplies is
//!    referenced by a `{{TOKEN}}` in the template. Surplus builder
//!    keys are silently dropped by `prompt::render`, leaving the LLM
//!    with no per-call context (2026-05-03 incident: classify and
//!    subcarve received only catalog tokens, no candidate context).
//!
//! Each per-layer test (`classify_prompt_token_coverage_is_bidirectional`
//! in `l3_classify`, `subcarve_prompt_token_coverage_is_bidirectional`
//! in `l8_recurse`, etc.) provides locality-of-failure for one prompt;
//! this module is the single matrix to extend when adding a new prompt
//! template. Wire the new prompt by:
//!
//! 1. Adding a parameterless `pub(crate) fn build_*_inputs_*for_tests()
//!    -> Value` to the layer that owns the new prompt.
//! 2. Adding a `Case { … }` row to `cases()` below.

use std::collections::HashSet;

use serde_json::Value;

struct Case {
    /// Display name of the prompt — appears in failure messages.
    prompt: &'static str,
    /// Display name of the input builder — appears in failure messages.
    builder: &'static str,
    /// The shipped prompt template body, embedded via `include_str!`.
    template: &'static str,
    /// Parameterless builder hook returning the inputs `Value` the
    /// engine would feed into `prompt::render` for one call.
    inputs: fn() -> Value,
    /// Keys the builder deliberately includes in the inputs JSON for
    /// cache-key fingerprinting only — never referenced by the prompt.
    /// `LlmCache` and `TestBackend` derive cache keys from the canonical
    /// JSON shape, so adding a sha-bearing field invalidates cache
    /// entries when underlying content changes without the LLM having
    /// to see the value. The inverse-direction check skips these.
    cache_only_keys: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            prompt: "classify.md",
            builder: "l3_classify::build_llm_inputs",
            template: include_str!("../../../defaults/prompts/classify.md"),
            inputs: crate::l3_classify::build_llm_inputs_for_tests,
            cache_only_keys: &[],
        },
        // `subcarve.md` is intentionally absent: as of the L8
        // map/reduce redesign the engine no longer renders that
        // template — the per-subdir map step routes through `Classify`
        // — so a token-coverage check on it would assert against an
        // unused prompt. The file remains in `EMBEDDED_PROMPTS` so its
        // sha continues to contribute to the run-wide template
        // fingerprint during the transition; deletion is slated for a
        // post-release cleanup pass.
        Case {
            prompt: "stage1-surface.md",
            builder: "l5_surface::build_inputs",
            template: include_str!("../../../defaults/prompts/stage1-surface.md"),
            inputs: crate::l5_surface::build_inputs_with_stubs_for_tests,
            // Per-segment content SHAs are baked into the inputs JSON
            // so a file-content change reshapes the cache key, even
            // though the LLM doesn't see the SHAs in the prompt.
            cache_only_keys: &["COMPONENT_CONTENT_SHAS"],
        },
        Case {
            prompt: "stage2-edges.md",
            builder: "l6_edges::build_inputs",
            template: include_str!("../../../defaults/prompts/stage2-edges.md"),
            inputs: crate::l6_edges::build_inputs_with_stubs_for_tests,
            cache_only_keys: &[],
        },
    ]
}

#[test]
fn every_prompt_and_builder_agree_on_token_set() {
    let mut failures: Vec<String> = Vec::new();

    for case in cases() {
        let inputs = (case.inputs)();
        let object = inputs
            .as_object()
            .unwrap_or_else(|| panic!("{} must return a JSON object", case.builder));

        let supplied: HashSet<String> = object.keys().cloned().collect();
        let referenced: HashSet<String> =
            collect_template_tokens(case.template).into_iter().collect();

        for token in &referenced {
            if !supplied.contains(token) {
                failures.push(format!(
                    "{} references `{{{{{token}}}}}` but {} does not populate key `{token}` \
                     — `prompt::render` will error with `TemplateSyntax` and the call will \
                     be absorbed by the caller's error handling",
                    case.prompt, case.builder
                ));
            }
        }
        let cache_only: HashSet<&str> = case.cache_only_keys.iter().copied().collect();
        for key in &supplied {
            if referenced.contains(key) || cache_only.contains(key.as_str()) {
                continue;
            }
            failures.push(format!(
                "{} supplies key `{key}` but {} does not reference `{{{{{key}}}}}` \
                 — `prompt::render` silently drops the value, leaving the LLM \
                 without that input. If the key is intentionally cache-only, \
                 add it to the case's `cache_only_keys` list with a comment.",
                case.builder, case.prompt
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "prompt/builder token coverage failed:\n  - {}",
        failures.join("\n  - ")
    );
}

/// Extract every `{{TOKEN}}` name referenced in `template`, using the
/// same grammar as `atlas_llm::prompt::render`: `{{TOKEN}}` substitutes,
/// `{{{{` and `}}}}` are literal-brace escapes.
fn collect_template_tokens(template: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = template;
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix("{{{{") {
            rest = body;
            continue;
        }
        if let Some(body) = rest.strip_prefix("}}}}") {
            rest = body;
            continue;
        }
        if let Some(body) = rest.strip_prefix("{{") {
            let end = body.find("}}").expect("template must close `{{`");
            tokens.push(body[..end].trim().to_string());
            rest = &body[end + 2..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        rest = &rest[ch.len_utf8()..];
    }
    tokens
}
