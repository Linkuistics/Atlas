//! Dockerfile classifier.
//!
//! Parses a Dockerfile (continuation-line aware) into a small
//! [`DockerfileShape`] and emits `kind: docker-image` confidently when
//! the manifest carries at least one `FROM` instruction.
//!
//! Phase 1 wires the analyser at L3 only — the L1 hook (file-enumeration
//! seeding for deliverable candidates) lands in PR-9. The shape struct
//! captures every piece PR-9 will consume (`COPY` source paths, `LABEL`
//! map, `ENV` map, `EXPOSE` ports, `CMD`/`ENTRYPOINT` argv) so PR-9
//! can extend without re-parsing.
//!
//! Parsing is line-based but honours backslash continuation: a line
//! ending in `\` is concatenated with the next line. Comments
//! (lines starting with `#` after whitespace stripping) and blank
//! lines are skipped. Heredocs (`<<EOF` … `EOF`) are not supported in
//! Phase 1 — Phase 2 may need a richer parser.

use atlas_index::{CostClass, Stage};
use serde::{Deserialize, Serialize};

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Analyser id. Stable across versions.
pub const ANALYZER_ID: &str = "dockerfile-l3";

/// Analyser version string. Bump on parser-shape changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// One Dockerfile classification verdict. The L3 adapter downcasts a
/// `Box<dyn StageOutput>` to this struct and translates onto
/// `Classification`. Mirrors [`crate::cargo_classifier::CargoClassificationOutput`]'s
/// shape; the `parsed` field carries the structural data PR-9 will
/// consume for composition edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerfileClassificationOutput {
    pub kind: String,
    pub lifecycle_roles: Vec<String>,
    pub build_system: Option<String>,
    pub role: Option<String>,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    pub is_boundary: bool,
    /// Extracted Dockerfile structure. PR-9 consumes the `copy_sources`
    /// list to emit `bundled-into` edges; PR-5 emits the field for
    /// future use only.
    pub parsed: DockerfileShape,
}

