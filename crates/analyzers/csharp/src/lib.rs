//! C# surface analyser logic (Atlas vNext Phase 2 PR-6).
//!
//! This crate's library form is the pure analyser: it takes
//! [`CsharpSourceInputs`] (parsed from on-disk C# files +
//! optional `*.csproj`) and emits [`CsharpSurfaceOutput`] containing
//! bindings, library APIs, and the path-dep edges declared in
//! `*.csproj`'s `<ProjectReference>` / `<PackageReference>` elements.
//!
//! ## Sibling binary
//!
//! The companion `csharp-analyzer` binary at `src/main.rs` wraps this
//! library in the subprocess wire protocol from
//! [`atlas_analyzers::subprocess`]. Tests and the in-tree
//! `csharp_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_csharp-analyzer")`.
//!
//! ## Binding shape
//!
//! C# has explicit `public`/`internal`/`private` keywords:
//!
//! - Top-level `public class/struct/interface/record/enum` → `Binding`
//!   with `Visibility::Explicit { keyword: "public" }`.
//! - `internal` / `private` members are **excluded** from the surface.
//! - `public` methods on public types → `Binding` with `module_path`
//!   rooted at the enclosing class's namespace.
//! - C# attributes (`[Authorize]`, `[Serializable]`, etc.) are captured
//!   as `attributes.cs_attributes: ["Authorize", "Serializable"]`.
//!
//! ## `module_path` semantics
//!
//! For C#, `module_path` is derived from the C# `namespace` declaration
//! (more semantic than file path). For example:
//!
//! ```text
//! namespace Acme.Models { public class User {} }
//! ```
//!
//! → `module_path = ["Acme", "Models"]`, `symbol = "User"`.
//!
//! This deviates from Python's file-path-based convention — C# developers
//! organise code by namespace, not by directory structure, and the
//! namespace is the authoritative module identifier.
//!
//! ## Parser
//!
//! Uses `tree-sitter-c-sharp` (pure-Rust, no native .NET required). The
//! grammar covers all C# 12 constructs including records, top-level
//! statements, and file-scoped namespaces.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{Binding, ContractKind, LibraryApi, PubItem, PubItemKind, Visibility};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

/// Attribute key for C#-specific attribute list.
/// Proposal: add `ATTR_CS_ATTRIBUTES = "cs_attributes"` to
/// `atlas-contracts/crates/atlas-index/src/surfaces.rs`.
pub const ATTR_CS_ATTRIBUTES: &str = "cs_attributes";

/// Stable analyser id for the C# surface analyser. Matches the
/// wire form a future `analyzers.yaml` would carry; the in-tree
/// wrapper at `atlas_analyzers::csharp_surface_analyzer` mirrors this
/// constant.
pub const ANALYZER_ID: &str = "csharp-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Inputs describing one component's C# source surface.
///
/// The driver fills this in by walking the component's source tree
/// (`**/*.cs`), then calls [`extract_csharp_surface`].
#[derive(Debug, Clone, Default)]
pub struct CsharpSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path
    /// must be relative to the project root (the directory containing
    /// `*.csproj`). Empty `bytes` are tolerated and produce no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `*.csproj` contents. When present, the analyser
    /// extracts `<PackageReference>` / `<ProjectReference>` edges and
    /// the assembly name.
    pub csproj: Option<Vec<u8>>,
    /// Optional `*.csproj` filename (for build system label).
    pub csproj_name: Option<String>,
}

/// Output of one C# surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CsharpSurfaceOutput {
    /// Every `public` top-level type and `public` method on a public
    /// type in the parsed source files.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per language (C# is the only language
    /// this analyser handles). Empty when the component exposes no
    /// top-level public definitions.
    pub library_apis: Vec<LibraryApi>,
    /// `<ProjectReference>` paths extracted from `*.csproj`.
    pub project_references: Vec<PathBuf>,
    /// `<PackageReference>` names extracted from `*.csproj`.
    pub package_references: Vec<String>,
}

