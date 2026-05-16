//! Analyser registry — ordered list of [`Analyzer`] trait objects
//! consulted by [`crate::dispatcher`].
//!
//! Two construction paths:
//!
//! - [`AnalyzerRegistry::builtin`] — three reference analysers
//!   (Cargo, Dockerfile, LLM-classify) shipped in this crate. The
//!   default registry every workspace gets.
//! - [`AnalyzerRegistry::merge_yaml`] — declarative override / extend
//!   from `<output>/.atlas/analyzers.yaml`. PR-5 ships the merge but
//!   does not yet emit per-analyser instances for every spec the
//!   YAML can name (subprocess transport is Phase 2). Specs whose
//!   `id` matches a built-in analyser update its applicability and
//!   version metadata in place; specs naming an unknown id are
//!   recorded for fingerprint purposes but do not produce a runnable
//!   analyser instance.
//!
//! Dispatch order (design §7.3): the registry is sorted by
//! `(stage, cost_class)` ascending. Tie-breaks fall to insertion
//! order. The dispatcher iterates the sorted view; the source list
//! is preserved so callers (and the fingerprint computation) see a
//! stable shape regardless of dispatch order.

use std::collections::BTreeMap;
use std::sync::Arc;

use atlas_index::{AnalyzerSpec, AnalyzersFile, Stage, ANALYZERS_SCHEMA_VERSION};
use sha2::{Digest, Sha256};

use crate::compose_classifier::ComposeClassifier;
use crate::csharp_classifier::CsharpClassifier;
use crate::dart_classifier::DartClassifier;
use crate::dockerfile_classifier::DockerfileClassifier;
use crate::elixir_classifier::ElixirClassifier;
use crate::lispkit_classifier::LispKitClassifier;
use crate::llm_classify::LlmClassifyAnalyzer;
use crate::python_classifier::PythonClassifier;
use crate::racket_classifier::RacketClassifier;
use crate::rust_surface_analyzer::RustSurfaceAnalyzer;
use crate::shell_script_llm_analyzer::ShellScriptLlmAnalyzer;
use crate::subprocess::{SubprocessAnalyzerProxy, SubprocessAnalyzerSpec};
use crate::ts_js_classifier::TsJsClassifier;
use crate::ts_js_surface_analyzer::TsJsSurfaceAnalyzer;
use crate::{Analyzer, AnalyzerError};

/// Namespace string mixed into the `analyzer_registry_sha` so a future
/// hash redefinition can be distinguished from the v1 form. The hash
/// computation is documented on
/// [`atlas_index::CacheFingerprints::analyzer_registry_sha`].
pub const REGISTRY_HASH_NAMESPACE: &str = "atlas-analyzers/v1";

/// Ordered analyser list. Public methods are immutable except for
/// [`AnalyzerRegistry::merge_yaml`], which is the one mutation path.
#[derive(Clone)]
pub struct AnalyzerRegistry {
    /// Analyser instances. Order is registration order; dispatch
    /// iterates a sorted view.
    analyzers: Vec<Arc<dyn Analyzer>>,
    /// Declarative shape for the merged registry, used to compute
    /// the canonical [`AnalyzersFile`] and its sha256.
    declared: AnalyzersFile,
    /// Per-analyser binary content shas for subprocess analysers
    /// registered via [`AnalyzerRegistry::register_subprocess`].
    /// Keyed on analyser id. The engine reads this map when
    /// computing L-stage cache fingerprints — every subprocess
    /// analyser the dispatcher consulted contributes its binary
    /// sha via `FingerprintBuilder::add_analyzer_binary_sha`
    /// (PR-2 tag `0x06`).
    binary_shas: BTreeMap<String, String>,
}

impl std::fmt::Debug for AnalyzerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyzerRegistry")
            .field("analyzer_count", &self.analyzers.len())
            .field("declared_count", &self.declared.analyzers.len())
            .finish()
    }
}

