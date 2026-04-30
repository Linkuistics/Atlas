//! Prompt template corpus embedded into the CLI binary, plus helpers
//! to materialise it into a directory the `ClaudeCodeBackend` can read.
//!
//! The backend reads prompts from disk on every call, so the CLI
//! writes the four embedded templates to a directory (a tempdir in
//! practice) before constructing the backend. Keeping the templates
//! in the binary means the CLI works from any working directory
//! without requiring a shipped `defaults/` folder beside it.

use std::io;
use std::path::Path;

use atlas_llm::PromptId;

/// `(prompt id, id string, embedded body)` for every prompt the CLI
/// ships. The second element matches `atlas_llm::prompt_filename`'s
/// stem so callers that want a per-prompt cache-fingerprint entry can
/// use it as the map key without a separate lookup.
pub const EMBEDDED_PROMPTS: &[(PromptId, &str, &str)] = &[
    (
        PromptId::Classify,
        "classify",
        include_str!("../../../defaults/prompts/classify.md"),
    ),
    (
        PromptId::Subcarve,
        "subcarve",
        include_str!("../../../defaults/prompts/subcarve.md"),
    ),
    (
        PromptId::Stage1Surface,
        "stage1-surface",
        include_str!("../../../defaults/prompts/stage1-surface.md"),
    ),
    (
        PromptId::Stage2Edges,
        "stage2-edges",
        include_str!("../../../defaults/prompts/stage2-edges.md"),
    ),
];

/// `defaults/ontology.yaml`, embedded. Re-exposed so the CLI can hash
/// it into the run-wide `LlmFingerprint.ontology_sha`.
pub const EMBEDDED_ONTOLOGY_YAML: &str = component_ontology::EMBEDDED_ONTOLOGY_YAML;

/// Write every embedded prompt to `dir`, using
/// `atlas_llm::prompt_filename` as the filename. `dir` must already
/// exist.
pub fn materialise_to(dir: &Path) -> io::Result<()> {
    for (id, _name, body) in EMBEDDED_PROMPTS {
        let filename = atlas_llm::claude_code::prompt_filename(*id);
        std::fs::write(dir.join(filename), body)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn materialise_writes_every_prompt() {
        let tmp = TempDir::new().unwrap();
        materialise_to(tmp.path()).unwrap();
        for (id, _, body) in EMBEDDED_PROMPTS {
            let filename = atlas_llm::claude_code::prompt_filename(*id);
            let written = std::fs::read_to_string(tmp.path().join(filename)).unwrap();
            assert_eq!(&written, *body);
        }
    }

    #[test]
    fn every_embedded_prompt_is_non_empty() {
        for (_, name, body) in EMBEDDED_PROMPTS {
            assert!(!body.is_empty(), "{name} is empty");
        }
    }

    #[test]
    fn embedded_ontology_yaml_matches_component_ontology_crate() {
        assert_eq!(
            EMBEDDED_ONTOLOGY_YAML,
            component_ontology::EMBEDDED_ONTOLOGY_YAML
        );
    }
}
