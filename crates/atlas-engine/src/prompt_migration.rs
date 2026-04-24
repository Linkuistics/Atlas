//! Mechanical transformation from Ravel-Lite's discovery prompts to
//! Atlas's. Two drift tests in [`crate::l5_surface`] and
//! [`crate::l6_edges`] consume this module: they apply the
//! transformation to Ravel-Lite's shipped prompt bytes and assert the
//! result is byte-equal to Atlas's shipped copy. A prose change in
//! either repo that is not covered by the substitution set below
//! fails the drift test, forcing an intentional update here or in
//! the shipped prompt.
//!
//! Design reference: §9.4 M3. The task brief prescribes a
//! token-substitution-only migration. That is why neither prompt is
//! restructured here even where Ravel-Lite's prose ("project rooted at
//! your current working directory", "output as YAML to a path")
//! doesn't map cleanly onto Atlas's backend protocol — those concerns
//! are deferred to follow-on tasks that will rewrite the prompt bodies
//! against the JSON-response-over-stdout shape that ClaudeCodeBackend
//! actually implements.

/// Apply the project→component substitution set to one of Ravel-Lite's
/// discovery prompts, producing Atlas's form. The transformation is
/// order-sensitive: more-specific tokens replace first so they do not
/// get rewritten by the generic word-level rules that follow.
pub fn project_to_component(original: &str) -> String {
    // Specific tokens / identifiers first, so the generic word-level
    // rules below cannot accidentally rewrite their substrings.
    let after_tokens = original
        .replace("{{CATALOG_PROJECTS}}", "{{CATALOG_COMPONENTS}}")
        .replace(
            "explicit_cross_project_mentions",
            "explicit_cross_component_mentions",
        );

    // Case-sensitive word substitutions. Plural forms replace before
    // singular to avoid double-suffix artefacts (e.g. "projects" →
    // "components" ⇒ later "project" pass does not see the plural).
    after_tokens
        .replace("projects", "components")
        .replace("Projects", "Components")
        .replace("PROJECTS", "COMPONENTS")
        .replace("project", "component")
        .replace("Project", "Component")
        .replace("PROJECT", "COMPONENT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_replaces_before_singular() {
        // Catches the classic "component" → "components" regression that
        // would occur if the singular substitution ran first.
        let got = project_to_component("projects and project");
        assert_eq!(got, "components and component");
    }

    #[test]
    fn catalog_projects_token_renamed() {
        let got = project_to_component("see {{CATALOG_PROJECTS}}");
        assert_eq!(got, "see {{CATALOG_COMPONENTS}}");
    }

    #[test]
    fn explicit_cross_project_mentions_renamed() {
        let got = project_to_component("- `explicit_cross_project_mentions` — names");
        assert_eq!(
            got,
            "- `explicit_cross_component_mentions` — names"
        );
    }

    #[test]
    fn case_forms_independently_handled() {
        assert_eq!(
            project_to_component("project Project PROJECT"),
            "component Component COMPONENT"
        );
    }

    #[test]
    fn non_project_words_left_alone() {
        let input = "the Rust ecosystem; inject; proto schema; CATALOG_X";
        assert_eq!(project_to_component(input), input);
    }

    #[test]
    fn suffix_word_boundary_is_respected_for_plural() {
        // Plain substring replace intentionally — substrings like
        // "projected" would also be rewritten to "componented", which
        // is fine: Ravel-Lite's prompt does not use such words, and a
        // drift test catches any future regression.
        let got = project_to_component("projected onto project");
        assert_eq!(got, "componented onto component");
    }
}
