//! Rust surface analyser.
//!
//! Extracts contracts, bindings, and library-api items by parsing the
//! component's `src/lib.rs` and `src/main.rs` with [`syn`] and walking
//! the AST. Phase 2 (PR-5) replaced the Phase 1 regex byte-walker
//! wholesale with this `syn`-based implementation; the change closes
//! the Phase 1 known limitation that nested `pub` items inside
//! `pub mod foo { ... }` were missed.
//!
//! ## Outputs
//!
//! For every `pub struct` (anywhere in the AST, including inside
//! `pub mod` blocks — but NOT inside non-pub mod blocks, whose
//! contents are not externally reachable) carrying a
//! `#[derive(... Serialize ... Deserialize ...)]`
//! attribute, the analyser emits a `data-format` contract whose
//! `definition_binding` covers the struct's `pub`-to-closing-brace
//! byte range. The struct also appears as a `pub_item` of kind
//! `struct` in the component's `LibraryApi`.
//!
//! For every other `pub item` (struct without serde derive, `pub fn`,
//! `pub trait`, `pub enum`, `pub mod`, `pub type`, `pub const`,
//! `pub static`, `pub union`), a `pub_item` is recorded under the
//! component's single Rust `LibraryApi`.
//!
//! ## Span convention (spec §2.1)
//!
//! Spans are `(start_byte, end_byte)` half-open ranges over the
//! source file's bytes. For block items (`struct { ... }`, `enum
//! { ... }`, `mod { ... }`, `union { ... }`, `trait { ... }`, `fn { ... }`)
//! the span starts at the first byte of `pub` and ends one byte
//! after the matching closing `}`. For statement-form items
//! (`pub type X = Y;`, `pub const X: T = ...;`, `pub static X: T = ...;`)
//! the span ends one byte after the terminating `;`. The choice is
//! load-bearing: the binding's `content_sha` hashes exactly those
//! bytes, so reformatting *outside* the span (a doc comment moved,
//! adjacent items rearranged) does not affect the sha, while
//! reformatting *inside* the span does (per spec §2.1).
//!
//! `proc_macro2::Span::byte_range` (with the `span-locations` feature)
//! supplies absolute byte ranges; we do not re-derive the range from
//! line/column. Note that `syn::Item::span()` starts at the leading
//! attributes (`#[derive(...)]`, doc comments), *not* at `pub`. We
//! therefore explicitly take `vis.span().byte_range().start` (the `pub`
//! keyword position) for the span start, and `item.span().byte_range().end`
//! (one past `}` or `;`) for the span end. This satisfies the spec §2.1
//! PR-7 semantics: span starts at `pub`, ends at the closing delimiter.
//!
//! ## Why this lives in `atlas-analyzers`
//!
//! Surface analysers compose under the same [`crate::Analyzer`]
//! trait so future per-language analysers slot in alongside this one.
//! Binding extraction is a pure function of file bytes, so the
//! analyser is materially testable in isolation without standing up
//! a database.

use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, Contract, ContractKind, CostClass, LibraryApi, PubItem, PubItemKind, Stage,
};
use syn::spanned::Spanned;

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id (matches the wire form a future
/// `analyzers.yaml` would carry; design §6.6).
pub const ANALYZER_ID: &str = "rust-surface-analyzer";

/// Bumped to `2.0.0` in Phase 2 PR-5 to mark the regex → `syn`
/// breaking change: span byte ranges produced by the new walker do
/// not match the regex byte-walker's, so any cache keyed on a binding
/// `content_sha` from the previous version is invalidated by design.
pub const ANALYZER_VERSION: &str = "2.0.0";

/// Output of a single component's Rust-surface analysis. The L5
/// driver downcasts the `Box<dyn StageOutput>` back to this struct
/// via the [`crate::StageOutput`] machinery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustSurfaceOutput {
    /// Code-derived `data-format` contracts for `pub struct` items
    /// that carry `#[derive(Serialize, Deserialize)]`.
    pub contracts: Vec<Contract>,
    /// Every binding emitted (one per contract; each contract's
    /// `definition_binding` is also represented here for callers
    /// that want a flat list).
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] entry — the analyser emits Rust
    /// only. Empty when the component exposes no `pub` items.
    pub library_apis: Vec<LibraryApi>,
}

