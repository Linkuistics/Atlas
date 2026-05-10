//! Per-component `surfaces.yaml` schema (Atlas vNext Phase 1).
//!
//! The surfaces file is the load-bearing input to L6 edge proposal: it
//! enumerates the contracts a component defines, implements, and
//! consumes, plus the language-specific bindings to those contracts and
//! the in-process library API surface (design §3.4, §6.3).
//!
//! Phase 1 ships `schema_version: 1`. Bindings are Rust-only; the
//! `Binding.span` is a byte-offset pair `(start, end)` over the
//! binding's source file. Phase 2 will generalise the span shape and
//! bump the schema version.
//!
//! Content sha conventions follow the companion spec
//! `2026-05-06-contract-content-sha-canonicalisation.md`. Phase 1
//! computes binding shas over the file-byte-range covered by `span`;
//! contract shas reduce to the defining-binding sha for code-derived
//! contracts. PR-7 implements those algorithms; this module only
//! defines the field shapes they project into.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use component_ontology::ComponentId;

/// Schema version for `surfaces.yaml`. Bumps on any breaking change
/// to the field shape (Phase 2 generalises the span form for
/// non-Rust bindings; that bump lands then).
pub const SURFACES_SCHEMA_VERSION: u32 = 1;

/// Closed vocabulary of contract kinds. Phase 1 emits Rust-binding-derived
/// `data-format` and Rust `library-api` contracts, plus a single
/// test-only `wire-protocol` fixture. The remaining variants are
/// specified now so Phase 2 has nothing to invent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContractKind {
    /// On-disk YAML / JSON / protobuf payload schema. The most common
    /// cross-language coupling.
    DataFormat,
    /// HTTP / gRPC / message-queue surface; the schema of network
    /// messages.
    WireProtocol,
    /// Conventions about a directory layout (e.g. `.atlas/`, well-known
    /// paths in `/var/run`).
    FilesystemLayout,
    /// CLI argv / env / exit-code shape; shell hook contract; systemd
    /// unit interface.
    ProcessInterface,
    /// A set of env vars consumed together (e.g. `DATABASE_URL`,
    /// `DATABASE_POOL_SIZE`).
    EnvironmentNamespace,
    /// In-process API surface, language-bound. The only contract kind
    /// that does not cross language boundaries.
    LibraryApi,
    /// Erlang/Elixir structural protocol pattern. A `defprotocol`
    /// declaration defines a `Behaviour` contract; `defimpl` or a
    /// module declaring `@behaviour MyModule` implements one. Phase 2
    /// PR-8 introduces this variant — it captures `@callback`
    /// declarations from `defprotocol` / `@behaviour` module bodies.
    ///
    /// Wire form: `"behaviour"` (British spelling, matching Erlang/OTP
    /// convention). The defining component emits a `Contract` with this
    /// kind and a `defines-contract` edge; implementing components emit
    /// `ImplementedContract` entries with kind `behaviour`.
    Behaviour,
}

/// Role a binding plays with respect to its contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingRole {
    /// The binding the contract is derived from (the contract's owning
    /// component declares this).
    DefiningBinding,
    /// An implementation of the contract for this language; the
    /// component supplies a binding compatible with the contract shape
    /// but is not the source of truth.
    ImplementingBinding,
    /// A usage site — the component reads/writes data through this
    /// binding.
    ConsumingBinding,
}