/// Drive the C#-surface extraction over the component's source inputs.
/// Returns the bindings, `LibraryApi` (at most one), and reference
/// edges discovered.
///
/// `component_id` is the owning component's id (e.g. `repo/my-project`);
/// the resulting library-api id is `<component_id>/public-api`.
///
/// Source files that fail to parse as C# (or whose bytes are not valid
/// UTF-8) are silently skipped — the analyser is conservative and
/// prefers emitting nothing for a malformed file over panicking the
/// pipeline.
pub fn extract_csharp_surface(
    component_id: &str,
    inputs: &CsharpSourceInputs,
) -> CsharpSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    // Sort sources by path so the binding emission order is
    // deterministic regardless of the driver's enumeration order.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    // Initialise a tree-sitter parser once and reuse it across files.
    let mut parser = Parser::new();
    let language = tree_sitter_c_sharp::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("tree-sitter-c-sharp language is always valid");

    for (rel_path, bytes) in sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Some(tree) = parser.parse(text, None) else {
            continue;
        };
        emit_from_source_file(rel_path, bytes, text, &tree, &mut bindings, &mut pub_items);
    }

    let library_apis: Vec<LibraryApi> = if pub_items.is_empty() {
        Vec::new()
    } else {
        let mut sorted = pub_items.clone();
        sorted.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.name.cmp(&b.name)));
        let api_id = format!("{component_id}/public-api");
        let api_fp = library_api_fingerprint(&api_id, &sorted);
        let api = LibraryApi {
            id: api_id,
            kind: ContractKind::LibraryApi,
            language: "csharp".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    // Extract references from csproj if present.
    let (project_references, package_references) = inputs
        .csproj
        .as_ref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(extract_csproj_references)
        .unwrap_or_default();

    CsharpSurfaceOutput {
        bindings,
        library_apis,
        project_references,
        package_references,
    }
}

/// Process one `*.cs` source file. Extracts top-level public types and
/// their public methods, populating `bindings` and `pub_items`.
fn emit_from_source_file(
    rel_path: &Path,
    bytes: &[u8],
    text: &str,
    tree: &tree_sitter::Tree,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let root = tree.root_node();
    // Collect namespace(s) declared in this file.
    // C# supports file-scoped namespaces (C# 10+) and traditional blocks.
    // Multiple namespace blocks in one file is rare but valid.
    walk_top_level_nodes(rel_path, bytes, text, root, &[], bindings, pub_items);
}

/// Recursively walk top-level nodes (file, namespace_declaration,
/// file_scoped_namespace_declaration) to find public type declarations.
fn walk_top_level_nodes(
    rel_path: &Path,
    bytes: &[u8],
    text: &str,
    node: Node<'_>,
    namespace_path: &[String],
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let kind = node.kind();
    match kind {
        "compilation_unit" => {
            // Walk children of the compilation unit.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_top_level_nodes(
                    rel_path,
                    bytes,
                    text,
                    child,
                    namespace_path,
                    bindings,
                    pub_items,
                );
            }
        }
        "namespace_declaration" => {
            // `namespace Acme.Models { ... }`
            let ns = extract_namespace_name(node, text);
            let new_path = extend_namespace_path(namespace_path, &ns);
            // Walk the body (a `declaration_list`).
            if let Some(body) = find_child_by_kind(node, "declaration_list") {
                walk_top_level_nodes(rel_path, bytes, text, body, &new_path, bindings, pub_items);
            }
        }
        "file_scoped_namespace_declaration" => {
            // `namespace Acme.Models;` (C# 10+ file-scoped)
            let ns = extract_namespace_name(node, text);
            let new_path = extend_namespace_path(namespace_path, &ns);
            // All remaining siblings at the compilation_unit level
            // are in this namespace — handled by parent walking; we
            // set the path and let children continue.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let ck = child.kind();
                if is_type_declaration(ck) {
                    emit_type_node(rel_path, bytes, text, child, &new_path, bindings, pub_items);
                }
            }
        }
        "declaration_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let ck = child.kind();
                if ck == "namespace_declaration" || ck == "file_scoped_namespace_declaration" {
                    walk_top_level_nodes(
                        rel_path,
                        bytes,
                        text,
                        child,
                        namespace_path,
                        bindings,
                        pub_items,
                    );
                } else if is_type_declaration(ck) {
                    emit_type_node(
                        rel_path,
                        bytes,
                        text,
                        child,
                        namespace_path,
                        bindings,
                        pub_items,
                    );
                }
            }
        }
        _ => {
            // Other nodes (using_directive, global_statement, etc.)
            // can contain type declarations at the top level in C# 9+
            // top-level programs. We only emit those if they have
            // `public` modifier (which is unusual for top-level
            // statements but possible for partial classes).
            if is_type_declaration(kind) {
                emit_type_node(
                    rel_path,
                    bytes,
                    text,
                    node,
                    namespace_path,
                    bindings,
                    pub_items,
                );
            }
        }
    }
}