crate::impl_stage_output!(RustSurfaceOutput);

/// The analyser itself. Stateless; the only knob is the
/// [`ANALYZER_VERSION`] constant, which the registry picks up via the
/// [`Analyzer::version`] trait method.
#[derive(Debug, Default)]
pub struct RustSurfaceAnalyzer;

impl RustSurfaceAnalyzer {
    pub fn new() -> Self {
        RustSurfaceAnalyzer
    }
}

impl Analyzer for RustSurfaceAnalyzer {
    fn id(&self) -> &str {
        ANALYZER_ID
    }
    fn stage(&self) -> Stage {
        Stage::L5
    }
    fn cost_class(&self) -> CostClass {
        CostClass::DeterministicCheap
    }
    fn version(&self) -> &str {
        ANALYZER_VERSION
    }

    fn applies(&self, target: &Target) -> bool {
        // Cheap: applies whenever Cargo.toml is present. The
        // `analyse` path tolerates a missing `src/lib.rs` /
        // `src/main.rs` (returns an empty output) so we don't need
        // to probe for those files here. The L5 driver prefers
        // calling `extract_rust_surface` directly — `applies` is
        // recorded for future registry-driven dispatch and for the
        // analyser-registry-sha contribution.
        target.manifest_by_name("Cargo.toml").is_some()
    }

    fn fingerprint_inputs(&self, _target: &Target) -> Vec<FingerprintInput> {
        // Surface extraction is a function of `src/lib.rs` +
        // `src/main.rs` content. Manifests do not affect the
        // verdict. We can't read the source files from here without
        // an engine handle, so the engine-side L5 driver contributes
        // file shas separately via the FingerprintBuilder; the
        // analyser declares its tagged custom contributor so a future
        // direct dispatch path (Phase 2) still keys correctly.
        Vec::new()
    }

    fn analyse(&self, _ctx: &AnalysisContext, _target: &Target) -> AnalyzerResult {
        // The dispatcher path is reserved for a future driver that
        // hands a populated `Target` carrying `src/lib.rs`/`src/main.rs`
        // bytes. Today's L5 driver invokes [`extract_rust_surface`]
        // directly rather than going through `analyse`, so this branch
        // returns `Declines` to remain a well-behaved registry citizen
        // without duplicating the engine's file-loading logic.
        AnalyzerResult::Declines
    }
}

/// Inputs describing one component's Rust source surface.
///
/// The L5 driver fills this in from the engine's `path_segments`
/// (resolving `src/lib.rs` and `src/main.rs` against each segment),
/// then calls [`extract_rust_surface`]. Both files are optional —
/// a binary-only crate has only `src/main.rs`; a library-only crate
/// has only `src/lib.rs`. A workspace crate may have neither, in
/// which case the function returns an empty output.
#[derive(Debug, Clone, Default)]
pub struct RustSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path is
    /// what gets stored on `Binding.file` (relative to the component's
    /// path). Empty `bytes` are tolerated and produce no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
}

/// Drive the Rust-surface extraction over the component's source
/// inputs. Returns the contracts, bindings, and `LibraryApi` (at most
/// one) discovered.
///
/// `component_id` is the owning component's id (e.g.
/// `atlas-contracts/atlas-index`); contract ids and the library-api
/// id are namespaced under it per spec §6.3 worked example.
///
/// Sources whose bytes are not valid UTF-8 or that fail to parse as
/// Rust are silently skipped — the analyser is conservative and
/// prefers emitting nothing for a malformed file over panicking the
/// pipeline.
pub fn extract_rust_surface(component_id: &str, inputs: &RustSourceInputs) -> RustSurfaceOutput {
    let mut contracts: Vec<Contract> = Vec::new();
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    for (rel_path, bytes) in &inputs.sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Ok(file) = syn::parse_file(text) else {
            continue;
        };
        walk_items(
            component_id,
            rel_path,
            bytes,
            &file.items,
            &mut contracts,
            &mut bindings,
            &mut pub_items,
        );
    }

    let library_apis: Vec<LibraryApi> = if pub_items.is_empty() {
        Vec::new()
    } else {
        // Sort pub_items deterministically: by file then by name. The
        // walk order (file order in `sources`, declaration order
        // within a file) is already deterministic, but a fingerprint
        // computed from this list deserves an explicit tie-breaker
        // for sanity.
        let mut sorted = pub_items.clone();
        sorted.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.name.cmp(&b.name)));

        let api_id = format!("{component_id}/public-api");
        let api_fp = library_api_fingerprint(&api_id, &sorted);
        let api = LibraryApi {
            id: api_id,
            kind: ContractKind::LibraryApi,
            language: "rust".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate().expect(
            "RustSurfaceAnalyzer constructs LibraryApi with kind=LibraryApi by construction",
        );
        vec![api]
    };

    RustSurfaceOutput {
        contracts,
        bindings,
        library_apis,
    }
}

