//! Shared types for the four reports: input bundle, error variants, and
//! the impact-target enum.
//!
//! The CLI handler builds a [`ReportInputs`] from an already-computed
//! engine database and workspace, then hands it to one of the four
//! report functions. Reports do not own engine state; they only borrow
//! it for the duration of the call.

use std::io;

use atlas_engine::{AtlasDatabase, Workspace};
use component_ontology::ComponentId;

/// Borrowed bundle of engine outputs handed to each report function.
///
/// In Phase 3 this is constructed by the CLI handler immediately after
/// loading or recomputing the engine database. In Phase 5 the same
/// shape is preserved but `db` becomes a Salsa-tracked database handle.
pub struct ReportInputs<'a> {
    /// Borrowed handle to the engine database. Phase 3 design spec
    /// names this `EngineDb`; the actual engine type is
    /// [`atlas_engine::AtlasDatabase`].
    pub db: &'a AtlasDatabase,
    /// Borrowed workspace (single-root, post-Phase-5-PR-3).
    pub workspace: &'a Workspace,
}

/// Stable string identifier for a contract, matching the
/// `contract_id: String` field on `atlas_index`'s surface records.
///
/// PR-9 may upgrade this to a newtype when impact's traversal needs a
/// richer set of behaviours; the alias matches existing codebase usage
/// today and keeps the design-spec names readable.
pub type ContractId = String;

/// Target of an `atlas impact <id>` query.
///
/// The two namespaces (contract ids and component ids) are disjoint by
/// Phase 1 construction — a single `<id>` argument resolves into one
/// or the other, and the CLI handler does that resolution before
/// constructing the [`ImpactTarget`] passed to [`crate::impact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactTarget {
    /// Walk impact downstream of a single contract id.
    Contract(ContractId),
    /// Walk impact downstream of every contract a component provides.
    Component(ComponentId),
}

/// Errors returned by the four report functions.
///
/// Variants are added as PR-8..PR-11 land the report logic; PR-7 only
/// returns [`ReportError::NotImplemented`].
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// PR-7 stubs: the report logic has not been implemented yet.
    /// PR-8..PR-11 replace each report's stub body and stop returning
    /// this variant.
    #[error("not yet implemented")]
    NotImplemented,

    /// PR-9: the user asked `atlas impact <id>` for an id that does
    /// not exist in either namespace. `candidates` carries the
    /// Levenshtein-1 suggestions the CLI handler renders to stderr.
    #[error("target not found: {needle}")]
    TargetNotFound {
        /// The id the user passed.
        needle: String,
        /// Suggestions produced by the report function (stable order).
        candidates: Vec<String>,
    },

    /// PR-8/PR-10/PR-11/PR-12: an I/O error surfaced from a helper that
    /// reads or writes report-adjacent state. The variant lives in
    /// `atlas-reports` so future helpers can return it without each PR
    /// adding its own error enum; PR-7 itself does not perform I/O.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}