impl AnalyzerRegistry {
    /// Reference analysers shipped in this crate. Every workspace
    /// starts here; `analyzers.yaml` (when present) merges on top.
    ///
    /// Phase 1 shipped four analysers: `cargo-toml-classifier`,
    /// `dockerfile-l3` (both L3 deterministic), `llm-classify-fallback`
    /// (L3 LLM), and `rust-surface-analyzer` (L5 deterministic; PR-7).
    /// Phase 8 WI-3 retired the deterministic `cargo-toml-classifier`;
    /// Rust components now fall through to `llm-classify-fallback`.
    /// Phase 2 PR-1 adds `ts-js-classifier` (L3 deterministic) and
    /// `ts-js-surface-analyzer` (L5 deterministic) for TypeScript /
    /// JavaScript components. Phase 2 PR-3 adds `python-classifier`
    /// (L3 deterministic, in-process) for Python components; the
    /// matching surface analyser at L5 is the out-of-process
    /// `python-surface-analyzer` (registered separately when its
    /// binary is locatable — see [`AnalyzerRegistry::builtin_with_python_surface`]).
    /// Phase 2 PR-11 adds `compose-classifier` (L3 deterministic,
    /// in-process) for Docker Compose files.
    /// Phase 2 PR-6 adds `csharp-classifier` (L3 deterministic, in-process)
    /// for C# components; the matching surface analyser at L5 is the
    /// out-of-process `csharp-surface-analyzer` (registered separately
    /// when its binary is locatable). Phase 2 PR-7 adds `dart-classifier`
    /// (L3 deterministic, in-process) for Dart / Flutter components;
    /// the matching surface analyser at L5 is the out-of-process
    /// `dart-surface-analyzer` (registered separately when its binary
    /// is locatable). Phase 2 PR-8 adds `elixir-classifier` (L3
    /// deterministic, in-process) for Elixir components; the matching
    /// surface analyser at L5 is the out-of-process
    /// `elixir-surface-analyzer`. Phase 2 PR-9 adds `racket-classifier`
    /// (L3 deterministic, in-process) for Racket components; the
    /// matching surface analyser at L5 is the out-of-process
    /// `racket-surface-analyzer`. Phase 2 PR-10 adds `lispkit-classifier`
    /// (L3 deterministic, in-process) for LispKit `*.sld` components;
    /// the matching surface analyser at L5 is the out-of-process
    /// `lispkit-surface-analyzer`.
    pub fn builtin() -> Self {
        let docker = Arc::new(DockerfileClassifier::new()) as Arc<dyn Analyzer>;
        let compose = Arc::new(ComposeClassifier::new()) as Arc<dyn Analyzer>;
        let ts_js = Arc::new(TsJsClassifier::new()) as Arc<dyn Analyzer>;
        let python = Arc::new(PythonClassifier::new()) as Arc<dyn Analyzer>;
        let csharp = Arc::new(CsharpClassifier::new()) as Arc<dyn Analyzer>;
        let dart = Arc::new(DartClassifier::new()) as Arc<dyn Analyzer>;
        let elixir = Arc::new(ElixirClassifier::new()) as Arc<dyn Analyzer>;
        let racket = Arc::new(RacketClassifier::new()) as Arc<dyn Analyzer>;
        let lispkit = Arc::new(LispKitClassifier::new()) as Arc<dyn Analyzer>;
        let llm = Arc::new(LlmClassifyAnalyzer::new()) as Arc<dyn Analyzer>;
        let shell = Arc::new(ShellScriptLlmAnalyzer::new()) as Arc<dyn Analyzer>;
        let rust_surface = Arc::new(RustSurfaceAnalyzer::new()) as Arc<dyn Analyzer>;
        let ts_js_surface = Arc::new(TsJsSurfaceAnalyzer::new()) as Arc<dyn Analyzer>;

        let analyzers = vec![
            docker.clone(),
            compose.clone(),
            ts_js.clone(),
            python.clone(),
            csharp.clone(),
            dart.clone(),
            elixir.clone(),
            racket.clone(),
            lispkit.clone(),
            llm.clone(),
            shell.clone(),
            rust_surface.clone(),
            ts_js_surface.clone(),
        ];
        let declared = AnalyzersFile {
            schema_version: ANALYZERS_SCHEMA_VERSION,
            analyzers: analyzers.iter().map(spec_for_analyzer).collect(),
            config: Default::default(),
        };

        AnalyzerRegistry {
            analyzers,
            declared,
            binary_shas: BTreeMap::new(),
        }
    }

