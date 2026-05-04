//! Bidirectional prompt template / build-inputs coverage check.
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
//!    referenced by a `{{TOKEN}}` in the template (or declared
//!    cache-only). Surplus builder keys are silently dropped by
//!    `prompt::render`, leaving the LLM with no per-call context
//!    (2026-05-03 incident: classify and subcarve received only catalog
//!    tokens, no candidate context).
//!
//! ## Validation surfaces
//!
//! - **Compile time** — `build.rs` extracts each template's `{{TOKEN}}`
//!   set into the `*_TEMPLATE_TOKENS` constants generated below; the
//!   `const _: ()` blocks then compare those against each layer's
//!   `BUILD_INPUTS_KEYS` and `CACHE_ONLY_KEYS` declarations. Mismatches
//!   become hard `cargo build` failures.
//!
//! - **Test time** — a single runtime test verifies that each builder's
//!   actual JSON output keys match its declared `BUILD_INPUTS_KEYS`,
//!   catching drift between the runtime builder and its const list (the
//!   compile-time check cannot inspect runtime code).
//!
//! ## Adding a new prompt template
//!
//! 1. Add a `(CONST_NAME, "defaults/prompts/<file>.md")` row to
//!    `TEMPLATES` in `build.rs`.
//! 2. Add `pub(crate) const BUILD_INPUTS_KEYS` and `CACHE_ONLY_KEYS`
//!    declarations to the layer that owns the new prompt's builder.
//! 3. Add a `const _: () = assert_bidirectional(...)` block here.
//! 4. Add a parameterless `pub(crate) fn build_*_for_tests() -> Value`
//!    on the layer and a corresponding row to `runtime_drift_cases()`.

include!(concat!(
    env!("OUT_DIR"),
    "/prompt_token_coverage_generated.rs"
));

// --- Compile-time bidirectional validation ---------------------------
//
// Each `assert_bidirectional` call panics at compile time on mismatch,
// turning the failure into a `cargo build` error. The panic message is
// a fixed string literal because const-context `panic!` does not
// interpolate. Source-location metadata in the build error narrows down
// which template and which direction failed.

const _: () = assert_bidirectional(
    CLASSIFY_TEMPLATE_TOKENS,
    crate::l3_classify::BUILD_INPUTS_KEYS,
    crate::l3_classify::CACHE_ONLY_KEYS,
    "classify.md and l3_classify::build_llm_inputs disagree: \
     prompt template references a {{TOKEN}} that the builder does not \
     populate, OR the builder supplies a key that the template does not \
     reference (and is not in CACHE_ONLY_KEYS).",
);

const _: () = assert_bidirectional(
    STAGE1_SURFACE_TEMPLATE_TOKENS,
    crate::l5_surface::BUILD_INPUTS_KEYS,
    crate::l5_surface::CACHE_ONLY_KEYS,
    "stage1-surface.md and l5_surface::build_inputs disagree: \
     prompt template references a {{TOKEN}} that the builder does not \
     populate, OR the builder supplies a key that the template does not \
     reference (and is not in CACHE_ONLY_KEYS).",
);

const _: () = assert_bidirectional(
    STAGE2_EDGES_TEMPLATE_TOKENS,
    crate::l6_edges::BUILD_INPUTS_KEYS,
    crate::l6_edges::CACHE_ONLY_KEYS,
    "stage2-edges.md and l6_edges::build_inputs disagree: \
     prompt template references a {{TOKEN}} that the builder does not \
     populate, OR the builder supplies a key that the template does not \
     reference (and is not in CACHE_ONLY_KEYS).",
);

const fn assert_bidirectional(
    template_tokens: &[&str],
    build_inputs_keys: &[&str],
    cache_only_keys: &[&str],
    msg: &'static str,
) {
    // Forward: every `{{TOKEN}}` in the template must be in BUILD_INPUTS_KEYS.
    let mut i = 0;
    while i < template_tokens.len() {
        if !contains(build_inputs_keys, template_tokens[i]) {
            panic!("{}", msg);
        }
        i += 1;
    }
    // Inverse: every key in BUILD_INPUTS_KEYS must appear as a
    // `{{TOKEN}}` in the template, OR be declared cache-only.
    let mut i = 0;
    while i < build_inputs_keys.len() {
        let key = build_inputs_keys[i];
        if !contains(template_tokens, key) && !contains(cache_only_keys, key) {
            panic!("{}", msg);
        }
        i += 1;
    }
}

const fn contains(haystack: &[&str], needle: &str) -> bool {
    let mut i = 0;
    while i < haystack.len() {
        if str_eq(haystack[i], needle) {
            return true;
        }
        i += 1;
    }
    false
}

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// --- Runtime drift check --------------------------------------------
//
// The compile-time check compares each template against the
// `BUILD_INPUTS_KEYS` const declaration. This runtime test catches the
// remaining drift surface: the const declaration no longer matching
// what the actual builder emits at runtime. Without this, a developer
// could change `build_inputs` without updating `BUILD_INPUTS_KEYS` and
// the compile-time check would still pass against the stale const.

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    struct RuntimeDriftCase {
        builder: &'static str,
        declared_keys: &'static [&'static str],
        actual_inputs: fn() -> Value,
    }

    fn runtime_drift_cases() -> Vec<RuntimeDriftCase> {
        vec![
            RuntimeDriftCase {
                builder: "l3_classify::build_llm_inputs",
                declared_keys: crate::l3_classify::BUILD_INPUTS_KEYS,
                actual_inputs: crate::l3_classify::build_llm_inputs_for_tests,
            },
            RuntimeDriftCase {
                builder: "l5_surface::build_inputs",
                declared_keys: crate::l5_surface::BUILD_INPUTS_KEYS,
                actual_inputs: crate::l5_surface::build_inputs_with_stubs_for_tests,
            },
            RuntimeDriftCase {
                builder: "l6_edges::build_inputs",
                declared_keys: crate::l6_edges::BUILD_INPUTS_KEYS,
                actual_inputs: crate::l6_edges::build_inputs_with_stubs_for_tests,
            },
        ]
    }

    #[test]
    fn build_inputs_keys_const_matches_runtime_builder_output() {
        let mut failures: Vec<String> = Vec::new();

        for case in runtime_drift_cases() {
            let value = (case.actual_inputs)();
            let object = value
                .as_object()
                .unwrap_or_else(|| panic!("{} must return a JSON object", case.builder));

            let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
            let declared: BTreeSet<&str> = case.declared_keys.iter().copied().collect();

            for key in actual.difference(&declared) {
                failures.push(format!(
                    "{} emits key `{key}` at runtime but `BUILD_INPUTS_KEYS` does not list it \
                     — update the const so the compile-time template/builder coverage check \
                     can validate it",
                    case.builder
                ));
            }
            for key in declared.difference(&actual) {
                failures.push(format!(
                    "{} declares `BUILD_INPUTS_KEYS` entry `{key}` but the runtime builder \
                     does not emit it — remove from the const or add to the builder",
                    case.builder
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "BUILD_INPUTS_KEYS drifted from runtime builder output:\n  - {}",
            failures.join("\n  - ")
        );
    }
}
