//! Analyser registry and three reference analysers for Atlas vNext.
//!
//! This crate establishes the plugin protocol described in design §5.2,
//! §7.1, §7.3 and ships three in-process reference analysers:
//!
//! - [`cargo_classifier::CargoClassifier`] — `Cargo.toml` → `kind:
//!   rust-library | rust-cli | workspace`. Cheapest applicable
//!   analyser (`CostClass::DeterministicCheap`).
//! - [`dockerfile_classifier::DockerfileClassifier`] — `Dockerfile` →
//!   `kind: docker-image`. Same cost class.
//! - [`llm_classify::LlmClassifyAnalyzer`] — fallback that wraps an
//!   LLM classify call. `CostClass::LlmCheap`,
//!   `Confidence::Declines` for cases the deterministic classifiers
//!   handled.
//!
//! Subprocess transport (design §5.2, §7.2) is Phase 2; every analyser
//! shipped here runs in-process.
//!
//! # Public API
//!
//! - [`Analyzer`] trait — every analyser implements this.
//! - [`AnalyzerRegistry`] — ordered analyser list with `dispatch`.
//! - [`Target`] — what an analyser sees: candidate dir, language tags,
//!   manifest bytes.
//! - [`AnalysisContext`] — LLM hook and any read-only engine state.
//! - [`AnalyzerResult`], [`AnalyzerError`], [`StageOutput`],
//!   [`FingerprintInput`].
//!
//! # Cache wiring (PR-10)
//!
//! Every analyser implements [`Analyzer::fingerprint_inputs`] so the
//! L3 / L5 / L6 dispatcher can pre-compute a cache fingerprint and
//! skip the call on hit. PR-5 makes the method *callable* and
//! deterministic; the actual cache short-circuit is wired by PR-10.
//! The L3 dispatcher in this PR calls `analyse` directly.

pub mod cargo_classifier;
pub mod dispatcher;
pub mod dockerfile_classifier;
pub mod llm_classify;
pub mod registry;
pub mod rust_surface_analyzer;

use std::any::Any;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use atlas_index::{CostClass, Stage};

pub use cargo_classifier::CargoClassifier;
pub use dispatcher::DispatchOutcome;
pub use dockerfile_classifier::DockerfileClassifier;
pub use llm_classify::{LlmClassifyAnalyzer, LlmHook, LlmHookError};
pub use registry::{AnalyzerRegistry, REGISTRY_HASH_NAMESPACE};
pub use rust_surface_analyzer::{
    extract_rust_surface, RustSourceInputs, RustSurfaceAnalyzer, RustSurfaceOutput,
};

/// SHA-256 hex of a half-open byte range `bytes[span.0..span.1]`.
/// Mirrors the engine-side `code_derived_content_sha` so the
/// analyser crate can compute binding shas without taking on a
/// dependency on `atlas-engine` (the dep arrow goes the other way).
/// Out-of-bounds spans hash the empty byte slice rather than panic.
pub fn sha256_hex_of_range(bytes: &[u8], span: (usize, usize)) -> String {
    use sha2::{Digest, Sha256};
    let (start, end) = span;
    let slice: &[u8] = if start <= end && end <= bytes.len() {
        &bytes[start..end]
    } else {
        b""
    };
    let digest: [u8; 32] = Sha256::digest(slice).into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}

/// What an analyser sees. Built by the engine before calling
/// [`AnalyzerRegistry::dispatch`].
///
/// Phase 1 carries the four pieces every reference analyser needs:
/// the candidate's directory, the language tags inferred at L1, the
/// manifests already loaded by L3 (basename → bytes), and the list of
/// file basenames present at the candidate dir's top level. The shape
/// is intentionally minimal — Phase 2 may extend it as new stages
/// adopt the trait.
#[derive(Debug, Clone)]
pub struct Target {
    /// Absolute path to the candidate directory the analyser is being
    /// asked about.
    pub dir: PathBuf,
    /// Language tags inferred upstream (L1's heuristics, manifest
    /// presence, file extensions). The set is used by analyser
    /// `applies` predicates that key on `languages: [...]`.
    pub languages: BTreeSet<String>,
    /// Manifests pre-read by the engine, keyed by basename. The L3
    /// pipeline already loads every manifest under the candidate dir;
    /// the value is the raw bytes (UTF-8 not assumed — analysers that
    /// need text decode themselves so binary input fails loudly
    /// instead of silently degrading).
    pub manifests: Vec<TargetFile>,
    /// Top-level file basenames at the candidate dir. Used by
    /// `file_globs`-keyed `applies` predicates.
    pub top_level_files: Vec<String>,
}