/// Structural Dockerfile data extracted by [`parse_dockerfile`].
///
/// All vectors preserve source order (so a stable golden test can pin
/// the parser shape). Phase 1 callers should treat `parsed` as
/// opaque; PR-9 consumes specific fields.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerfileShape {
    /// One entry per `FROM` instruction. Multi-stage builds produce
    /// multiple entries.
    pub from_images: Vec<FromImage>,
    /// One entry per `COPY` instruction. The `from_stage` slot
    /// records `--from=...` references so PR-9 can emit
    /// `bundled-into` edges that span build stages.
    pub copy_directives: Vec<CopyDirective>,
    /// `LABEL key=value` pairs. Multiple `LABEL` lines accumulate.
    pub labels: Vec<(String, String)>,
    /// `ENV key=value` pairs.
    pub envs: Vec<(String, String)>,
    /// Argv from `EXPOSE`. Each entry is one port spec (`80`,
    /// `443/tcp`, etc.).
    pub exposed_ports: Vec<String>,
    /// Argv from `CMD`. Phase 1 records the literal slice; PR-9 may
    /// resolve it to a deliverable.
    pub cmd: Option<Vec<String>>,
    /// Argv from `ENTRYPOINT`.
    pub entrypoint: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromImage {
    /// Image reference as written (`alpine:3.20`,
    /// `ghcr.io/foo/bar@sha256:...`).
    pub image: String,
    /// Optional `AS <name>` alias.
    pub stage_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyDirective {
    /// `--from=...` reference (a stage alias or numeric index), if
    /// the directive carries one.
    pub from_stage: Option<String>,
    /// One or more source paths.
    pub sources: Vec<String>,
    /// Destination path (`COPY`'s last positional arg).
    pub destination: String,
}

/// The Dockerfile classifier itself. Stateless.
#[derive(Debug, Default)]
pub struct DockerfileClassifier;

impl DockerfileClassifier {
    pub fn new() -> Self {
        DockerfileClassifier
    }
}

impl Analyzer for DockerfileClassifier {
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
            .any(|m| is_dockerfile_basename(&m.name))
            || target
                .top_level_files
                .iter()
                .any(|n| is_dockerfile_basename(n))
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        target
            .manifests
            .iter()
            .filter(|m| is_dockerfile_basename(&m.name))
            .map(|m| FingerprintInput::FileContentSha(m.content_sha.clone()))
            .collect()
    }

    fn analyse(&self, _ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        // Pick the lexicographically-first Dockerfile-named manifest
        // (e.g. `Dockerfile` before `Dockerfile.backend.buildkite`).
        // Phase 2 PR-14 extends the Phase 1 PR-9 classifier to
        // recognise `Dockerfile.<suffix>` files in addition to the
        // canonical `Dockerfile` basename — the brief's polyglot
        // fixture verifies dull/'s `*.buildkite` CI naming flows
        // through here unchanged.
        let mut dockerfile_manifests: Vec<&crate::TargetFile> = target
            .manifests
            .iter()
            .filter(|m| is_dockerfile_basename(&m.name))
            .collect();
        dockerfile_manifests.sort_by(|a, b| a.name.cmp(&b.name));
        let Some(manifest) = dockerfile_manifests.first().copied() else {
            // The engine pre-loads `Dockerfile`-shaped manifests into
            // `Target.manifests` when it discovers them; absent any,
            // the analyser cannot run. Decline so the dispatcher
            // consults the next.
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

        let parsed = parse_dockerfile(text);
        if parsed.from_images.is_empty() {
            // A Dockerfile without any `FROM` instruction is not a
            // valid build context. Decline rather than confidently
            // misclassifying.
            return AnalyzerResult::Declines;
        }

        AnalyzerResult::Confident(Box::new(DockerfileClassificationOutput {
            kind: "docker-image".into(),
            lifecycle_roles: vec!["deploy".into()],
            build_system: Some("docker".into()),
            role: None,
            evidence_fields: vec![format!("{}:FROM", manifest.name)],
            rationale: format!(
                "{} declares {} FROM instruction(s); base image: {}",
                manifest.name,
                parsed.from_images.len(),
                parsed.from_images[0].image
            ),
            is_boundary: true,
            parsed,
        }))
    }
}

/// Return `true` when `basename` is the canonical `Dockerfile` or any
/// `Dockerfile.<suffix>` form (e.g. dull/'s CI naming
/// `Dockerfile.frontend.buildkite`). The trailing-dot edge case
/// (`Dockerfile.`) is rejected — a non-empty suffix is required.
///
/// Mirrors the engine-side `manifest_patterns::is_dockerfile_basename`
/// so the L1 manifest walk and the analyser's `applies` predicate
/// stay in sync. The two functions exist as siblings because the
/// analyser crate does not depend on `atlas-engine`.
fn is_dockerfile_basename(basename: &str) -> bool {
    if basename == "Dockerfile" {
        return true;
    }
    if let Some(rest) = basename.strip_prefix("Dockerfile.") {
        if !rest.is_empty() {
            return true;
        }
    }
    false
}

/// Parse a Dockerfile body into a [`DockerfileShape`]. Pure function;
/// public so PR-9's L1 seeding hook can reuse it without going
/// through the analyser plumbing.
///
/// Limitations: heredocs (`<<EOF` … `EOF`) are not supported; the
/// parser treats them as raw lines. Phase 2 may swap in a richer
/// parser. The exec-vs-shell form is detected by the leading `[` in
/// the argv slice — `["foo", "bar"]` is exec form, otherwise shell.
pub fn parse_dockerfile(text: &str) -> DockerfileShape {
    let logical = collapse_continuations(text);
    let mut shape = DockerfileShape::default();

    for raw_line in logical.lines() {
        let line = strip_comment(raw_line.trim());
        if line.is_empty() {
            continue;
        }
        let (instruction, rest) = match line.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim()),
            None => (line, ""),
        };
        match instruction.to_ascii_uppercase().as_str() {
            "FROM" => {
                if let Some(image) = parse_from(rest) {
                    shape.from_images.push(image);
                }
            }
            "COPY" => {
                if let Some(directive) = parse_copy(rest) {
                    shape.copy_directives.push(directive);
                }
            }
            "LABEL" => {
                shape.labels.extend(parse_kv_pairs(rest));
            }
            "ENV" => {
                shape.envs.extend(parse_kv_pairs(rest));
            }
            "EXPOSE" => {
                for port in rest.split_whitespace() {
                    shape.exposed_ports.push(port.to_string());
                }
            }
            "CMD" => {
                shape.cmd = Some(parse_argv(rest));
            }
            "ENTRYPOINT" => {
                shape.entrypoint = Some(parse_argv(rest));
            }
            _ => {
                // Phase 1 ignores other instructions (RUN, ARG,
                // WORKDIR, USER, …). They do not affect the L3
                // verdict; PR-9 may consume some.
            }
        }
    }

    shape
}

