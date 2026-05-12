//! Classifier `Tool` wrappers — weak-tooling tier (PR-3c).
//!
//! Each wrapper delegates to the corresponding in-process
//! `atlas_analyzers` classifier and returns the classification output
//! as JSON.

pub mod compose;
pub mod dockerfile;
pub mod elixir;
pub mod lispkit;
pub mod racket;