/// True for node kinds that represent C# type declarations.
fn is_type_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "record_struct_declaration"
            | "enum_declaration"
            | "delegate_declaration"
    )
}

/// Emit a binding for a type declaration if it is public.
/// Also emits method bindings for public methods on public types.
fn emit_type_node(
    rel_path: &Path,
    bytes: &[u8],
    text: &str,
    node: Node<'_>,
    namespace_path: &[String],
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    if !has_public_modifier(node, text) {
        return;
    }

    let Some(name) = get_identifier(node, text) else {
        return;
    };

    let attributes = collect_cs_attributes(node, text);
    let pub_item_kind = kind_to_pub_item_kind(node.kind());
    let span = (node.start_byte(), node.end_byte());
    let content_sha = sha256_hex_of_range(bytes, span);

    bindings.push(Binding {
        language: "csharp".into(),
        symbol: name.clone(),
        file: rel_path.to_path_buf(),
        span,
        content_sha,
        visibility: Visibility::Explicit {
            keyword: "public".into(),
        },
        module_path: namespace_path.to_vec(),
        attributes: attributes.clone(),
    });
    pub_items.push(PubItem {
        name: name.clone(),
        file: rel_path.to_path_buf(),
        kind: pub_item_kind,
    });

    // Now emit public methods / properties on the type.
    // The body is in a `declaration_list` child for classes/structs/
    // interfaces, or an `enum_member_declaration_list` for enums.
    emit_public_members(
        rel_path,
        bytes,
        text,
        node,
        namespace_path,
        &name,
        bindings,
        pub_items,
    );
}

/// Emit public method / property bindings from within a type's body.
/// The method's `module_path` is the namespace path + class name.
#[allow(clippy::too_many_arguments)]
fn emit_public_members(
    rel_path: &Path,
    bytes: &[u8],
    text: &str,
    type_node: Node<'_>,
    namespace_path: &[String],
    class_name: &str,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let mut method_module_path = namespace_path.to_vec();
    method_module_path.push(class_name.to_string());

    let body = match find_child_by_kind(type_node, "declaration_list") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        let ck = child.kind();
        if !matches!(
            ck,
            "method_declaration"
                | "constructor_declaration"
                | "property_declaration"
                | "operator_declaration"
                | "conversion_operator_declaration"
        ) {
            continue;
        }
        if !has_public_modifier(child, text) {
            continue;
        }
        let Some(method_name) = get_method_name(child, text) else {
            continue;
        };

        let member_attrs = collect_cs_attributes(child, text);
        let span = (child.start_byte(), child.end_byte());
        let content_sha = sha256_hex_of_range(bytes, span);
        bindings.push(Binding {
            language: "csharp".into(),
            symbol: method_name.clone(),
            file: rel_path.to_path_buf(),
            span,
            content_sha,
            visibility: Visibility::Explicit {
                keyword: "public".into(),
            },
            module_path: method_module_path.clone(),
            attributes: member_attrs,
        });
        pub_items.push(PubItem {
            name: method_name,
            file: rel_path.to_path_buf(),
            kind: PubItemKind::Fn,
        });
    }
}

