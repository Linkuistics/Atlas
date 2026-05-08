//! Pure-function projections of the Atlas engine's L4–L8 outputs into the
//! four Phase 3 reports: **drift**, **impact**, **modularity**, and
//! **composition divergence**.
//!
//! This crate is intentionally I/O-free. Inputs arrive as borrows of an
//! already-computed [`atlas_engine::AtlasDatabase`] plus the resolved
//! [`atlas_engine::Workspace`], and outputs are returned as in-memory
//! report values for the CLI handler to render or persist. Reports never
//! trigger engine recomputation; they observe whatever the engine has
//! already produced (see Phase 3 design spec §3.1, §3.3).
//!
//! # Phase 5 conversion path
//!
//! Phase 5 turns Atlas into a long-running server backed by Salsa
//! incremental compilation. Each `pub fn` exposed here ([`drift`],
//! [`impact`], [`modularity`], [`divergence`]) is shaped today as a pure
//! function over its inputs precisely so the migration is mechanical:
//!
//! - The function body becomes the body of a `#[salsa::tracked]` query
//!   method on a `ReportsDatabase` extension trait.
//! - [`snapshot::ContractShaSnapshot`] (consumed by [`drift`]) becomes a
//!   Salsa input updated by the file-watcher when the on-disk snapshot
//!   changes.
//! - [`modularity::ModularityHistory`] entries (one per component)
//!   become a Salsa input keyed by [`component_ontology::ComponentId`].
//! - All file-write side-effects stay outside this crate — they live in
//!   the CLI handlers in Phase 3 and migrate to the server's
//!   settled-state writer in Phase 5.
//!
//! Because the Phase 3 functions are already side-effect-free over their
//! inputs, the conversion is mechanical: the public API shape stays
//! identical, callers (CLI today, server tomorrow) don't need to change,
//! and Salsa's invalidation tracking simply replaces the recompute-from-
//! scratch model the CLI uses today. See Phase 3 design spec §3.5 for
//! the canonical write-up.

pub mod divergence;
pub mod drift;
pub mod impact;
pub mod modularity;
pub mod snapshot;
pub mod types;

pub use divergence::{
    divergence, DivergenceCoupling, DivergencePair, DivergenceReport, DivergenceSummary,
};
pub use drift::{
    drift, ContractAdded, ContractChange, ContractRemoved, DriftReport, DriftSummary, PinnedBinding,
};
pub use impact::{
    impact, ImpactNode, ImpactNodeKind, ImpactPartitions, ImpactReport, ImpactSummary,
    ImpactTargetView,
};
pub use modularity::{
    modularity, ComponentModularity, ModularityHistory, ModularityHistoryEntry,
    ModularityHistoryMetrics, ModularityMetrics, ModularityReport, ModularityRollup,
    SubsystemAggregate, SubsystemAggregateMetrics, SubsystemMetricStats, SubsystemOutlier,
    UnattachedComponents,
};
pub use snapshot::{ContractShaEntry, ContractShaSnapshot};
pub use types::{ContractId, ImpactTarget, ReportError, ReportInputs};
