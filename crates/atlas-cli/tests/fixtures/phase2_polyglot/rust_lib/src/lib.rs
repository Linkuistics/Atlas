//! Rust crate for the PR-14 polyglot acceptance fixture.
//!
//! Verifies the syn-based Rust surface analyser (Phase 2 PR-5) — a
//! `pub mod foo { pub struct Bar; }` produces at least one binding.

pub mod foo {
    /// Public struct nested inside a `pub mod`. PR-5's syn-based
    /// extractor must descend into nested modules.
    pub struct Bar;
}

/// Top-level public function — guarantees at least one binding even if
/// the syn extractor misses the nested `pub struct`.
pub fn entry() -> u32 {
    42
}
