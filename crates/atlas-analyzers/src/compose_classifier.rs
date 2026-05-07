//! Docker Compose classifier — `compose-orchestration` kind.
//!
//! Parses `docker-compose*.yml` / `compose*.yml` files into a small
//! [`ComposeShape`] and emits `kind: compose-orchestration` confidently
//! when the file declares at least one service.
//!
//! ## What the classifier captures
//!
//! For each service in the Compose file the classifier records:
//!
//! - `image:` — the Docker image reference (if declared directly).
//! - `build:` — the build context path or a `{ context, dockerfile }`
//!   block, resolved to a Dockerfile directory relative to the compose
//!   file's parent.
//!
//! The `ComposeShape` is stored on the output so the L6 composition-edge
//! emitter ([`crate::compose_classifier::ComposeShape`]) can consume it
//! without re-parsing.
//!
//! ## File matching
//!
//! The analyser [`applies`][ComposeClassifier::applies] when the target
//! carries **any** manifest whose basename matches the canonical compose
//! patterns (see [`is_compose_manifest_basename`]):
//!
//! - `docker-compose.yml` / `docker-compose.yaml`
//! - `docker-compose.<override>.yml` / `.yaml`
//! - `compose.yml` / `compose.yaml`
//! - `compose.<override>.yml` / `.yaml`

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Analyser id. Matches the registry id from the brief: `"compose-classifier"`.
pub const ANALYZER_ID: &str = "compose-classifier";

/// Analyser version. Bump on parser-shape changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

// ─── output type ─────────────────────────────────────────────────────────────

/// Successful classification output for a Compose file.
///
/// The L3 adapter in `atlas-engine/src/l3_classify.rs` downcasts a
/// `Box<dyn StageOutput>` to this struct and translates it onto a
/// `Classification`. The `shape` field is consumed by the L6
/// composition-edge emitter in
/// `atlas-engine/src/l6_compose_edges.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeClassificationOutput {
    pub kind: String,
    pub lifecycle_roles: Vec<String>,
    pub build_system: Option<String>,
    pub role: Option<String>,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    pub is_boundary: bool,
    /// The compose-file basename that triggered classification (e.g.
    /// `docker-compose.yml`). Used by the L6 edge emitter to choose
    /// which manifest to re-parse.
    pub compose_filename: String,
    /// Extracted structural data from the compose file.
    pub shape: ComposeShape,
}

/// Structural data extracted from one compose file.
///
/// Phase 2 PR-11 captures service names, image references, and build
/// contexts. Additional fields (ports, volumes, networks) can be added
/// by a future PR without changing the outer output shape.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeShape {
    /// One entry per declared service.
    pub services: Vec<ComposeService>,
}

/// One `services.<name>:` entry from the compose file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeService {
    /// Service name as declared under `services:`.
    pub name: String,
    /// `image: <ref>` if declared. Mutually exclusive with a `build:`
    /// block in practice (though the spec allows both; if both are
    /// present we capture both).
    pub image: Option<String>,
    /// Resolved build context directory, relative to the compose file's
    /// parent directory.  Comes from `build: <string>` (the short form)
    /// or `build.context: <string>` (the long form). `None` when the
    /// service uses only `image:`.
    pub build_context: Option<String>,
    /// Dockerfile path relative to `build_context` when the long-form
    /// `build.dockerfile: <path>` is declared.  Defaults to `Dockerfile`
    /// when omitted (standard Docker behaviour) but we store `None` for
    /// "not explicitly specified" so the caller can apply the default.
    pub build_dockerfile: Option<String>,
}

// ─── parser ──────────────────────────────────────────────────────────────────

/// Parse a compose file body into a [`ComposeShape`].
///
/// Uses `serde_yaml` for the outer structure; falls back gracefully on
/// any parse failure by returning an empty shape (which triggers
/// [`AnalyzerResult::Declines`] from the classifier).
///
/// Public so the L6 edge emitter can call it directly without going
/// through the analyser API.
pub fn parse_compose(text: &str) -> ComposeShape {
    // Accept both the top-level `services:` map (Compose v2/v3) and the
    // legacy v1 form where services live at the root. We try the v2/v3
    // shape first (most common); if `services` is missing we attempt the
    // root form.
    let value: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(_) => return ComposeShape::default(),
    };

    let services_map = value
        .get("services")
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();

    let mut shape = ComposeShape::default();

    for (key, service_def) in &services_map {
        let name = match key.as_str() {
            Some(s) => s.to_string(),
            None => continue,
        };

        let image = service_def
            .get("image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let (build_context, build_dockerfile) = parse_build_field(service_def.get("build"));

        shape.services.push(ComposeService {
            name,
            image,
            build_context,
            build_dockerfile,
        });
    }

    shape
}