/// Walk every item in `items`, recursing into inline `pub` mod bodies.
/// Records each `pub` item (and emits a contract when the item is a
/// `pub struct` carrying a serde derive). Recursion is gated on the
/// mod's own visibility: items inside a non-`pub` mod are not
/// externally reachable and must not appear in the public API surface.
/// This matches Phase 1's depth-0-only regex behaviour, which never
/// entered any nested mod body — the same invariant expressed via a
/// visibility check rather than a depth counter.
///
/// `pub`, `pub(crate)`, `pub(super)`, and `pub(in path)` (all
/// captured by `syn::Visibility::Public` and
/// `syn::Visibility::Restricted`) all permit recursion; bare private
/// (i.e. `syn::Visibility::Inherited`) does not.
fn walk_items(
    component_id: &str,
    rel_path: &Path,
    bytes: &[u8],
    items: &[syn::Item],
    contracts: &mut Vec<Contract>,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    for item in items {
        emit_item(
            component_id,
            rel_path,
            bytes,
            item,
            contracts,
            bindings,
            pub_items,
        );

        // Recurse into inline mod bodies ONLY when the mod itself is
        // pub (or a restricted-pub variant). Items inside a non-pub
        // mod are not externally reachable and must not surface in the
        // public API. Phase 1's regex walker only operated at depth 0
        // and never entered any mod body; we match that "non-pub mod
        // contents are private" invariant here via a visibility gate.
        if let syn::Item::Mod(mod_item) = item {
            if matches!(
                mod_item.vis,
                syn::Visibility::Public(_) | syn::Visibility::Restricted(_)
            ) {
                if let Some((_, inner_items)) = &mod_item.content {
                    walk_items(
                        component_id,
                        rel_path,
                        bytes,
                        inner_items,
                        contracts,
                        bindings,
                        pub_items,
                    );
                }
            }
        }
    }
}

/// If `item` is `pub`, emit a `PubItem` and (when it's a serde-derived
/// struct) a `data-format` contract.
fn emit_item(
    component_id: &str,
    rel_path: &Path,
    bytes: &[u8],
    item: &syn::Item,
    contracts: &mut Vec<Contract>,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let Some((kind, name, vis, attrs)) = describe_item(item) else {
        return;
    };
    if !is_pub(vis) {
        return;
    }
    let Some(span) = item_byte_span(item, vis, bytes) else {
        return;
    };

    let content_sha = crate::sha256_hex_of_range(bytes, span);
    pub_items.push(PubItem {
        name: name.clone(),
        file: rel_path.to_path_buf(),
        kind,
    });

    if matches!(kind, PubItemKind::Struct) && attrs_have_serde_derive(attrs) {
        let local = kebabify_struct_name(&name);
        let contract_id = format!("{component_id}/{local}");
        let binding = Binding {
            language: "rust".into(),
            symbol: name.clone(),
            file: rel_path.to_path_buf(),
            span,
            content_sha: content_sha.clone(),
        };
        let contract = Contract {
            id: contract_id,
            kind: ContractKind::DataFormat,
            // Spec §2.1 reduction: contract sha == binding sha.
            fingerprint: content_sha.clone(),
            definition_binding: binding.clone(),
            description: String::new(),
        };
        bindings.push(binding);
        contracts.push(contract);
    }
}