/// How a binding's visibility is expressed in its source language.
///
/// Phase 2 PR-3 introduced this enum because Python (and later Dart,
/// Racket, Elixir, LispKit) has no `pub`-style keyword: a top-level
/// `def foo` is library-public by *convention*, with leading-underscore
/// names treated as library-private. The two variants:
///
/// - [`Visibility::Explicit`] — the source language carries an explicit
///   visibility keyword. The `keyword` field stores the surface form
///   (`"pub"`, `"public"`, `"export"`) so consumers that care about
///   sub-distinctions (Rust's `pub(crate)` vs `pub`, C#'s `internal`
///   vs `public`) can branch on it. Languages that PR-3 has migrated:
///   Rust (`pub`), TypeScript / JavaScript ESM (`export`).
/// - [`Visibility::Conventional`] — the source language has no
///   visibility keyword; the binding is in the public surface by
///   convention (and any "private" qualifier appears in the
///   [`Binding::attributes`] map, e.g. `private: true` for Python's
///   leading-underscore rule).
///
/// Wire form: serde-tagged, kebab-cased on the `kind` discriminator.
/// Round-trip example:
///
/// ```yaml
/// visibility:
///   kind: explicit
///   keyword: pub
/// # … or …
/// visibility:
///   kind: conventional
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Visibility {
    /// Language carries a visibility keyword (`pub`, `public`,
    /// `export`). The `keyword` field stores the surface form.
    Explicit { keyword: String },
    /// Language has no keyword; visibility is determined by
    /// convention (Python `_foo`, Dart `_foo`, Racket non-`provide`,
    /// Elixir `defp`).
    Conventional,
}

impl Visibility {
    /// Convenience constructor for the common Rust-`pub` case.
    pub fn pub_keyword() -> Self {
        Visibility::Explicit {
            keyword: "pub".to_string(),
        }
    }
}

/// A binding is a language-specific projection of a contract. Phase 1
/// emitted Rust bindings only; Phase 2 PR-3 introduces non-Rust
/// language analysers (Python first), each contributing bindings
/// uniformly through the same struct shape. The `language` field is
/// recorded explicitly so schema readers can branch on it without
/// inferring from the file extension.
///
/// `span` is a `(start_byte, end_byte)` half-open range over the
/// binding source file's bytes. This matches the form already in use
/// at the L5 boundary (the code-derived contract sha algorithm in the
/// companion spec is `sha256(bytes[span.0..span.1])`). The Rust
/// `pub struct Foo { … }` start-of-`pub` to closing-brace convention
/// is the canonical span.
///
/// ## PR-3 schema mutations (greenfield)
///
/// The v1 *shape* is mutating freely as each Phase 2 language analyser
/// lands; PR-3 adds:
///
/// - `visibility: Visibility` — uniform across languages with explicit
///   visibility keywords (Rust `pub`, TS `export`) and conventional
///   visibility (Python no-leading-`_`, Dart no-leading-`_`).
/// - `module_path: Vec<String>` — Python `pkg.mod.fn`'s dotted module
///   path; later C# `Namespace.Class.Method`. Empty for languages
///   that don't use it (Rust currently emits empty).
/// - `attributes: BTreeMap<String, serde_yaml::Value>` — language-
///   specific decorations. Python decorators land here as
///   `decorator_chain: [<name>, …]`; TS module-system metadata as
///   `module_system: "commonjs" | "esm"`; private-by-convention flag
///   as `private: true`.
///
/// `schema_version` stays integer `1` — these are shape mutations of
/// the v1 schema (per Phase 2 plan §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Programming language of this binding (`rust`, `typescript`,
    /// `javascript`, `python`, …).
    pub language: String,
    /// Symbol name within the source language (e.g. `ComponentEntry`,
    /// `load_components_yaml`). Used for human-readable rationale and
    /// for cross-component lookup; not load-bearing for cache keying.
    pub symbol: String,
    /// File the binding lives in, relative to the component path.
    pub file: PathBuf,
    /// Byte-offset half-open range `[start, end)` covered by the
    /// binding. The algorithm in the content-sha canonicalisation
    /// spec (§2.1) hashes `bytes[start..end]`.
    pub span: (usize, usize),
    /// SHA-256 hex of the bytes covered by `span`.
    pub content_sha: String,
    /// How visibility is expressed in the source language. Languages
    /// with explicit keywords (`pub`, `export`) use
    /// [`Visibility::Explicit`]; languages with conventional
    /// visibility (Python's leading-underscore convention) use
    /// [`Visibility::Conventional`].
    pub visibility: Visibility,
    /// Dotted module path identifying the binding's *file location*
    /// within its component. These are the file-path components
    /// (excluding the symbol). Python's `pkg/mod.py` produces
    /// `["pkg", "mod"]`; the binding's `symbol` slot carries the
    /// identifier (`fn`/`Bar`) on its own. The fully-qualified
    /// dotted form is reconstructed by joining
    /// `module_path + [symbol]` (e.g. `pkg.mod.fn`). Later
    /// C#/Elixir use the same file-path-derived shape. Languages
    /// that don't use it (e.g. Rust today) emit an empty vector.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module_path: Vec<String>,
    /// Language-specific decorations as a free-form map. Keys are
    /// kebab-case strings (`decorator-chain`, `module-system`,
    /// `private`). Values are arbitrary YAML — analysers should
    /// stick to short scalars and small arrays for diffability.
    ///
    /// Wave-2 / Wave-3 analysers must reference the `ATTR_*` consts
    /// declared below the struct rather than embedding bare string
    /// literals — the keys are load-bearing for downstream consumers
    /// branching on them, and a single source of truth catches typos
    /// at compile time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, serde_yaml::Value>,
}

