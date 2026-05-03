//! Prompt template rendering. `{{TOKEN}}` substitution with
//! double-brace escape (`{{{{X}}}}` → literal `{{X}}`).
//!
//! Templates may declare a single `<!-- CACHE_BOUNDARY -->` marker on
//! its own line. [`render_split`] uses the marker to split the rendered
//! output into a stable prefix (everything before the marker) and a
//! variable suffix (everything after). Backends that support prompt
//! caching (Anthropic HTTP) attach `cache_control` to the prefix block
//! so identical prefixes across calls hit the provider-side cache.

use std::collections::BTreeMap;

use crate::LlmError;

/// Marker line that splits a template into a cacheable prefix and a
/// per-call suffix. Must appear on its own line.
pub const CACHE_BOUNDARY_MARKER: &str = "<!-- CACHE_BOUNDARY -->";

/// Render a template, then split on [`CACHE_BOUNDARY_MARKER`].
///
/// Returns `(prefix, Some(suffix))` when the marker is present, or
/// `(rendered, None)` when it is absent. The marker line itself is
/// dropped from the output so backends do not have to filter it.
pub fn render_split(
    template: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<(String, Option<String>), LlmError> {
    let rendered = render(template, tokens)?;
    match split_at_cache_boundary(&rendered) {
        Some((prefix, suffix)) => Ok((prefix.to_string(), Some(suffix.to_string()))),
        None => Ok((rendered, None)),
    }
}

fn split_at_cache_boundary(rendered: &str) -> Option<(&str, &str)> {
    let idx = rendered.find(CACHE_BOUNDARY_MARKER)?;
    let prefix = &rendered[..idx];
    let after = &rendered[idx + CACHE_BOUNDARY_MARKER.len()..];
    // Drop a single trailing newline that immediately follows the
    // marker on its own line, and any leading newline at the start of
    // the suffix, so the split is byte-clean.
    let prefix = prefix.strip_suffix('\n').unwrap_or(prefix);
    let suffix = after.strip_prefix('\n').unwrap_or(after);
    Some((prefix, suffix))
}

pub fn render(template: &str, tokens: &BTreeMap<String, String>) -> Result<String, LlmError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix("{{{{") {
            out.push_str("{{");
            rest = body;
            continue;
        }
        if let Some(body) = rest.strip_prefix("}}}}") {
            out.push_str("}}");
            rest = body;
            continue;
        }
        if let Some(body) = rest.strip_prefix("{{") {
            let end = body
                .find("}}")
                .ok_or_else(|| LlmError::TemplateSyntax("unclosed '{{' in template".to_string()))?;
            let name = body[..end].trim();
            let value = tokens.get(name).ok_or_else(|| {
                LlmError::TemplateSyntax(format!("unknown token `{{{{{name}}}}}` in template"))
            })?;
            out.push_str(value);
            rest = &body[end + 2..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_single_token() {
        let mut tokens = BTreeMap::new();
        tokens.insert("NAME".to_string(), "atlas".to_string());

        let out = render("Hello {{NAME}}.", &tokens).expect("render ok");

        assert_eq!(out, "Hello atlas.");
    }

    #[test]
    fn missing_token_is_error() {
        let tokens = BTreeMap::new();

        let err = render("Hello {{NAME}}.", &tokens).unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("NAME"),
            "error message should name the missing token, got {msg:?}"
        );
    }

    #[test]
    fn substitutes_multiple_tokens() {
        let mut tokens = BTreeMap::new();
        tokens.insert("GREETING".to_string(), "Hi".to_string());
        tokens.insert("NAME".to_string(), "world".to_string());

        let out = render("{{GREETING}}, {{NAME}}!", &tokens).expect("render ok");

        assert_eq!(out, "Hi, world!");
    }

    #[test]
    fn plain_text_passes_through() {
        let tokens = BTreeMap::new();

        let out = render("no tokens here", &tokens).expect("render ok");

        assert_eq!(out, "no tokens here");
    }

    #[test]
    fn token_name_is_trimmed() {
        let mut tokens = BTreeMap::new();
        tokens.insert("NAME".to_string(), "atlas".to_string());

        let out = render("Hello {{  NAME  }}.", &tokens).expect("render ok");

        assert_eq!(out, "Hello atlas.");
    }

    #[test]
    fn unclosed_brace_is_error() {
        let tokens = BTreeMap::new();

        let err = render("Hello {{NAME", &tokens).unwrap_err();

        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn double_brace_escape_leaves_literal_braces() {
        let tokens = BTreeMap::new();

        // `{{{{X}}}}` should render as literal `{{X}}` — i.e. the
        // template author asked for braces in the output, not a
        // substitution.
        let out = render("literal: {{{{X}}}}", &tokens).expect("render ok");

        assert_eq!(out, "literal: {{X}}");
    }

    #[test]
    fn utf8_non_token_text_is_preserved() {
        let mut tokens = BTreeMap::new();
        tokens.insert("GREETING".to_string(), "Привет".to_string());

        let out = render("日本語 {{GREETING}} 🙂", &tokens).expect("render ok");

        assert_eq!(out, "日本語 Привет 🙂");
    }

    #[test]
    fn render_split_returns_none_suffix_when_marker_absent() {
        let mut tokens = BTreeMap::new();
        tokens.insert("NAME".to_string(), "atlas".to_string());

        let (prefix, suffix) =
            render_split("Hello {{NAME}}.", &tokens).expect("render_split ok");

        assert_eq!(prefix, "Hello atlas.");
        assert!(suffix.is_none());
    }

    #[test]
    fn render_split_returns_prefix_and_suffix_when_marker_present() {
        let mut tokens = BTreeMap::new();
        tokens.insert("CATALOG".to_string(), "kinds: a, b, c".to_string());
        tokens.insert("INPUT".to_string(), "candidate-42".to_string());

        let template = "stable: {{CATALOG}}\n<!-- CACHE_BOUNDARY -->\nvariable: {{INPUT}}\n";

        let (prefix, suffix) = render_split(template, &tokens).expect("render_split ok");

        assert_eq!(prefix, "stable: kinds: a, b, c");
        assert_eq!(suffix.as_deref(), Some("variable: candidate-42\n"));
    }

    #[test]
    fn render_split_handles_marker_at_end_of_template() {
        let tokens = BTreeMap::new();

        let template = "everything stable\n<!-- CACHE_BOUNDARY -->\n";

        let (prefix, suffix) = render_split(template, &tokens).expect("render_split ok");

        assert_eq!(prefix, "everything stable");
        assert_eq!(suffix.as_deref(), Some(""));
    }
}
