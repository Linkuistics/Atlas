//! Elixir surface analyser logic (Atlas vNext Phase 2 PR-8).
//!
//! This crate's library form is the pure analyser: it takes
//! [`ElixirSourceInputs`] (parsed from on-disk Elixir files +
//! `mix.exs`) and emits [`ElixirSurfaceOutput`] containing bindings,
//! library APIs, and behaviour contracts.
//!
//! ## Sibling binary
//!
//! The companion `elixir-analyzer` binary at `src/main.rs` wraps this
//! library in the subprocess wire protocol from
//! [`atlas_analyzers::subprocess`]. Tests and the in-tree
//! `elixir_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_elixir-analyzer")`.
//!
//! ## Binding shape
//!
//! Elixir has no `pub`/`priv` keyword at the Erlang-module level:
//!
//! - `def foo(...)` — public by convention → `Visibility::Conventional`.
//! - `defp foo(...)` — private in Erlang semantics → excluded from surface.
//! - `defmodule Mod do ... end` → module-level binding with
//!   `Visibility::Conventional`.
//!
//! `module_path` is derived from `defmodule MyApp.Foo.Bar do`:
//!
//! - `module_path`: `["MyApp", "Foo"]` (all but the last segment).
//! - `symbol`: `"Bar"` (the last segment).
//!
//! **Deviation from Python convention:** Python derives `module_path`
//! from the file path; Elixir derives it from the defmodule identifier
//! (Elixir modules are named by convention, not file path). This is
//! documented in the PR-8 DONE report.
//!
//! ## @spec / @doc attributes
//!
//! `@spec` and `@doc` annotations preceding a `def` are captured in
//! `attributes` using the [`atlas_index::ATTR_SPEC`] and
//! [`atlas_index::ATTR_DOC`] keys introduced by PR-8's atlas-contracts
//! changes.
//!
//! ## Behaviour contracts
//!
//! - `defprotocol Stringable do ... end` → `Contract` with
//!   `kind: ContractKind::Behaviour` and a `defines-contract` edge.
//! - `@behaviour MyModule` in a module body → `ImplementedContract`
//!   with `kind: ContractKind::Behaviour`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, BindingRole, Contract, ContractKind, ImplementedContract, LibraryApi, PubItem,
    PubItemKind, Visibility, ATTR_DOC, ATTR_SPEC,
};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

/// Stable analyser id for the Elixir surface analyser. Matches the
/// wire form a future `analyzers.yaml` would carry; the in-tree
/// wrapper at `atlas_analyzers::elixir_surface_analyzer` mirrors this
/// constant.
pub const ANALYZER_ID: &str = "elixir-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Inputs describing one component's Elixir source surface.
#[derive(Debug, Clone, Default)]
pub struct ElixirSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path
    /// must be relative to the mix project root (the directory
    /// containing `mix.exs`). Empty `bytes` are tolerated and produce
    /// no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `mix.exs` contents (bytes). When present the analyser
    /// may extract the project name for the LibraryApi id.
    pub mix_exs: Option<Vec<u8>>,
}

/// Output of one Elixir surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElixirSurfaceOutput {
    /// `def` bindings with `Visibility::Conventional`. `defp` bindings
    /// are excluded. Module bindings (`defmodule`) are also included.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] (the component's public Elixir API).
    pub library_apis: Vec<LibraryApi>,
    /// `defprotocol` / `defimpl` → `Contract` entries with
    /// `kind: ContractKind::Behaviour`.
    pub contracts: Vec<Contract>,
    /// `@behaviour MyModule` → `ImplementedContract` entries.
    pub implemented_contracts: Vec<ImplementedContract>,
}