    /// Empty registry. Tests use this to construct a controlled
    /// dispatch order; production code always starts from
    /// [`AnalyzerRegistry::builtin`].
    pub fn empty() -> Self {
        AnalyzerRegistry {
            analyzers: Vec::new(),
            declared: AnalyzersFile::default(),
            binary_shas: BTreeMap::new(),
        }
    }

    /// Register an analyser. Used by [`AnalyzerRegistry::empty`]'s
    /// test path and by future Phase 2 wiring (subprocess analysers).
    /// The declared spec is built from the analyser's metadata.
    pub fn register(&mut self, analyzer: Arc<dyn Analyzer>) {
        self.declared.analyzers.push(spec_for_analyzer(&analyzer));
        self.analyzers.push(analyzer);
    }

    /// Register a subprocess analyser. Constructs a
    /// [`SubprocessAnalyzerProxy`] (which hashes the binary at
    /// `spec.binary_path`), stores it as the registered analyser
    /// instance, and records the binary sha so engine-side
    /// fingerprinting can look it up by id.
    ///
    /// Returns the proxy's binary sha on success — the engine may
    /// already know it (PR-2 wires the cache integration) but
    /// surfacing it here keeps the registry the single source of
    /// truth.
    pub fn register_subprocess(
        &mut self,
        spec: SubprocessAnalyzerSpec,
    ) -> Result<String, AnalyzerError> {
        let proxy = SubprocessAnalyzerProxy::new(spec.clone())?;
        let id = proxy.id().to_string();
        let binary_sha = proxy.binary_sha().to_string();
        // Build a declared `AnalyzerSpec` mirroring the in-process
        // form. The subprocess transport carries the binary_sha
        // load-bearingly via the SubprocessConfig.
        let declared = AnalyzerSpec {
            id: spec.id.clone(),
            stage: spec.stage,
            applicability: spec.applicability.clone(),
            cost_class: spec.cost_class,
            confidence: Some(atlas_index::Confidence::Binary),
            transport: atlas_index::Transport::Subprocess,
            subprocess: Some(atlas_index::SubprocessConfig {
                command: spec.command.clone(),
                timeout_seconds: spec
                    .timeout
                    .map(|d| d.as_secs().min(u32::MAX as u64) as u32),
                binary_sha: Some(binary_sha.clone()),
            }),
            version: spec.version.clone(),
        };
        self.declared.analyzers.push(declared);
        self.binary_shas.insert(id, binary_sha.clone());
        self.analyzers.push(Arc::new(proxy) as Arc<dyn Analyzer>);
        Ok(binary_sha)
    }

    /// Look up a registered subprocess analyser's binary content
    /// sha by id. Returns `None` for in-process analysers (they
    /// have no binary on disk distinct from the engine itself).
    pub fn binary_sha(&self, analyzer_id: &str) -> Option<&str> {
        self.binary_shas.get(analyzer_id).map(|s| s.as_str())
    }

    /// Number of registered analyser instances (built-ins plus any
    /// added via [`AnalyzerRegistry::register`]).
    pub fn len(&self) -> usize {
        self.analyzers.len()
    }

    /// True when no analysers are registered (test convenience; the
    /// production builtin registry is non-empty).
    pub fn is_empty(&self) -> bool {
        self.analyzers.is_empty()
    }

