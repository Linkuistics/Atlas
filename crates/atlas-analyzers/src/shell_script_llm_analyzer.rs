//! Shell-script / Makefile LLM-fallback analyser (Phase 2 PR-12).
//!
//! **Closes the "fuzzy orchestration" case.** This analyser is in-process
//! (uses Phase 1's [`LlmHook`] from `llm_classify`) and always emits
//! `Confidence::Graded` — never `Confident`.
//!
//! ## L3 behaviour
//!
//! Applies to any target whose top-level files include at least one of:
//! `*.sh`, `*.bash`, `*.zsh`, `Makefile`, `*.mk`. Sends the first 200
//! lines of the primary shell/Makefile file to the LLM with a Stage 1
//! classify prompt that asks the model to identify the script's primary
//! purpose:
//!
//! - `build-glue` — wraps a build system (make, cmake, cargo, npm…)
//! - `deploy` — provisions or releases to an environment
//! - `dev-convenience` — developer-workflow helper (lint, format, test
//!   runner that delegates to other tools)
//! - `ci-step` — a CI pipeline step (GitHub Actions step script, etc.)
//!
//! The LLM verdict is mapped onto one of two `ComponentKind`s:
//!
//! - `shell-script` — any purpose in the set `{build-glue, deploy,
//!   dev-convenience, ci-step}` for `.sh`/`.bash`/`.zsh` inputs.
//! - `makefile-orchestration` — any `Makefile`/`*.mk` input.
//!
//! Output is always `AnalyzerResult::Graded { confidence }`. The
//! confidence value equals the LLM's `evidence_grade` mapping (see
//! [`confidence_from_grade`]). A confidence below the configured
//! threshold (`0.6` default, overridable in `analyzers.yaml` under the
//! key `shell-script-llm-analyzer.threshold`) causes the engine to
//! surface `Confidence::Declines` in the final record.
//!
//! ## L5 behaviour
//!
//! Extracts function definitions from shell files and targets from
//! Makefiles, emitting one [`atlas_index::Binding`] per definition with:
//!
//! - `visibility: Visibility::Conventional` (shell has no access-control
//!   keywords)
//! - `attributes: { shell_function: true }`
//!
//! The extraction is deterministic (pure regex over file bytes); it does
//! not call the LLM.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{Binding, CostClass, LibraryApi, PubItem, PubItemKind, Stage, Visibility};
use serde::{Deserialize, Serialize};
use serde_yaml::Value as YamlValue;

use crate::{AnalysisContext, Analyzer, AnalyzerError, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id (§4 PR-12 brief).
pub const ANALYZER_ID: &str = "shell-script-llm-analyzer";

/// Version string; bump when prompt shape or extraction algorithm changes.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Default confidence threshold below which the engine marks the
/// classification as `Confidence::Declines`. Configurable via
/// `analyzers.yaml` at key `shell-script-llm-analyzer.threshold`.
pub const DEFAULT_THRESHOLD: f32 = 0.6;

/// `attributes` key for shell function / Makefile target bindings.
/// Wave-3 constant — propose `ATTR_SHELL_FUNCTION` for `atlas-contracts`
/// in the DONE report; mirrors the existing `ATTR_*` convention.
pub const ATTR_SHELL_FUNCTION: &str = "shell_function";

/// Maximum number of lines of the primary script file passed to the LLM.
const SNIPPET_LINE_LIMIT: usize = 200;

// ── L3 output type ──────────────────────────────────────────────────────────

/// L3 output produced by [`ShellScriptLlmAnalyzer`].
///
/// The engine's L3 adapter downcasts the `Box<dyn StageOutput>` back to this
/// type via [`crate::StageOutput::as_any`] and translates onto
/// `atlas_engine::types::Classification`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellScriptClassificationOutput {
    /// `"shell-script"` or `"makefile-orchestration"`.
    pub kind: String,
    /// Human-readable rationale from the LLM response (or fallback).
    pub rationale: String,
    /// Purpose tag the LLM assigned (`build-glue`, `deploy`,
    /// `dev-convenience`, `ci-step`, or `unknown`).
    pub purpose: String,
    /// Language set: always `["shell"]` or `["makefile"]`.
    pub language: String,
    /// Lifecycle roles derived from purpose mapping.
    pub lifecycle_roles: Vec<String>,
    /// Evidence field names fed back to the engine.
    pub evidence_fields: Vec<String>,
    /// Always `true`; here for symmetry with other classifiers.
    pub is_boundary: bool,
}