/// Conventional attribute key for the conventional-private flag
/// (Python `_foo`, Elixir `defp`, Dart `_foo`). Value is `true` for
/// conventionally-private bindings; absent otherwise. Languages with
/// explicit visibility keywords use [`Visibility::Explicit`] and do
/// not emit this attribute.
pub const ATTR_PRIVATE: &str = "private";

/// Conventional attribute key for a binding's decorator chain (Python
/// `@decorator`, future C# `[Attribute]`). Value is a YAML sequence
/// of decorator names in source order.
pub const ATTR_DECORATOR_CHAIN: &str = "decorator_chain";

/// Conventional attribute key for module-system flavour
/// (TypeScript / JavaScript: `"esm"` vs `"commonjs"`). Value is a
/// YAML string.
pub const ATTR_MODULE_SYSTEM: &str = "module_system";

/// Conventional attribute key for the type-only flag
/// (TypeScript `export type ...`). Value is `true` for type-only
/// bindings; absent otherwise.
pub const ATTR_TYPE_ONLY: &str = "type_only";

/// Conventional attribute key for Elixir `@spec` type annotations.
/// Value is a YAML string containing the raw spec expression
/// (e.g. `"(binary, integer) :: {:ok, String.t()} | {:error, term()}"`).
/// Absent when no `@spec` precedes the function definition.
/// Phase 2 PR-8 introduces this for the Elixir surface analyser.
pub const ATTR_SPEC: &str = "spec";

/// Conventional attribute key for Elixir `@doc` documentation strings.
/// Value is a YAML string with the documentation text (heredoc content
/// stripped of leading/trailing whitespace). Absent when no `@doc` precedes
/// the function definition.
/// Phase 2 PR-8 introduces this for the Elixir surface analyser.
pub const ATTR_DOC: &str = "doc";

/// A first-class contract definition. The owning component is
/// implicit — the component whose `surfaces.yaml` lists this contract
/// in `contracts_defined`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    /// Stable contract id (e.g. `atlas-contracts/components-yaml-schema`).
    /// Namespaced under the defining component's id prefix by
    /// convention.
    pub id: String,
    pub kind: ContractKind,
    /// On-disk field name is `fingerprint:` (per design §6.3 YAML
    /// schema); the value is the contract's content sha computed per
    /// `2026-05-06-contract-content-sha-canonicalisation.md`. Bindings
    /// keep their separate `content_sha` field — the contract's
    /// equivalent is renamed to `fingerprint` to match the design.
    pub fingerprint: String,
    /// The defining binding: the language-bound projection from which
    /// the contract was derived.
    pub definition_binding: Binding,
    /// Optional human-readable description; emitted by analysers when
    /// the LLM produced one, otherwise empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A contract this component implements (its language-specific
