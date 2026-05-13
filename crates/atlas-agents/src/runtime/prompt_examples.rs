//! Shared helpers for prompt-template authoring and prompt-shape tests.
//!
//! The fence-extraction function is the inverse of the LLM's "emit one
//! fenced ```yaml block" contract — it locates and returns the body so
//! Lane A can deserialize it. Production prompts in
//! [`crate::runtime::dispatch`] (and PR-3's classify / reduce / project
//! prompts) embed a canonical fenced YAML example as the
//! schema-advertisement section; the same fence shape is what the LLM
//! is expected to emit. PR-2's prompt-shape drift catcher in
//! `tests/dispatch_prompt_shape.rs` round-trips the embedded example
//! through this extractor.
//!
//! Byte-cursor scanner (no regex dependency): the markers are
//! ASCII-stable so substring search is well-defined.

use thiserror::Error;

/// Failure modes for [`extract_yaml_fence`]. The variants name the
/// specific shape violation so Lane A retry prompts can mention what
/// to fix.
#[derive(Debug, Error)]
pub enum FenceExtractError {
    #[error("no opening ```yaml fence found in LLM output")]
    NoOpeningFence,
    #[error("opening ```yaml fence at byte {open_at} has no matching closing ``` fence")]
    NoClosingFence { open_at: usize },
    #[error("multiple ```yaml fences found ({count}); LLM output must contain exactly one")]
    MultipleFences { count: usize },
}

/// Extract the body of the single fenced ```yaml block in `text`.
///
/// Returns the body as a borrowed `&str`; the caller passes it directly
/// to `serde_yaml::from_str`. Multiple fenced ```yaml blocks → error
/// (the prompts contract calls for exactly one).
///
/// # Algorithm
///
/// 1. Scan for every occurrence of the opening marker `"```yaml"`. More
///    than one is an error (the prompt contract: one envelope).
/// 2. Skip optional whitespace + newline immediately after the opening
///    marker so the body starts on the first content line.
/// 3. Find the next `"```"` after the body start; that delimits the
///    body's end.
pub fn extract_yaml_fence(text: &str) -> Result<&str, FenceExtractError> {
    let opening_marker = "```yaml";
    let closing_marker = "```";
    let mut fence_positions: Vec<usize> = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(opening_marker) {
        fence_positions.push(search_from + rel);
        search_from += rel + opening_marker.len();
    }
    if fence_positions.is_empty() {
        return Err(FenceExtractError::NoOpeningFence);
    }
    if fence_positions.len() > 1 {
        return Err(FenceExtractError::MultipleFences {
            count: fence_positions.len(),
        });
    }
    let open_at = fence_positions[0];
    let after_marker = open_at + opening_marker.len();
    // Skip optional whitespace + the newline that closes the fence-line.
    let body_start = text[after_marker..]
        .find('\n')
        .map(|nl| after_marker + nl + 1)
        .unwrap_or(after_marker);
    let close_rel = text[body_start..]
        .find(closing_marker)
        .ok_or(FenceExtractError::NoClosingFence { open_at })?;
    Ok(&text[body_start..body_start + close_rel])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_yaml_fence() {
        let text = "before\n```yaml\nschema_version: 1\n```\nafter";
        assert_eq!(extract_yaml_fence(text).unwrap(), "schema_version: 1\n");
    }

    #[test]
    fn rejects_no_opening_fence() {
        assert!(matches!(
            extract_yaml_fence("no fence here"),
            Err(FenceExtractError::NoOpeningFence)
        ));
    }

    #[test]
    fn rejects_unclosed_fence() {
        assert!(matches!(
            extract_yaml_fence("before\n```yaml\nschema_version: 1\n"),
            Err(FenceExtractError::NoClosingFence { .. })
        ));
    }

    #[test]
    fn rejects_multiple_fences() {
        let text = "```yaml\na: 1\n```\nmid\n```yaml\nb: 2\n```";
        assert!(matches!(
            extract_yaml_fence(text),
            Err(FenceExtractError::MultipleFences { count: 2 })
        ));
    }

    #[test]
    fn extracts_multiline_body_with_nested_structure() {
        let text = "preamble\n```yaml\nschema_version: 1\nsubsystems:\n  - id: agents\n    members:\n      - foo\n      - bar\n```\ntrailing";
        let body = extract_yaml_fence(text).unwrap();
        assert!(body.contains("schema_version: 1\n"));
        assert!(body.contains("- id: agents\n"));
        assert!(body.contains("- bar\n"));
    }
}
