//! Dispatcher for analyser invocation.
//!
//! [`AnalyzerRegistry::dispatch`] is the main entry point: it takes a
//! [`Target`] and an [`AnalysisContext`] and returns the verdict of
//! the cheapest applicable analyser.
//!
//! Algorithm (design §7.3):
//!
//! 1. Iterate analysers in `(stage, cost_class)` ascending order.
//! 2. Skip analysers whose `applies(target)` returns false.
//! 3. Invoke `analyse(ctx, target)`. On `Confident` or `Graded`,
//!    short-circuit. On `Declines`, fall through. On `Error`, record
//!    the error and fall through (the next analyser may still
//!    succeed).
//! 4. If every analyser declined, return [`DispatchOutcome::AllDeclined`]
//!    with the accumulated errors.
//!
//! The dispatcher is pure: it owns no mutable state and does not
//! short-circuit on the first error. The caller (an L3 / L5 / L6
//! adapter) decides how to translate the outcome into a stage-shaped
//! result.

use crate::registry::AnalyzerRegistry;
use crate::{AnalysisContext, AnalyzerError, AnalyzerResult, Stage, Target};

/// Outcome of [`AnalyzerRegistry::dispatch`]. Distinct from
/// [`AnalyzerResult`] because the dispatcher accumulates errors that
/// a single analyser would lose.
pub enum DispatchOutcome {
    /// One analyser produced a `Confident` verdict.
    Confident {
        /// TODO(post-PR-4): redundant with the &str returned by `dispatch`;
        /// remove once all callers use the tuple form.
        analyzer_id: String,
        output: Box<dyn crate::StageOutput>,
    },
    /// One analyser produced a `Graded` verdict.
    Graded {
        /// TODO(post-PR-4): redundant with the &str returned by `dispatch`;
        /// remove once all callers use the tuple form.
        analyzer_id: String,
        output: Box<dyn crate::StageOutput>,
        confidence: f32,
    },
    /// Every applicable analyser declined. `errors` holds any
    /// out-of-band failures encountered along the way (analysers
    /// that errored rather than declined). When `errors` is empty,
    /// no analyser was applicable; when non-empty, at least one
    /// analyser tried and failed.
    AllDeclined { errors: Vec<AnalyzerError> },
}

impl std::fmt::Debug for DispatchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Confident { analyzer_id, .. } => f
                .debug_struct("DispatchOutcome::Confident")
                .field("analyzer_id", analyzer_id)
                .field("output", &"<output>")
                .finish(),
            Self::Graded {
                analyzer_id,
                confidence,
                ..
            } => f
                .debug_struct("DispatchOutcome::Graded")
                .field("analyzer_id", analyzer_id)
                .field("confidence", confidence)
                .field("output", &"<output>")
                .finish(),
            Self::AllDeclined { errors } => f
                .debug_struct("DispatchOutcome::AllDeclined")
                .field("error_count", &errors.len())
                .finish(),
        }
    }
}

/// Sentinel id reported when every applicable analyser declines.
/// Carried through `dispatch` / `dispatch_with_filter` so the L3
/// adapter (and the per-component projection downstream of it) can
/// emit a non-empty analyser identity even on the all-declined path.
pub const NONE_ANALYZER_ID: &str = "none";

/// Sentinel version reported alongside [`NONE_ANALYZER_ID`].
pub const NONE_ANALYZER_VERSION: &str = "0.0.0";