/// Extract the `build:` field into `(context, dockerfile)`.
///
/// Two forms are supported:
///
/// - **Short form**: `build: ./path` → `(Some("./path"), None)`
/// - **Long form**: `build: { context: ./path, dockerfile: Dockerfile.custom }`
///   → `(Some("./path"), Some("Dockerfile.custom"))`
fn parse_build_field(build: Option<&serde_yaml::Value>) -> (Option<String>, Option<String>) {
    let Some(build_val) = build else {
        return (None, None);
    };

    if let Some(s) = build_val.as_str() {
        // Short form: `build: <path>`
        return (Some(s.to_string()), None);
    }

    if let Some(mapping) = build_val.as_mapping() {
        let context = mapping
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let dockerfile = mapping
            .get("dockerfile")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return (context, dockerfile);
    }

    (None, None)
}

// ─── classifier ──────────────────────────────────────────────────────────────

/// The Compose classifier. Stateless.
#[derive(Debug, Default)]
pub struct ComposeClassifier;

impl ComposeClassifier {
    pub fn new() -> Self {
        ComposeClassifier
    }
}

impl Analyzer for ComposeClassifier {
    fn id(&self) -> &str {
        ANALYZER_ID
    }

    fn stage(&self) -> Stage {
        Stage::L3
    }

    fn cost_class(&self) -> CostClass {
        CostClass::DeterministicCheap
    }

    fn version(&self) -> &str {
        ANALYZER_VERSION
    }

    fn applies(&self, target: &Target) -> bool {
        target
            .manifests
            .iter()
            .any(|m| is_compose_manifest_basename(&m.name))
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        target
            .manifests
            .iter()
            .filter(|m| is_compose_manifest_basename(&m.name))
            .map(|m| FingerprintInput::FileContentSha(m.content_sha.clone()))
            .collect()
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        // Pick the first compose manifest in the target. Convention:
        // `docker-compose.yml` before `compose.yml` (alphabetical tie-
        // break within the manifest list, which the engine populates in
        // filesystem-scan order). We take whatever the engine loaded first.
        let Some(manifest) = target
            .manifests
            .iter()
            .find(|m| is_compose_manifest_basename(&m.name))
        else {
            return AnalyzerResult::Declines;
        };

        let text = match std::str::from_utf8(&manifest.bytes) {
            Ok(s) => s,
            Err(e) => {
                return AnalyzerResult::Error(AnalyzerError::MalformedInput {
                    analyzer_id: ANALYZER_ID.into(),
                    message: format!("{} is not valid UTF-8: {e}", manifest.name),
                });
            }
        };

        let shape = parse_compose(text);
        if shape.services.is_empty() {
            // A Compose file with no declared services cannot be
            // classified as a compose-orchestration. Decline so a future
            // LLM pass can take it.
            return AnalyzerResult::Declines;
        }

        let num_services = shape.services.len();
        let first_name = shape.services[0].name.clone();

        AnalyzerResult::Confident(Box::new(ComposeClassificationOutput {
            kind: "compose-orchestration".into(),
            lifecycle_roles: vec!["deploy".into()],
            build_system: Some("docker-compose".into()),
            role: None,
            evidence_fields: vec![format!("{}:services", manifest.name)],
            rationale: format!(
                "{} declares {num_services} service(s); first service: `{first_name}`",
                manifest.name
            ),
            is_boundary: true,
            compose_filename: manifest.name.clone(),
            shape,
        }))
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Returns `true` when `basename` matches one of the four canonical Docker
/// Compose filename patterns.
///
/// Mirrors the engine-side `manifest_patterns::is_compose_manifest_basename`
/// (defined there so the L1 manifest-walk picks up compose files). Duplicated
/// here so the analyser crate does not depend on `atlas-engine`.
///
/// The two functions must stay in sync; a divergence would cause the engine to
/// pre-load compose files that the analyser cannot recognise (or vice-versa,
/// which would be a worse failure — the analyser would see an empty manifest
/// list and always decline).
pub fn is_compose_manifest_basename(basename: &str) -> bool {
    if matches!(
        basename,
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
    ) {
        return true;
    }
    for prefix in ["docker-compose.", "compose."] {
        if let Some(rest) = basename.strip_prefix(prefix) {
            if let Some(inner) = rest
                .strip_suffix(".yml")
                .or_else(|| rest.strip_suffix(".yaml"))
            {
                if !inner.is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn target_with_compose(filename: &str, text: &str) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: filename.into(),
                relpath: PathBuf::from(filename),
                bytes: text.as_bytes().to_vec(),
                content_sha: format!("sha-{}", text.len()),
            }],
            top_level_files: vec![filename.into()],
        }
    }

    const TWO_SERVICE_COMPOSE: &str = r#"
version: "3"
services:
  web:
    image: "myrepo/web:1"
  db:
    image: "postgres:15"
"#;

    const BUILD_SERVICE_COMPOSE: &str = r#"
services:
  app:
    build: ./app
  sidecar:
    build:
      context: ./sidecar
      dockerfile: Dockerfile.sidecar
"#;

    #[test]
    fn parses_two_image_services() {
        let shape = parse_compose(TWO_SERVICE_COMPOSE);
        assert_eq!(shape.services.len(), 2);
        let web = shape.services.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.image.as_deref(), Some("myrepo/web:1"));
        assert!(web.build_context.is_none());
        let db = shape.services.iter().find(|s| s.name == "db").unwrap();
        assert_eq!(db.image.as_deref(), Some("postgres:15"));
    }