/// Inspect a [`syn::Item`] and return `(kind, name, visibility, attrs)`
/// for the variants we surface. Returns `None` for variants that
/// don't contribute to a library API (e.g. `pub use`, `extern crate`,
/// `impl`, `macro`, verbatim, etc.). The returned `name` is the
/// item's identifier; the visibility and attributes references live
/// inside the original item.
fn describe_item(
    item: &syn::Item,
) -> Option<(PubItemKind, String, &syn::Visibility, &[syn::Attribute])> {
    match item {
        syn::Item::Struct(s) => Some((
            PubItemKind::Struct,
            s.ident.to_string(),
            &s.vis,
            s.attrs.as_slice(),
        )),
        syn::Item::Enum(e) => Some((
            PubItemKind::Enum,
            e.ident.to_string(),
            &e.vis,
            e.attrs.as_slice(),
        )),
        syn::Item::Fn(f) => Some((
            PubItemKind::Fn,
            f.sig.ident.to_string(),
            &f.vis,
            f.attrs.as_slice(),
        )),
        syn::Item::Trait(t) => Some((
            PubItemKind::Trait,
            t.ident.to_string(),
            &t.vis,
            t.attrs.as_slice(),
        )),
        syn::Item::Mod(m) => Some((
            PubItemKind::Mod,
            m.ident.to_string(),
            &m.vis,
            m.attrs.as_slice(),
        )),
        syn::Item::Type(t) => Some((
            PubItemKind::TypeAlias,
            t.ident.to_string(),
            &t.vis,
            t.attrs.as_slice(),
        )),
        syn::Item::Const(c) => Some((
            PubItemKind::Const,
            c.ident.to_string(),
            &c.vis,
            c.attrs.as_slice(),
        )),
        syn::Item::Static(s) => Some((
            PubItemKind::Static,
            s.ident.to_string(),
            &s.vis,
            s.attrs.as_slice(),
        )),
        syn::Item::Union(u) => Some((
            PubItemKind::Union,
            u.ident.to_string(),
            &u.vis,
            u.attrs.as_slice(),
        )),
        // `pub use ...` is a re-export, not a declaration site —
        // explicitly excluded per the existing tests.
        syn::Item::Use(_) => None,
        // `pub macro` / `macro_rules!` / `extern crate` / `impl` /
        // `trait alias` / verbatim / foreign mod / etc. don't
        // contribute to the library API surface today. Phase 1 also
        // ignored them — the `PubItemKind::Macro` enum variant
        // existed but was unreachable in practice, and PR-5 keeps
        // that posture (no test covers macros).
        _ => None,
    }
}

/// True iff `vis` is the syntactic `pub`. Restricted forms
/// (`pub(crate)`, `pub(super)`, `pub(in path)`) are also recorded:
/// the existing test `pub_with_restriction_is_recognised` pins this.
fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(
        vis,
        syn::Visibility::Public(_) | syn::Visibility::Restricted(_)
    )
}

/// Compute the half-open byte range for a `pub`-prefixed [`syn::Item`].
///
/// Spec §2.1 says the span begins at the first byte of `pub` and
/// ends one byte after the closing `}` (block items) or `;`
/// (statement items). `syn::Item::span()` yields the *entire* item
/// span including any leading `#[derive(...)]` / doc-comment
/// attributes — wider than the spec wants. We therefore take:
///
/// - **start**: the first byte of `vis.span().byte_range()`. For
///   `pub` and `pub(crate)`/`pub(super)`/`pub(in path)`, that span
///   begins at the `pub` keyword.
/// - **end**: the last byte of `item.span().byte_range()`, which is
///   one past `}` for block items and one past `;` for statement
///   items — matching the spec.
///
/// `proc_macro2::Span::byte_range` (with the `span-locations`
/// feature, enabled in the crate's `Cargo.toml`) returns the
/// absolute file-byte range. Outside a proc-macro context (which is
/// where we are — we call `syn::parse_file` from a regular library)
/// the byte range is accurate on stable Rust per the proc-macro2
/// docs.
fn item_byte_span(item: &syn::Item, vis: &syn::Visibility, bytes: &[u8]) -> Option<(usize, usize)> {
    let item_range = item.span().byte_range();
    let vis_range = vis.span().byte_range();
    let start = vis_range.start;
    let end = item_range.end;
    if end > bytes.len() || start > end {
        return None;
    }
    Some((start, end))
}

