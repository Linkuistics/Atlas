//! Types for the six YAMLs that sit at the Atlas/Ravel-Lite boundary:
//! `components.yaml`, `components.overrides.yaml`,
//! `external-components.yaml`, `subsystems.yaml`,
//! `subsystems.overrides.yaml`, and `related-components.yaml`.
//!
//! All but the last are owned by this crate. `related-components.yaml`
//! is owned by `component-ontology` and re-exported here so consumers
//! only need one crate. Each generated file carries its own
//! `schema_version` — independent versions let us evolve one file
//! without forcing a reader to relearn all six at once.
//!
//! `kind`, `role`, `language`, and `build_system` are kept as `String`
//! at this layer. The typed `ComponentKind` enum lands in `atlas-engine`
//! (see backlog task 5); anchoring the vocabulary to a not-yet-written
//! enum here would churn every downstream consumer every time the
//! vocabulary grows.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use component_ontology::{ComponentId, EvidenceGrade, LifecycleScope};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const COMPONENTS_SCHEMA_VERSION: u32 = 1;
pub const OVERRIDES_SCHEMA_VERSION: u32 = 1;
pub const EXTERNALS_SCHEMA_VERSION: u32 = 1;
pub const SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION: u32 = 1;
pub const SUBSYSTEMS_SCHEMA_VERSION: u32 = 1;

