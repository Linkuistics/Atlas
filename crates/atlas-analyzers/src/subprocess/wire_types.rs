//! Wire types for the subprocess analyser stdio JSON protocol.
//!
//! Every frame on the wire is a length-prefixed JSON value (see
//! [`crate::subprocess::transport`] for framing). The two top-level
//! shapes are:
//!
//! - [`Request`] — what the parent sends to the child. Discriminated
//!   on `kind`, which is one of `applies`, `fingerprint_inputs`, or
//!   `analyse`. The same envelope carries all three so a child only
//!   needs one decode step.
//! - [`Response`] — what the child sends back. Discriminated on
//!   `kind`, which is one of `confident`, `graded`, `declines`, or
//!   `error`. The boolean `applies` reply and the
//!   `fingerprint_inputs` reply ride a `confident` envelope (the
//!   payload is the boolean / array; the dispatcher knows which
//!   request it is replying to).
//!
//! The wire form is stable for Phase 2; Phase 3+ may add new variants
//! (each one explicitly).
//!
//! ## Why one envelope and not three
//!
//! Three discriminants — `applies`, `fingerprint_inputs`, `analyse` —
//! gives a single request/response cycle the same shape, which makes
//! the transport's job trivial (one read, one write, one parse). The
//! response payload is a `serde_json::Value` so the parent can decode
//! it into the request-specific shape after the framing has been
//! resolved.
//!
//! ## Greenfield
//!
//! Phase 1 had no wire form for analysers; this is a new design.
//! Versioning is not a load-bearing concern — every subprocess
//! analyser is shipped together with the parent that knows how to
//! drive it (the binary content sha makes that pairing
//! cache-observable).

use serde::{Deserialize, Serialize};

/// Top-level request envelope. Three `kind` values in Phase 2:
/// `applies`, `fingerprint_inputs`, `analyse`.
///
/// `target` carries the subset of [`crate::Target`] that crosses the
/// process boundary; `context` carries any request-specific extras
/// (currently empty — Phase 2 subprocess analysers do not have LLM
/// access). Both are JSON values rather than typed structs so the
/// envelope stays open to new request shapes without churning the
/// transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Ask the analyser whether it would handle this target. The
    /// reply is a [`Response::Confident`] whose payload is a JSON
    /// bool. Cheap; meant to mirror [`crate::Analyzer::applies`].
    Applies { target: WireTarget },
    /// Ask the analyser to declare its fingerprint inputs for this
    /// target. The reply is a [`Response::Confident`] whose payload
    /// is a JSON array of [`WireFingerprintInput`].
    FingerprintInputs { target: WireTarget },
    /// Ask the analyser to actually run on this target. The reply
    /// can be any of `confident`, `graded`, `declines`, or `error`.
    Analyse { target: WireTarget },
}

/// Cross-process projection of [`crate::Target`]. The `manifests`
/// field is the bytes-and-content-sha pairs the engine pre-loaded;
/// the analyser is free to ignore them. UTF-8 is not assumed for
/// `manifests[*].bytes` — the wire encoding is base64 for safety.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireTarget {
    /// Absolute path to the candidate directory.
    pub dir: String,
    /// L1-inferred language tags.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Pre-loaded manifests (basename + relpath + bytes-as-base64 +
    /// content sha).
    #[serde(default)]
    pub manifests: Vec<WireTargetFile>,
    /// Top-level file basenames at `dir`.
    #[serde(default)]
    pub top_level_files: Vec<String>,
}

/// Cross-process projection of [`crate::TargetFile`]. Bytes are
/// base64-encoded so the JSON encoding is safe for non-UTF-8
/// payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireTargetFile {
    pub name: String,
    pub relpath: String,
    /// Base64-encoded `bytes` field of the source [`crate::TargetFile`].
    pub bytes_b64: String,
    pub content_sha: String,
}

/// One element of the [`Request::FingerprintInputs`] reply payload.
/// Mirrors [`crate::FingerprintInput`] on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireFingerprintInput {
    /// Sha256 hex of one file's contents.
    FileContentSha { sha: String },
    /// Tagged opaque payload; matches
    /// [`crate::FingerprintInput::Custom`].
    Custom {
        tag: String,
        /// Base64 of the contributed bytes.
        bytes_b64: String,
    },
}

/// Top-level response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// Confident verdict — payload is request-specific JSON.
    /// For `applies`, payload is a bool; for `fingerprint_inputs`,
    /// payload is an array of [`WireFingerprintInput`]; for
    /// `analyse`, payload is an opaque JSON value the parent's
    /// stage adapter decodes into the analyser's stage-specific
    /// output type.
    Confident { payload: serde_json::Value },
    /// Graded verdict — `analyse` only. The parent compares
    /// `confidence` against the configured threshold.
    Graded {
        payload: serde_json::Value,
        confidence: f32,
    },
    /// The analyser does not handle this input. Used for `applies`
    /// = false (encoded as `Confident { payload: false }`) when the
    /// analyser is purely opt-out, but `Declines` is the canonical
    /// reply for `analyse` to signal fallthrough.
    Declines,
    /// Out-of-band failure. The parent maps this onto
    /// [`crate::AnalyzerError::CallFailed`] (or
    /// [`crate::AnalyzerError::MalformedInput`] when the
    /// `error_kind` hint is `"malformed_input"`).
    ///
    /// Note: the field is named `error_kind` (not `kind`) to
    /// avoid colliding with the envelope's serde tag (`kind`),
    /// which discriminates the variant itself.
    Error {
        /// Free-form error message. The parent surfaces this on
        /// the `AnalyzerError` it constructs.
        message: String,
        /// Optional kind hint. `"malformed_input"` is recognised
        /// and routes to [`crate::AnalyzerError::MalformedInput`];
        /// any other value (including `None`) routes to
        /// [`crate::AnalyzerError::CallFailed`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_kind: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let r = Request::Applies {
            target: WireTarget {
                dir: "/x".into(),
                languages: vec!["rust".into()],
                manifests: vec![WireTargetFile {
                    name: "Cargo.toml".into(),
                    relpath: "Cargo.toml".into(),
                    bytes_b64: "W3BhY2thZ2Vd".into(), // [package]
                    content_sha: "abc".into(),
                }],
                top_level_files: vec!["Cargo.toml".into()],
            },
        };
        let bytes = serde_json::to_vec(&r).unwrap();
        let parsed: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn response_kinds_round_trip() {
        let cases: Vec<Response> = vec![
            Response::Confident {
                payload: serde_json::json!({"kind": "rust-library"}),
            },
            Response::Graded {
                payload: serde_json::json!({"x": 1}),
                confidence: 0.75,
            },
            Response::Declines,
            Response::Error {
                message: "boom".into(),
                error_kind: Some("malformed_input".into()),
            },
        ];
        for r in cases {
            let bytes = serde_json::to_vec(&r).unwrap();
            let parsed: Response = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn request_kind_is_snake_case() {
        let r = Request::FingerprintInputs {
            target: WireTarget {
                dir: "/x".into(),
                languages: Vec::new(),
                manifests: Vec::new(),
                top_level_files: Vec::new(),
            },
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            json.contains(r#""kind":"fingerprint_inputs""#),
            "got: {json}"
        );
    }
}