/// Drive the Elixir-surface extraction over the component's source
/// inputs. Returns the bindings, library API, and behaviour contracts
/// discovered.
///
/// `component_id` is the owning component's id (e.g. `repo/my-app`);
/// the resulting library-api id is `<component_id>/public-api`.
///
/// Source files that fail to parse are silently skipped — the analyser
/// is conservative and prefers emitting nothing for a malformed file
/// over panicking the pipeline.
pub fn extract_elixir_surface(
    component_id: &str,
    inputs: &ElixirSourceInputs,
) -> ElixirSurfaceOutput {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elixir::LANGUAGE.into())
        .expect("tree-sitter-elixir language loads");

    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();
    let mut contracts: Vec<Contract> = Vec::new();
    let mut implemented_contracts: Vec<ImplementedContract> = Vec::new();

    // Sort sources by path for deterministic output ordering.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, bytes) in sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Some(tree) = parser.parse(text, None) else {
            continue;
        };
        let root = tree.root_node();
        extract_from_root(
            &root,
            text,
            bytes,
            rel_path,
            &mut bindings,
            &mut pub_items,
            &mut contracts,
            &mut implemented_contracts,
        );
    }

    let project_name = inputs
        .mix_exs
        .as_ref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(extract_mix_project_name);

    let effective_id = project_name.as_deref().unwrap_or(component_id);

    let library_apis: Vec<LibraryApi> = if pub_items.is_empty() {
        Vec::new()
    } else {
        let mut sorted = pub_items.clone();
        sorted.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.name.cmp(&b.name)));
        let api_id = format!("{effective_id}/public-api");
        let api_fp = library_api_fingerprint(&api_id, &sorted);
        let api = LibraryApi {
            id: api_id,
            kind: ContractKind::LibraryApi,
            language: "elixir".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    ElixirSurfaceOutput {
        bindings,
        library_apis,
        contracts,
        implemented_contracts,
    }
}

// ---------------------------------------------------------------------------
// Tree-sitter extraction helpers
// ---------------------------------------------------------------------------

/// Context carried during the walk of a single `defmodule` body.
struct ModuleContext<'a> {
    /// Module path segments (all but last of the dotted name).
    module_path: Vec<String>,
    /// Whether this module is a `defprotocol` (behaviour contract
    /// defining site). Recorded for potential Phase 3 use.
    #[allow(dead_code)]
    is_protocol: bool,
    /// Pending `@spec` text — set when a `@spec foo ...` is seen,
    /// consumed by the next `def`.
    pending_spec: Option<String>,
    /// Pending `@doc` text — set when a `@doc "..."` is seen,
    /// consumed by the next `def`.
    pending_doc: Option<String>,
    /// The source bytes for span computation.
    bytes: &'a [u8],
    /// Relative file path (for `Binding.file`).
    rel_path: &'a Path,
}

/// Walk the top-level nodes of a parsed Elixir source file.
#[allow(clippy::too_many_arguments)]
fn extract_from_root(
    root: &Node,
    text: &str,
    bytes: &[u8],
    rel_path: &Path,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
    contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        extract_top_level_node(
            &child,
            text,
            bytes,
            rel_path,
            bindings,
            pub_items,
            contracts,
            implemented_contracts,
        );
    }
}

/// Dispatch on a top-level (or recursively encountered) call node.
#[allow(clippy::too_many_arguments)]
fn extract_top_level_node(
    node: &Node,
    text: &str,
    bytes: &[u8],
    rel_path: &Path,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
    contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    if node.kind() != "call" {
        return;
    }
    let fn_name = first_child_text(node, text);
    match fn_name {
        Some("defmodule") | Some("defprotocol") | Some("defimpl") => {
            let is_protocol = fn_name == Some("defprotocol");
            let module_name = extract_module_name(node, text).unwrap_or_default();
            let (module_path, symbol) = split_module_name(&module_name);

            let span = node_byte_span(node, bytes);
            let content_sha = sha256_hex_of_range(bytes, span);

            // Emit a module-level binding.
            if !symbol.is_empty() {
                let module_binding = Binding {
                    language: "elixir".into(),
                    symbol: symbol.clone(),
                    file: rel_path.to_path_buf(),
                    span,
                    content_sha: content_sha.clone(),
                    visibility: Visibility::Conventional,
                    module_path: module_path.clone(),
                    attributes: BTreeMap::new(),
                };
                bindings.push(module_binding.clone());
                pub_items.push(PubItem {
                    name: symbol.clone(),
                    file: rel_path.to_path_buf(),
                    kind: PubItemKind::Mod,
                });

                // `defprotocol` → Contract with kind Behaviour.
                if is_protocol {
                    contracts.push(Contract {
                        id: format!("{module_name}/behaviour"),
                        kind: ContractKind::Behaviour,
                        fingerprint: content_sha.clone(),
                        definition_binding: module_binding,
                        description: format!("Erlang/Elixir behaviour protocol: {module_name}"),
                    });
                }
            }

            // Recurse into the module body.
            let mut ctx = ModuleContext {
                module_path,
                is_protocol,
                pending_spec: None,
                pending_doc: None,
                bytes,
                rel_path,
            };
            extract_module_body(
                node,
                text,
                &mut ctx,
                bindings,
                pub_items,
                contracts,
                implemented_contracts,
            );
        }
        _ => {}
    }
}