/// Fingerprint set written into `components.yaml` so a second tool run
/// can detect whether any cache-invalidating input changed. `prompt_shas`
/// is keyed by prompt id (e.g., `"classify"`, `"stage1-surface"`); all
/// SHAs are stored as lowercase hex strings to keep the YAML diffable.
///
/// `analyzer_registry_sha` is the canonical sha256 of the merged
/// [`crate::analyzers::AnalyzersFile`] (built-in defaults plus any
/// per-workspace overrides). It is computed as
/// `sha256(serde_yaml::to_string(&analyzers_file).as_bytes())` rendered
/// as 64-character lowercase hex, and contributes to L3+ stage cache
/// keys per design §8.1 — a registry shape change invalidates every
/// downstream cache entry automatically. The field is
/// `#[serde(default)]` so prior on-disk YAML written before PR-5 still
/// parses (the legacy value is the empty string, which differs from the
/// hash of any non-empty registry and therefore correctly forces a
/// recompute on first run after upgrade).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheFingerprints {
    pub ontology_sha: String,
    #[serde(default)]
    pub prompt_shas: BTreeMap<String, String>,
    pub model_id: String,
    pub backend_version: String,
    /// Sha256 hex of the canonical YAML rendering of the merged
    /// [`crate::analyzers::AnalyzersFile`]. Empty in records written
    /// before PR-5; non-empty going forward. See the type-level
    /// docstring for the exact computation.
    #[serde(default)]
    pub analyzer_registry_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathSegment {
    /// Relative to one of `ComponentsFile::roots`. The component's
    /// id namespace identifies which root.
    pub path: PathBuf,
    /// Hex-encoded SHA256 of the directory tree at `path`. Rename-match
    /// compares prior vs. new segments by this value (see
    /// `rename_match.rs`).
    pub content_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocAnchor {
    /// Relative to one of `ComponentsFile::roots`.
    pub path: PathBuf,
    /// ATX heading text (no leading `#` marks).
    pub heading: String,
}

/// One entry in `components.yaml` (and the projected
/// `<component-path>/.atlas/component.yaml`).
///
/// Atlas vNext makes two structural breaks against v1:
///
/// 1. `language: Option<String>` becomes `languages: BTreeSet<String>` —
///    a polyglot component carries every language in one record
///    (design §3.1).
/// 2. The `kind` vocabulary expands beyond source-code kinds with
///    deliverable kinds (`docker-image`, `published-crate`,
///    `helm-release`, `k8s-deployment`, `homebrew-bottle`,
///    `orchestration-script`, `ci-pipeline`) and per-language source
///    kinds (`typescript-package`, `javascript-package`,
///    `python-package`, `elixir-project`, etc.). The wire form remains
///    `String` at this layer; the typed `ComponentKind` enum in
///    atlas-engine carries the closed vocabulary.
///
/// Phase 2 PR-3 contributes `python-package` to the open vocabulary
/// (consumed by the `atlas-analyzers::python_classifier` deterministic
/// L3 rule).
///
/// Phase 2 PR-8 contributes `elixir-project` to the open vocabulary
/// (consumed by the `atlas-analyzers::elixir_classifier` deterministic
/// L3 rule, keyed on `mix.exs` presence).
///
/// Phase 2 PR-6 contributes `csharp-project` and `csharp-solution` to
/// the open vocabulary (consumed by the `atlas-analyzers::csharp_classifier`
/// deterministic L3 rule, keyed on `*.csproj` and `*.sln` presence —
/// `*.sln` takes precedence when both appear at the same path).
///
/// Phase 2 PR-7 contributes `dart-package` and `flutter-package` to the
/// open vocabulary (consumed by the `atlas-analyzers::dart_classifier`
/// deterministic L3 rule, keyed on `pubspec.yaml` presence — the
/// `flutter:` top-level block discriminates `flutter-package` from the
/// plain `dart-package`).
///
/// Phase 2 PR-9 contributes `racket-package` to the open vocabulary
/// (consumed by the `atlas-analyzers::racket_classifier` deterministic
/// L3 rule, keyed on `info.rkt` presence; surface extraction at L5 by
/// the `racket-surface-analyzer` subprocess).
///
/// Phase 2 PR-10 contributes `lispkit-package` to the open vocabulary
/// (consumed by the `atlas-analyzers::lispkit_classifier` deterministic
/// L3 rule, keyed on `*.sld` R7RS Scheme Library Definition file
/// presence; surface extraction at L5 by the `lispkit-surface-analyzer`
/// subprocess).
///
/// Phase 2 PR-12 contributes `shell-script` and `makefile-orchestration`
/// to the open vocabulary (consumed by the
/// `atlas-analyzers::shell_script_llm_analyzer` in-process LLM-fallback
/// rule; keys on `*.sh`/`*.bash`/`*.zsh`/`Makefile`/`GNUmakefile`/`*.mk`
/// presence and dispatches an LLM-classify call below the
/// `Confidence::Declines` threshold of 0.6).
///
/// Phase 2 PR-11 contributes `compose-orchestration` to the open
/// vocabulary (consumed by the `atlas-analyzers::compose_classifier`
/// deterministic L3 rule, keyed on canonical Docker Compose filename
/// patterns: `docker-compose.yml` / `docker-compose.yaml` /
/// `docker-compose.*.yml` / `docker-compose.*.yaml` /
/// `compose.yml` / `compose.yaml` / `compose.*.yml` /
/// `compose.*.yaml`). At L6 the analyser emits `bundled-into` edges
/// (image/build → orchestration) and `deployed-with` edges (between
/// co-declared service sources).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentEntry {
    pub id: ComponentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ComponentId>,
    pub kind: String,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    /// Set of programming languages this component contains. Empty
    /// for deliverables whose source is purely declarative (e.g.
    /// `kind: docker-image`). `BTreeSet` to keep YAML output
    /// deterministic.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub languages: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub path_segments: Vec<PathSegment>,
    #[serde(default)]
    pub manifests: Vec<PathBuf>,
    #[serde(default)]
    pub doc_anchors: Vec<DocAnchor>,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub deleted: bool,
}

/// Top-level `components.yaml` envelope.
///
/// Atlas vNext replaces v1's singular `root: PathBuf` with a plural
/// `roots: Vec<PathBuf>` so multi-root workspaces (path-dep-walked
/// peer roots, design §5.3) record every analysed root in the same
/// record. The first entry is conventionally the primary root —
/// the directory `atlas index` was invoked from — but the schema
/// does not enforce that; consumers that need the primary root read
/// it from `config.yaml` (see `config::AtlasConfigFile`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentsFile {
    pub schema_version: u32,
    /// Discovered roots, primary first. Atlas vNext drops the v1
    /// `root: PathBuf` field; consumers expecting it must read this
    /// list instead.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    pub generated_at: String,
    pub cache_fingerprints: CacheFingerprints,
    #[serde(default)]
    pub components: Vec<ComponentEntry>,
}