/// binding satisfies the contract shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementedContract {
    pub contract_id: String,
    pub role: BindingRole,
    pub binding: Binding,
}

/// A contract this component consumes (it reads/writes data through
/// the contract's binding).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumedContract {
    pub contract_id: String,
    pub binding: Binding,
}

/// One public item exported from a component's library API. The
/// `kind` is one of `struct`, `enum`, `fn`, `trait`, `mod`, etc., in
/// kebab-case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubItem {
    pub name: String,
    pub file: PathBuf,
    pub kind: PubItemKind,
}

/// The kind of a `pub` item in a Rust component's library API. Kept
/// as a closed kebab-case enum so unknown values fail loudly at
/// deserialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PubItemKind {
    Struct,
    Enum,
    Fn,
    Trait,
    Mod,
    /// `type Alias = …;`
    TypeAlias,
    /// `const X: …;`
    Const,
    /// `static X: …;`
    Static,
    /// `union U { … }` — rare but legal in `pub` position.
    Union,
    /// `macro_rules!` exports, `#[proc_macro]` items, etc.
    Macro,
}

/// In-process library API surface for one language inside the
/// component. A polyglot component emits multiple `LibraryApi`
/// entries, one per language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryApi {
    /// Stable id (e.g. `atlas-contracts/atlas-index/public-api`).
    pub id: String,
    /// Always `library-api`. Carried explicitly so the YAML is
    /// self-describing without context. The invariant
    /// `kind == ContractKind::LibraryApi` is enforced by
    /// [`LibraryApi::validate`]; callers should call it before
    /// serialising.
    pub kind: ContractKind,
    /// Programming language this surface is bound to. `library-api`
    /// contracts are per-language by construction.
    pub language: String,
    /// Content sha of the canonicalised public-API surface.
    pub fingerprint: String,
    #[serde(default)]
    pub pub_items: Vec<PubItem>,
}

/// Reason a [`LibraryApi`] can be semantically inconsistent.
/// The on-disk schema permits the inconsistency to be expressed
/// (`kind:` is a public field of the closed [`ContractKind`] enum);
/// validation is a runtime check on top of `serde`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryApiValidationError {
    /// `LibraryApi.kind` is not [`ContractKind::LibraryApi`]. The
    /// observed kind is carried so error renderers can name it.
    WrongKind { actual: ContractKind },
}

impl std::fmt::Display for LibraryApiValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongKind { actual } => {
                write!(f, "LibraryApi.kind must be `library-api`, got `{actual:?}`")
            }
        }
    }
}

impl std::error::Error for LibraryApiValidationError {}

impl LibraryApi {
    /// Enforce the doc-comment invariant that `kind` is always
    /// [`ContractKind::LibraryApi`]. Call before serialising.
    pub fn validate(&self) -> Result<(), LibraryApiValidationError> {
        if self.kind != ContractKind::LibraryApi {
            return Err(LibraryApiValidationError::WrongKind { actual: self.kind });
        }
        Ok(())
    }
}

/// Top-level shape of `<component-path>/.atlas/surfaces.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacesFile {
    pub schema_version: u32,
    pub component_id: ComponentId,
    /// Aggregate fingerprint over all contract / binding / library-API
    /// content shas. The value other components' L6 cache keys cite
    /// when this component appears as an edge participant
    /// (design §8.2).
    pub fingerprint: String,
    #[serde(default)]
    pub contracts_defined: Vec<Contract>,
    #[serde(default)]
    pub contracts_implemented: Vec<ImplementedContract>,
    #[serde(default)]
    pub contracts_consumed: Vec<ConsumedContract>,
    #[serde(default)]
    pub library_apis: Vec<LibraryApi>,
}

