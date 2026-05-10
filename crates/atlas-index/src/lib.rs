//! Schema and reader for the Atlas YAMLs
//! (`components.yaml`, `components.overrides.yaml`,
//! `external-components.yaml`, `subsystems.yaml`,
//! `subsystems.overrides.yaml`, `related-components.yaml`,
//! plus the Atlas vNext additions `surfaces.yaml`, `analyzers.yaml`,
//! `config.yaml`, and `<component-path>/.atlas/component.yaml`);
//! rename-matching for identifier stability across runs.
//!
//! `related-components.yaml` lives in the `component-ontology` crate
//! and is re-exported here so consumers need only one dependency.
//!
//! The crate deliberately depends on nothing heavy — no Salsa, no
//! async runtime, no LLM. A host tool (Ravel-Lite, any future
//! consumer) can pull in just this crate and read/write the
//! files without transitive weight.

pub mod analyzers;
pub mod config;
pub mod per_component;
pub mod rename_match;
pub mod schema;
pub mod surfaces;
pub mod yaml_io;

pub use analyzers::{
    AnalyzerSpec, AnalyzerSpecValidationError, AnalyzersFile, ApplicabilityPredicate, Confidence,
    CostClass, Stage, SubprocessConfig, Transport, ANALYZERS_SCHEMA_VERSION,
};
pub use config::{AtlasConfigFile, ModelRouting, CONFIG_SCHEMA_VERSION};
pub use per_component::{PerComponentFile, PER_COMPONENT_SCHEMA_VERSION};
pub use rename_match::{
    rename_match, RenameMatchInput, RenameMatchOutput, DEFAULT_RENAME_MATCH_THRESHOLD,
};
pub use schema::{
    AlwaysTrue, CacheFingerprints, ComponentEntry, ComponentFieldOverrides, ComponentsFile,
    DocAnchor, EdgeAdd, EdgeSuppress, ExternalEntry, ExternalsFile, MemberEvidence, OverridesFile,
    PathSegment, PinValue, SubsystemEntry, SubsystemOverride, SubsystemsFile,
    SubsystemsOverridesFile, COMPONENTS_SCHEMA_VERSION, EXTERNALS_SCHEMA_VERSION,
    OVERRIDES_SCHEMA_VERSION, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION, SUBSYSTEMS_SCHEMA_VERSION,
};
pub use surfaces::{
    Binding, BindingRole, ConsumedContract, Contract, ContractKind, ImplementedContract,
    LibraryApi, LibraryApiValidationError, PubItem, PubItemKind, SurfacesFile, Visibility,
    ATTR_DECORATOR_CHAIN, ATTR_DOC, ATTR_MODULE_SYSTEM, ATTR_PRIVATE, ATTR_SPEC, ATTR_TYPE_ONLY,
    SURFACES_SCHEMA_VERSION,
};
pub use yaml_io::{
    load_components, load_externals, load_or_default_components, load_or_default_externals,
    load_or_default_overrides, load_or_default_subsystems, load_or_default_subsystems_overrides,
    load_overrides, load_subsystems, load_subsystems_overrides, save_components_atomic,
    save_externals_atomic, save_overrides_atomic, save_subsystems_atomic,
    save_subsystems_overrides_atomic,
};

// The related-components.yaml schema lives in `component-ontology` so
// the vocabulary of edge kinds / lifecycles / evidence grades has one
// owner. Re-export the surface that consumers need.
pub use component_ontology::{
    load as load_related_components, load_or_default as load_or_default_related_components,
    save_atomic as save_related_components_atomic, ComponentId, ComponentIdError, Edge, EdgeKind,
    EvidenceGrade, LifecycleScope, RelatedComponentsFile,
    SCHEMA_VERSION as RELATED_COMPONENTS_SCHEMA_VERSION,
};