impl Default for ComponentsFile {
    fn default() -> Self {
        ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            roots: Vec::new(),
            generated_at: String::new(),
            cache_fingerprints: CacheFingerprints::default(),
            components: Vec::new(),
        }
    }
}

/// Pin value for `components.overrides.yaml`.
///
/// Carries hand-written guidance for one slot in a component's pin map.
/// `Value` overrides a classification field (kind, role, language, …);
/// `Suppress` removes the component entirely; `SuppressChildren` prunes
/// specific children of the component.
///
/// On the wire, each variant accepts a natural form (a map for `Value`,
/// a bool for `Suppress`, a sequence for `SuppressChildren`) plus, for
/// `Suppress` and `SuppressChildren` only, a legacy doubly-nested map
/// form left over from when the enum used `#[serde(untagged)]` field-name
/// dispatch. Output (`Serialize`) is always the natural form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinValue {
    Suppress {
        suppress: AlwaysTrue,
    },
    SuppressChildren {
        suppress_children: Vec<ComponentId>,
    },
    Value {
        value: String,
        reason: Option<String>,
    },
}

/// Marker that serialises as the literal `true` and rejects anything
/// else on deserialisation. Required because the `Suppress` variant
/// above must accept only `suppress: true` — silently accepting
/// `suppress: false` would let users disable a pin by typo.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlwaysTrue;

impl Serialize for AlwaysTrue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        true.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AlwaysTrue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = bool::deserialize(deserializer)?;
        if !v {
            return Err(serde::de::Error::custom(
                "`suppress: false` is not meaningful; remove the pin instead",
            ));
        }
        Ok(AlwaysTrue)
    }
}

impl Serialize for PinValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PinValue::Suppress { .. } => serializer.serialize_bool(true),
            PinValue::SuppressChildren { suppress_children } => {
                suppress_children.serialize(serializer)
            }
            PinValue::Value { value, reason } => {
                let len = if reason.is_some() { 2 } else { 1 };
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("value", value)?;
                if let Some(r) = reason {
                    map.serialize_entry("reason", r)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for PinValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(PinValueVisitor)
    }
}

struct PinValueVisitor;