impl Target {
    /// Helper: find a manifest by basename. Returns the first matching
    /// entry or `None`. Phase 1 has at most one manifest per basename
    /// at any given candidate dir, so this is sufficient.
    pub fn manifest_by_name(&self, name: &str) -> Option<&TargetFile> {
        self.manifests.iter().find(|f| f.name == name)
    }
}

/// One file the engine has pre-loaded for the analyser. Used by the
/// `manifests` field of [`Target`] and could be reused by future
/// stages that need other pre-read inputs.
#[derive(Debug, Clone)]
pub struct TargetFile {
    /// Basename (e.g. `Cargo.toml`, `Dockerfile`).
    pub name: String,
    /// Path relative to the candidate dir (typically just the
    /// basename, but may be a sub-path for nested manifests).
    pub relpath: PathBuf,
    /// Raw file contents. UTF-8 not assumed.
    pub bytes: Vec<u8>,
    /// Sha256 hex of `bytes`. Pre-computed by the engine so analyser
    /// `fingerprint_inputs` can return it cheaply.
    pub content_sha: String,
}

/// Read-only context an analyser may consult during `analyse`.
///
/// Phase 1 carries an [`LlmHook`] for the [`LlmClassifyAnalyzer`].
/// Future stages may extend the context with other engine handles
/// (e.g. a graph snapshot for L6). Trait objects so the engine can
/// install a different hook in tests.
pub struct AnalysisContext {
    /// LLM call hook. Analysers that need an LLM (currently only
    /// `LlmClassifyAnalyzer`) call into this. `None` means LLM calls
    /// are not available — analysers that require one and find `None`
    /// must return [`AnalyzerResult::Declines`] rather than erroring.
    pub llm: Option<Arc<dyn LlmHook>>,
}

impl AnalysisContext {
    /// Construct a context with no LLM hook installed. Useful in
    /// tests that only exercise deterministic analysers.
    pub fn deterministic_only() -> Self {
        AnalysisContext { llm: None }
    }

    /// Construct a context with the given LLM hook.
    pub fn with_llm(llm: Arc<dyn LlmHook>) -> Self {
        AnalysisContext { llm: Some(llm) }
    }
}

/// One contributor an analyser declares for the L3+ stage cache
/// fingerprint. The dispatcher hashes the returned set via
/// `FingerprintBuilder` before calling `analyse`. PR-10 wires the
/// short-circuit; for PR-5 the method must be callable and
/// deterministic.
///
/// Variants are intentionally narrow — any new shape of contributor
/// (e.g. a participant surface sha for L6) is added explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FingerprintInput {
    /// Sha256 hex of one file's contents. Maps onto
    /// `FingerprintBuilder::add_file_content_sha`.
    FileContentSha(String),
    /// Tagged opaque payload for analyser-specific contributions
    /// (e.g. a registry identifier slice, a dependency hash). The
    /// dispatcher hashes the tag and bytes together via the
    /// `add_file_content_sha` channel — it appears as just-another
    /// content sha to the cache key.
    Custom { tag: &'static str, bytes: Vec<u8> },
}