impl Default for SurfacesFile {
    fn default() -> Self {
        SurfacesFile {
            schema_version: SURFACES_SCHEMA_VERSION,
            component_id: ComponentId::parse("placeholder")
                .expect("`placeholder` is a valid ComponentId"),
            fingerprint: String::new(),
            contracts_defined: Vec::new(),
            contracts_implemented: Vec::new(),
            contracts_consumed: Vec::new(),
            library_apis: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_binding() -> Binding {
        Binding {
            language: "rust".into(),
            symbol: "ComponentEntry".into(),
            file: PathBuf::from("src/schema.rs"),
            span: (69, 95),
            content_sha: "0123456789abcdef".repeat(4),
            visibility: Visibility::pub_keyword(),
            module_path: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    fn sample_contract() -> Contract {
        Contract {
            id: "atlas-contracts/components-yaml-schema".into(),
            kind: ContractKind::DataFormat,
            fingerprint: "fedcba9876543210".repeat(4),
            definition_binding: sample_binding(),
            description: "On-disk YAML schema for components.yaml.".into(),
        }
    }

    fn sample_pub_item() -> PubItem {
        PubItem {
            name: "ComponentsFile".into(),
            file: PathBuf::from("src/schema.rs"),
            kind: PubItemKind::Struct,
        }
    }

    fn sample_library_api() -> LibraryApi {
        LibraryApi {
            id: "atlas-contracts/atlas-index/public-api".into(),
            kind: ContractKind::LibraryApi,
            language: "rust".into(),
            fingerprint: "0".repeat(64),
            pub_items: vec![
                sample_pub_item(),
                PubItem {
                    name: "load_components_yaml".into(),
                    file: PathBuf::from("src/yaml_io.rs"),
                    kind: PubItemKind::Fn,
                },
            ],
        }
    }

    fn sample_surfaces_file() -> SurfacesFile {
        SurfacesFile {
            schema_version: SURFACES_SCHEMA_VERSION,
            component_id: ComponentId::parse("atlas-contracts/atlas-index").unwrap(),
            fingerprint: "abcdef0123456789".repeat(4),
            contracts_defined: vec![sample_contract()],
            contracts_implemented: vec![ImplementedContract {
                contract_id: "atlas-contracts/components-yaml-schema".into(),
                role: BindingRole::DefiningBinding,
                binding: sample_binding(),
            }],
            contracts_consumed: vec![ConsumedContract {
                contract_id: "external/some-other-contract".into(),
                binding: Binding {
                    symbol: "use_site".into(),
                    file: PathBuf::from("src/consumer.rs"),
                    span: (10, 42),
                    ..sample_binding()
                },
            }],
            library_apis: vec![sample_library_api()],
        }
    }

    #[test]
    fn binding_round_trips_through_yaml() {
        let original = sample_binding();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: Binding = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn contract_round_trips_through_yaml() {
        let original = sample_contract();
        let yaml = serde_yaml::to_string(&original).unwrap();
        // Design §6.3 prescribes the on-disk field name `fingerprint:`
        // for the contract's content sha. The defining binding still
        // uses `content_sha:` (binding's field name is correct).
        assert!(
            yaml.contains("fingerprint:"),
            "expected `fingerprint:` field, got:\n{yaml}"
        );
        assert!(
            !yaml.contains("\ncontent_sha: fedcba"),
            "Contract must not emit a top-level `content_sha:` field; got:\n{yaml}"
        );
        let parsed: Contract = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn contract_yaml_matches_design_section_6_3_field_names() {
        // Verbatim subset of the design §6.3 example, with concrete
        // sha hex placeholders. Asserts that the field shape on disk
        // is exactly what the spec prescribes.
        //
        // Phase 2 PR-3: the binding now carries `visibility`,
        // `module_path`, and `attributes`; the new fields are
        // required-on-output but `module_path` and `attributes` skip
        // serialisation when empty (the stable on-disk wire form
        // remains compact for languages that don't use them).
        let yaml = r#"
id: atlas-contracts/components-yaml-schema
kind: data-format
fingerprint: fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210
definition_binding:
  language: rust
  symbol: ComponentEntry
  file: src/schema.rs
  span: [69, 95]
  content_sha: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
  visibility:
    kind: explicit
    keyword: pub
description: |
  The on-disk YAML schema for .atlas/components.yaml.
"#;
        let parsed: Contract = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.id, "atlas-contracts/components-yaml-schema");
        assert_eq!(parsed.kind, ContractKind::DataFormat);
        assert_eq!(
            parsed.fingerprint,
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
        );
        assert_eq!(parsed.definition_binding.language, "rust");
        assert_eq!(parsed.definition_binding.symbol, "ComponentEntry");
        assert_eq!(parsed.definition_binding.span, (69, 95));
        // Round-trip preserves the same shape.
        let reemitted = serde_yaml::to_string(&parsed).unwrap();
        let reparsed: Contract = serde_yaml::from_str(&reemitted).unwrap();
        assert_eq!(reparsed, parsed);
        // Pin the load-bearing field names from design §6.3 (plus
        // PR-3's mandatory `visibility:` field) so a future serde
        // rename drifting away from the wire form trips here rather
        // than slipping through the round-trip identity.
        for field in [
            "definition_binding:",
            "fingerprint:",
            "content_sha:",
            "span:",
            "visibility:",
        ] {
            assert!(
                reemitted.contains(field),
                "design §6.3 field `{field}` missing from re-emitted YAML:\n{reemitted}"
            );
        }
    }

    #[test]
    fn pub_item_round_trips_through_yaml() {
        let original = sample_pub_item();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: PubItem = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn library_api_round_trips_through_yaml() {
        let original = sample_library_api();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: LibraryApi = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn library_api_validate_accepts_library_api_kind() {
        sample_library_api().validate().unwrap();
    }

    #[test]
    fn library_api_validate_rejects_other_kinds() {
        for wrong in [
            ContractKind::DataFormat,
            ContractKind::WireProtocol,
            ContractKind::FilesystemLayout,
            ContractKind::ProcessInterface,
            ContractKind::EnvironmentNamespace,
            ContractKind::Behaviour,
        ] {
            let mut api = sample_library_api();
            api.kind = wrong;
            assert_eq!(
                api.validate().unwrap_err(),
                LibraryApiValidationError::WrongKind { actual: wrong }
            );
        }
    }

    #[test]
    fn surfaces_file_round_trips_through_yaml() {
        let original = sample_surfaces_file();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: SurfacesFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn contract_kind_variants_serialise_kebab_case() {
        for (kind, expected) in [
            (ContractKind::DataFormat, "data-format"),
            (ContractKind::WireProtocol, "wire-protocol"),
            (ContractKind::FilesystemLayout, "filesystem-layout"),
            (ContractKind::ProcessInterface, "process-interface"),
            (ContractKind::EnvironmentNamespace, "environment-namespace"),
            (ContractKind::LibraryApi, "library-api"),
            (ContractKind::Behaviour, "behaviour"),
        ] {
            let yaml = serde_yaml::to_string(&kind).unwrap();
            assert_eq!(yaml.trim(), expected, "wrong wire form for {kind:?}");
            let parsed: ContractKind = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn binding_role_variants_serialise_kebab_case() {
        for (role, expected) in [
            (BindingRole::DefiningBinding, "defining-binding"),
            (BindingRole::ImplementingBinding, "implementing-binding"),
            (BindingRole::ConsumingBinding, "consuming-binding"),
        ] {
            let yaml = serde_yaml::to_string(&role).unwrap();
            assert_eq!(yaml.trim(), expected, "wrong wire form for {role:?}");
            let parsed: BindingRole = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn pub_item_kind_variants_serialise_kebab_case() {
        for (kind, expected) in [
            (PubItemKind::Struct, "struct"),
            (PubItemKind::Enum, "enum"),
            (PubItemKind::Fn, "fn"),
            (PubItemKind::Trait, "trait"),
            (PubItemKind::Mod, "mod"),
            (PubItemKind::TypeAlias, "type-alias"),
            (PubItemKind::Const, "const"),
            (PubItemKind::Static, "static"),
            (PubItemKind::Union, "union"),
            (PubItemKind::Macro, "macro"),
        ] {
            let yaml = serde_yaml::to_string(&kind).unwrap();
            assert_eq!(yaml.trim(), expected, "wrong wire form for {kind:?}");
            let parsed: PubItemKind = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn surfaces_file_default_has_current_schema_version() {
        let f = SurfacesFile::default();
        assert_eq!(f.schema_version, SURFACES_SCHEMA_VERSION);
        assert!(f.contracts_defined.is_empty());
        assert!(f.contracts_implemented.is_empty());
        assert!(f.contracts_consumed.is_empty());
        assert!(f.library_apis.is_empty());
    }

    #[test]
    fn visibility_explicit_round_trips_through_yaml() {
        let v = Visibility::Explicit {
            keyword: "pub".into(),
        };
        let yaml = serde_yaml::to_string(&v).unwrap();
        assert!(
            yaml.contains("kind: explicit"),
            "explicit variant must serialise with kind=explicit, got:\n{yaml}"
        );
        assert!(yaml.contains("keyword: pub"), "got:\n{yaml}");
        let parsed: Visibility = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn visibility_conventional_round_trips_through_yaml() {
        let v = Visibility::Conventional;
        let yaml = serde_yaml::to_string(&v).unwrap();
        assert!(yaml.contains("kind: conventional"), "got:\n{yaml}");
        let parsed: Visibility = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn visibility_pub_keyword_helper_constructs_explicit_pub() {
        let v = Visibility::pub_keyword();
        assert_eq!(
            v,
            Visibility::Explicit {
                keyword: "pub".into()
            }
        );
    }

    #[test]
    fn binding_with_module_path_and_attributes_round_trips() {
        // PR-3 acceptance: a Python-shaped binding with conventional
        // visibility, dotted module_path, and decorator attributes
        // round-trips cleanly.
        let mut attrs: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
        attrs.insert(
            "decorator_chain".into(),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("dataclass".into()),
                serde_yaml::Value::String("frozen".into()),
            ]),
        );
        let b = Binding {
            language: "python".into(),
            symbol: "Bar".into(),
            file: PathBuf::from("pkg/mod.py"),
            span: (12, 56),
            content_sha: "0".repeat(64),
            visibility: Visibility::Conventional,
            module_path: vec!["pkg".into(), "mod".into(), "Bar".into()],
            attributes: attrs,
        };
        let yaml = serde_yaml::to_string(&b).unwrap();
        let parsed: Binding = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn binding_omits_empty_module_path_and_attributes_from_yaml() {
        // Languages that don't use module_path / attributes (today:
        // Rust) should not be forced to emit empty `module_path: []` /
        // `attributes: {}` lines; the on-disk form stays compact.
        let b = Binding {
            language: "rust".into(),
            symbol: "Foo".into(),
            file: PathBuf::from("src/lib.rs"),
            span: (0, 10),
            content_sha: "0".repeat(64),
            visibility: Visibility::pub_keyword(),
            module_path: Vec::new(),
            attributes: BTreeMap::new(),
        };
        let yaml = serde_yaml::to_string(&b).unwrap();
        assert!(
            !yaml.contains("module_path:"),
            "empty module_path must not serialise, got:\n{yaml}"
        );
        assert!(
            !yaml.contains("attributes:"),
            "empty attributes must not serialise, got:\n{yaml}"
        );
        // Round-trip: the absent fields default back to empty.
        let parsed: Binding = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, b);
    }
}
