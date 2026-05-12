//! Fixed-point iteration wrapper around `run_iteration`
//! (recast §4.4, plan §4 Task 5.1, brainstorm §6).
//!
//! PR-5 wraps PR-4's single-iteration spine in a fixed-point loop:
//! the runtime repeatedly drives `run_iteration` until the L9
//! projection content-sha equals the previous iteration's
//! content-sha. Convergence yields the final projection; divergence
//! after `max_iter` iterations is
//! [`crate::runtime::AgentError::FixedpointDiverged`].
//!
//! The iteration boundary is the natural place to emit
//! [`AgentEvent::IterationBoundary`] — `run_fixedpoint` knows both the
//! iteration number and the prior-iteration sha, while
//! [`crate::runtime::AgentRuntime::run_iteration`] does not. PR-5 moves
//! the emission here so each iteration boundary fires exactly once.

use sha2::{Digest, Sha256};

use crate::events::AgentEvent;

use super::{AgentError, AgentRuntime, ContentSha, L9Projection, Workspace};

/// Default max-iterations cap when `AgentRuntime::max_iterations` is
/// not overridden by the caller. Matches brainstorm §2 row 7.
pub const DEFAULT_MAX_ITERATIONS: u32 = 5;

/// Drive `run_iteration` to a fixed point (recast §4.4).
///
/// Convergence rule (idempotent-projection contract):
///
/// - Iteration 1 always runs (no prior projection to compare against).
/// - Iteration K ≥ 2 compares the new projection sha to the prior
///   iteration's sha; equal shas signal convergence and the new
///   projection is returned.
/// - If `max_iter` iterations elapse without convergence, surfaces
///   [`AgentError::FixedpointDiverged`] with the diagnostic
///   `iterations = max_iter`.
///
/// `last_changed_agents` is a PR-5 minimum-viable diagnostic (empty
/// vector). PR-7 enriches it with per-agent transcript-sha diffs
/// across iterations once those structures are tracked. The diagnostic
/// is hand-wavy in the plan (known-unknown #3 of the implementer
/// brief); keeping it empty for PR-5 is the documented choice.
pub async fn run_fixedpoint(
    runtime: &AgentRuntime,
    workspace: &Workspace,
    max_iter: u32,
) -> Result<L9Projection, AgentError> {
    let effective_max = if max_iter == 0 {
        DEFAULT_MAX_ITERATIONS
    } else {
        max_iter
    };
    let mut prior_model_sha: Option<ContentSha> = None;
    let mut prior_projection: Option<L9Projection> = None;
    for iter in 1..=effective_max {
        runtime.event_bus.emit(AgentEvent::IterationBoundary {
            iter,
            prior_model_sha: prior_model_sha.as_ref().map(ContentSha::to_hex),
        });
        let l9 = runtime
            .run_iteration(workspace, iter, prior_model_sha.clone())
            .await?;
        let l9_sha = content_sha(&l9);
        if let Some(prior) = &prior_model_sha {
            if prior == &l9_sha {
                // Converged: the new projection sha equals the prior
                // sha. Return the prior projection (the one whose sha
                // we already verified produces the same output) so the
                // return value is the same object the comparison
                // covered.
                return Ok(prior_projection.unwrap_or(l9));
            }
        }
        prior_model_sha = Some(l9_sha);
        prior_projection = Some(l9);
    }
    // `effective_max == 1` is the "single-iteration mode" sentinel:
    // when the caller asked for exactly one iteration, divergence
    // detection is moot — there's no prior iteration to compare
    // against. Return the single iteration's projection rather than
    // surfacing `FixedpointDiverged`. Tests and the PR-4 smoke fixture
    // exercise this mode.
    if effective_max == 1 {
        if let Some(p) = prior_projection {
            return Ok(p);
        }
    }
    Err(AgentError::FixedpointDiverged {
        iterations: effective_max,
        last_changed_agents: Vec::new(),
    })
}