impl<'de> Visitor<'de> for PinValueVisitor {
    type Value = PinValue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "a pin value: `true` to suppress the component, a sequence of child ids \
             to suppress specific children, or a `{value, reason?}` map to override a field",
        )
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
        if !v {
            return Err(E::custom(
                "`suppress: false` is not meaningful; remove the pin instead",
            ));
        }
        Ok(PinValue::Suppress {
            suppress: AlwaysTrue,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut items: Vec<ComponentId> = Vec::new();
        while let Some(item) = seq.next_element::<ComponentId>()? {
            items.push(item);
        }
        Ok(PinValue::SuppressChildren {
            suppress_children: items,
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let first_key: Option<String> = map.next_key()?;
        match first_key.as_deref() {
            Some("value") => {
                let value: String = map.next_value()?;
                let mut reason: Option<String> = None;
                while let Some(k) = map.next_key::<String>()? {
                    match k.as_str() {
                        "reason" => reason = Some(map.next_value()?),
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &["value", "reason"],
                            ));
                        }
                    }
                }
                Ok(PinValue::Value { value, reason })
            }
            Some("reason") => {
                // Field order is not guaranteed when YAML is round-tripped through
                // serde data models; accept {reason, value} too.
                let reason: String = map.next_value()?;
                let mut value: Option<String> = None;
                while let Some(k) = map.next_key::<String>()? {
                    match k.as_str() {
                        "value" => value = Some(map.next_value()?),
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &["value", "reason"],
                            ));
                        }
                    }
                }
                let value = value.ok_or_else(|| serde::de::Error::missing_field("value"))?;
                Ok(PinValue::Value {
                    value,
                    reason: Some(reason),
                })
            }
            // Legacy doubly-nested form: `suppress: { suppress: true }` was the only
            // accepted shape before this enum gained custom (de)serialise impls. Still
            // accepted on input because pre-existing override files in the wild may use
            // it; output always emits the natural single-nested form.
            Some("suppress") => {
                let v: bool = map.next_value()?;
                if !v {
                    return Err(serde::de::Error::custom(
                        "`suppress: false` is not meaningful; remove the pin instead",
                    ));
                }
                if map.next_key::<String>()?.is_some() {
                    return Err(serde::de::Error::custom(
                        "`suppress` pin accepts only a single `suppress: true` field",
                    ));
                }
                Ok(PinValue::Suppress {
                    suppress: AlwaysTrue,
                })
            }
            Some("suppress_children") => {
                let children: Vec<ComponentId> = map.next_value()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(serde::de::Error::custom(
                        "`suppress_children` pin accepts only a single `suppress_children: [...]` field",
                    ));
                }
                Ok(PinValue::SuppressChildren {
                    suppress_children: children,
                })
            }
            None => Err(serde::de::Error::custom("empty pin value")),
            Some(other) => Err(serde::de::Error::custom(format!(
                "unknown pin value field `{other}`; expected `value` (with optional `reason`), \
                 a bool, or a sequence of child ids"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverridesFile {
    pub schema_version: u32,
    /// Outer key: component id. Inner key: field name being pinned
    /// (e.g., `"role"`, `"kind"`, `"deleted"`). `BTreeMap` keeps the
    /// serialised output deterministic.
    #[serde(default)]
    pub pins: BTreeMap<ComponentId, BTreeMap<String, PinValue>>,
    /// Components authored by hand when no evidence exists for them
    /// (e.g., a spec directory with no manifest). They bypass L2/L3 and
    /// land in `components.yaml` directly.
    #[serde(default)]
    pub additions: Vec<ComponentEntry>,
    /// Hand-authored edges to add to the analyser-discovered set
    /// (Phase 3 PR-6 / design §5.5). Top-level `overrides.yaml` only;
    /// per-component files may also declare these and the engine
    /// unions across all discovered files. Empty vectors are omitted
    /// from the on-disk YAML so workspaces that never use the
    /// mechanism keep byte-stable overrides files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges_add: Vec<EdgeAdd>,
    /// Hand-authored edges to subtract from the analyser-discovered
    /// set (Phase 3 PR-6 / design §5.5). Match is by `(kind, from, to)`
    /// triple. An entry that matches no analyser-discovered edge logs
    /// a warning and is otherwise a no-op.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges_suppress: Vec<EdgeSuppress>,
    /// Per-component field overrides (Phase 3 PR-6 / design §5.5).
    /// Carried under the YAML key `overrides:` to mirror the design
    /// example. Meaningful only on per-component override files; a
    /// top-level file may serialise this block but the engine ignores
    /// it (the targeting is implicit on the file's directory).
    #[serde(
        rename = "overrides",
        default,
        skip_serializing_if = "ComponentFieldOverrides::is_empty"
    )]
    pub field_overrides: ComponentFieldOverrides,
}

impl Default for OverridesFile {
    fn default() -> Self {
        OverridesFile {
            schema_version: OVERRIDES_SCHEMA_VERSION,
            pins: BTreeMap::new(),
            additions: Vec::new(),
            edges_add: Vec::new(),
            edges_suppress: Vec::new(),
            field_overrides: ComponentFieldOverrides::default(),
        }
    }
}

/// Hand-authored edge to add to the analyser-discovered set.
///
/// `reason` is **required** by the design spec — Atlas refuses to
/// load an `edges_add` entry that omits it. The field is intentionally
/// not `#[serde(default)]` so a missing `reason` produces a clear
/// deserialise error rather than a silent empty string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeAdd {
    /// Edge kind name from the open ontology (e.g. `bundled-into`,
    /// `depends-on`). Validated by the engine when the entry is merged
    /// into the canonical edge set.
    pub kind: String,
    /// Source participant — a component id or external id.
    pub from: String,
    /// Target participant — a component id or external id.
    pub to: String,
    /// Why this edge was added by hand. Required so the audit trail
    /// captures the rationale alongside the analyser-discovered
    /// edges.
    pub reason: String,
}

