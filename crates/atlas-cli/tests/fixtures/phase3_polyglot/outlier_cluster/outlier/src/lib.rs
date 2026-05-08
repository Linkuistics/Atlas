//! Modularity-outlier component for Phase 3 PR-13.
//!
//! Defines no public serde-derived structs (so `provides = ∅`, keeping
//! the cohesion / surface_complexity metrics tightly bounded). The
//! workspace `components.overrides.yaml` injects six `consumes-contract`
//! edges from this component to peer1..peer6, plus one `depends-on`
//! edge to `rust-lib` — the latter is the divergence-trigger #2
//! build-only edge (no composition between `outlier` and `rust-lib`).
//!
//! With six consumed contracts and zero provided ones, the outlier's
//! `efferent_coupling = 6` while every peer's `efferent_coupling = 0`.
//! In a 7-member subsystem the outlier value lands at
//! `5/sqrt(6) ≈ 2.04σ` from the mean — just enough to flag.

/// Stub function — required so the analyser does not classify this
/// crate as an empty / non-component directory.
pub fn entry() -> u32 {
    0
}