crate::impl_stage_output!(ShellScriptClassificationOutput);

// ── L5 output type ──────────────────────────────────────────────────────────

/// L5 output produced by [`ShellScriptLlmAnalyzer`].
///
/// Contains the extracted shell-function / Makefile-target bindings plus
/// an optional [`LibraryApi`] entry summarising the public interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellScriptSurfaceOutput {
    /// One binding per shell function / Makefile target extracted from
    /// the component's files.
    pub bindings: Vec<Binding>,
    /// Zero or one `LibraryApi` entry covering the public function set.
    pub library_apis: Vec<LibraryApi>,
}

crate::impl_stage_output!(ShellScriptSurfaceOutput);

// ── Analyser struct ──────────────────────────────────────────────────────────

/// Shell-script / Makefile LLM-fallback analyser.
///
/// Stateless; the only configuration knob (threshold) is read from the
/// `AnalysisContext` future extension slot — for Phase 2 the default
/// `DEFAULT_THRESHOLD` is always used unless the caller overrides it via
/// the `threshold` field.
#[derive(Debug)]
pub struct ShellScriptLlmAnalyzer {
    /// Confidence threshold for classification acceptance. Values below
    /// this are treated as `Confidence::Declines` by the engine. Default
    /// is [`DEFAULT_THRESHOLD`].
    pub threshold: f32,
}

impl Default for ShellScriptLlmAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellScriptLlmAnalyzer {
    /// Construct a new analyser with the default threshold (`0.6`).
    pub fn new() -> Self {
        ShellScriptLlmAnalyzer {
            threshold: DEFAULT_THRESHOLD,
        }
    }

    /// Construct with an explicit threshold. Used by tests and future
    /// `analyzers.yaml`-driven instantiation.
    pub fn with_threshold(threshold: f32) -> Self {
        ShellScriptLlmAnalyzer { threshold }
    }
}

// ── Analyzer trait impl ──────────────────────────────────────────────────────

impl Analyzer for ShellScriptLlmAnalyzer {
    fn id(&self) -> &str {
        ANALYZER_ID
    }
    fn stage(&self) -> Stage {
        // The analyser registers at L3 for classification and has a
        // separate L5 extraction path driven by `l5_surface.rs`.
        Stage::L3
    }
    fn cost_class(&self) -> CostClass {
        // LLM call at L3; threshold gates whether result is accepted.
        CostClass::LlmCheap
    }
    fn version(&self) -> &str {
        ANALYZER_VERSION
    }

    /// Applies to any target containing shell or Makefile files.
    ///
    /// Detection is intentionally broad — the engine pre-loads manifests
    /// for canonically named files (e.g. `Makefile`) and the target's
    /// `top_level_files` list carries every entry under the candidate dir.
    fn applies(&self, target: &Target) -> bool {
        // Exact manifest names first (cheap).
        if target.manifest_by_name("Makefile").is_some()
            || target.manifest_by_name("GNUmakefile").is_some()
        {
            return true;
        }
        // Extension-based scan over top-level files.
        target
            .top_level_files
            .iter()
            .any(|n| is_shell_or_make_file(n))
    }

    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput> {
        let mut out = Vec::new();
        // Contribute every shell/make file's content sha so any change
        // invalidates the L3 cache entry.
        for m in &target.manifests {
            if is_shell_or_make_file(&m.name) || m.name == "Makefile" || m.name == "GNUmakefile" {
                out.push(FingerprintInput::FileContentSha(m.content_sha.clone()));
            }
        }
        // Also contribute the directory path so the cache key is per-dir.
        out.push(FingerprintInput::Custom {
            tag: "candidate-dir",
            bytes: target.dir.to_string_lossy().as_bytes().to_vec(),
        });
        out
    }

