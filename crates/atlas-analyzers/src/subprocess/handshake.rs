//! Capability negotiation for subprocess analysers.
//!
//! On spawn, the child emits a single [`Capabilities`] frame
//! announcing its identity (`id`, `version`, `stage`, `cost_class`)
//! and its applicability predicate. The parent compares this to
//! the registered [`SubprocessAnalyzerSpec`] and rejects the
//! handshake if any of the spec-bearing fields disagree.
//!
//! This is a sanity check, not a security boundary: the parent
//! already trusts the binary it spawned. The point is to catch
//! drift — a deployment where the subprocess binary on disk
//! disagrees with the YAML / built-in registration trips here
//! rather than producing silently wrong cache fingerprints.
//!
//! ## Wire shape
//!
//! The capabilities envelope is plain JSON, written as a single
//! length-prefixed frame (see [`crate::subprocess::transport`]).
//!
//! ```json
//! {
//!   "id": "py-surface",
//!   "version": "0.1.0",
//!   "stage": "l5",
//!   "cost_class": "deterministic-expensive",
//!   "applicability_predicate": { "languages": ["python"] }
//! }
//! ```

use atlas_index::{ApplicabilityPredicate, CostClass, Stage};
use serde::{Deserialize, Serialize};

/// Capability envelope a subprocess emits on startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub id: String,
    pub version: String,
    pub stage: Stage,
    pub cost_class: CostClass,
    pub applicability_predicate: ApplicabilityPredicate,
}

/// Reasons the handshake can fail. The proxy converts these into
/// [`crate::AnalyzerError::CallFailed`] with a stable message
/// prefix so logs remain greppable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    #[error("analyser id mismatch: parent expected `{expected}`, child reported `{actual}`")]
    IdMismatch { expected: String, actual: String },
    #[error("analyser version mismatch: parent expected `{expected}`, child reported `{actual}`")]
    VersionMismatch { expected: String, actual: String },
    #[error(
        "analyser stage mismatch: parent expected `{expected:?}`, child reported `{actual:?}`"
    )]
    StageMismatch { expected: Stage, actual: Stage },
    #[error(
        "analyser cost_class mismatch: parent expected `{expected:?}`, child reported `{actual:?}`"
    )]
    CostClassMismatch {
        expected: CostClass,
        actual: CostClass,
    },
    #[error(
        "analyser applicability_predicate mismatch: parent expected `{expected:?}`, child reported `{actual:?}`"
    )]
    ApplicabilityMismatch {
        expected: ApplicabilityPredicate,
        actual: ApplicabilityPredicate,
    },
}

/// Verify that a child's capabilities envelope agrees with the
/// parent's registered spec. Returns the first mismatch (if any).
///
/// The `ApplicabilityPredicate` comparison is structural: the YAML
/// would be normalised on both sides, so byte-identity is the
/// right contract here.
///
/// The `Err` is boxed because `HandshakeError` carries two
/// `ApplicabilityPredicate` clones (vectors of strings), which
/// pushes the unboxed variant past clippy's `result_large_err`
/// threshold.
pub fn verify_capabilities(
    expected: &Capabilities,
    actual: &Capabilities,
) -> Result<(), Box<HandshakeError>> {
    if expected.id != actual.id {
        return Err(Box::new(HandshakeError::IdMismatch {
            expected: expected.id.clone(),
            actual: actual.id.clone(),
        }));
    }
    if expected.version != actual.version {
        return Err(Box::new(HandshakeError::VersionMismatch {
            expected: expected.version.clone(),
            actual: actual.version.clone(),
        }));
    }
    if expected.stage != actual.stage {
        return Err(Box::new(HandshakeError::StageMismatch {
            expected: expected.stage,
            actual: actual.stage,
        }));
    }
    if expected.cost_class != actual.cost_class {
        return Err(Box::new(HandshakeError::CostClassMismatch {
            expected: expected.cost_class,
            actual: actual.cost_class,
        }));
    }
    if expected.applicability_predicate != actual.applicability_predicate {
        return Err(Box::new(HandshakeError::ApplicabilityMismatch {
            expected: expected.applicability_predicate.clone(),
            actual: actual.applicability_predicate.clone(),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(stage: Stage) -> Capabilities {
        Capabilities {
            id: "py-surface".into(),
            version: "0.1.0".into(),
            stage,
            cost_class: CostClass::DeterministicExpensive,
            applicability_predicate: ApplicabilityPredicate {
                languages: vec!["python".into()],
                ..Default::default()
            },
        }
    }

    #[test]
    fn capabilities_round_trip_through_json() {
        let c = sample(Stage::L5);
        let bytes = serde_json::to_vec(&c).unwrap();
        let parsed: Capabilities = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn verify_capabilities_accepts_equal_envelopes() {
        let c = sample(Stage::L5);
        verify_capabilities(&c, &c.clone()).unwrap();
    }

    #[test]
    fn handshake_rejects_mismatched_capabilities() {
        // Acceptance criterion: capabilities envelope declaring
        // `stage: l3` against a spec that declared `stage: l5`
        // errors with a clear message.
        let expected = sample(Stage::L5);
        let actual = sample(Stage::L3);
        let err = verify_capabilities(&expected, &actual).unwrap_err();
        match *err {
            HandshakeError::StageMismatch {
                expected: e,
                actual: a,
            } => {
                assert_eq!(e, Stage::L5);
                assert_eq!(a, Stage::L3);
            }
            other => panic!("expected StageMismatch, got {other:?}"),
        }
        // The Display impl carries both stages in the message.
        let err = verify_capabilities(&sample(Stage::L5), &sample(Stage::L3)).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("L5") && msg.contains("L3"), "got: {msg}");
    }

    #[test]
    fn verify_capabilities_detects_id_mismatch() {
        let expected = sample(Stage::L5);
        let mut actual = expected.clone();
        actual.id = "wrong-id".into();
        match *verify_capabilities(&expected, &actual).unwrap_err() {
            HandshakeError::IdMismatch {
                expected: ref e,
                actual: ref a,
            } => {
                assert_eq!(e, "py-surface");
                assert_eq!(a, "wrong-id");
            }
            ref other => panic!("expected IdMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_capabilities_detects_version_mismatch() {
        let expected = sample(Stage::L5);
        let mut actual = expected.clone();
        actual.version = "9.9.9".into();
        assert!(matches!(
            *verify_capabilities(&expected, &actual).unwrap_err(),
            HandshakeError::VersionMismatch { .. }
        ));
    }

    #[test]
    fn verify_capabilities_detects_cost_class_mismatch() {
        let expected = sample(Stage::L5);
        let mut actual = expected.clone();
        actual.cost_class = CostClass::DeterministicCheap;
        assert!(matches!(
            *verify_capabilities(&expected, &actual).unwrap_err(),
            HandshakeError::CostClassMismatch { .. }
        ));
    }

    #[test]
    fn verify_capabilities_detects_applicability_mismatch() {
        let expected = sample(Stage::L5);
        let mut actual = expected.clone();
        actual.applicability_predicate.languages.push("rust".into());
        assert!(matches!(
            *verify_capabilities(&expected, &actual).unwrap_err(),
            HandshakeError::ApplicabilityMismatch { .. }
        ));
    }
}
