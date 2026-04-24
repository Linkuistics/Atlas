//! Structural invariant: Atlas's L4 fixedpoint terminates within 8
//! iterations (design §8.2). The harness records the iteration count
//! the tool emits; when the tool doesn't emit one, the invariant is a
//! no-op.

use atlas_eval::fixedpoint_termination;

#[test]
fn fixedpoint_termination_skips_when_iterations_not_reported() {
    assert!(fixedpoint_termination(None, 8).is_ok());
}

#[test]
fn fixedpoint_termination_passes_at_limit() {
    assert!(fixedpoint_termination(Some(8), 8).is_ok());
}

#[test]
fn fixedpoint_termination_fails_above_limit() {
    let err = fixedpoint_termination(Some(9), 8).unwrap_err();
    assert_eq!(err.invariant, "fixedpoint_termination");
    assert!(err.message.contains("9"));
}