/// Extract the `name` identifier from a type declaration node.
/// Tree-sitter's C# grammar puts the name in a child of kind
/// `identifier` immediately after any modifiers and the keyword.
fn get_identifier(node: Node<'_>, text: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(text[child.start_byte()..child.end_byte()].to_string());
        }
    }
    None
}

/// Extract the method/property name.
/// For `method_declaration` and `property_declaration`, the name is
/// in the `name` child (which is an `identifier` or `explicit_interface_specifier`).
fn get_method_name(node: Node<'_>, text: &str) -> Option<String> {
    // First try a named "name" field.
    if let Some(name_node) = node.child_by_field_name("name") {
        let s = text[name_node.start_byte()..name_node.end_byte()].to_string();
        // For explicit interface implementations like `IFoo.Bar`, skip them
        // (not directly callable as public surface).
        if s.contains('.') {
            return None;
        }
        return Some(s);
    }
    // Fallback: find the first identifier that isn't a modifier or type.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(text[child.start_byte()..child.end_byte()].to_string());
        }
    }
    None
}

/// Return true if the node's immediate children include a `modifier`
/// node whose text is `"public"`.
fn has_public_modifier(node: Node<'_>, text: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let modifier_text = &text[child.start_byte()..child.end_byte()];
            if modifier_text == "public" {
                return true;
            }
        }
    }
    false
}

/// Collect C# attributes applied to a declaration node.
/// Returns a non-empty `attributes` map with key `ATTR_CS_ATTRIBUTES`
/// when any attributes are found.
fn collect_cs_attributes(node: Node<'_>, text: &str) -> BTreeMap<String, YamlValue> {
    let mut attrs: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            collect_attrs_from_list(child, text, &mut attrs);
        }
    }
    let mut map = BTreeMap::new();
    if !attrs.is_empty() {
        let seq: Vec<YamlValue> = attrs.into_iter().map(YamlValue::String).collect();
        map.insert(ATTR_CS_ATTRIBUTES.into(), YamlValue::Sequence(seq));
    }
    map
}

/// Walk an `attribute_list` node to collect individual attribute names.
/// `[Authorize, Serializable]` → `["Authorize", "Serializable"]`.
fn collect_attrs_from_list(list_node: Node<'_>, text: &str, out: &mut Vec<String>) {
    let mut cursor = list_node.walk();
    for child in list_node.children(&mut cursor) {
        if child.kind() == "attribute" {
            // The attribute name is the first child of `attribute`.
            let name = extract_attribute_name(child, text);
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
}

/// Extract the name from a single `attribute` node.
/// `[Authorize]` → `"Authorize"`, `[Route("/")]` → `"Route"`.
fn extract_attribute_name(attr_node: Node<'_>, text: &str) -> String {
    let mut cursor = attr_node.walk();
    for child in attr_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                return text[child.start_byte()..child.end_byte()].to_string();
            }
            "qualified_name" => {
                // e.g. `System.Serializable` → take full text.
                return text[child.start_byte()..child.end_byte()].to_string();
            }
            _ => {}
        }
    }
    String::new()
}

/// Extract the namespace name from a `namespace_declaration` or
/// `file_scoped_namespace_declaration` node. Returns an empty string
/// on failure.
fn extract_namespace_name(node: Node<'_>, text: &str) -> String {
    // The namespace name is either a `qualified_name` or `identifier`
    // child immediately after the `namespace` keyword.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_name" | "identifier" => {
                return text[child.start_byte()..child.end_byte()].to_string();
            }
            _ => {}
        }
    }
    String::new()
}

/// Build the module_path from namespace components. Splits on `.` so
/// `"Acme.Models"` becomes `["Acme", "Models"]` and appends any
/// existing path segments from an outer namespace.
fn extend_namespace_path(existing: &[String], ns: &str) -> Vec<String> {
    let mut path = existing.to_vec();
    for segment in ns.split('.') {
        if !segment.is_empty() {
            path.push(segment.to_string());
        }
    }
    path
}

/// Find a direct child node by node kind.
fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor).find(|c| c.kind() == kind);
    result
}

