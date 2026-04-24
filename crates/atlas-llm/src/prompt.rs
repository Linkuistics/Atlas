//! Prompt template rendering. `{{TOKEN}}` substitution with
//! double-brace escape (`{{{{X}}}}` → literal `{{X}}`).

use std::collections::BTreeMap;

use crate::LlmError;

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
            let end = body.find("}}").ok_or_else(|| {
                LlmError::TemplateSyntax("unclosed '{{' in template".to_string())
            })?;
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
}