/// SHA-256 of the canonical-JSON-serialisation of an `L9Projection`.
/// Pure helper; the round-trip is deterministic because
/// `serde_json::to_vec` over an object backed by `BTreeMap` /
/// `HashMap` is *not* generally deterministic (HashMap iteration
/// order is randomised), so we route through a sorted intermediate.
///
/// Design choice: rather than swap `L9Projection.components` /
/// `subsystems` to `BTreeMap` (which touches PR-4 / PR-7 wiring), the
/// fixedpoint loop projects into a `BTreeMap`-backed `serde_json::Value`
/// before hashing. The `L9Projection` shape is preserved.
pub fn content_sha(projection: &L9Projection) -> ContentSha {
    let canonical = to_canonical_json(projection);
    let bytes = canonical.as_bytes();
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest.as_slice() {
        hex.push_str(&format!("{byte:02x}"));
    }
    ContentSha(hex)
}

/// Render `projection` as a canonical JSON string for content-hashing.
/// Object keys are sorted lexicographically (`BTreeMap` ordering); the
/// inner `AgentOutput.value` field is serialised via `serde_json`'s
/// default, which already sorts object keys in the canonical shape.
fn to_canonical_json(projection: &L9Projection) -> String {
    use serde_json::Value;
    let mut components: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut comp_ids: Vec<&String> = projection.components.keys().collect();
    comp_ids.sort();
    for id in comp_ids {
        components.insert(id.clone(), projection.components[id].value.clone());
    }
    let mut subsystems: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut subs_ids: Vec<&String> = projection.subsystems.keys().collect();
    subs_ids.sort();
    for id in subs_ids {
        subsystems.insert(id.clone(), projection.subsystems[id].value.clone());
    }
    let project = projection
        .project
        .as_ref()
        .map(|p| p.value.clone())
        .unwrap_or(Value::Null);
    let canonical = serde_json::json!({
        "components": Value::Object(components),
        "subsystems": Value::Object(subsystems),
        "project": project,
    });
    serde_json::to_string(&canonical).expect("canonical JSON serialisation must succeed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::AgentOutput;
    use serde_json::json;
    use std::collections::HashMap;

    fn projection_with(components: &[(&str, serde_json::Value)]) -> L9Projection {
        let mut m: HashMap<String, AgentOutput> = HashMap::new();
        for (id, val) in components {
            m.insert(id.to_string(), AgentOutput::from_value(val.clone()));
        }
        L9Projection {
            components: m,
            subsystems: HashMap::new(),
            project: None,
        }
    }

    #[test]
    fn content_sha_is_deterministic_across_runs() {
        let p = projection_with(&[("foo", json!({"kind": "rust"}))]);
        let a = content_sha(&p);
        let b = content_sha(&p);
        assert_eq!(a, b);
    }

    #[test]
    fn content_sha_is_order_invariant_via_canonical_json() {
        // The HashMap may iterate in any order; the canonical-JSON
        // projection sorts keys. Build two projections with the same
        // contents but with keys inserted in different orders and
        // verify the sha is stable.
        let p1 = projection_with(&[("foo", json!({"k": 1})), ("bar", json!({"k": 2}))]);
        let p2 = projection_with(&[("bar", json!({"k": 2})), ("foo", json!({"k": 1}))]);
        assert_eq!(content_sha(&p1), content_sha(&p2));
    }

    #[test]
    fn content_sha_differs_on_distinct_payloads() {
        let p1 = projection_with(&[("foo", json!({"k": 1}))]);
        let p2 = projection_with(&[("foo", json!({"k": 2}))]);
        assert_ne!(content_sha(&p1), content_sha(&p2));
    }

    #[test]
    fn default_max_iterations_is_five() {
        // Pin the brainstorm §2 row 7 default. A future tunable swap
        // should advance through review, not silently.
        assert_eq!(DEFAULT_MAX_ITERATIONS, 5);
    }
}