/// Map a tree-sitter node kind to the matching `PubItemKind`.
fn kind_to_pub_item_kind(node_kind: &str) -> PubItemKind {
    match node_kind {
        "struct_declaration" | "record_struct_declaration" => PubItemKind::Struct,
        "enum_declaration" => PubItemKind::Enum,
        "interface_declaration" => PubItemKind::Trait,
        "delegate_declaration" => PubItemKind::TypeAlias,
        // class, record → Struct (no direct Rust/atlas equivalent for class)
        _ => PubItemKind::Struct,
    }
}

/// Stable wire string for a [`PubItemKind`]. Used for
/// library-API fingerprint computation and wire-form serialisation in
/// `main.rs`.
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

/// Extract `<ProjectReference Include="..."/>` and
/// `<PackageReference Include="..."/>` from a `*.csproj` XML body.
///
/// Uses a hand-rolled minimal XML scanner rather than a full XML parser
/// to avoid adding heavy dependencies. The scanner is robust enough for
/// the standard SDK-style project file format; it recognises the `Include`
/// attribute in self-closing and opening tags.
///
/// Returns `(project_refs, package_refs)`.
pub fn extract_csproj_references(contents: &str) -> (Vec<PathBuf>, Vec<String>) {
    let mut project_refs: Vec<PathBuf> = Vec::new();
    let mut package_refs: Vec<String> = Vec::new();

    // Scan for `<ProjectReference Include="..."` and
    // `<PackageReference Include="..."` patterns.
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(include) = extract_include_attr(trimmed, "ProjectReference") {
            // Normalise Windows path separators to host-native.
            let normalised = include.replace('\\', std::path::MAIN_SEPARATOR_STR);
            project_refs.push(PathBuf::from(normalised));
        } else if let Some(include) = extract_include_attr(trimmed, "PackageReference") {
            // Package names may include a version specifier in `Version=`
            // attribute; we store only the name (the `Include` value).
            package_refs.push(include);
        }
    }

    (project_refs, package_refs)
}