/// Marker trait carried by every successful analyser output. Any
/// type that implements [`StageOutput`] can be downcast through
/// `&dyn StageOutput` back to the concrete stage-specific type
/// (e.g. `Classification` for L3) via [`StageOutput::as_any`]. Trait
/// bounds match those of [`std::any::Any`] plus thread-safety so the
/// result can flow through `Arc<...>` if a caller wants to memoise
/// it.
///
/// The trait is implemented only via the [`impl_stage_output!`]
/// macro (for the three built-in analyser outputs in this crate).
/// A blanket impl is deliberately avoided — `Box<dyn StageOutput>`
/// would itself qualify under a blanket impl, and method resolution
/// on `Box::as_any()` would silently route through the box rather
/// than the inner value, breaking downcasts. The macro stamps a
/// per-type `as_any` body that always returns the inner value's
/// `&dyn Any` projection.
pub trait StageOutput: Send + Sync + 'static {
    /// Cast helper used by stage adapters (e.g. L3) to recover the
    /// concrete output type. Implementations always return
    /// `self as &dyn Any` (the macro stamps this body).
    fn as_any(&self) -> &dyn Any;
}

/// Stamp [`StageOutput`] for a concrete `Send + Sync + 'static`
/// type. The `as_any` body is the trivial `self` cast; per-type
/// instantiation prevents method resolution ambiguity that a
/// blanket impl would introduce on `Box<dyn StageOutput>` itself.
#[macro_export]
macro_rules! impl_stage_output {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $crate::StageOutput for $ty {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self as &dyn ::std::any::Any
                }
            }
        )+
    };
}

/// Errors an analyser can return out-of-band. Distinct from
/// [`AnalyzerResult::Declines`] (which is *normal* — the analyser
/// chose not to handle the input) and from `Confident` /  `Graded`
/// outputs (success).
#[derive(Debug, thiserror::Error)]
pub enum AnalyzerError {
    /// The analyser ran but its inputs were malformed (e.g.
    /// `Cargo.toml` failed to parse). The stage adapter typically
    /// degrades to a weak unknown classification.
    #[error("malformed input for analyser `{analyzer_id}`: {message}")]
    MalformedInput {
        analyzer_id: String,
        message: String,
    },
    /// The analyser tried to delegate to an external service (e.g.
    /// LLM call) and the call failed. The stage adapter degrades the
    /// same way.
    #[error("call failed for analyser `{analyzer_id}`: {message}")]
    CallFailed {
        analyzer_id: String,
        message: String,
    },
    /// Catch-all for unexpected failures inside an analyser.
    #[error("internal error in analyser `{analyzer_id}`: {source}")]
    Internal {
        analyzer_id: String,
        #[source]
        source: anyhow::Error,
    },
}

/// Outcome of a single [`Analyzer::analyse`] call. The dispatcher
/// short-circuits on `Confident` / `Graded` and falls through on
/// `Declines`; `Error` propagates back to the caller as a soft
/// failure (the stage adapter decides whether to degrade or escalate).
pub enum AnalyzerResult {
    /// The analyser is sure of its answer. Highest precedence.
    Confident(Box<dyn StageOutput>),
    /// The analyser produced an answer with a graded confidence in
    /// `[0.0, 1.0]`. The stage adapter compares against the
    /// configured threshold.
    Graded {
        output: Box<dyn StageOutput>,
        confidence: f32,
    },
    /// The analyser does not handle this target. The dispatcher
    /// consults the next analyser in registry order.
    Declines,
    /// The analyser tried but failed. Treated as a soft failure: the
    /// dispatcher records the error and moves on.
    Error(AnalyzerError),
}

impl std::fmt::Debug for AnalyzerResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Confident(_) => f.write_str("AnalyzerResult::Confident(<output>)"),
            Self::Graded { confidence, .. } => f
                .debug_struct("AnalyzerResult::Graded")
                .field("confidence", confidence)
                .field("output", &"<output>")
                .finish(),
            Self::Declines => f.write_str("AnalyzerResult::Declines"),
            Self::Error(e) => f.debug_tuple("AnalyzerResult::Error").field(e).finish(),
        }
    }
}