/// Collapse `\`-continuation lines into single logical lines. A
/// trailing `\` (with optional whitespace before the newline) joins
/// the next line; otherwise the newline is preserved.
fn collapse_continuations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut iter = text.lines().peekable();
    while let Some(mut line) = iter.next() {
        // Collect continuation tail.
        let mut buffer = String::new();
        loop {
            // A continuation marker is a `\` at end-of-line, possibly
            // preceded by trailing spaces. Tolerate one or the other.
            let trimmed_end = line.trim_end();
            if let Some(stripped) = trimmed_end.strip_suffix('\\') {
                buffer.push_str(stripped);
                buffer.push(' ');
                if let Some(next) = iter.next() {
                    line = next;
                    continue;
                } else {
                    break;
                }
            } else {
                buffer.push_str(line);
                break;
            }
        }
        out.push_str(&buffer);
        out.push('\n');
    }
    out
}

/// Strip a trailing `# ...` comment from a line. Naive: a `#` inside
/// a quoted string is treated as a comment too. Phase 1 accepts the
/// simplification — Dockerfiles rarely embed `#` inside strings.
fn strip_comment(line: &str) -> &str {
    if line.starts_with('#') {
        return "";
    }
    match line.split_once('#') {
        Some((before, _after)) if before.trim_end().ends_with(' ') => before.trim_end(),
        _ => line,
    }
}

/// Parse `FROM <image>[:<tag>] [AS <stage>]`. Returns `None` for an
/// empty rest (malformed `FROM`).
fn parse_from(rest: &str) -> Option<FromImage> {
    let mut tokens = rest.split_whitespace();
    let image = tokens.next()?.to_string();
    let mut stage_alias = None;
    if let Some(token) = tokens.next() {
        if token.eq_ignore_ascii_case("AS") {
            stage_alias = tokens.next().map(|s| s.to_string());
        }
    }
    Some(FromImage { image, stage_alias })
}

/// Parse `COPY [--from=<stage>] <src1> [<src2> …] <dest>`.
fn parse_copy(rest: &str) -> Option<CopyDirective> {
    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut from_stage = None;
    // Strip leading `--flag=...` directives. `--from=` is the one
    // with composition-edge significance; other flags (e.g.
    // `--chown=`, `--chmod=`) are recorded but ignored for Phase 1.
    while let Some(first) = tokens.first().copied() {
        if let Some(rest) = first.strip_prefix("--from=") {
            from_stage = Some(rest.to_string());
            tokens.remove(0);
        } else if first.starts_with("--") {
            tokens.remove(0);
        } else {
            break;
        }
    }
    if tokens.len() < 2 {
        return None;
    }
    let destination = tokens.pop().unwrap().to_string();
    let sources: Vec<String> = tokens.into_iter().map(String::from).collect();
    Some(CopyDirective {
        from_stage,
        sources,
        destination,
    })
}