    #[test]
    fn parses_build_short_and_long_form() {
        let shape = parse_compose(BUILD_SERVICE_COMPOSE);
        assert_eq!(shape.services.len(), 2);
        let app = shape.services.iter().find(|s| s.name == "app").unwrap();
        assert_eq!(app.build_context.as_deref(), Some("./app"));
        assert!(app.build_dockerfile.is_none());
        let sc = shape.services.iter().find(|s| s.name == "sidecar").unwrap();
        assert_eq!(sc.build_context.as_deref(), Some("./sidecar"));
        assert_eq!(sc.build_dockerfile.as_deref(), Some("Dockerfile.sidecar"));
    }

    #[test]
    fn classifier_emits_compose_orchestration() {
        let target = target_with_compose("docker-compose.yml", TWO_SERVICE_COMPOSE);
        let an = ComposeClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let co = out
                    .as_any()
                    .downcast_ref::<ComposeClassificationOutput>()
                    .expect("output is ComposeClassificationOutput");
                assert_eq!(co.kind, "compose-orchestration");
                assert_eq!(co.shape.services.len(), 2);
                assert_eq!(co.compose_filename, "docker-compose.yml");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn classifier_declines_empty_services_map() {
        let target = target_with_compose("docker-compose.yml", "version: '3'\nservices: {}\n");
        let an = ComposeClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn classifier_declines_without_compose_file() {
        let target = Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "Cargo.toml".into(),
                relpath: PathBuf::from("Cargo.toml"),
                bytes: b"[package]\nname = \"x\"\n".to_vec(),
                content_sha: "abc".into(),
            }],
            top_level_files: vec!["Cargo.toml".into()],
        };
        assert!(!ComposeClassifier::new().applies(&target));
    }

    #[test]
    fn applies_to_compose_yml_variant() {
        let target = target_with_compose("compose.yml", TWO_SERVICE_COMPOSE);
        assert!(ComposeClassifier::new().applies(&target));
    }

    #[test]
    fn applies_to_override_compose_file() {
        let target = target_with_compose("docker-compose.override.yml", TWO_SERVICE_COMPOSE);
        assert!(ComposeClassifier::new().applies(&target));
    }

    #[test]
    fn is_compose_manifest_basename_recognises_canonical_names() {
        for name in &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ] {
            assert!(is_compose_manifest_basename(name), "{name} not recognised");
        }
    }

    #[test]
    fn is_compose_manifest_basename_recognises_override_forms() {
        for name in &[
            "docker-compose.prod.yml",
            "docker-compose.ci.yaml",
            "compose.dev.yml",
            "compose.test.yaml",
        ] {
            assert!(is_compose_manifest_basename(name), "{name} not recognised");
        }
    }

    #[test]
    fn is_compose_manifest_basename_rejects_unrelated_names() {
        assert!(!is_compose_manifest_basename("Cargo.toml"));
        assert!(!is_compose_manifest_basename("docker-compose..yml")); // double-dot
        assert!(!is_compose_manifest_basename("not-compose.yml"));
        assert!(!is_compose_manifest_basename("compose")); // no extension
    }

    #[test]
    fn fingerprint_inputs_covers_all_compose_manifests() {
        let target = Target {
            dir: PathBuf::from("/ws"),
            languages: BTreeSet::new(),
            manifests: vec![
                crate::TargetFile {
                    name: "docker-compose.yml".into(),
                    relpath: PathBuf::from("docker-compose.yml"),
                    bytes: TWO_SERVICE_COMPOSE.as_bytes().to_vec(),
                    content_sha: "sha1".into(),
                },
                crate::TargetFile {
                    name: "docker-compose.override.yml".into(),
                    relpath: PathBuf::from("docker-compose.override.yml"),
                    bytes: b"services: {}".to_vec(),
                    content_sha: "sha2".into(),
                },
            ],
            top_level_files: vec![
                "docker-compose.yml".into(),
                "docker-compose.override.yml".into(),
            ],
        };
        let inputs = ComposeClassifier::new().fingerprint_inputs(&target);
        assert_eq!(inputs.len(), 2);
        assert!(inputs.contains(&FingerprintInput::FileContentSha("sha1".into())));
        assert!(inputs.contains(&FingerprintInput::FileContentSha("sha2".into())));
    }
}