    fn analyse(&self, ctx: &AnalysisContext, target: &Target) -> AnalyzerResult {
        let Some(hook) = ctx.llm.as_ref() else {
            // No LLM hook — decline gracefully.
            return AnalyzerResult::Declines;
        };

        // Select the primary file to pass to the LLM.
        let (primary_name, primary_bytes, is_makefile) = match pick_primary_file(target) {
            Some(r) => r,
            None => return AnalyzerResult::Declines,
        };

        // Build the snippet (first SNIPPET_LINE_LIMIT lines).
        let snippet = first_n_lines(primary_bytes, SNIPPET_LINE_LIMIT);

        // Build the LLM call payload.
        let inputs = serde_json::json!({
            "dir": target.dir.to_string_lossy(),
            "primary_file": primary_name,
            "is_makefile": is_makefile,
            "snippet": snippet,
        });

        match hook.classify(&inputs) {
            Ok(value) => {
                let purpose = (*value)
                    .get("purpose")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let rationale = (*value)
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let confidence = (*value)
                    .get("evidence_grade")
                    .and_then(|v| v.as_str())
                    .map(confidence_from_grade)
                    .unwrap_or(0.5);

                let kind = if is_makefile {
                    "makefile-orchestration".to_string()
                } else {
                    "shell-script".to_string()
                };
                let language = if is_makefile {
                    "makefile".to_string()
                } else {
                    "shell".to_string()
                };
                let lifecycle_roles = lifecycle_roles_for_purpose(&purpose);

                let output = ShellScriptClassificationOutput {
                    kind,
                    rationale,
                    purpose: purpose.clone(),
                    language,
                    lifecycle_roles,
                    evidence_fields: vec![primary_name.to_string()],
                    is_boundary: true,
                };

                if confidence < self.threshold {
                    return AnalyzerResult::Declines;
                }
                AnalyzerResult::Graded {
                    output: Box::new(output),
                    confidence,
                }
            }
            Err(e) => AnalyzerResult::Error(AnalyzerError::CallFailed {
                analyzer_id: ANALYZER_ID.to_string(),
                message: e.to_string(),
            }),
        }
    }
}

// ── L5 surface extraction (public, called by l5_surface.rs) ─────────────────