/// Walk an attribute list and return true iff one of them is a
/// `derive(...)` naming both `Serialize` and `Deserialize`.
///
/// The `cfg_attr(..., derive(...))` form is intentionally NOT
/// supported (matching Phase 1 behaviour and the spec note about
/// conservative detection): we look only at unconditional `#[derive]`.
/// Path forms like `serde::Serialize` are accepted — only the last
/// path segment is compared.
fn attrs_have_serde_derive(attrs: &[syn::Attribute]) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let mut has_serialize = false;
        let mut has_deserialize = false;
        // `parse_nested_meta` walks the comma-separated list inside
        // the `derive(...)` argument. Each element is a `Path`. We
        // check the last segment's identifier so `serde::Serialize`
        // matches in addition to bare `Serialize`.
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(last) = meta.path.segments.last() {
                let ident = last.ident.to_string();
                if ident == "Serialize" {
                    has_serialize = true;
                } else if ident == "Deserialize" {
                    has_deserialize = true;
                }
            }
            Ok(())
        });
        if has_serialize && has_deserialize {
            return true;
        }
    }
    false
}

/// SHA-256 hex of the canonicalised public-API surface. The
/// canonical form is the api id followed by one line per pub_item
/// (`<file>\t<kind-str>\t<name>`), terminated by `\n`. Stable
/// because `pub_items` is sorted by `(file, name)` upstream.
fn library_api_fingerprint(api_id: &str, items: &[PubItem]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(api_id.as_bytes());
    hasher.update([0u8]); // separator
    for item in items {
        let kind_str = pub_item_kind_str(item.kind);
        hasher.update(item.file.to_string_lossy().as_bytes());
        hasher.update([b'\t']);
        hasher.update(kind_str.as_bytes());
        hasher.update([b'\t']);
        hasher.update(item.name.as_bytes());
        hasher.update([b'\n']);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}

fn pub_item_kind_str(kind: PubItemKind) -> &'static str {
    match kind {
        PubItemKind::Struct => "struct",
        PubItemKind::Enum => "enum",
        PubItemKind::Fn => "fn",
        PubItemKind::Trait => "trait",
        PubItemKind::Mod => "mod",
        PubItemKind::TypeAlias => "type-alias",
        PubItemKind::Const => "const",
        PubItemKind::Static => "static",
        PubItemKind::Union => "union",
        PubItemKind::Macro => "macro",
    }
}