/// Walk the body of a `defmodule`/`defprotocol` node.
fn extract_module_body(
    module_node: &Node,
    text: &str,
    ctx: &mut ModuleContext<'_>,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
    contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    let Some(body) = find_do_block(module_node) else {
        return;
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "call" => {
                let fn_name = first_child_text(&child, text);
                match fn_name {
                    Some("def") => {
                        let fn_sym = extract_call_first_arg_name(&child, text).unwrap_or_default();
                        if fn_sym.is_empty() {
                            ctx.pending_spec = None;
                            ctx.pending_doc = None;
                            continue;
                        }
                        let span = node_byte_span(&child, ctx.bytes);
                        let content_sha = sha256_hex_of_range(ctx.bytes, span);
                        let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
                        if let Some(spec) = ctx.pending_spec.take() {
                            attributes.insert(ATTR_SPEC.to_string(), YamlValue::String(spec));
                        }
                        if let Some(doc) = ctx.pending_doc.take() {
                            attributes.insert(ATTR_DOC.to_string(), YamlValue::String(doc));
                        }
                        bindings.push(Binding {
                            language: "elixir".into(),
                            symbol: fn_sym.clone(),
                            file: ctx.rel_path.to_path_buf(),
                            span,
                            content_sha,
                            visibility: Visibility::Conventional,
                            module_path: ctx.module_path.clone(),
                            attributes,
                        });
                        pub_items.push(PubItem {
                            name: fn_sym,
                            file: ctx.rel_path.to_path_buf(),
                            kind: PubItemKind::Fn,
                        });
                    }
                    Some("defp") => {
                        // Private — excluded from surface, clear pending.
                        ctx.pending_spec = None;
                        ctx.pending_doc = None;
                    }
                    Some("@") => {
                        handle_attribute_call(&child, text, ctx, contracts, implemented_contracts);
                    }
                    Some("defprotocol") | Some("defimpl") => {
                        // Nested — recurse.
                        extract_top_level_node(
                            &child,
                            text,
                            ctx.bytes,
                            ctx.rel_path,
                            bindings,
                            pub_items,
                            contracts,
                            implemented_contracts,
                        );
                    }
                    _ => {
                        ctx.pending_spec = None;
                        ctx.pending_doc = None;
                    }
                }
            }
            "unary_operator" => {
                handle_unary_attribute(&child, text, ctx, contracts, implemented_contracts);
            }
            _ => {
                ctx.pending_spec = None;
                ctx.pending_doc = None;
            }
        }
    }
}

/// Handle `@`-prefixed attribute calls (`@spec`, `@doc`, `@behaviour`,
/// `@callback`) inside a module body. These appear as `call` nodes
/// whose first identifier is `@`.
fn handle_attribute_call(
    node: &Node,
    text: &str,
    ctx: &mut ModuleContext<'_>,
    contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    if let Some(arg) = extract_at_argument_text(node, text) {
        handle_attr_string(&arg, ctx, contracts, implemented_contracts);
    }
}

/// Handle `unary_operator` nodes that represent `@attr value` in some
/// tree-sitter-elixir grammar versions.
fn handle_unary_attribute(
    node: &Node,
    text: &str,
    ctx: &mut ModuleContext<'_>,
    contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    let Some(op) = node.child_by_field_name("operator") else {
        return;
    };
    if node_text(op, text) != "@" {
        return;
    }
    let Some(operand) = node.child_by_field_name("operand") else {
        return;
    };
    let operand_text = node_text(operand, text).to_string();
    handle_attr_string(&operand_text, ctx, contracts, implemented_contracts);
}