impl AnalyzerRegistry {
    /// Dispatch a single target through the registry. The `stage`
    /// filter restricts iteration to analysers that opted into the
    /// given stage; analysers from other stages are ignored.
    ///
    /// Returns `(outcome, analyser_id, analyser_version)`. On a
    /// winning verdict (`Confident` / `Graded`) the id and version
    /// are the dispatching analyser's `id()` and `version()` reports.
    /// On `AllDeclined` they are the [`NONE_ANALYZER_ID`] /
    /// [`NONE_ANALYZER_VERSION`] sentinels — non-empty so downstream
    /// consumers (per-component YAML, fingerprints) never have to
    /// special-case the empty-string case.
    ///
    /// The id/version slices borrow from the registry's analyser
    /// instances, so they live as long as `&self`.
    pub fn dispatch<'a>(
        &'a self,
        ctx: &AnalysisContext,
        target: &Target,
        stage: Stage,
    ) -> (DispatchOutcome, &'a str, &'a str) {
        self.dispatch_with_filter(ctx, target, stage, |_| true)
    }

    /// Dispatch a single target consulting only analysers that pass
    /// `cost_filter`. Used by L3's two-pass strategy: pass 1 includes
    /// only deterministic analysers (so a `Cargo.toml` with neither
    /// `[lib]` nor `[[bin]]` declines and the pre-existing
    /// `classify_deterministic` non-Cargo rules can still take a
    /// turn before the LLM is consulted); pass 2 includes the LLM
    /// analyser. The brief mandates the LLM analyser sit in the
    /// registry; this method gives the L3 adapter the seam it needs
    /// to interleave the legacy non-Cargo deterministic rules.
    ///
    /// Same return contract as [`AnalyzerRegistry::dispatch`].
    pub fn dispatch_with_filter<'a, F>(
        &'a self,
        ctx: &AnalysisContext,
        target: &Target,
        stage: Stage,
        cost_filter: F,
    ) -> (DispatchOutcome, &'a str, &'a str)
    where
        F: Fn(atlas_index::CostClass) -> bool,
    {
        let mut errors: Vec<AnalyzerError> = Vec::new();
        for analyzer in self.iter_dispatch_order() {
            if analyzer.stage() != stage {
                continue;
            }
            if !cost_filter(analyzer.cost_class()) {
                continue;
            }
            if !analyzer.applies(target) {
                continue;
            }
            match analyzer.analyse(ctx, target) {
                AnalyzerResult::Confident(output) => {
                    return (
                        DispatchOutcome::Confident {
                            analyzer_id: analyzer.id().to_string(),
                            output,
                        },
                        analyzer.id(),
                        analyzer.version(),
                    );
                }
                AnalyzerResult::Graded { output, confidence } => {
                    return (
                        DispatchOutcome::Graded {
                            analyzer_id: analyzer.id().to_string(),
                            output,
                            confidence,
                        },
                        analyzer.id(),
                        analyzer.version(),
                    );
                }
                AnalyzerResult::Declines => continue,
                AnalyzerResult::Error(e) => {
                    errors.push(e);
                    continue;
                }
            }
        }
        (
            DispatchOutcome::AllDeclined { errors },
            NONE_ANALYZER_ID,
            NONE_ANALYZER_VERSION,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo_classifier::CargoClassifier;
    use crate::llm_classify::LlmClassifyAnalyzer;
    use crate::registry::AnalyzerRegistry;
    use crate::TargetFile;
    use crate::{Analyzer, FingerprintInput};
    use atlas_index::CostClass;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Spy that returns a canned `AnalyzerResult` on every call and
    /// records the call count. Used to make the dispatcher's
    /// fallthrough behaviour observable.
    struct SpyAnalyzer {
        id: &'static str,
        stage: Stage,
        cost: CostClass,
        applies: bool,
        result: std::sync::Mutex<Option<Box<dyn FnMut() -> AnalyzerResult + Send>>>,
        calls: std::sync::Mutex<u32>,
    }

    impl SpyAnalyzer {
        fn new(
            id: &'static str,
            stage: Stage,
            cost: CostClass,
            applies: bool,
            f: impl FnMut() -> AnalyzerResult + Send + 'static,
        ) -> Self {
            SpyAnalyzer {
                id,
                stage,
                cost,
                applies,
                result: std::sync::Mutex::new(Some(Box::new(f))),
                calls: std::sync::Mutex::new(0),
            }
        }

        fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    impl Analyzer for SpyAnalyzer {
        fn id(&self) -> &str {
            self.id
        }
        fn stage(&self) -> Stage {
            self.stage
        }
        fn cost_class(&self) -> CostClass {
            self.cost
        }
        fn version(&self) -> &str {
            "test"
        }
        fn applies(&self, _target: &Target) -> bool {
            self.applies
        }
        fn fingerprint_inputs(&self, _target: &Target) -> Vec<FingerprintInput> {
            Vec::new()
        }
        fn analyse(&self, _ctx: &AnalysisContext, _target: &Target) -> AnalyzerResult {
            *self.calls.lock().unwrap() += 1;
            let mut guard = self.result.lock().unwrap();
            if let Some(f) = guard.as_mut() {
                f()
            } else {
                AnalyzerResult::Declines
            }
        }
    }

    struct StringOutput(&'static str);
    crate::impl_stage_output!(StringOutput);

    fn cargo_target_with_lib() -> Target {
        Target {
            dir: PathBuf::from("/ws/x"),
            languages: BTreeSet::new(),
            manifests: vec![TargetFile {
                name: "Cargo.toml".into(),
                relpath: PathBuf::from("Cargo.toml"),
                bytes: b"[package]\nname = \"x\"\n[lib]\npath = \"src/lib.rs\"\n".to_vec(),
                content_sha: "abc".into(),
            }],
            top_level_files: Vec::new(),
        }
    }

    #[test]
    fn dispatch_picks_cheapest_applicable() {
        // Built-in registry: Cargo (deterministic-cheap) + LLM
        // (llm-cheap). Both are applicable to a Cargo.toml. The
        // cheapest wins; the LLM is never consulted.
        let llm_calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let _llm_hook = {
            // `LlmClassifyAnalyzer.applies` is unconditional, so an
            // LLM hook would be consulted if Cargo did NOT win. We
            // install a stub hook to make the loss observable: if
            // the dispatcher reached LLM, the call count would be
            // non-zero.
            let count = llm_calls.clone();
            struct CountHook(std::sync::Arc<std::sync::Mutex<u32>>);
            impl crate::llm_classify::LlmHook for CountHook {
                fn classify(
                    &self,
                    _inputs: &serde_json::Value,
                ) -> Result<Arc<serde_json::Value>, crate::llm_classify::LlmHookError>
                {
                    *self.0.lock().unwrap() += 1;
                    Ok(Arc::new(serde_json::json!({"kind": "rust-library"})))
                }
            }
            std::sync::Arc::new(CountHook(count))
                as std::sync::Arc<dyn crate::llm_classify::LlmHook>
        };
        let ctx = AnalysisContext::with_llm(_llm_hook);

        let registry = AnalyzerRegistry::builtin();
        let target = cargo_target_with_lib();
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        match outcome {
            DispatchOutcome::Confident { analyzer_id, .. } => {
                assert_eq!(analyzer_id, CargoClassifier.id());
            }
            other => panic!("expected Confident from cargo-toml-classifier, got {other:?}"),
        }
        assert_eq!(*llm_calls.lock().unwrap(), 0, "LLM must not be consulted");
    }

    #[test]
    fn dispatch_falls_through_on_declines() {
        // Two analysers at the same stage; the cheaper declines, the
        // next confidently wins. The dispatcher's fall-through is
        // observable through the call counts.
        let mut registry = AnalyzerRegistry::empty();
        let cheap = Arc::new(SpyAnalyzer::new(
            "cheap-decliner",
            Stage::L3,
            CostClass::DeterministicCheap,
            true,
            || AnalyzerResult::Declines,
        ));
        let expensive = Arc::new(SpyAnalyzer::new(
            "expensive-confident",
            Stage::L3,
            CostClass::LlmCheap,
            true,
            || AnalyzerResult::Confident(Box::new(StringOutput("hello"))),
        ));
        registry.register(cheap.clone() as Arc<dyn Analyzer>);
        registry.register(expensive.clone() as Arc<dyn Analyzer>);

        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        match outcome {
            DispatchOutcome::Confident {
                analyzer_id,
                output,
            } => {
                assert_eq!(analyzer_id, "expensive-confident");
                let s = output
                    .as_any()
                    .downcast_ref::<StringOutput>()
                    .expect("output is StringOutput");
                assert_eq!(s.0, "hello");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
        assert_eq!(
            cheap.call_count(),
            1,
            "cheap analyser must have been called"
        );
        assert_eq!(
            expensive.call_count(),
            1,
            "fallthrough must reach expensive analyser"
        );
    }

    #[test]
    fn dispatch_skips_non_applicable_analysers() {
        let mut registry = AnalyzerRegistry::empty();
        let inapplicable = Arc::new(SpyAnalyzer::new(
            "inapplicable",
            Stage::L3,
            CostClass::DeterministicCheap,
            false,
            || AnalyzerResult::Confident(Box::new(StringOutput("never"))),
        ));
        let applicable = Arc::new(SpyAnalyzer::new(
            "applicable",
            Stage::L3,
            CostClass::LlmCheap,
            true,
            || AnalyzerResult::Confident(Box::new(StringOutput("yes"))),
        ));
        registry.register(inapplicable.clone() as Arc<dyn Analyzer>);
        registry.register(applicable.clone() as Arc<dyn Analyzer>);

        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        assert_eq!(inapplicable.call_count(), 0);
        assert_eq!(applicable.call_count(), 1);
        match outcome {
            DispatchOutcome::Confident { analyzer_id, .. } => {
                assert_eq!(analyzer_id, "applicable");
            }
            other => panic!("expected Confident, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_records_errors_and_falls_through() {
        let mut registry = AnalyzerRegistry::empty();
        let erroring = Arc::new(SpyAnalyzer::new(
            "erroring",
            Stage::L3,
            CostClass::DeterministicCheap,
            true,
            || {
                AnalyzerResult::Error(AnalyzerError::CallFailed {
                    analyzer_id: "erroring".into(),
                    message: "boom".into(),
                })
            },
        ));
        let confident = Arc::new(SpyAnalyzer::new(
            "confident",
            Stage::L3,
            CostClass::LlmCheap,
            true,
            || AnalyzerResult::Confident(Box::new(StringOutput("ok"))),
        ));
        registry.register(erroring.clone() as Arc<dyn Analyzer>);
        registry.register(confident.clone() as Arc<dyn Analyzer>);

        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        // Confident wins; the erroring analyser's error is dropped on
        // success per the semantics — only `AllDeclined` carries the
        // accumulated error list.
        assert!(matches!(outcome, DispatchOutcome::Confident { .. }));
    }

    #[test]
    fn dispatch_returns_all_declined_with_empty_errors_when_nothing_applies() {
        let registry = AnalyzerRegistry::empty();
        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        match outcome {
            DispatchOutcome::AllDeclined { errors } => assert!(errors.is_empty()),
            other => panic!("expected AllDeclined, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_filters_by_stage() {
        // L5 analyser must not respond to L3 dispatch.
        let mut registry = AnalyzerRegistry::empty();
        let l5 = Arc::new(SpyAnalyzer::new(
            "l5-only",
            Stage::L5,
            CostClass::DeterministicCheap,
            true,
            || AnalyzerResult::Confident(Box::new(StringOutput("never"))),
        ));
        registry.register(l5.clone() as Arc<dyn Analyzer>);

        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        assert_eq!(l5.call_count(), 0);
        assert!(matches!(outcome, DispatchOutcome::AllDeclined { .. }));
    }

    #[test]
    fn declines_then_decline_returns_all_declined() {
        // Used to verify the LLM-classify falls through to AllDeclined
        // when the LLM hook is None.
        let registry = AnalyzerRegistry::builtin();
        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, _id, _version) = registry.dispatch(&ctx, &target, Stage::L3);
        match outcome {
            DispatchOutcome::AllDeclined { errors } => assert!(errors.is_empty()),
            other => panic!("expected AllDeclined, got {other:?}"),
        }
        let _ = LlmClassifyAnalyzer; // proves symbol presence
    }

    #[test]
    fn dispatch_returns_winning_analyser_identity() {
        // Built-in registry: Cargo + LLM both apply at L3. The Cargo
        // analyser wins on cost; the dispatcher must return its
        // (id, version) tuple alongside the outcome.
        let llm_calls = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let _llm_hook = {
            let count = llm_calls.clone();
            struct CountHook(std::sync::Arc<std::sync::Mutex<u32>>);
            impl crate::llm_classify::LlmHook for CountHook {
                fn classify(
                    &self,
                    _inputs: &serde_json::Value,
                ) -> Result<Arc<serde_json::Value>, crate::llm_classify::LlmHookError>
                {
                    *self.0.lock().unwrap() += 1;
                    Ok(Arc::new(serde_json::json!({"kind": "rust-library"})))
                }
            }
            std::sync::Arc::new(CountHook(count))
                as std::sync::Arc<dyn crate::llm_classify::LlmHook>
        };
        let ctx = AnalysisContext::with_llm(_llm_hook);

        let registry = AnalyzerRegistry::builtin();
        let target = cargo_target_with_lib();
        let (outcome, id, version) = registry.dispatch(&ctx, &target, Stage::L3);
        assert!(matches!(outcome, DispatchOutcome::Confident { .. }));
        assert_eq!(id, crate::cargo_classifier::ANALYZER_ID);
        assert_eq!(version, crate::cargo_classifier::ANALYZER_VERSION);
        assert_eq!(*llm_calls.lock().unwrap(), 0, "LLM must not be consulted");
    }

    #[test]
    fn dispatch_all_declined_returns_none_identity() {
        // No analyser applies → the dispatcher returns the
        // ("none", "0.0.0") sentinel rather than an empty string,
        // so downstream consumers (per-component YAML, fingerprints)
        // do not have to special-case empty identities.
        let registry = AnalyzerRegistry::empty();
        let ctx = AnalysisContext::deterministic_only();
        let target = Target {
            dir: PathBuf::from("/x"),
            languages: BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        let (outcome, id, version) = registry.dispatch(&ctx, &target, Stage::L3);
        assert!(matches!(outcome, DispatchOutcome::AllDeclined { .. }));
        assert_eq!(id, NONE_ANALYZER_ID);
        assert_eq!(version, NONE_ANALYZER_VERSION);
    }
}