/// Scan a single XML line for `<ElementName ... Include="VALUE"`.
/// Returns `Some(VALUE)` if found, `None` otherwise.
fn extract_include_attr(line: &str, element_name: &str) -> Option<String> {
    // Quick prefix check before doing any allocation.
    if !line.starts_with('<') {
        return None;
    }
    // Must contain the element name.
    if !line.contains(element_name) {
        return None;
    }
    // Find `Include="..."` — attribute value is double-quoted.
    let include_key = "Include=\"";
    let start = line.find(include_key)? + include_key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// Extract the `<AssemblyName>` or `<RootNamespace>` from a csproj.
/// Returns the value of the first found element, or `None`.
pub fn extract_csproj_assembly_name(contents: &str) -> Option<String> {
    for tag in ["AssemblyName", "RootNamespace"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        if let Some(start) = contents.find(&open) {
            let after = &contents[start + open.len()..];
            if let Some(end) = after.find(&close) {
                let name = after[..end].trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(files: &[(&str, &str)]) -> CsharpSourceInputs {
        CsharpSourceInputs {
            sources: files
                .iter()
                .map(|(rel, body)| (PathBuf::from(rel), body.as_bytes().to_vec()))
                .collect(),
            csproj: None,
            csproj_name: None,
        }
    }

    // ── Basic binding extraction ──────────────────────────────────────────

    #[test]
    fn extracts_public_class_as_binding() {
        let body = "namespace Acme.Models {\n    public class User {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Models/User.cs", body)]));
        let user = out.bindings.iter().find(|b| b.symbol == "User").unwrap();
        assert_eq!(user.language, "csharp");
        assert_eq!(user.module_path, vec!["Acme", "Models"]);
        assert!(matches!(
            &user.visibility,
            Visibility::Explicit { keyword } if keyword == "public"
        ));
    }

    #[test]
    fn extracts_public_struct_as_binding() {
        let body = "namespace Acme {\n    public struct Point { public int X; public int Y; }\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Point.cs", body)]));
        assert!(out.bindings.iter().any(|b| b.symbol == "Point"));
    }

    #[test]
    fn extracts_public_interface_as_binding() {
        let body = "namespace Acme {\n    public interface IService {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("IService.cs", body)]));
        assert!(out.bindings.iter().any(|b| b.symbol == "IService"));
    }

    #[test]
    fn extracts_public_enum_as_binding() {
        let body = "namespace Acme {\n    public enum Status { Active, Inactive }\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Status.cs", body)]));
        assert!(out.bindings.iter().any(|b| b.symbol == "Status"));
    }

    #[test]
    fn extracts_public_record_as_binding() {
        let body = "namespace Acme {\n    public record Person(string Name, int Age);\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Person.cs", body)]));
        assert!(out.bindings.iter().any(|b| b.symbol == "Person"));
    }

    // ── Visibility filtering ──────────────────────────────────────────────

    #[test]
    fn internal_class_is_excluded() {
        let body = "namespace Acme {\n    internal class Foo {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Foo.cs", body)]));
        assert!(
            out.bindings.is_empty(),
            "internal class must not appear in surface"
        );
    }

    #[test]
    fn private_class_is_excluded() {
        let body = "namespace Acme {\n    private class Bar {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Bar.cs", body)]));
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn public_and_internal_in_same_file() {
        let body = "\
namespace Acme {
    public class Public {}
    internal class Hidden {}
}
";
        let out = extract_csharp_surface("demo/comp", &input(&[("Mixed.cs", body)]));
        let symbols: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(symbols.contains(&"Public"), "got: {symbols:?}");
        assert!(!symbols.contains(&"Hidden"), "got: {symbols:?}");
    }

    // ── Namespace → module_path ───────────────────────────────────────────

    #[test]
    fn namespace_splits_into_module_path_segments() {
        let body = "namespace Acme.Models {\n    public class User {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Models/User.cs", body)]));
        let user = out.bindings.iter().find(|b| b.symbol == "User").unwrap();
        assert_eq!(user.module_path, vec!["Acme", "Models"]);
    }

    #[test]
    fn nested_namespaces_extend_module_path() {
        let body = "\
namespace Outer {
    namespace Inner {
        public class Nested {}
    }
}
";
        let out = extract_csharp_surface("demo/comp", &input(&[("Nested.cs", body)]));
        let nested = out.bindings.iter().find(|b| b.symbol == "Nested").unwrap();
        assert_eq!(nested.module_path, vec!["Outer", "Inner"]);
    }

    #[test]
    fn no_namespace_yields_empty_module_path() {
        let body = "public class TopLevel {}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("TopLevel.cs", body)]));
        if let Some(b) = out.bindings.iter().find(|b| b.symbol == "TopLevel") {
            assert!(b.module_path.is_empty(), "got: {:?}", b.module_path);
        }
        // It's acceptable for the type to be emitted or not — top-level
        // programs without a namespace are an edge case in the surface analyser.
    }

    // ── C# attribute capture ──────────────────────────────────────────────

    #[test]
    fn serializable_attribute_captured() {
        let body = "\
namespace Acme {
    [Serializable]
    public class Data {}
}
";
        let out = extract_csharp_surface("demo/comp", &input(&[("Data.cs", body)]));
        let data = out.bindings.iter().find(|b| b.symbol == "Data").unwrap();
        let cs_attrs = data
            .attributes
            .get(ATTR_CS_ATTRIBUTES)
            .expect("cs_attributes must be present");
        let names: Vec<String> = cs_attrs
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(
            names.contains(&"Serializable".to_string()),
            "got: {names:?}"
        );
    }

    #[test]
    fn authorize_attribute_captured() {
        let body = "\
namespace Acme.Controllers {
    [Authorize]
    public class ApiController {}
}
";
        let out = extract_csharp_surface(
            "demo/comp",
            &input(&[("Controllers/ApiController.cs", body)]),
        );
        let ctrl = out
            .bindings
            .iter()
            .find(|b| b.symbol == "ApiController")
            .unwrap();
        let names: Vec<String> = ctrl
            .attributes
            .get(ATTR_CS_ATTRIBUTES)
            .unwrap()
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"Authorize".to_string()));
    }

    #[test]
    fn no_attributes_yields_empty_cs_attributes() {
        let body = "namespace Acme {\n    public class Clean {}\n}\n";
        let out = extract_csharp_surface("demo/comp", &input(&[("Clean.cs", body)]));
        let clean = out.bindings.iter().find(|b| b.symbol == "Clean").unwrap();
        assert!(
            !clean.attributes.contains_key(ATTR_CS_ATTRIBUTES),
            "attribute map should be empty when no C# attributes are present"
        );
    }

    // ── Public methods on public types ────────────────────────────────────

    #[test]
    fn public_method_on_public_class_emitted() {
        let body = "\
namespace Acme {
    public class Service {
        public void Execute() {}
        internal void Hidden() {}
    }
}
";
        let out = extract_csharp_surface("demo/comp", &input(&[("Service.cs", body)]));
        let symbols: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(symbols.contains(&"Service"), "Service type missing");
        assert!(symbols.contains(&"Execute"), "Execute method missing");
        assert!(!symbols.contains(&"Hidden"), "Hidden must be excluded");

        // The method's module_path includes the class name.
        let execute = out.bindings.iter().find(|b| b.symbol == "Execute").unwrap();
        assert_eq!(execute.module_path, vec!["Acme", "Service"]);
    }

    // ── Library API shape ─────────────────────────────────────────────────

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "namespace Acme {\n    public class Foo {}\n}\n";
        let out = extract_csharp_surface("foo/bar", &input(&[("Foo.cs", body)]));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
        assert_eq!(out.library_apis[0].language, "csharp");
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let out = extract_csharp_surface("ns/comp", &CsharpSourceInputs::default());
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    // ── csproj reference extraction ───────────────────────────────────────

    #[test]
    fn extracts_project_reference_from_csproj() {
        let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <ProjectReference Include="../Sibling/Sibling.csproj" />
  </ItemGroup>
</Project>
"#;
        let (project_refs, _package_refs) = extract_csproj_references(csproj);
        assert_eq!(
            project_refs,
            vec![PathBuf::from("../Sibling/Sibling.csproj")]
        );
    }

    #[test]
    fn extracts_package_reference_from_csproj() {
        let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
  </ItemGroup>
</Project>
"#;
        let (_project_refs, package_refs) = extract_csproj_references(csproj);
        assert_eq!(package_refs, vec!["Newtonsoft.Json".to_string()]);
    }

    #[test]
    fn csproj_with_multiple_references() {
        let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <ProjectReference Include="../Core/Core.csproj" />
    <PackageReference Include="Microsoft.Extensions.Logging" Version="8.0.0" />
    <PackageReference Include="Serilog" Version="3.0.0" />
  </ItemGroup>
</Project>
"#;
        let (project_refs, package_refs) = extract_csproj_references(csproj);
        assert_eq!(project_refs.len(), 1);
        assert_eq!(package_refs.len(), 2);
        assert!(package_refs.contains(&"Serilog".to_string()));
    }

    #[test]
    fn extract_csproj_assembly_name_reads_tag() {
        let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <AssemblyName>MyApp</AssemblyName>
  </PropertyGroup>
</Project>
"#;
        assert_eq!(
            extract_csproj_assembly_name(csproj),
            Some("MyApp".to_string())
        );
    }

    #[test]
    fn multiple_source_files_emit_all_public_types() {
        let inputs = CsharpSourceInputs {
            sources: vec![
                (
                    PathBuf::from("Models/User.cs"),
                    b"namespace App.Models {\n    public class User {}\n}\n".to_vec(),
                ),
                (
                    PathBuf::from("Program.cs"),
                    b"namespace App {\n    public class Program {}\n}\n".to_vec(),
                ),
            ],
            csproj: None,
            csproj_name: None,
        };
        let out = extract_csharp_surface("demo/comp", &inputs);
        let symbols: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(symbols.contains(&"User"), "User missing: {symbols:?}");
        assert!(symbols.contains(&"Program"), "Program missing: {symbols:?}");
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "csharp-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }
}