/// Extract shell function / Makefile target bindings from all shell and
/// Makefile files under `dir`. Returns a [`ShellScriptSurfaceOutput`]
/// containing one [`Binding`] per extracted definition.
///
/// The extraction is purely deterministic (regex-based). The `component_id`
/// string is used to construct the [`LibraryApi::id`].
pub fn extract_shell_surface(
    component_id: &str,
    sources: &[(PathBuf, Vec<u8>)],
) -> ShellScriptSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();

    for (rel_path, bytes) in sources {
        let is_make = is_makefile_path(rel_path);
        if is_make {
            extract_makefile_targets(rel_path, bytes, &mut bindings);
        } else {
            extract_shell_functions(rel_path, bytes, &mut bindings);
        }
    }

    let library_apis = if bindings.is_empty() {
        Vec::new()
    } else {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for b in &bindings {
            hasher.update(b.symbol.as_bytes());
            hasher.update(b.content_sha.as_bytes());
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let mut fingerprint = String::with_capacity(64);
        use std::fmt::Write;
        for byte in digest {
            write!(&mut fingerprint, "{byte:02x}").expect("write to String infallible");
        }

        let pub_items: Vec<PubItem> = bindings
            .iter()
            .map(|b| PubItem {
                name: b.symbol.clone(),
                file: b.file.clone(),
                kind: PubItemKind::Fn,
            })
            .collect();

        vec![LibraryApi {
            id: format!("{component_id}/public-api"),
            kind: atlas_index::ContractKind::LibraryApi,
            language: if sources.iter().any(|(p, _)| is_makefile_path(p)) {
                "makefile".to_string()
            } else {
                "shell".to_string()
            },
            fingerprint,
            pub_items,
        }]
    };

    ShellScriptSurfaceOutput {
        bindings,
        library_apis,
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Map an `evidence_grade` string onto a `[0.0, 1.0]` confidence value.
/// Mirrors the mapping in [`crate::llm_classify`].
fn confidence_from_grade(grade: &str) -> f32 {
    match grade {
        "strong" => 0.9,
        "medium" => 0.6,
        "weak" => 0.3,
        _ => 0.5,
    }
}

/// Map a purpose tag onto lifecycle roles.
fn lifecycle_roles_for_purpose(purpose: &str) -> Vec<String> {
    match purpose {
        "build-glue" => vec!["build".to_string()],
        "deploy" => vec!["deploy".to_string()],
        "dev-convenience" => vec!["dev-workflow".to_string()],
        "ci-step" => vec!["build".to_string(), "deploy".to_string()],
        _ => Vec::new(),
    }
}

/// True when `name` looks like a shell or Makefile file.
fn is_shell_or_make_file(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    name_lower.ends_with(".sh")
        || name_lower.ends_with(".bash")
        || name_lower.ends_with(".zsh")
        || name_lower == "makefile"
        || name_lower == "gnumakefile"
        || name_lower.ends_with(".mk")
}

/// True when a path looks like a Makefile.
fn is_makefile_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    name == "makefile" || name == "gnumakefile" || name.ends_with(".mk")
}

/// Pick the primary file for the LLM snippet. Prefers Makefiles over shell
/// scripts; within each category, prefers alphabetically first. Returns
/// `(basename, bytes, is_makefile)`.
fn pick_primary_file(target: &Target) -> Option<(&str, &[u8], bool)> {
    // Check pre-loaded manifests first.
    for name in ["Makefile", "GNUmakefile"] {
        if let Some(tf) = target.manifest_by_name(name) {
            return Some((&tf.name, &tf.bytes, true));
        }
    }
    // *.mk in manifests
    if let Some(tf) = target
        .manifests
        .iter()
        .find(|m| m.name.to_lowercase().ends_with(".mk"))
    {
        return Some((&tf.name, &tf.bytes, true));
    }
    // Shell files in manifests
    if let Some(tf) = target.manifests.iter().find(|m| {
        let n = m.name.to_lowercase();
        n.ends_with(".sh") || n.ends_with(".bash") || n.ends_with(".zsh")
    }) {
        return Some((&tf.name, &tf.bytes, false));
    }
    // Nothing pre-loaded — decline (engine hasn't given us the bytes).
    None
}

/// Return the first `n` lines of `bytes` as a `String`. Non-UTF-8 bytes
/// are replaced with the Unicode replacement character.
fn first_n_lines(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines().take(n).collect::<Vec<_>>().join("\n")
}

/// Extract shell function definitions from `bytes` and push bindings.
///
/// Recognises two patterns:
/// - `function name() {` or `function name {`
/// - `name() {`
///
/// Uses a line-scanning approach so it works without a full shell parser.
fn extract_shell_functions(rel_path: &Path, bytes: &[u8], out: &mut Vec<Binding>) {
    use regex::Regex;
    use std::sync::OnceLock;

    // Two patterns:
    // 1. `function name() {` or `function name {` (with optional space/tab)
    // 2. `name() {` (POSIX style)
    static RE_FUNCTION_KW: OnceLock<Regex> = OnceLock::new();
    static RE_POSIX: OnceLock<Regex> = OnceLock::new();

    let re_kw = RE_FUNCTION_KW
        .get_or_init(|| Regex::new(r"^\s*function\s+([A-Za-z_][A-Za-z0-9_\-.]*)").unwrap());
    let re_posix =
        RE_POSIX.get_or_init(|| Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_\-.]*)[ \t]*\(\)").unwrap());

    let text = String::from_utf8_lossy(bytes);
    let mut byte_offset: usize = 0;

    for line in text.lines() {
        let line_start = byte_offset;
        let line_end = line_start + line.len();
        // Advance past this line plus its newline (1 byte for `\n`; 2 for `\r\n`).
        byte_offset = line_end
            + if bytes.get(line_end) == Some(&b'\r') {
                2
            } else if bytes.get(line_end) == Some(&b'\n') {
                1
            } else {
                0
            };

        let symbol = if let Some(cap) = re_kw.captures(line) {
            cap.get(1).map(|m| m.as_str().to_string())
        } else if let Some(cap) = re_posix.captures(line) {
            cap.get(1).map(|m| m.as_str().to_string())
        } else {
            None
        };

        if let Some(sym) = symbol {
            let span = (line_start, line_end);
            let content_sha = crate::sha256_hex_of_range(bytes, span);
            let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
            attributes.insert(ATTR_SHELL_FUNCTION.to_string(), YamlValue::Bool(true));

            out.push(Binding {
                language: "shell".to_string(),
                symbol: sym,
                file: rel_path.to_path_buf(),
                span,
                content_sha,
                visibility: Visibility::Conventional,
                module_path: Vec::new(),
                attributes,
            });
        }
    }
}