/// Split `arg` (the text of an `@attr ...` expression) into the leading
/// keyword and the residual value, normalising whitespace and optional
/// parentheses.
///
/// The two paths that feed this function produce different text:
///
/// * `call` path (`extract_at_argument_text`) strips the outer `(…)` of the
///   `arguments` node but keeps the inner text verbatim, e.g. `spec foo() :: t`
///   or `doc "hello"`.
/// * `unary_operator` path (`handle_unary_attribute`) yields the raw operand
///   slice which may be `spec(foo() :: t)` (paren after keyword, no space) or
///   `doc("hello")` (common after `mix format` re-flow), or the plain-space
///   forms.
///
/// Normalisation: scan for the first byte that is neither ASCII alphanumeric
/// nor `_` to find the keyword boundary; then strip a single leading `(` and
/// matching trailing `)` from the residual so both paths produce the same
/// canonical `(keyword, rest)` pair.
fn attr_keyword_and_rest(arg: &str) -> (&str, &str) {
    let split = arg
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(arg.len());
    let keyword = &arg[..split];
    let raw_rest = arg[split..].trim();
    // Strip one level of enclosing parentheses (e.g. `(foo() :: t)` → `foo() :: t`).
    let rest = raw_rest
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(raw_rest);
    (keyword, rest)
}

/// Shared attribute-text handler used by both call and unary_operator paths.
fn handle_attr_string(
    arg: &str,
    ctx: &mut ModuleContext<'_>,
    _contracts: &mut Vec<Contract>,
    implemented_contracts: &mut Vec<ImplementedContract>,
) {
    let (keyword, rest) = attr_keyword_and_rest(arg);
    if keyword == "spec" {
        ctx.pending_spec = Some(rest.trim().to_string());
    } else if keyword == "doc" {
        ctx.pending_doc = Some(strip_elixir_string_delimiters(rest.trim()));
    } else if keyword == "behaviour" {
        let behaviour_name = rest.trim().to_string();
        // A module declaring `@behaviour Mod` produces an ImplementedContract.
        // We also need a Binding to fulfil ImplementedContract.binding.
        let span = (0usize, 0usize);
        let binding = Binding {
            language: "elixir".into(),
            symbol: format!("behaviour:{behaviour_name}"),
            file: ctx.rel_path.to_path_buf(),
            span,
            content_sha: sha256_hex_of_range(ctx.bytes, span),
            visibility: Visibility::Conventional,
            module_path: ctx.module_path.clone(),
            attributes: BTreeMap::new(),
        };
        implemented_contracts.push(ImplementedContract {
            contract_id: format!("{behaviour_name}/behaviour"),
            role: BindingRole::ImplementingBinding,
            binding,
        });
    } else if keyword == "callback" {
        // `@callback` inside `defprotocol` body — Phase 2 records its
        // presence only; full signature capture is Phase 3.
        // Do not clear pending_spec/doc.
    } else {
        ctx.pending_spec = None;
        ctx.pending_doc = None;
    }
}

// ---------------------------------------------------------------------------
// Tree-sitter navigation helpers
// ---------------------------------------------------------------------------

/// Return the text of the first identifier/atom child of `node`.
fn first_child_text<'a>(node: &Node, text: &'a str) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "atom" {
            let start = child.start_byte();
            let end = child.end_byte();
            return text.get(start..end);
        }
    }
    None
}

/// Return the node's source text slice.
fn node_text<'a>(node: Node, text: &'a str) -> &'a str {
    text.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

/// Find the `do_block` child of a module/protocol node.
fn find_do_block<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.children(&mut node.walk())
        .find(|c| c.kind() == "do_block")
}

/// Extract the module/protocol name from a `defmodule`/`defprotocol`
/// call node. The name is the first argument (an alias or identifier).
fn extract_module_name(node: &Node, text: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "arguments" {
            let mut acursor = child.walk();
            for arg in child.children(&mut acursor) {
                let akind = arg.kind();
                if akind == "alias" || akind == "dot" || akind == "identifier" {
                    let start = arg.start_byte();
                    let end = arg.end_byte();
                    return text.get(start..end).map(str::to_string);
                }
            }
        }
        if kind == "alias" {
            let start = child.start_byte();
            let end = child.end_byte();
            return text.get(start..end).map(str::to_string);
        }
    }
    None
}

/// Extract the first argument name of a `def foo(...)` call.
/// Returns the function name identifier only (no arity info).
fn extract_call_first_arg_name(node: &Node, text: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let mut acursor = child.walk();
            for arg in child.children(&mut acursor) {
                match arg.kind() {
                    "identifier" | "atom" => {
                        return text
                            .get(arg.start_byte()..arg.end_byte())
                            .map(str::to_string);
                    }
                    "call" => return first_child_text(&arg, text).map(str::to_string),
                    // Skip operators, commas, etc.
                    _ => continue,
                }
            }
        }
    }
    None
}