    /// Iterate analysers in dispatch order (`(stage, cost_class)`
    /// ascending; insertion order on ties).
    pub fn iter_dispatch_order(&self) -> impl Iterator<Item = &Arc<dyn Analyzer>> {
        let mut indexed: Vec<(usize, &Arc<dyn Analyzer>)> =
            self.analyzers.iter().enumerate().collect();
        indexed.sort_by(|a, b| {
            a.1.stage()
                .cmp(&b.1.stage())
                .then(cost_class_rank(a.1.cost_class()).cmp(&cost_class_rank(b.1.cost_class())))
                .then(a.0.cmp(&b.0))
        });
        // Collect into a Vec then return an owned iterator so the
        // caller is not borrow-locked to the registry beyond the
        // sort.
        indexed
            .into_iter()
            .map(|(_, a)| a)
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Collect analysers for a particular stage in dispatch order.
    /// L3 dispatch uses this filter to skip L5+ analysers entirely.
    pub fn analyzers_for_stage(&self, stage: Stage) -> Vec<Arc<dyn Analyzer>> {
        self.iter_dispatch_order()
            .filter(|a| a.stage() == stage)
            .cloned()
            .collect()
    }

    /// Merge a declarative `analyzers.yaml` file. Specs whose `id`
    /// matches a built-in update that built-in's `declared` spec in
    /// place; unknown ids accumulate so the fingerprint reflects the
    /// user's intent. Phase 1 does not yet instantiate subprocess
    /// analysers — those land in Phase 2.
    ///
    /// Validates each spec via [`AnalyzerSpec::validate`]; an invalid
    /// pair (e.g. `transport: subprocess` with no `subprocess:` map)
    /// is dropped with a warning to stderr — the merge is otherwise
    /// best-effort.
    pub fn merge_yaml(&mut self, yaml: &AnalyzersFile) {
        for spec in &yaml.analyzers {
            if let Err(e) = spec.validate() {
                eprintln!(
                    "warning: analyzers.yaml spec `{}` failed validation: {e}; skipping",
                    spec.id
                );
                continue;
            }
            if let Some(existing) = self.declared.analyzers.iter_mut().find(|s| s.id == spec.id) {
                // Update applicability + version + cost class so the
                // user's overrides flow into the dispatcher's
                // fingerprint. The instance is not replaced — Phase 1
                // built-ins ignore the YAML applicability filter
                // because their `applies` predicates have richer
                // semantics (e.g. parsed-Cargo workspace detection)
                // than the YAML can express. Phase 2 will tighten
                // this once subprocess analysers and full
                // `applies`-by-YAML are wired.
                *existing = spec.clone();
            } else {
                // Unknown id: record but do not instantiate. The
                // fingerprint includes the spec, so a user-authored
                // analyser declaration invalidates the cache as
                // expected even when the runtime cannot yet honour
                // it.
                self.declared.analyzers.push(spec.clone());
            }
        }
        // Merge the optional config map last-writer-wins.
        for (k, v) in &yaml.config {
            self.declared.config.insert(k.clone(), v.clone());
        }
    }

    /// Canonical sha256 hex of the merged [`AnalyzersFile`]. The
    /// computation is `sha256(serde_yaml::to_string(&file).as_bytes())`
    /// rendered as 64-character lowercase hex; the hash is mixed
    /// with [`REGISTRY_HASH_NAMESPACE`] so a future wire-form change
    /// can be disambiguated.
    pub fn registry_sha(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(REGISTRY_HASH_NAMESPACE.as_bytes());
        hasher.update([0u8]); // separator
        let yaml = serde_yaml::to_string(&self.declared)
            .expect("AnalyzersFile must serialise — every field is plain serde");
        hasher.update(yaml.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut hex = String::with_capacity(64);
        use std::fmt::Write;
        for b in digest {
            write!(&mut hex, "{b:02x}").expect("writing to String never fails");
        }
        hex
    }

    /// View of the merged declarative registry. Useful for tests
    /// and for the L9 projection that emits
    /// `<output>/.atlas/analyzers.yaml`-derived metadata.
    pub fn declared(&self) -> &AnalyzersFile {
        &self.declared
    }
}

/// Total order on `CostClass` matching the dispatch order in
/// design §7.3: cheapest first.
fn cost_class_rank(c: atlas_index::CostClass) -> u8 {
    use atlas_index::CostClass::*;
    match c {
        DeterministicCheap => 0,
        DeterministicExpensive => 1,
        LlmCheap => 2,
        LlmExpensive => 3,
    }
}

/// Build an [`AnalyzerSpec`] from an analyser instance. The spec is
/// the declarative form mirrored into `analyzers.yaml`.
fn spec_for_analyzer(analyzer: &Arc<dyn Analyzer>) -> AnalyzerSpec {
    let id = analyzer.id().to_string();
    let stage = analyzer.stage();
    let cost_class = analyzer.cost_class();
    let version = analyzer.version().to_string();

    // Built-in confidence flavours. The dispatcher ignores this
    // field today (it makes its decisions on the runtime
    // `AnalyzerResult` arms) but the wire form is canonical.
    let confidence = if id == crate::llm_classify::ANALYZER_ID
        || id == crate::shell_script_llm_analyzer::ANALYZER_ID
    {
        // Both LLM analysers emit `Graded` results; the wire form
        // reflects that with `Declines` (threshold-gated confidence).
        Some(atlas_index::Confidence::Declines)
    } else {
        Some(atlas_index::Confidence::Binary)
    };

    let applicability = if id == crate::dockerfile_classifier::ANALYZER_ID {
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/Dockerfile".into()],
            ..Default::default()
        }
    } else if id == crate::llm_classify::ANALYZER_ID {
        atlas_index::ApplicabilityPredicate {
            always: true,
            ..Default::default()
        }
    } else if id == crate::rust_surface_analyzer::ANALYZER_ID {
        // Rust-surface analysis applies wherever a Cargo.toml is
        // present (the L5 driver invokes the deterministic
        // extraction directly; the registry contribution is
        // primarily for the analyser_registry_sha lineage).
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/Cargo.toml".into()],
            ..Default::default()
        }
    } else if id == crate::ts_js_classifier::ANALYZER_ID
        || id == crate::ts_js_surface_analyzer::ANALYZER_ID
    {
        // Both TS/JS analysers key on `package.json` presence.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/package.json".into()],
            ..Default::default()
        }
    } else if id == crate::python_classifier::ANALYZER_ID {
        // Python L3 classifier keys on the three canonical Python
        // manifest signals.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec![
                "**/pyproject.toml".into(),
                "**/setup.py".into(),
                "**/requirements.txt".into(),
            ],
            ..Default::default()
        }
    } else if id == crate::compose_classifier::ANALYZER_ID {
        // Compose classifier keys on the four canonical Docker Compose
        // filename patterns (exact and override forms).
        atlas_index::ApplicabilityPredicate {
            file_globs: vec![
                "**/docker-compose.yml".into(),
                "**/docker-compose.yaml".into(),
                "**/docker-compose.*.yml".into(),
                "**/docker-compose.*.yaml".into(),
                "**/compose.yml".into(),
                "**/compose.yaml".into(),
                "**/compose.*.yml".into(),
                "**/compose.*.yaml".into(),
            ],
            ..Default::default()
        }
    } else if id == crate::csharp_classifier::ANALYZER_ID {
        // C# L3 classifier keys on *.csproj and *.sln manifest signals.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/*.csproj".into(), "**/*.sln".into()],
            ..Default::default()
        }
    } else if id == crate::dart_classifier::ANALYZER_ID {
        // Dart L3 classifier keys on `pubspec.yaml` presence.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/pubspec.yaml".into()],
            ..Default::default()
        }
    } else if id == crate::racket_classifier::ANALYZER_ID {
        // Racket L3 classifier keys on `info.rkt` presence.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/info.rkt".into()],
            ..Default::default()
        }
    } else if id == crate::lispkit_classifier::ANALYZER_ID {
        // LispKit L3 classifier keys on R7RS `*.sld` library
        // declaration files.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec!["**/*.sld".into()],
            ..Default::default()
        }
    } else if id == crate::shell_script_llm_analyzer::ANALYZER_ID {
        // Shell-script / Makefile LLM analyser keys on shell and make
        // file patterns.
        atlas_index::ApplicabilityPredicate {
            file_globs: vec![
                "**/*.sh".into(),
                "**/*.bash".into(),
                "**/*.zsh".into(),
                "**/Makefile".into(),
                "**/GNUmakefile".into(),
                "**/*.mk".into(),
            ],
            ..Default::default()
        }
    } else {
        atlas_index::ApplicabilityPredicate::default()
    };

    AnalyzerSpec {
        id,
        stage,
        applicability,
        cost_class,
        confidence,
        transport: atlas_index::Transport::InProcess,
        subprocess: None,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lists_thirteen_analysers() {
        let r = AnalyzerRegistry::builtin();
        assert_eq!(r.len(), 13);
        let ids: Vec<&str> = r.iter_dispatch_order().map(|a| a.id()).collect();
        assert!(ids.contains(&crate::compose_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::dockerfile_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::ts_js_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::python_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::csharp_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::dart_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::racket_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::lispkit_classifier::ANALYZER_ID));
        assert!(ids.contains(&crate::llm_classify::ANALYZER_ID));
        assert!(ids.contains(&crate::shell_script_llm_analyzer::ANALYZER_ID));
        assert!(ids.contains(&crate::rust_surface_analyzer::ANALYZER_ID));
        assert!(ids.contains(&crate::ts_js_surface_analyzer::ANALYZER_ID));
        assert!(ids.contains(&crate::elixir_classifier::ANALYZER_ID));
        // L3 deterministic analysers come first, then L3 LLM, then L5
        // analysers (sorted by `(stage, cost_class)`). The two L5
        // analysers land after every L3 entry.
        let last_two: Vec<&str> = ids.iter().rev().take(2).copied().collect();
        assert!(last_two.contains(&crate::rust_surface_analyzer::ANALYZER_ID));
        assert!(last_two.contains(&crate::ts_js_surface_analyzer::ANALYZER_ID));
    }

    #[test]
    fn registry_sha_is_64_char_hex() {
        let sha = AnalyzerRegistry::builtin().registry_sha();
        assert_eq!(sha.len(), 64);
        assert!(sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn registry_sha_changes_when_config_changes() {
        let a = AnalyzerRegistry::builtin().registry_sha();
        let mut r = AnalyzerRegistry::builtin();
        let mut yaml = AnalyzersFile::default();
        yaml.config.insert(
            "dockerfile-l3".into(),
            serde_yaml::Value::String("custom".into()),
        );
        r.merge_yaml(&yaml);
        assert_ne!(a, r.registry_sha());
    }

    #[test]
    fn merge_yaml_accepts_unknown_id_and_keeps_builtin_count() {
        let mut r = AnalyzerRegistry::builtin();
        let mut yaml = AnalyzersFile::default();
        yaml.analyzers.push(AnalyzerSpec {
            id: "future-cool-analyzer".into(),
            stage: Stage::L5,
            applicability: atlas_index::ApplicabilityPredicate::default(),
            cost_class: atlas_index::CostClass::DeterministicCheap,
            confidence: None,
            transport: atlas_index::Transport::InProcess,
            subprocess: None,
            version: "0.1.0".into(),
        });
        r.merge_yaml(&yaml);
        // The built-in analyser instances are unchanged in count
        // (unknown spec is recorded but does not produce a runnable
        // instance).
        assert_eq!(r.len(), 13);
        // The declared list grew by one.
        assert_eq!(r.declared().analyzers.len(), 14);
    }

    #[test]
    fn merge_yaml_updates_existing_built_in_spec_in_place() {
        let mut r = AnalyzerRegistry::builtin();
        let mut yaml = AnalyzersFile::default();
        yaml.analyzers.push(AnalyzerSpec {
            id: crate::dockerfile_classifier::ANALYZER_ID.into(),
            stage: Stage::L3,
            applicability: atlas_index::ApplicabilityPredicate {
                file_globs: vec!["**/Dockerfile".into(), "**/Containerfile".into()],
                ..Default::default()
            },
            cost_class: atlas_index::CostClass::DeterministicCheap,
            confidence: Some(atlas_index::Confidence::Binary),
            transport: atlas_index::Transport::InProcess,
            subprocess: None,
            version: "9.9.9".into(),
        });
        r.merge_yaml(&yaml);
        assert_eq!(r.declared().analyzers.len(), 13);
        let docker = r
            .declared()
            .analyzers
            .iter()
            .find(|s| s.id == crate::dockerfile_classifier::ANALYZER_ID)
            .unwrap();
        assert_eq!(docker.version, "9.9.9");
        assert_eq!(docker.applicability.file_globs.len(), 2);
    }

    #[test]
    fn merge_yaml_skips_invalid_specs() {
        let mut r = AnalyzerRegistry::builtin();
        let before = r.registry_sha();
        let mut yaml = AnalyzersFile::default();
        yaml.analyzers.push(AnalyzerSpec {
            id: "broken-spec".into(),
            stage: Stage::L3,
            applicability: atlas_index::ApplicabilityPredicate::default(),
            cost_class: atlas_index::CostClass::DeterministicCheap,
            confidence: None,
            // Mismatched transport / subprocess pair fails validate().
            transport: atlas_index::Transport::Subprocess,
            subprocess: None,
            version: "0".into(),
        });
        r.merge_yaml(&yaml);
        // The invalid spec is dropped silently; the registry is
        // unchanged.
        assert_eq!(r.registry_sha(), before);
    }

    #[test]
    fn built_in_ids_match_analyser_self_reports() {
        // Sanity check that the analyser-id constants match the
        // analyser instances' own `id()` returns. A drift here would
        // silently break `merge_yaml`'s in-place update behaviour.
        assert_eq!(CsharpClassifier.id(), crate::csharp_classifier::ANALYZER_ID);
        assert_eq!(
            DockerfileClassifier.id(),
            crate::dockerfile_classifier::ANALYZER_ID
        );
        assert_eq!(LlmClassifyAnalyzer.id(), crate::llm_classify::ANALYZER_ID);
        assert_eq!(
            RustSurfaceAnalyzer.id(),
            crate::rust_surface_analyzer::ANALYZER_ID
        );
        assert_eq!(TsJsClassifier.id(), crate::ts_js_classifier::ANALYZER_ID);
        assert_eq!(
            TsJsSurfaceAnalyzer.id(),
            crate::ts_js_surface_analyzer::ANALYZER_ID
        );
        assert_eq!(PythonClassifier.id(), crate::python_classifier::ANALYZER_ID);
        assert_eq!(
            ComposeClassifier.id(),
            crate::compose_classifier::ANALYZER_ID
        );
        assert_eq!(DartClassifier.id(), crate::dart_classifier::ANALYZER_ID);
        assert_eq!(RacketClassifier.id(), crate::racket_classifier::ANALYZER_ID);
        assert_eq!(
            LispKitClassifier.id(),
            crate::lispkit_classifier::ANALYZER_ID
        );
        assert_eq!(
            ShellScriptLlmAnalyzer::new().id(),
            crate::shell_script_llm_analyzer::ANALYZER_ID
        );
    }
}