/// Extract Makefile target definitions from `bytes` and push bindings.
///
/// A Makefile target line looks like: `<name>:` optionally followed by
/// whitespace and dependencies. Lines starting with `\t` are recipes,
/// not targets. Lines starting with `.` are special targets (`.PHONY`,
/// etc.) — we include `.PHONY` targets' listed names but skip the
/// `.PHONY:` line itself.
fn extract_makefile_targets(rel_path: &Path, bytes: &[u8], out: &mut Vec<Binding>) {
    use regex::Regex;
    use std::sync::OnceLock;

    static RE_TARGET: OnceLock<Regex> = OnceLock::new();
    static RE_PHONY: OnceLock<Regex> = OnceLock::new();

    // Target: a line starting with a non-whitespace, non-`#` name
    // followed by `:` (and not `::=` / `=` / `?=`, which are variable
    // assignments).
    let re_target = RE_TARGET
        .get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_\-./]*)[ \t]*:(?:[^:=]|$)").unwrap());
    // `.PHONY: target1 target2 ...`
    let re_phony = RE_PHONY.get_or_init(|| Regex::new(r"^\.PHONY[ \t]*:[ \t]*(.+)$").unwrap());

    let text = String::from_utf8_lossy(bytes);

    // Collect phony targets for later filtering of duplicates.
    let mut phony_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for line in text.lines() {
        if let Some(cap) = re_phony.captures(line.trim_start()) {
            if let Some(targets) = cap.get(1) {
                for t in targets.as_str().split_whitespace() {
                    phony_names.insert(t.to_string());
                }
            }
        }
    }

    // Now scan for target definitions.
    let mut byte_offset: usize = 0;
    for line in text.lines() {
        let line_start = byte_offset;
        let line_end = line_start + line.len();
        byte_offset = line_end
            + if bytes.get(line_end) == Some(&b'\r') {
                2
            } else if bytes.get(line_end) == Some(&b'\n') {
                1
            } else {
                0
            };

        // Skip comments, recipe lines, and empty lines.
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with('\t') || trimmed.is_empty() {
            continue;
        }
        // Skip special Makefile directives (`.PHONY:` etc.) — their
        // listed names are already collected above, but we don't emit
        // a binding for the `.PHONY` line itself.
        if trimmed.starts_with('.') {
            continue;
        }

        if let Some(cap) = re_target.captures(line) {
            if let Some(m) = cap.get(1) {
                let sym = m.as_str().to_string();
                let span = (line_start, line_end);
                let content_sha = crate::sha256_hex_of_range(bytes, span);
                let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
                attributes.insert(ATTR_SHELL_FUNCTION.to_string(), YamlValue::Bool(true));
                // Mark phony targets so consumers can distinguish them
                // from file-producing targets.
                if phony_names.contains(&sym) {
                    attributes.insert("phony".to_string(), YamlValue::Bool(true));
                }

                out.push(Binding {
                    language: "makefile".to_string(),
                    symbol: sym,
                    file: rel_path.to_path_buf(),
                    span,
                    content_sha,
                    visibility: Visibility::Conventional,
                    module_path: Vec::new(),
                    attributes,
                });
            }
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    // ── Stub LLM hook ────────────────────────────────────────────────────────

    struct StubHook {
        responses: Mutex<Vec<serde_json::Value>>,
        calls: Mutex<u32>,
    }

    impl StubHook {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            StubHook {
                responses: Mutex::new(responses),
                calls: Mutex::new(0),
            }
        }
    }

    impl crate::llm_classify::LlmHook for StubHook {
        fn classify(
            &self,
            _inputs: &serde_json::Value,
        ) -> Result<std::sync::Arc<serde_json::Value>, crate::llm_classify::LlmHookError> {
            *self.calls.lock().unwrap() += 1;
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                return Err(crate::llm_classify::LlmHookError::Call(
                    "no canned response".into(),
                ));
            }
            Ok(std::sync::Arc::new(q.remove(0)))
        }
    }

    // ── Target builder ───────────────────────────────────────────────────────

    fn target_with_files(files: &[(&str, &[u8])]) -> crate::Target {
        let manifests: Vec<crate::TargetFile> = files
            .iter()
            .map(|(name, bytes)| {
                let content_sha = crate::sha256_hex_of_range(bytes, (0, bytes.len()));
                crate::TargetFile {
                    name: name.to_string(),
                    relpath: PathBuf::from(name),
                    bytes: bytes.to_vec(),
                    content_sha,
                }
            })
            .collect();
        let top_level_files = files.iter().map(|(n, _)| n.to_string()).collect();
        crate::Target {
            dir: PathBuf::from("/ws/scripts"),
            languages: BTreeSet::new(),
            manifests,
            top_level_files,
        }
    }

    // ── applies tests ────────────────────────────────────────────────────────

    #[test]
    fn applies_to_makefile_target() {
        let an = ShellScriptLlmAnalyzer::new();
        let t = target_with_files(&[("Makefile", b"build:\n\tcargo build\n")]);
        assert!(an.applies(&t));
    }

    #[test]
    fn applies_to_sh_file() {
        let an = ShellScriptLlmAnalyzer::new();
        let t = target_with_files(&[("deploy.sh", b"#!/bin/bash\n")]);
        assert!(an.applies(&t));
    }

    #[test]
    fn does_not_apply_to_rust_target() {
        let an = ShellScriptLlmAnalyzer::new();
        let t = target_with_files(&[("Cargo.toml", b"[package]\nname = \"foo\"\n")]);
        assert!(!an.applies(&t));
    }

    // ── L3 analyse tests ─────────────────────────────────────────────────────

    #[test]
    fn declines_when_no_llm_hook() {
        let an = ShellScriptLlmAnalyzer::new();
        let ctx = crate::AnalysisContext::deterministic_only();
        let t = target_with_files(&[("Makefile", b"build:\n\tcargo build\n")]);
        assert!(matches!(an.analyse(&ctx, &t), AnalyzerResult::Declines));
    }

    #[test]
    fn graded_result_for_deploy_sh() {
        let hook = std::sync::Arc::new(StubHook::new(vec![serde_json::json!({
            "purpose": "deploy",
            "rationale": "Deploys the service to the staging environment.",
            "evidence_grade": "strong",
        })]));
        let ctx = crate::AnalysisContext::with_llm(hook.clone());
        let an = ShellScriptLlmAnalyzer::new();
        let content = b"#!/bin/bash\nfunction deploy() {\n  echo deploying\n}\ndeploy\n" as &[u8];
        let t = target_with_files(&[("deploy.sh", content)]);
        let r = an.analyse(&ctx, &t);
        match r {
            AnalyzerResult::Graded { confidence, output } => {
                assert!((confidence - 0.9).abs() < 1e-4, "confidence={confidence}");
                let out = output
                    .as_any()
                    .downcast_ref::<ShellScriptClassificationOutput>()
                    .expect("must be ShellScriptClassificationOutput");
                assert_eq!(out.kind, "shell-script");
                assert_eq!(out.purpose, "deploy");
                assert!(out.lifecycle_roles.contains(&"deploy".to_string()));
            }
            other => panic!("expected Graded, got {other:?}"),
        }
        assert_eq!(*hook.calls.lock().unwrap(), 1);
    }

    #[test]
    fn makefile_produces_makefile_orchestration_kind() {
        let hook = std::sync::Arc::new(StubHook::new(vec![serde_json::json!({
            "purpose": "build-glue",
            "rationale": "Orchestrates the build pipeline.",
            "evidence_grade": "medium",
        })]));
        let ctx = crate::AnalysisContext::with_llm(hook);
        let an = ShellScriptLlmAnalyzer::new();
        let t = target_with_files(&[(
            "Makefile",
            b".PHONY: build clean\nbuild:\n\tcargo build\nclean:\n\tcargo clean\n",
        )]);
        let r = an.analyse(&ctx, &t);
        match r {
            AnalyzerResult::Graded { output, .. } => {
                let out = output
                    .as_any()
                    .downcast_ref::<ShellScriptClassificationOutput>()
                    .unwrap();
                assert_eq!(out.kind, "makefile-orchestration");
            }
            other => panic!("expected Graded, got {other:?}"),
        }
    }

    #[test]
    fn errors_on_llm_hook_failure() {
        let hook = std::sync::Arc::new(StubHook::new(Vec::new())); // no responses
        let ctx = crate::AnalysisContext::with_llm(hook);
        let an = ShellScriptLlmAnalyzer::new();
        let t = target_with_files(&[("deploy.sh", b"#!/bin/bash\n")]);
        assert!(matches!(an.analyse(&ctx, &t), AnalyzerResult::Error(_)));
    }

    #[test]
    fn declines_when_no_shell_or_make_files_in_manifests() {
        let hook = std::sync::Arc::new(StubHook::new(vec![serde_json::json!({
            "purpose": "unknown",
            "rationale": "",
            "evidence_grade": "weak",
        })]));
        let ctx = crate::AnalysisContext::with_llm(hook);
        let an = ShellScriptLlmAnalyzer::new();
        // manifest has bytes but no shell/make file — pick_primary_file returns None
        let t = target_with_files(&[("README.md", b"# hello\n")]);
        // applies() returns false, so in real dispatch it's skipped,
        // but we also verify the direct call declines.
        let result = an.analyse(&ctx, &t);
        // Because pick_primary_file finds nothing, analyse returns Declines.
        assert!(matches!(result, AnalyzerResult::Declines));
    }

    // ── Shell function extraction tests ──────────────────────────────────────

    #[test]
    fn extracts_function_keyword_syntax() {
        let src = b"function deploy() {\n  echo hi\n}\n";
        let sources = vec![(PathBuf::from("deploy.sh"), src.to_vec())];
        let out = extract_shell_surface("my-comp", &sources);
        assert_eq!(out.bindings.len(), 1);
        let b = &out.bindings[0];
        assert_eq!(b.symbol, "deploy");
        assert_eq!(b.language, "shell");
        assert_eq!(b.visibility, Visibility::Conventional);
        assert_eq!(
            b.attributes.get(ATTR_SHELL_FUNCTION),
            Some(&YamlValue::Bool(true))
        );
    }

    #[test]
    fn extracts_posix_function_syntax() {
        let src = b"deploy() {\n  echo hi\n}\n";
        let sources = vec![(PathBuf::from("deploy.sh"), src.to_vec())];
        let out = extract_shell_surface("my-comp", &sources);
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "deploy");
    }

    #[test]
    fn extracts_multiple_shell_functions() {
        let src = b"function foo() {\n  echo foo\n}\nbar() {\n  echo bar\n}\n";
        let sources = vec![(PathBuf::from("script.sh"), src.to_vec())];
        let out = extract_shell_surface("comp", &sources);
        let symbols: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(symbols.contains(&"foo"), "symbols: {symbols:?}");
        assert!(symbols.contains(&"bar"), "symbols: {symbols:?}");
    }

    // ── Makefile target extraction tests ─────────────────────────────────────

    #[test]
    fn extracts_makefile_build_and_clean_targets() {
        let src = b".PHONY: build clean\nbuild:\n\tcargo build\nclean:\n\tcargo clean\n";
        let sources = vec![(PathBuf::from("Makefile"), src.to_vec())];
        let out = extract_shell_surface("comp", &sources);
        let symbols: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(symbols.contains(&"build"), "symbols: {symbols:?}");
        assert!(symbols.contains(&"clean"), "symbols: {symbols:?}");
        // Both should be marked phony.
        for b in &out.bindings {
            assert_eq!(
                b.attributes.get("phony"),
                Some(&YamlValue::Bool(true)),
                "target `{}` should be marked phony",
                b.symbol
            );
        }
    }

    #[test]
    fn extracts_makefile_targets_with_language_makefile() {
        let src = b"install:\n\tcp foo /usr/local/bin/\n";
        let sources = vec![(PathBuf::from("Makefile"), src.to_vec())];
        let out = extract_shell_surface("comp", &sources);
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].language, "makefile");
    }

    #[test]
    fn library_api_produced_when_bindings_present() {
        let src = b"function foo() {\n  echo foo\n}\n";
        let sources = vec![(PathBuf::from("script.sh"), src.to_vec())];
        let out = extract_shell_surface("test-comp", &sources);
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].id, "test-comp/public-api");
        assert_eq!(out.library_apis[0].pub_items.len(), 1);
        assert_eq!(out.library_apis[0].pub_items[0].name, "foo");
    }

    #[test]
    fn no_library_api_when_no_bindings() {
        let src = b"#!/bin/bash\necho hello\n";
        let sources = vec![(PathBuf::from("simple.sh"), src.to_vec())];
        let out = extract_shell_surface("comp", &sources);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn fingerprint_inputs_are_deterministic() {
        let t = target_with_files(&[("deploy.sh", b"#!/bin/bash\n")]);
        let an = ShellScriptLlmAnalyzer::new();
        let a = an.fingerprint_inputs(&t);
        let b = an.fingerprint_inputs(&t);
        assert_eq!(a, b);
    }

    #[test]
    fn first_n_lines_truncates_correctly() {
        let bytes = b"line1\nline2\nline3\nline4\n";
        let result = first_n_lines(bytes, 2);
        assert_eq!(result, "line1\nline2");
    }

    #[test]
    fn confidence_from_grade_maps_correctly() {
        assert!((confidence_from_grade("strong") - 0.9).abs() < 1e-4);
        assert!((confidence_from_grade("medium") - 0.6).abs() < 1e-4);
        assert!((confidence_from_grade("weak") - 0.3).abs() < 1e-4);
        assert!((confidence_from_grade("unknown") - 0.5).abs() < 1e-4);
    }

    /// Threshold gate: a "weak" evidence_grade (confidence 0.3) must produce
    /// `AnalyzerResult::Declines` when using the default threshold of 0.6.
    #[test]
    fn weak_grade_below_threshold_declines() {
        let hook = std::sync::Arc::new(StubHook::new(vec![serde_json::json!({
            "purpose": "deploy",
            "rationale": "Unclear deployment intent.",
            "evidence_grade": "weak",
        })]));
        let ctx = crate::AnalysisContext::with_llm(hook.clone());
        // Default threshold is 0.6; weak → 0.3 < 0.6 → must Decline.
        let an = ShellScriptLlmAnalyzer::new();
        let content = b"#!/bin/bash\ndeploy_service\n" as &[u8];
        let t = target_with_files(&[("deploy.sh", content)]);
        let r = an.analyse(&ctx, &t);
        assert!(
            matches!(r, AnalyzerResult::Declines),
            "expected Declines for weak evidence_grade below threshold, got {r:?}"
        );
        assert_eq!(
            *hook.calls.lock().unwrap(),
            1,
            "LLM hook should have been called once"
        );
    }

    /// Regression guard: `Default::default()` must produce the same threshold
    /// as `::new()`, not `0.0`. If `#[derive(Default)]` is ever re-introduced
    /// this test will catch the regression before it silently disables the gate.
    #[test]
    fn default_impl_uses_default_threshold() {
        let an = ShellScriptLlmAnalyzer::default();
        assert_eq!(
            an.threshold, DEFAULT_THRESHOLD,
            "Default::default() must set threshold to DEFAULT_THRESHOLD ({}), not 0.0",
            DEFAULT_THRESHOLD
        );
    }
}