/// Hand-authored edge to subtract from the analyser-discovered set.
///
/// Identical shape to `EdgeAdd` but carries opposite semantics: a
/// match against the analyser-discovered set drops the edge. A
/// `reason` is required for the same audit-trail rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeSuppress {
    /// Edge kind name. Match is exact on the `(kind, from, to)`
    /// triple — a suppress entry that names a different `kind` than
    /// the analyser used does not match.
    pub kind: String,
    /// Source participant.
    pub from: String,
    /// Target participant.
    pub to: String,
    /// Why this edge is suppressed. Required so a future reader knows
    /// whether the analyser is wrong, the import is dev-only, or some
    /// other rationale.
    pub reason: String,
}

/// Per-component field overrides (design §5.5). Each field, when
/// `Some`, supersedes the analyser-emitted value for the
/// corresponding field on the owning component's descriptor.
///
/// All four fields are optional — the override block may set any
/// subset. An empty block (all `None`) round-trips through serde as
/// the absence of the `overrides:` key on disk (see
/// `is_empty` + the parent's `skip_serializing_if`).
///
/// `subsystem` is captured here for forward compatibility — the
/// current `ComponentEntry` has no `subsystem` field, so a value
/// supplied here is recorded but does not flow into the projected
/// per-component cache blob until a future PR adds the destination
/// field. The other three (`language`, `kind`, `lifecycle`) flow
/// through L4 directly onto the owning component's `ComponentEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentFieldOverrides {
    /// Override for the analyser-emitted language. Single-valued —
    /// the per-component override expresses one canonical language;
    /// the engine widens it into the `BTreeSet<String>` shape
    /// `ComponentEntry.languages` carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Override for the analyser-emitted `kind` (`rust-library`,
    /// `docker-image`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Override for the analyser-emitted lifecycle. Mapped onto
    /// `ComponentEntry.lifecycle_roles` (a Vec) by L4 with the single
    /// authored scope replacing the analyser-derived set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// Subsystem tag. Carried in the schema for forward compatibility;
    /// no current destination field on `ComponentEntry`. See the
    /// type-level docstring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
}

impl ComponentFieldOverrides {
    /// `true` when every field is `None`. Used by the parent's
    /// `skip_serializing_if` so an empty block omits the
    /// `overrides:` key entirely from on-disk YAML.
    pub fn is_empty(&self) -> bool {
        self.language.is_none()
            && self.kind.is_none()
            && self.lifecycle.is_none()
            && self.subsystem.is_none()
    }
}

/// Hand-authored subsystem boundary. Lives in `subsystems.overrides.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemOverride {
    pub id: String,
    /// Mixed glob and id forms. A `members` entry containing `/` or `*`
    /// is matched against component path segments; otherwise it is
    /// matched against component id directly. See the design spec for
    /// the resolution algorithm.
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemsOverridesFile {
    pub schema_version: u32,
    #[serde(default)]
    pub subsystems: Vec<SubsystemOverride>,
}