/// Plugin trait every analyser implements. The dispatcher iterates
/// the registry in `(stage, cost_class)` ascending order and consults
/// each analyser whose [`Analyzer::applies`] returns true.
///
/// `Send + Sync` so a single registry can be shared across L8's
/// rayon-driven map step and across the L9 pre-warm thread pool.
pub trait Analyzer: Send + Sync {
    /// Stable analyser id (`cargo-toml-classifier`, `dockerfile-l3`,
    /// `llm-classify-fallback`). Contributes to the cache fingerprint.
    fn id(&self) -> &str;
    /// Which L-stage this analyser plugs into. PR-5 ships only L3
    /// analysers; PR-7 will add L5 surface analysers.
    fn stage(&self) -> Stage;
    /// Cost class (design §7.3). The dispatcher prefers cheaper
    /// analysers and falls through to more expensive ones only when
    /// they `Decline`.
    fn cost_class(&self) -> CostClass;
    /// Free-form version string. Bumped when an analyser's behaviour
    /// changes; included in the cache fingerprint so a version bump
    /// invalidates cached outputs.
    fn version(&self) -> &str;

    /// Pre-flight applicability check. Cheap (no I/O); returns false
    /// to skip [`Analyzer::analyse`] entirely. The dispatcher honours
    /// the result; an analyser that returns `false` is never invoked
    /// and consequently does not contribute to the fingerprint
    /// either.
    fn applies(&self, target: &Target) -> bool;

    /// Declare every input this analyser would consult on the given
    /// target. Used by the L3+ dispatcher (in PR-10) to compute the
    /// stage cache fingerprint without invoking the analyser. Must
    /// be deterministic — two calls on the same `Target` return
    /// equal sets.
    fn fingerprint_inputs(&self, target: &Target) -> Vec<FingerprintInput>;

    /// Run the analyser. May involve I/O (file reads, LLM calls).
    /// Returns one of the four [`AnalyzerResult`] arms; `Declines`
    /// is normal fallthrough and not an error.
    fn analyse(&self, ctx: &AnalysisContext, target: &Target) -> AnalyzerResult;
}

// Stamp `StageOutput` for every concrete output type shipped by
// this crate's reference analysers. External crates that want to
// register their own analysers call `impl_stage_output!` at their
// definition site.
impl_stage_output!(
    cargo_classifier::CargoClassificationOutput,
    dockerfile_classifier::DockerfileClassificationOutput,
    llm_classify::LlmClassifyOutput,
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Debug, PartialEq, Eq)]
    struct DummyOutput {
        value: u32,
    }
    impl_stage_output!(DummyOutput);

    fn target() -> Target {
        Target {
            dir: PathBuf::from("/tmp/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        }
    }

    #[test]
    fn stage_output_round_trips_through_box_dyn() {
        let boxed: Box<dyn StageOutput> = Box::new(DummyOutput { value: 42 });
        let recovered = boxed.as_any().downcast_ref::<DummyOutput>().unwrap();
        assert_eq!(recovered.value, 42);
    }

    #[test]
    fn target_manifest_by_name_returns_first_match() {
        let mut t = target();
        t.manifests.push(TargetFile {
            name: "Cargo.toml".into(),
            relpath: PathBuf::from("Cargo.toml"),
            bytes: b"[package]".to_vec(),
            content_sha: "abc".into(),
        });
        t.manifests.push(TargetFile {
            name: "package.json".into(),
            relpath: PathBuf::from("package.json"),
            bytes: b"{}".to_vec(),
            content_sha: "def".into(),
        });
        assert!(t.manifest_by_name("Cargo.toml").is_some());
        assert!(t.manifest_by_name("Dockerfile").is_none());
    }

    #[test]
    fn analyzer_result_debug_does_not_require_output_debug() {
        // The `Confident`/`Graded` variants carry `Box<dyn StageOutput>`
        // which does NOT require Debug; the manual impl prints a
        // placeholder so logs/tests can format the variant anyway.
        let r = AnalyzerResult::Confident(Box::new(DummyOutput { value: 1 }));
        let s = format!("{r:?}");
        assert!(s.contains("Confident"), "got `{s}`");
    }
}