/// Extract the text of the argument to an `@` call node, used for
/// `@spec`, `@doc`, `@behaviour`, `@callback`.
fn extract_at_argument_text(node: &Node, text: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "arguments" {
            let start = child.start_byte();
            let end = child.end_byte();
            let raw = text.get(start..end)?;
            let stripped = raw
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(raw);
            return Some(stripped.trim().to_string());
        }
    }
    None
}

/// Compute the half-open byte range `(start, end)` for a tree-sitter node.
fn node_byte_span(node: &Node, bytes: &[u8]) -> (usize, usize) {
    let start = node.start_byte();
    let end = node.end_byte();
    if start > end || end > bytes.len() {
        return (0, 0);
    }
    (start, end)
}

// ---------------------------------------------------------------------------
// Module name splitting
// ---------------------------------------------------------------------------

/// Split `"MyApp.Foo.Bar"` into `(["MyApp", "Foo"], "Bar")`.
/// Matches the §4 PR-8 spec: `module_path` is all but the last
/// dot-separated segment; `symbol` is the last segment.
///
/// An un-dotted name (e.g. `"Repo"`) becomes `([], "Repo")`.
pub fn split_module_name(name: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = name.split('.').collect();
    match parts.len() {
        0 => (Vec::new(), String::new()),
        1 => (Vec::new(), parts[0].to_string()),
        _ => {
            let (init, last) = parts.split_at(parts.len() - 1);
            (
                init.iter().map(|s| s.to_string()).collect(),
                last[0].to_string(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// mix.exs helpers (pragmatic regex, not full Elixir parse)
// ---------------------------------------------------------------------------

/// Extract the project name from `mix.exs` contents. Recognises the
/// canonical `def project do [..., app: :my_app, ...]` form.
///
/// `mix.exs` is Elixir code, so a full parse via tree-sitter is
/// possible, but a pragmatic regex is sufficient for the one field we
/// need at Phase 2. A malformed or non-matching file returns `None`.
pub fn extract_mix_project_name(contents: &str) -> Option<String> {
    let re = regex::Regex::new(r"app:\s*:([A-Za-z_][A-Za-z0-9_]*)").ok()?;
    let cap = re.captures(contents)?;
    Some(cap[1].replace('_', "-"))
}

/// Extract path-dep targets from `mix.exs`. Recognises the canonical:
///
/// ```text
/// defp deps do
///   [
///     {:foo, path: "../foo"},
///     {:bar, "~> 1.0"},
///   ]
/// end
/// ```
///
/// Uses a pragmatic regex (per §4 PR-8: "pragmatic regex; mix.exs is
/// Elixir code"). A regex cannot handle all edge cases (nested
/// brackets, multi-line continuation) but covers the overwhelming
/// majority of real-world `deps/0` bodies.
pub fn extract_mix_path_deps(contents: &str) -> Vec<PathBuf> {
    let re = match regex::Regex::new(r#"path:\s*"([^"]+)""#) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    re.captures_iter(contents)
        .map(|cap| PathBuf::from(&cap[1]))
        .collect()
}

// ---------------------------------------------------------------------------
// SHA / fingerprint helpers
// ---------------------------------------------------------------------------

/// SHA-256 hex of the bytes covered by `span`.
pub fn sha256_hex_of_range(bytes: &[u8], span: (usize, usize)) -> String {
    let (start, end) = span;
    let slice: &[u8] = if start <= end && end <= bytes.len() {
        &bytes[start..end]
    } else {
        b""
    };
    let digest: [u8; 32] = Sha256::digest(slice).into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}

/// Stable canonical fingerprint over a sorted [`PubItem`] list.
fn library_api_fingerprint(api_id: &str, items: &[PubItem]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(api_id.as_bytes());
    hasher.update([0u8]);
    for item in items {
        hasher.update(item.file.to_string_lossy().as_bytes());
        hasher.update([b'\t']);
        hasher.update(pub_item_kind_str(item.kind).as_bytes());
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

/// Stable wire string for a [`PubItemKind`].
pub fn pub_item_kind_str(kind: PubItemKind) -> &'static str {
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

/// Strip leading/trailing Elixir string delimiters from a `@doc` value.
fn strip_elixir_string_delimiters(s: &str) -> String {
    if let Some(inner) = s
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return inner.trim().to_string();
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return inner.to_string();
    }
    if let Some(inner) = s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.to_string();
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn source(rel: &str, body: &str) -> ElixirSourceInputs {
        ElixirSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
            mix_exs: None,
        }
    }

    // --- module_path derivation ---

    #[test]
    fn split_module_name_dotted() {
        let (path, sym) = split_module_name("MyApp.Foo.Bar");
        assert_eq!(path, vec!["MyApp".to_string(), "Foo".to_string()]);
        assert_eq!(sym, "Bar");
    }

    #[test]
    fn split_module_name_simple() {
        let (path, sym) = split_module_name("Repo");
        assert!(path.is_empty());
        assert_eq!(sym, "Repo");
    }

    #[test]
    fn split_module_name_two_segments() {
        let (path, sym) = split_module_name("MyApp.Foo");
        assert_eq!(path, vec!["MyApp".to_string()]);
        assert_eq!(sym, "Foo");
    }

    // --- def / defp extraction ---

    #[test]
    fn def_produces_binding() {
        let body = "defmodule MyApp.Repo do\n  def foo do\n    :ok\n  end\nend\n";
        let out = extract_elixir_surface("demo/comp", &source("lib/repo.ex", body));
        let fns: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(fns.contains(&"foo"), "expected 'foo' binding, got: {fns:?}");
        let foo = out.bindings.iter().find(|b| b.symbol == "foo").unwrap();
        assert!(matches!(foo.visibility, Visibility::Conventional));
        assert_eq!(foo.language, "elixir");
    }

    #[test]
    fn defp_excluded_from_surface() {
        let body = concat!(
            "defmodule MyApp.Repo do\n",
            "  def public_fn do\n    :ok\n  end\n\n",
            "  defp private_fn do\n    :hidden\n  end\n",
            "end\n"
        );
        let out = extract_elixir_surface("demo/comp", &source("lib/repo.ex", body));
        let fns: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(fns.contains(&"public_fn"), "expected 'public_fn'");
        assert!(
            !fns.contains(&"private_fn"),
            "defp 'private_fn' must not appear in bindings"
        );
    }

    #[test]
    fn module_binding_uses_last_segment_as_symbol() {
        let body = "defmodule MyApp.Foo.Bar do\nend\n";
        let out = extract_elixir_surface("demo/comp", &source("lib/bar.ex", body));
        let module_binding = out.bindings.iter().find(|b| b.symbol == "Bar");
        assert!(module_binding.is_some(), "expected module binding 'Bar'");
        let mb = module_binding.unwrap();
        assert_eq!(mb.module_path, vec!["MyApp".to_string(), "Foo".to_string()]);
    }

    #[test]
    fn def_has_conventional_visibility() {
        let body = "defmodule Foo do\n  def bar do\n    :ok\n  end\nend\n";
        let out = extract_elixir_surface("demo/comp", &source("lib/foo.ex", body));
        let bar = out.bindings.iter().find(|b| b.symbol == "bar");
        assert!(bar.is_some());
        assert!(matches!(bar.unwrap().visibility, Visibility::Conventional));
    }

    // --- defprotocol → behaviour contract ---

    #[test]
    fn defprotocol_emits_behaviour_contract() {
        let body = concat!(
            "defprotocol Stringable do\n",
            "  @callback to_string(t) :: String.t()\n",
            "end\n"
        );
        let out = extract_elixir_surface("demo/comp", &source("lib/stringable.ex", body));
        assert!(
            !out.contracts.is_empty(),
            "expected at least one behaviour contract"
        );
        let contract = out
            .contracts
            .iter()
            .find(|c| c.kind == ContractKind::Behaviour);
        assert!(contract.is_some(), "expected ContractKind::Behaviour");
        assert!(
            contract.unwrap().id.contains("Stringable"),
            "contract id should contain module name, got: {}",
            contract.unwrap().id
        );
    }

    // --- library API ---

    #[test]
    fn library_api_emitted_when_bindings_present() {
        let body = "defmodule Foo do\n  def bar do\n    :ok\n  end\nend\n";
        let out = extract_elixir_surface("my/comp", &source("lib/foo.ex", body));
        // library_api should include id starting with my/comp or the mix name
        assert_eq!(out.library_apis.len(), 1);
    }

    // --- mix.exs helpers ---

    #[test]
    fn extract_mix_project_name_standard() {
        let mix = concat!(
            "defmodule MyApp.MixProject do\n",
            "  use Mix.Project\n\n",
            "  def project do\n",
            "    [\n",
            "      app: :my_app,\n",
            "      version: \"0.1.0\",\n",
            "    ]\n",
            "  end\n",
            "end\n"
        );
        let name = extract_mix_project_name(mix);
        assert_eq!(name, Some("my-app".to_string()));
    }

    #[test]
    fn extract_mix_path_deps_standard() {
        let mix = concat!(
            "defp deps do\n",
            "  [\n",
            "    {:sibling, path: \"../sibling\"},\n",
            "    {:other, \"~> 1.0\"},\n",
            "  ]\n",
            "end\n"
        );
        let deps = extract_mix_path_deps(mix);
        assert_eq!(deps, vec![PathBuf::from("../sibling")]);
    }

    #[test]
    fn extract_mix_path_deps_multiple() {
        let mix = concat!(
            "defp deps do\n",
            "  [\n",
            "    {:a, path: \"../a\"},\n",
            "    {:b, path: \"../b\"},\n",
            "    {:c, \"~> 1.0\"},\n",
            "  ]\n",
            "end\n"
        );
        let deps = extract_mix_path_deps(mix);
        assert_eq!(deps, vec![PathBuf::from("../a"), PathBuf::from("../b")]);
    }

    #[test]
    fn extract_mix_path_deps_empty() {
        let mix = concat!(
            "defp deps do\n",
            "  [\n",
            "    {:telemetry, \"~> 1.0\"},\n",
            "  ]\n",
            "end\n"
        );
        let deps = extract_mix_path_deps(mix);
        assert!(deps.is_empty());
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "elixir-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }

    // --- @spec / @doc attribute extraction (unary_operator path) ---

    /// Standard plain-space form: `@spec foo() :: :ok` and `@doc "hello"`.
    /// Exercises the path that tree-sitter-elixir may parse as `unary_operator`.
    #[test]
    fn spec_and_doc_attributes_attached_to_def() {
        let body = concat!(
            "defmodule Foo do\n",
            "  @spec foo() :: :ok\n",
            "  @doc \"hello\"\n",
            "  def foo, do: :ok\n",
            "end\n"
        );
        let out = extract_elixir_surface("demo/comp", &source("lib/foo.ex", body));
        let foo = out
            .bindings
            .iter()
            .find(|b| b.symbol == "foo")
            .expect("expected 'foo' binding");
        assert!(
            foo.attributes.contains_key(ATTR_SPEC),
            "expected ATTR_SPEC on foo, got attributes: {:?}",
            foo.attributes
        );
        assert!(
            foo.attributes.contains_key(ATTR_DOC),
            "expected ATTR_DOC on foo, got attributes: {:?}",
            foo.attributes
        );
        let spec_val = foo.attributes.get(ATTR_SPEC).unwrap();
        assert!(
            spec_val
                .as_str()
                .map(|s| s.contains("foo()"))
                .unwrap_or(false),
            "ATTR_SPEC should contain function signature, got: {spec_val:?}"
        );
        let doc_val = foo.attributes.get(ATTR_DOC).unwrap();
        assert_eq!(
            doc_val.as_str(),
            Some("hello"),
            "ATTR_DOC should be the bare string without delimiters"
        );
    }

    /// Parenthesised form: `@spec(foo() :: :ok)` and `@doc("hello")`.
    /// This is the form produced by `mix format` in some Elixir versions and
    /// exercises the paren-stripping path in `attr_keyword_and_rest`.
    #[test]
    fn spec_and_doc_parenthesised_forms() {
        let body = concat!(
            "defmodule Bar do\n",
            "  @spec(bar() :: :ok)\n",
            "  @doc(\"world\")\n",
            "  def bar, do: :ok\n",
            "end\n"
        );
        let out = extract_elixir_surface("demo/comp", &source("lib/bar.ex", body));
        let bar = out
            .bindings
            .iter()
            .find(|b| b.symbol == "bar")
            .expect("expected 'bar' binding");
        assert!(
            bar.attributes.contains_key(ATTR_SPEC),
            "expected ATTR_SPEC on bar (paren form), got attributes: {:?}",
            bar.attributes
        );
        assert!(
            bar.attributes.contains_key(ATTR_DOC),
            "expected ATTR_DOC on bar (paren form), got attributes: {:?}",
            bar.attributes
        );
        let doc_val = bar.attributes.get(ATTR_DOC).unwrap();
        assert_eq!(
            doc_val.as_str(),
            Some("world"),
            "ATTR_DOC (paren form) should be the bare string without delimiters"
        );
    }
}