impl Default for SubsystemsOverridesFile {
    fn default() -> Self {
        SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEvidence {
    /// The source member entry's role here is dual: when `matched_via`
    /// indicates a successful resolution (`"id"` or a glob string),
    /// `id` carries the resolved component id. When `matched_via`
    /// indicates a resolution failure (`"<glob> (no matches)"`,
    /// `"invalid glob"`, `"no such component"`, etc.), `id` carries
    /// the raw source member string verbatim. Kept as `String` so a
    /// failed resolution can still be recorded faithfully — this field
    /// is an audit log entry, not a join key.
    pub id: String,
    /// Glob string when the member resolved via a glob, the literal
    /// `"id"` when the member entry was an id form, or
    /// `"<glob> (no matches)"` when a glob produced zero matches.
    pub matched_via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    /// Resolved component ids, sorted and deduped.
    #[serde(default)]
    pub members: Vec<ComponentId>,
    /// Audit trail: how each resolved member was matched.
    #[serde(default)]
    pub member_evidence: Vec<MemberEvidence>,
    /// Soft warnings about this subsystem (e.g. `"all members unresolved"`).
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemsFile {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub subsystems: Vec<SubsystemEntry>,
}

impl Default for SubsystemsFile {
    fn default() -> Self {
        SubsystemsFile {
            schema_version: SUBSYSTEMS_SCHEMA_VERSION,
            generated_at: String::new(),
            subsystems: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEntry {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Package URL (pURL) spec identifier, when a manifest supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Manifest paths (relative to the repo root) that surfaced this
    /// external. Multiple entries are expected — the same crate can
    /// show up in several `Cargo.toml`s.
    #[serde(default)]
    pub discovered_from: Vec<String>,
    pub evidence_grade: EvidenceGrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub externals: Vec<ExternalEntry>,
}

impl Default for ExternalsFile {
    fn default() -> Self {
        ExternalsFile {
            schema_version: EXTERNALS_SCHEMA_VERSION,
            externals: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_value_value_variant_round_trips() {
        let pin = PinValue::Value {
            value: "library".into(),
            reason: Some("override the classifier".into()),
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_value_variant_omits_none_reason_in_yaml() {
        let pin = PinValue::Value {
            value: "library".into(),
            reason: None,
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        assert!(
            !yaml.contains("reason"),
            "reason: null should be skipped, got:\n{yaml}"
        );
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_serialises_as_bare_true() {
        let pin = PinValue::Suppress {
            suppress: AlwaysTrue,
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        assert_eq!(yaml.trim(), "true", "got:\n{yaml}");
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_accepts_legacy_doubly_nested_form() {
        let pin: PinValue = serde_yaml::from_str("suppress: true").unwrap();
        assert_eq!(
            pin,
            PinValue::Suppress {
                suppress: AlwaysTrue
            }
        );
    }

    #[test]
    fn pin_value_suppress_children_serialises_as_bare_sequence() {
        let pin = PinValue::SuppressChildren {
            suppress_children: vec![
                ComponentId::parse("a").unwrap(),
                ComponentId::parse("b").unwrap(),
            ],
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        // Natural form: a YAML sequence at the root, no `suppress_children:` wrapper.
        assert!(!yaml.contains("suppress_children"), "got:\n{yaml}");
        assert!(yaml.contains("- a"), "got:\n{yaml}");
        assert!(yaml.contains("- b"), "got:\n{yaml}");
        let parsed: PinValue = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, pin);
    }

    #[test]
    fn pin_value_suppress_children_accepts_natural_inline_sequence() {
        // Reported user form: `suppress_children: [a, b]` written as the inner-map
        // value. The PinValue deserialiser sees only the value side (`[a, b]`).
        let pin: PinValue = serde_yaml::from_str("[a, b]").unwrap();
        assert_eq!(
            pin,
            PinValue::SuppressChildren {
                suppress_children: vec![
                    ComponentId::parse("a").unwrap(),
                    ComponentId::parse("b").unwrap(),
                ]
            }
        );
    }

    #[test]
    fn pin_value_suppress_children_accepts_legacy_doubly_nested_form() {
        let pin: PinValue = serde_yaml::from_str("suppress_children:\n- a\n- b\n").unwrap();
        assert_eq!(
            pin,
            PinValue::SuppressChildren {
                suppress_children: vec![
                    ComponentId::parse("a").unwrap(),
                    ComponentId::parse("b").unwrap(),
                ]
            }
        );
    }

    #[test]
    fn pin_value_suppress_rejects_bare_false() {
        let err = serde_yaml::from_str::<PinValue>("false").unwrap_err();
        assert!(
            err.to_string().contains("not meaningful"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pin_value_suppress_rejects_legacy_false() {
        let err = serde_yaml::from_str::<PinValue>("suppress: false").unwrap_err();
        assert!(
            err.to_string().contains("not meaningful"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pin_value_value_variant_with_only_value_serialises_compactly() {
        let pin = PinValue::Value {
            value: "rust-library".into(),
            reason: None,
        };
        let yaml = serde_yaml::to_string(&pin).unwrap();
        assert!(yaml.contains("value: rust-library"), "got:\n{yaml}");
        assert!(!yaml.contains("reason"), "got:\n{yaml}");
    }

    #[test]
    fn pin_value_value_variant_unknown_field_is_rejected() {
        // A typo like `valeu:` lands us in the "unknown field" branch rather
        // than silently degrading to a different variant.
        let err = serde_yaml::from_str::<PinValue>("valeu: rust-library").unwrap_err();
        assert!(
            err.to_string().contains("unknown pin value field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn components_file_default_has_current_schema_version() {
        let f = ComponentsFile::default();
        assert_eq!(f.schema_version, COMPONENTS_SCHEMA_VERSION);
        assert!(f.components.is_empty());
    }

    #[test]
    fn overrides_file_default_has_current_schema_version() {
        let f = OverridesFile::default();
        assert_eq!(f.schema_version, OVERRIDES_SCHEMA_VERSION);
    }

    #[test]
    fn externals_file_default_has_current_schema_version() {
        let f = ExternalsFile::default();
        assert_eq!(f.schema_version, EXTERNALS_SCHEMA_VERSION);
    }

    #[test]
    fn component_entry_round_trips_through_yaml() {
        let entry = ComponentEntry {
            id: ComponentId::parse("workspace/crate-a").unwrap(),
            parent: Some(ComponentId::parse("workspace").unwrap()),
            kind: "rust-library".into(),
            lifecycle_roles: vec![LifecycleScope::Build, LifecycleScope::Runtime],
            languages: BTreeSet::from(["rust".to_string()]),
            build_system: Some("cargo".into()),
            role: Some("library".into()),
            path_segments: vec![PathSegment {
                path: PathBuf::from("crates/crate-a"),
                content_sha: "abc123".into(),
            }],
            manifests: vec![PathBuf::from("crates/crate-a/Cargo.toml")],
            doc_anchors: vec![DocAnchor {
                path: PathBuf::from("crates/crate-a/README.md"),
                heading: "Crate A".into(),
            }],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Cargo.toml:[lib]".into()],
            rationale: "has Cargo.toml [lib] section".into(),
            deleted: false,
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: ComponentEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn component_entry_polyglot_languages_round_trip() {
        let entry = ComponentEntry {
            id: ComponentId::parse("workspace/billing-service").unwrap(),
            parent: None,
            kind: "rust-binary".into(),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            languages: BTreeSet::from(["rust".to_string(), "c".to_string(), "python".to_string()]),
            build_system: Some("cargo".into()),
            role: None,
            path_segments: vec![],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["Cargo.toml".into()],
            rationale: "polyglot service".into(),
            deleted: false,
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: ComponentEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.languages.len(), 3);
    }

    #[test]
    fn component_entry_deliverable_kind_serialises_as_kebab_case_string() {
        // The `kind` field is a string at this layer (the typed
        // `ComponentKind` enum lives in atlas-engine). Confirm a
        // deliverable kind round-trips so consumers can write
        // `docker-image`, `published-crate`, etc., without a schema
        // bump.
        for kind in [
            "docker-image",
            "published-crate",
            "helm-release",
            "k8s-deployment",
            "homebrew-bottle",
            "orchestration-script",
            "ci-pipeline",
        ] {
            let entry = ComponentEntry {
                id: ComponentId::parse("workspace/deliverable").unwrap(),
                parent: None,
                kind: kind.into(),
                lifecycle_roles: vec![LifecycleScope::Deploy],
                languages: BTreeSet::new(),
                build_system: None,
                role: None,
                path_segments: vec![],
                manifests: vec![],
                doc_anchors: vec![],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec!["Dockerfile".into()],
                rationale: "deliverable".into(),
                deleted: false,
            };
            let yaml = serde_yaml::to_string(&entry).unwrap();
            assert!(yaml.contains(&format!("kind: {kind}")), "got:\n{yaml}");
            let parsed: ComponentEntry = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, entry);
        }
    }

    #[test]
    fn components_file_round_trips_through_yaml_with_multi_root() {
        let f = ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            roots: vec![
                PathBuf::from("/Users/antony/Development/Ravel-Lite"),
                PathBuf::from("/Users/antony/Development/atlas-contracts"),
            ],
            generated_at: "2026-05-06T07:12:12Z".into(),
            cache_fingerprints: CacheFingerprints::default(),
            components: vec![],
        };
        let yaml = serde_yaml::to_string(&f).unwrap();
        let parsed: ComponentsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, f);
        assert_eq!(parsed.roots.len(), 2);
    }

    #[test]
    fn cache_fingerprints_round_trip_includes_analyzer_registry_sha() {
        // The new `analyzer_registry_sha` field (PR-5) is part of every
        // record going forward; this round-trip pins the wire form.
        let original = CacheFingerprints {
            ontology_sha: "ont".into(),
            prompt_shas: BTreeMap::from([("classify".to_string(), "psh".to_string())]),
            model_id: "model".into(),
            backend_version: "bv".into(),
            analyzer_registry_sha: "ars".into(),
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        assert!(yaml.contains("analyzer_registry_sha: ars"), "got:\n{yaml}");
        let parsed: CacheFingerprints = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn cache_fingerprints_default_has_empty_analyzer_registry_sha() {
        let f = CacheFingerprints::default();
        assert!(f.analyzer_registry_sha.is_empty());
    }

    #[test]
    fn cache_fingerprints_loads_legacy_record_without_analyzer_registry_sha() {
        // A pre-PR-5 record on disk omits `analyzer_registry_sha`; the
        // `#[serde(default)]` attribute keeps it parseable. The value
        // is the empty string, which differs from the hash of any
        // non-empty registry and so triggers a recompute on first run
        // after upgrade.
        let yaml = "ontology_sha: o\nmodel_id: m\nbackend_version: bv\n";
        let parsed: CacheFingerprints = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.analyzer_registry_sha.is_empty());
    }

    #[test]
    fn subsystems_overrides_file_default_has_current_schema_version() {
        let f = SubsystemsOverridesFile::default();
        assert_eq!(f.schema_version, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION);
        assert!(f.subsystems.is_empty());
    }

    #[test]
    fn subsystem_override_round_trips_through_yaml() {
        let s = SubsystemOverride {
            id: "auth".into(),
            members: vec!["services/auth/*".into(), "identity-core".into()],
            role: Some("identity-and-authorisation".into()),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            rationale: "owns all session/token surfaces".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
        };
        let yaml = serde_yaml::to_string(&s).unwrap();
        let parsed: SubsystemOverride = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn subsystems_overrides_file_round_trips_through_yaml() {
        let f = SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["libs/identity".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        };
        let yaml = serde_yaml::to_string(&f).unwrap();
        let parsed: SubsystemsOverridesFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, f);
    }

    #[test]
    fn subsystems_file_default_has_current_schema_version() {
        let f = SubsystemsFile::default();
        assert_eq!(f.schema_version, SUBSYSTEMS_SCHEMA_VERSION);
        assert!(f.subsystems.is_empty());
        assert!(f.generated_at.is_empty());
    }

    #[test]
    fn subsystem_entry_round_trips_through_yaml() {
        let e = SubsystemEntry {
            id: "auth".into(),
            role: Some("identity".into()),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            rationale: "x".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            members: vec![
                ComponentId::parse("auth-service").unwrap(),
                ComponentId::parse("identity-lib").unwrap(),
            ],
            member_evidence: vec![
                MemberEvidence {
                    id: "auth-service".into(),
                    matched_via: "services/auth/*".into(),
                },
                MemberEvidence {
                    id: "identity-lib".into(),
                    matched_via: "libs/identity".into(),
                },
            ],
            notes: vec![],
        };
        let yaml = serde_yaml::to_string(&e).unwrap();
        let parsed: SubsystemEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn member_evidence_round_trips_through_yaml() {
        let m = MemberEvidence {
            id: "x-component".into(),
            matched_via: "id".into(),
        };
        let yaml = serde_yaml::to_string(&m).unwrap();
        let parsed: MemberEvidence = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn external_entry_round_trips_through_yaml() {
        let entry = ExternalEntry {
            id: "crate:serde".into(),
            kind: "external".into(),
            language: Some("rust".into()),
            purl: Some("pkg:cargo/serde@1".into()),
            homepage: None,
            url: None,
            discovered_from: vec!["Cargo.toml".into()],
            evidence_grade: EvidenceGrade::Strong,
        };
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let parsed: ExternalEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }
}