/// Parse `KEY=VALUE [KEY=VALUE …]` pairs. Tolerant of whitespace
/// around `=` and of repeated keys (the resulting Vec preserves
/// source order; PR-9 may dedupe).
fn parse_kv_pairs(rest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for token in rest.split_whitespace() {
        if let Some((k, v)) = token.split_once('=') {
            // Strip surrounding double-quotes if present.
            let v = v.trim_matches('"');
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

/// Parse a Dockerfile argv list. Accepts both exec form
/// (`["a", "b"]`) and shell form (`a b c`). Exec form parses through
/// `serde_json` so quoting is honoured.
fn parse_argv(rest: &str) -> Vec<String> {
    let trimmed = rest.trim();
    if trimmed.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(trimmed) {
            return parsed;
        }
    }
    trimmed.split_whitespace().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn target_with_dockerfile(text: &str) -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "Dockerfile".into(),
                relpath: PathBuf::from("Dockerfile"),
                bytes: text.as_bytes().to_vec(),
                content_sha: format!("sha-{}", text.len()),
            }],
            top_level_files: vec!["Dockerfile".into()],
        }
    }

    #[test]
    fn parses_minimal_from_only_dockerfile() {
        let shape = parse_dockerfile("FROM alpine:3.20\n");
        assert_eq!(shape.from_images.len(), 1);
        assert_eq!(shape.from_images[0].image, "alpine:3.20");
        assert!(shape.from_images[0].stage_alias.is_none());
    }

    #[test]
    fn parses_multi_stage_with_alias_and_copy_from() {
        let text = r#"
FROM rust:1.79 AS builder
COPY src/ /workspace/src
COPY --from=builder /workspace/target/release/atlas /usr/local/bin/atlas
FROM debian:bookworm-slim
COPY --from=builder /usr/local/bin/atlas /usr/local/bin/atlas
EXPOSE 8080
LABEL org.opencontainers.image.title="atlas"
ENV ATLAS_ROOT=/workspace
CMD ["/usr/local/bin/atlas", "index"]
ENTRYPOINT ["/sbin/tini", "--"]
"#;
        let shape = parse_dockerfile(text);
        assert_eq!(shape.from_images.len(), 2);
        assert_eq!(shape.from_images[0].image, "rust:1.79");
        assert_eq!(shape.from_images[0].stage_alias.as_deref(), Some("builder"));
        assert_eq!(shape.from_images[1].image, "debian:bookworm-slim");
        assert_eq!(shape.copy_directives.len(), 3);
        assert_eq!(
            shape.copy_directives[1].from_stage.as_deref(),
            Some("builder")
        );
        assert_eq!(shape.exposed_ports, vec!["8080".to_string()]);
        assert_eq!(shape.labels.len(), 1);
        assert_eq!(shape.envs, vec![("ATLAS_ROOT".into(), "/workspace".into())]);
        assert_eq!(
            shape.cmd.as_ref().unwrap(),
            &vec!["/usr/local/bin/atlas".to_string(), "index".to_string()]
        );
        assert_eq!(
            shape.entrypoint.as_ref().unwrap(),
            &vec!["/sbin/tini".to_string(), "--".to_string()]
        );
    }

    #[test]
    fn collapses_backslash_continuations() {
        let text = "RUN apt-get update \\\n  && apt-get install -y curl\nFROM alpine\n";
        let shape = parse_dockerfile(text);
        assert_eq!(shape.from_images.len(), 1);
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let text = "# header\n\n\nFROM alpine\n# trailing\n";
        let shape = parse_dockerfile(text);
        assert_eq!(shape.from_images.len(), 1);
    }

    #[test]
    fn classifier_emits_docker_image_for_dockerfile_with_from() {
        let target = target_with_dockerfile("FROM alpine:3.20\nLABEL x=1\n");
        let an = DockerfileClassifier::new();
        assert!(an.applies(&target));
        let ctx = AnalysisContext::deterministic_only();
        match an.analyse(&ctx, &target) {
            AnalyzerResult::Confident(out) => {
                let docker = out
                    .as_any()
                    .downcast_ref::<DockerfileClassificationOutput>()
                    .expect("output is DockerfileClassificationOutput");
                assert_eq!(docker.kind, "docker-image");
                assert_eq!(docker.parsed.from_images[0].image, "alpine:3.20");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn classifier_declines_dockerfile_without_from() {
        let target = target_with_dockerfile("# just a comment\n");
        let an = DockerfileClassifier::new();
        let ctx = AnalysisContext::deterministic_only();
        let r = an.analyse(&ctx, &target);
        assert!(matches!(r, AnalyzerResult::Declines), "got {r:?}");
    }

    #[test]
    fn applies_returns_false_without_dockerfile() {
        let target = Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(!DockerfileClassifier::new().applies(&target));
    }
}