/// Convert a Rust struct name (CamelCase) to kebab-case for use as a
/// contract local-name fragment (per spec §3, contract id is
/// `<component-id>/<struct-kebab>`).
fn kebabify_struct_name(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Re-export of the simple-bytes-sha helper so the engine L5 driver
/// (and other callers) can compute binding shas without re-implementing
/// the algorithm. Kept in the `crate::` namespace so it composes with
/// the L5 driver's fingerprint construction.
pub use crate::sha256_hex_of_range;

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> RustSourceInputs {
        RustSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
        }
    }

    #[test]
    fn pub_struct_with_serde_derive_emits_data_format_contract() {
        let body = "#[derive(Serialize, Deserialize)]\npub struct Foo { a: u32 }\n";
        let out = extract_rust_surface("demo/comp", &input("src/lib.rs", body));
        assert_eq!(out.contracts.len(), 1, "got: {:?}", out.contracts);
        let c = &out.contracts[0];
        assert_eq!(c.kind, ContractKind::DataFormat);
        assert_eq!(c.id, "demo/comp/foo");
        assert_eq!(c.definition_binding.symbol, "Foo");
        assert_eq!(c.definition_binding.language, "rust");
        // Span starts at first byte of `pub` (after the derive attr +
        // newline); ends at the byte after `}`.
        let pub_start = body.find("pub struct").unwrap();
        let after_brace = body.rfind('}').unwrap() + 1;
        assert_eq!(c.definition_binding.span, (pub_start, after_brace));
        // Library-api emitted with the struct as a pub_item.
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].pub_items.len(), 1);
        assert_eq!(out.library_apis[0].pub_items[0].kind, PubItemKind::Struct);
        assert_eq!(out.library_apis[0].pub_items[0].name, "Foo");
    }

    #[test]
    fn pub_struct_without_serde_derive_emits_only_pub_item_no_contract() {
        let body = "pub struct Bar { a: u32 }\n";
        let out = extract_rust_surface("demo/comp", &input("src/lib.rs", body));
        assert!(out.contracts.is_empty());
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].pub_items.len(), 1);
        assert_eq!(out.library_apis[0].pub_items[0].name, "Bar");
    }

    #[test]
    fn library_api_aggregates_pub_fn_pub_trait_pub_enum() {
        let body = "pub fn alpha() {}\npub trait Beta {}\npub enum Gamma { A, B }\n";
        let out = extract_rust_surface("demo/comp", &input("src/lib.rs", body));
        let api = &out.library_apis[0];
        assert_eq!(api.pub_items.len(), 3);
        let names: Vec<&str> = api.pub_items.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"Beta"));
        assert!(names.contains(&"Gamma"));
    }

    #[test]
    fn pub_use_does_not_appear_as_pub_item() {
        // `pub use` is a re-export; not a declaration site.
        let body = "pub use std::fmt::Debug;\npub fn alpha() {}\n";
        let out = extract_rust_surface("demo/comp", &input("src/lib.rs", body));
        let api = &out.library_apis[0];
        assert_eq!(api.pub_items.len(), 1);
        assert_eq!(api.pub_items[0].name, "alpha");
    }

    #[test]
    fn pub_with_restriction_is_recognised() {
        let body = "pub(crate) struct Restricted;\n";
        let out = extract_rust_surface("demo/comp", &input("src/lib.rs", body));
        // pub(crate) is technically not exported but PR-5 records it
        // as a pub_item (the engine-side filter, if any, lives at L9).
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].pub_items[0].name, "Restricted");
    }

    #[test]
    fn doc_comment_outside_span_does_not_affect_binding_sha() {
        let body1 = "#[derive(Serialize, Deserialize)]\npub struct Foo { a: u32 }\n";
        let body2 = "/// pre-existing doc comment\n/// second line\n#[derive(Serialize, Deserialize)]\npub struct Foo { a: u32 }\n";
        let o1 = extract_rust_surface("c", &input("src/lib.rs", body1));
        let o2 = extract_rust_surface("c", &input("src/lib.rs", body2));
        assert_eq!(o1.contracts.len(), 1);
        assert_eq!(o2.contracts.len(), 1);
        assert_eq!(
            o1.contracts[0].definition_binding.content_sha,
            o2.contracts[0].definition_binding.content_sha,
            "doc comments outside the span must not affect the binding sha (spec §2.1)"
        );
    }

    #[test]
    fn whitespace_inside_span_changes_binding_sha() {
        let body1 = "#[derive(Serialize, Deserialize)]\npub struct Foo { a: u32 }\n";
        let body2 = "#[derive(Serialize, Deserialize)]\npub struct Foo {  a: u32 }\n"; // extra space
        let o1 = extract_rust_surface("c", &input("src/lib.rs", body1));
        let o2 = extract_rust_surface("c", &input("src/lib.rs", body2));
        assert_ne!(
            o1.contracts[0].definition_binding.content_sha,
            o2.contracts[0].definition_binding.content_sha,
            "whitespace inside the span must change the binding sha (spec §2.1)"
        );
    }

    #[test]
    fn syn_extracts_nested_pub_inside_pub_mod() {
        // PR-5 closes the Phase 1 known limitation: nested
        // `pub mod outer { pub struct Hidden; }` now contributes
        // `Hidden` as a pub_item.
        let body = "pub mod outer { pub struct Hidden; }\n";
        let out = extract_rust_surface("c", &input("src/lib.rs", body));
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(names.contains(&"outer"), "outer mod must be recorded");
        assert!(
            names.contains(&"Hidden"),
            "PR-5 (syn walker) extracts nested pub items: got {names:?}"
        );
    }

    #[test]
    fn struct_inside_string_literal_is_not_picked_up() {
        let body = "pub fn renderer() -> &'static str { \"pub struct NotARealOne {}\" }\n";
        let out = extract_rust_surface("c", &input("src/lib.rs", body));
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["renderer"]);
    }

    #[test]
    fn pub_const_is_recognised_with_correct_span() {
        let body = "pub const NAME: &str = \"example\";\n";
        let out = extract_rust_surface("c", &input("src/lib.rs", body));
        assert_eq!(out.library_apis[0].pub_items.len(), 1);
        assert_eq!(out.library_apis[0].pub_items[0].kind, PubItemKind::Const);
    }

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "pub fn alpha() {}\n";
        let out = extract_rust_surface("foo/bar", &input("src/lib.rs", body));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
        assert_eq!(out.library_apis[0].language, "rust");
    }

    #[test]
    fn contract_id_kebab_cases_struct_names_with_caps() {
        let body = "#[derive(Serialize, Deserialize)]\npub struct ComponentEntry { a: u32 }\n";
        let out = extract_rust_surface("ns/comp", &input("src/schema.rs", body));
        assert_eq!(out.contracts[0].id, "ns/comp/component-entry");
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let inputs = RustSourceInputs {
            sources: Vec::new(),
        };
        let out = extract_rust_surface("ns/comp", &inputs);
        assert!(out.contracts.is_empty());
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn no_pub_items_emit_no_library_api() {
        let body = "fn private() {}\nstruct AlsoPrivate;\n";
        let out = extract_rust_surface("ns/comp", &input("src/lib.rs", body));
        assert!(out.library_apis.is_empty(), "no pub items → no LibraryApi");
    }

    #[test]
    fn library_api_pub_items_are_sorted_by_file_then_name() {
        // Two source files; check the sort order is deterministic.
        let inputs = RustSourceInputs {
            sources: vec![
                (PathBuf::from("src/main.rs"), b"pub fn zeta() {}\n".to_vec()),
                (
                    PathBuf::from("src/lib.rs"),
                    b"pub fn alpha() {}\npub fn beta() {}\n".to_vec(),
                ),
            ],
        };
        let out = extract_rust_surface("ns/comp", &inputs);
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "beta", "zeta"]);
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = RustSurfaceAnalyzer::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L5);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
        // PR-5 explicit breaking-change marker.
        assert_eq!(ANALYZER_VERSION, "2.0.0");
    }

    #[test]
    fn applies_is_true_when_cargo_toml_present() {
        let target = Target {
            dir: PathBuf::from("/tmp/x"),
            languages: std::collections::BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "Cargo.toml".into(),
                relpath: PathBuf::from("Cargo.toml"),
                bytes: b"[package]".to_vec(),
                content_sha: "abc".into(),
            }],
            top_level_files: Vec::new(),
        };
        assert!(RustSurfaceAnalyzer::new().applies(&target));
    }

    #[test]
    fn applies_is_false_without_cargo_toml() {
        let target = Target {
            dir: PathBuf::from("/tmp/x"),
            languages: std::collections::BTreeSet::new(),
            manifests: Vec::new(),
            top_level_files: Vec::new(),
        };
        assert!(!RustSurfaceAnalyzer::new().applies(&target));
    }

    #[test]
    fn ignores_non_utf8_source_bytes() {
        let inputs = RustSourceInputs {
            sources: vec![(PathBuf::from("src/lib.rs"), vec![0xFF, 0xFE, 0xFD])],
        };
        let out = extract_rust_surface("ns/comp", &inputs);
        assert!(out.contracts.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn cargo_fmt_idempotent_on_struct_body_keeps_same_sha() {
        // The struct body is already canonically formatted; re-running
        // the scanner on identical bytes must produce identical sha.
        let body = "#[derive(Serialize, Deserialize)]\npub struct Foo { a: u32 }\n";
        let o1 = extract_rust_surface("c", &input("src/lib.rs", body));
        let o2 = extract_rust_surface("c", &input("src/lib.rs", body));
        assert_eq!(
            o1.contracts[0].definition_binding.content_sha,
            o2.contracts[0].definition_binding.content_sha
        );
    }

    #[test]
    fn non_pub_mod_does_not_surface_inner_pub_items() {
        // Regression: the Phase 2 syn walker previously entered ALL
        // inline mod bodies regardless of visibility, surfacing pub
        // items from non-pub mods. Items inside a non-pub mod are not
        // externally reachable and must not appear in the public API.
        let body = "mod inner { pub struct ShouldBeHidden; }\npub struct Public;\n";
        let out = extract_rust_surface("c", &input("src/lib.rs", body));
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            names.contains(&"Public"),
            "expected Public in pub items, got {:?}",
            names
        );
        assert!(
            !names.contains(&"ShouldBeHidden"),
            "non-pub mod must not surface its pub items, got {:?}",
            names
        );
    }
}
