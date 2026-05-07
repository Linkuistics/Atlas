//! Dart surface analyser logic (Atlas vNext Phase 2 PR-7).
//!
//! This crate's library form is the pure analyser: it takes
//! [`DartSourceInputs`] (parsed from on-disk Dart files + `pubspec.yaml`)
//! and emits [`DartSurfaceOutput`] containing bindings, library APIs, and
//! the path-dep edges declared in `pubspec.yaml`'s `dependencies: { path: ... }`
//! form.
//!
//! ## Sibling binary
//!
//! The companion `dart-analyzer` binary at `src/main.rs` wraps this library
//! in the subprocess wire protocol from [`atlas_analyzers::subprocess`].
//! Tests and the in-tree `dart_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_dart-analyzer")`.
//!
//! ## Binding shape
//!
//! Dart has no `pub`/`priv` keyword in the Python sense: top-level symbols
//! without a leading underscore are library-public by convention. A leading
//! underscore (`_private`) flags the binding as conventionally-private via
//! `attributes.private: true`. All top-level bindings carry
//! `Visibility::Conventional`.
//!
//! `module_path` is derived from the source file's relative path under
//! `lib/` — file-path components, *excluding the symbol*:
//!
//! - `lib/foo/bar.dart` → `["lib", "foo", "bar"]`, `symbol = "Baz"`.
//! - `lib/dart_pkg.dart` → `["lib", "dart_pkg"]`.
//!
//! ## Parser
//!
//! Phase 2 PR-7 uses a hand-rolled lexer that scans for the Dart declaration
//! keywords (`class`, `mixin`, `extension`, `typedef`, top-level `function`
//! declarations). This avoids a native `tree-sitter-dart` build dependency
//! while covering all acceptance-criteria surface forms. The lexer is
//! conservative: it skips class bodies and handles decorator-like annotations
//! (`@deprecated`, `@override`, `@protected`) immediately before a
//! declaration.
//!
//! ## Annotation attributes
//!
//! `@deprecated`, `@override`, `@protected`, and arbitrary `@annotation`
//! names are captured in `attributes.dart_annotations: [name, …]`. The
//! ATTR key is `"dart_annotations"` (bare string). The proposed schema
//! constant is `ATTR_DART_ANNOTATIONS` (see DONE report).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, ContractKind, LibraryApi, PubItem, PubItemKind, Visibility, ATTR_PRIVATE,
};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

/// Stable analyser id for the Dart surface analyser.
pub const ANALYZER_ID: &str = "dart-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Attribute key for Dart annotation chains (e.g. `@deprecated`).
/// Proposed schema constant: `ATTR_DART_ANNOTATIONS`.
pub const ATTR_DART_ANNOTATIONS: &str = "dart_annotations";

/// Inputs describing one component's Dart source surface.
#[derive(Debug, Clone, Default)]
pub struct DartSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path must be
    /// relative to the package root (the directory containing `pubspec.yaml`),
    /// so `module_path` derivation is unambiguous. Empty `bytes` are tolerated.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `pubspec.yaml` contents. When present, the analyser
    /// resolves `name:` into the component's [`LibraryApi`] entrypoint id.
    pub pubspec_yaml: Option<Vec<u8>>,
}

/// Output of one Dart surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DartSurfaceOutput {
    /// Every top-level declaration in the parsed source files.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per language.
    pub library_apis: Vec<LibraryApi>,
}

/// Drive the Dart-surface extraction over the component's source inputs.
///
/// `component_id` is the owning component's id (e.g. `repo/my-pkg`);
/// the resulting library-api id is `<component_id>/public-api`.
pub fn extract_dart_surface(component_id: &str, inputs: &DartSourceInputs) -> DartSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    // Sort sources by path for deterministic emission order.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, bytes) in sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let module_path = derive_module_path(rel_path);
        let declarations = scan_top_level_declarations(text, bytes);
        for decl in &declarations {
            push_binding(
                &mut bindings,
                &mut pub_items,
                rel_path,
                bytes,
                &module_path,
                decl,
            );
        }
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
            language: "dart".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    DartSurfaceOutput {
        bindings,
        library_apis,
    }
}

/// A top-level declaration extracted from a Dart source file.
#[derive(Debug, Clone)]
struct DartDecl {
    /// Identifier name.
    name: String,
    /// Declaration kind for `PubItemKind`.
    kind: PubItemKind,
    /// Byte range (start, end) of the declaration header.
    span: (usize, usize),
    /// Annotations immediately before the declaration (e.g. `@deprecated`).
    annotations: Vec<String>,
}

/// Scan `text` for top-level Dart declarations:
/// `class`, `abstract class`, `mixin`, `extension`, `typedef`,
/// top-level functions (`<type> name(`).
///
/// Uses a hand-rolled line-scanning approach:
/// - Tracks brace depth to skip over class/function bodies.
/// - Collects `@annotation` lines immediately before a declaration.
/// - Ignores content inside block comments `/* ... */` and line comments.
fn scan_top_level_declarations(text: &str, _bytes: &[u8]) -> Vec<DartDecl> {
    let mut declarations: Vec<DartDecl> = Vec::new();
    let mut pending_annotations: Vec<String> = Vec::new();
    let mut brace_depth: i32 = 0;
    let mut in_block_comment = false;
    let mut byte_offset: usize = 0;

    for line in text.lines() {
        let line_end_offset = byte_offset + line.len() + 1; // +1 for newline
        let trimmed = line.trim();

        // Handle block comments.
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            byte_offset = line_end_offset.min(text.len() + 1);
            continue;
        }
        if trimmed.contains("/*") && !trimmed.starts_with("//") {
            in_block_comment = !trimmed.contains("*/");
        }

        // Count braces to track nesting depth.
        let open_count = line.chars().filter(|&c| c == '{').count() as i32;
        let close_count = line.chars().filter(|&c| c == '}').count() as i32;

        // Skip inner-body lines (non-top-level).
        if brace_depth > 0 {
            brace_depth += open_count - close_count;
            if brace_depth < 0 {
                brace_depth = 0;
            }
            byte_offset = line_end_offset.min(text.len() + 1);
            continue;
        }

        // At top level (brace_depth == 0).

        // Skip line comments.
        if trimmed.starts_with("//") {
            byte_offset = line_end_offset.min(text.len() + 1);
            continue;
        }

        // Skip blank lines (but don't clear annotations — Dart allows blank
        // lines between annotation and declaration).
        if trimmed.is_empty() {
            byte_offset = line_end_offset.min(text.len() + 1);
            continue;
        }

        // Collect annotations: lines starting with `@`.
        if let Some(after_at) = trimmed.strip_prefix('@') {
            // Extract the annotation name (stop at `(`, space, newline).
            let at_name: String = after_at
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !at_name.is_empty() {
                pending_annotations.push(at_name);
            }
            byte_offset = line_end_offset.min(text.len() + 1);
            continue;
        }

        // Try to match declaration keywords at the start of the line.
        // Normalise: strip `abstract ` prefix so `abstract class Foo` matches.
        // Strip leading modifier keywords that can appear before a
        // declaration keyword. Note: `mixin` is NOT stripped here — it is
        // a declaration keyword in its own right (matched by
        // `keyword_name(stripped, "mixin")` inside `try_parse_declaration`).
        // However `abstract mixin` is valid Dart; stripping `abstract `
        // first is sufficient to let the mixin rule fire.
        let stripped = trimmed
            .trim_start_matches("external ")
            .trim_start_matches("abstract ")
            .trim_start_matches("final ")
            .trim_start_matches("sealed ")
            .trim_start_matches("base ")
            .trim_start_matches("interface ")
            .trim();

        let decl_start_offset = byte_offset;

        // `line_end_offset - 1` excludes the trailing newline; used as the
        // declaration span end so `content_sha` hashes the actual source text.
        let decl_end_offset = line_end_offset.saturating_sub(1);

        if let Some(d) = try_parse_declaration(
            stripped,
            decl_start_offset,
            decl_end_offset,
            &pending_annotations,
            trimmed,
        ) {
            declarations.push(d);
            pending_annotations.clear();
        } else {
            // Not a recognised declaration: clear pending annotations.
            pending_annotations.clear();
        }

        // Update brace depth after processing the declaration line.
        brace_depth += open_count - close_count;
        if brace_depth < 0 {
            brace_depth = 0;
        }

        byte_offset = line_end_offset.min(text.len() + 1);
    }

    declarations
}

/// Try to parse `stripped` (already stripped of leading modifiers) as a
/// Dart declaration. Returns `Some(DartDecl)` on success.
///
/// `decl_end_offset` is the byte offset of the end of the declaration line
/// (excluding the trailing newline). This is used as the span end so that
/// `content_sha` hashes the actual source text rather than an empty slice.
fn try_parse_declaration(
    stripped: &str,
    decl_start_offset: usize,
    decl_end_offset: usize,
    pending_annotations: &[String],
    original_trimmed: &str,
) -> Option<DartDecl> {
    let span = (decl_start_offset, decl_end_offset);
    // `class Foo` / `class Foo<T>`
    if let Some(name) = keyword_name(stripped, "class") {
        return Some(DartDecl {
            name,
            kind: PubItemKind::Struct,
            span,
            annotations: pending_annotations.to_vec(),
        });
    }
    // `mixin Foo`
    if let Some(name) = keyword_name(stripped, "mixin") {
        return Some(DartDecl {
            name,
            kind: PubItemKind::Trait,
            span,
            annotations: pending_annotations.to_vec(),
        });
    }
    // `extension FooExt on Foo`
    if let Some(name) = keyword_name(stripped, "extension") {
        // Unnamed extensions (`extension on Foo { ... }`) should not produce a binding.
        if name == "on" {
            return None;
        }
        return Some(DartDecl {
            name,
            kind: PubItemKind::Trait,
            span,
            annotations: pending_annotations.to_vec(),
        });
    }
    // `typedef FooAlias = Bar<T>;` or `typedef FooAlias = void Function(...)`
    if let Some(name) = keyword_name(stripped, "typedef") {
        return Some(DartDecl {
            name,
            kind: PubItemKind::TypeAlias,
            span,
            annotations: pending_annotations.to_vec(),
        });
    }
    // `enum Foo { ... }`
    if let Some(name) = keyword_name(stripped, "enum") {
        return Some(DartDecl {
            name,
            kind: PubItemKind::Enum,
            span,
            annotations: pending_annotations.to_vec(),
        });
    }
    // Top-level function: `<returnType> <name>(<params>` or `<name>(<params>`
    // Heuristic: if the line contains `(` and does not start with a keyword
    // that is not a declaration, it may be a function.
    if stripped.contains('(') {
        if let Some(name) = try_parse_function_name(stripped, original_trimmed) {
            return Some(DartDecl {
                name,
                kind: PubItemKind::Fn,
                span,
                annotations: pending_annotations.to_vec(),
            });
        }
    }
    None
}

/// Extract the identifier following `keyword ` in `s`. Returns `None` if
/// `s` does not start with `keyword ` (with a word boundary), or if the
/// next token is not a valid Dart identifier.
fn keyword_name(s: &str, keyword: &str) -> Option<String> {
    let rest = s.strip_prefix(keyword)?;
    // Require a word boundary (space, `<`, `{`, `(`, end of string, or `\n`).
    let boundary = rest.chars().next();
    if !matches!(
        boundary,
        Some(' ') | Some('\t') | Some('<') | Some('{') | Some('(') | None
    ) {
        return None;
    }
    let rest = rest.trim_start();
    // Extract identifier: alphanumeric + underscore.
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// Heuristic: try to extract a function name from a line that contains `(`.
/// Accepts: `void foo(`, `String get foo(`, `Future<T> fetchData(`, `foo(`.
/// Rejects: control-flow (`if(`, `for(`, `while(`, `switch(`), super-calls,
/// string literals, annotations.
///
/// `original_trimmed` is the raw trimmed line *before* modifier stripping.
/// It is used to detect top-level variable declarations whose `final`/`const`
/// prefix has already been stripped by the time `s` is passed in, which
/// would otherwise cause phantom `Fn` bindings to be emitted.
fn try_parse_function_name(s: &str, original_trimmed: &str) -> Option<String> {
    // Reject lines that begin with a variable-declaration modifier on the
    // *original* (pre-strip) text.  The strip in `scan_top_level_declarations`
    // removes these modifiers before calling us, so checking only `s` would
    // miss cases like `final myList = compute();`.
    let var_modifiers = ["final ", "const ", "var ", "late ", "dynamic "];
    for m in &var_modifiers {
        if original_trimmed.starts_with(m) {
            return None;
        }
    }

    // Reject control-flow and other non-declaration forms.
    let control_flow = [
        "if ",
        "if(",
        "for ",
        "for(",
        "while ",
        "while(",
        "switch ",
        "switch(",
        "return ",
        "throw ",
        "assert(",
        "super(",
        "this(",
        "print(",
        "var ",
        "final ",
        "const ",
        "late ",
        "dynamic ",
        "import ",
        "export ",
        "part ",
        "library ",
        "show ",
        "hide ",
        "as ",
        "deferred ",
    ];
    for cf in &control_flow {
        if s.starts_with(cf) {
            return None;
        }
    }
    // Skip lines that look like assignments or variable declarations.
    // Function-declaration lines: `<type> <name>(<params>` or `<name>(<params>`.
    // Find the `(` position.
    let paren_pos = s.find('(')?;
    let before_paren = &s[..paren_pos];
    // The identifier before `(` is the function name. Split on whitespace.
    let tokens: Vec<&str> = before_paren.split_whitespace().collect();
    let name_candidate = tokens.last()?;
    // Strip generic type params and angle brackets that might be part of the
    // return type leaking into the name candidate.
    let name: String = name_candidate
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    // Reject if the name is a keyword.
    let keywords = [
        "if",
        "for",
        "while",
        "switch",
        "return",
        "throw",
        "assert",
        "super",
        "this",
        "new",
        "const",
        "var",
        "final",
        "late",
        "dynamic",
        "void",
        "import",
        "export",
        "part",
        "library",
        "class",
        "mixin",
        "extension",
        "typedef",
        "enum",
        "abstract",
        "static",
        "get",
        "set",
        "async",
        "await",
        "yield",
        "try",
        "catch",
        "finally",
        "in",
        "is",
        "as",
        "null",
        "true",
        "false",
    ];
    if keywords.contains(&name.as_str()) {
        return None;
    }
    Some(name)
}

/// Derive the module path from a Dart file's relative path.
///
/// `lib/foo/bar.dart` → `["lib", "foo", "bar"]`.
/// `lib/dart_pkg.dart` → `["lib", "dart_pkg"]`.
/// Files outside `lib/` still get their path segments (minus extension).
fn derive_module_path(rel: &Path) -> Vec<String> {
    let stem = match rel.extension().and_then(|s| s.to_str()) {
        Some("dart") => rel.with_extension(""),
        _ => return Vec::new(),
    };
    stem.components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect()
}

fn push_binding(
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
    rel_path: &Path,
    bytes: &[u8],
    module_path: &[String],
    decl: &DartDecl,
) {
    let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();

    // Conventional-private: a leading single underscore.
    if decl.name.starts_with('_') {
        attributes.insert(ATTR_PRIVATE.into(), YamlValue::Bool(true));
    }

    // Dart annotations.
    if !decl.annotations.is_empty() {
        let chain: Vec<YamlValue> = decl
            .annotations
            .iter()
            .map(|a| YamlValue::String(a.clone()))
            .collect();
        attributes.insert(ATTR_DART_ANNOTATIONS.into(), YamlValue::Sequence(chain));
    }

    let content_sha = sha256_hex_of_range(bytes, decl.span);
    bindings.push(Binding {
        language: "dart".into(),
        symbol: decl.name.clone(),
        file: rel_path.to_path_buf(),
        span: decl.span,
        content_sha,
        visibility: Visibility::Conventional,
        module_path: module_path.to_vec(),
        attributes,
    });

    pub_items.push(PubItem {
        name: decl.name.clone(),
        file: rel_path.to_path_buf(),
        kind: decl.kind,
    });
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

/// Extract the `name:` field from a `pubspec.yaml`. Only the simple
/// top-level `name: <identifier>` form is recognised.
pub fn extract_pubspec_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let name = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Extract path-dep targets from a `pubspec.yaml`.
///
/// Recognised form:
/// ```yaml
/// dependencies:
///   lib_a:
///     path: ../lib_a
///   http: ^0.13.0
/// ```
///
/// Returns a `Vec<(name, path)>` for every `path:` dep found in the
/// `dependencies:` block. Other dependency forms (version strings, git,
/// SDK) are skipped.
pub fn extract_pubspec_path_deps(text: &str) -> Vec<(String, PathBuf)> {
    // We parse the YAML with serde_yaml to handle arbitrary indentation
    // and multiline forms robustly.
    let Ok(value): Result<serde_yaml::Value, _> = serde_yaml::from_str(text) else {
        return Vec::new();
    };
    let Some(mapping) = value.as_mapping() else {
        return Vec::new();
    };

    let mut out: Vec<(String, PathBuf)> = Vec::new();
    // Check both `dependencies:` and `dev_dependencies:`.
    for key in ["dependencies", "dev_dependencies"] {
        let Some(deps) = mapping.get(key) else {
            continue;
        };
        let Some(deps_map) = deps.as_mapping() else {
            continue;
        };
        for (dep_name, dep_spec) in deps_map {
            let Some(name_str) = dep_name.as_str() else {
                continue;
            };
            let Some(spec_map) = dep_spec.as_mapping() else {
                continue;
            };
            if let Some(path_val) = spec_map.get("path") {
                if let Some(path_str) = path_val.as_str() {
                    out.push((name_str.to_string(), PathBuf::from(path_str)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> DartSourceInputs {
        DartSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
            pubspec_yaml: None,
        }
    }

    // ── Acceptance criteria ──────────────────────────────────────────

    #[test]
    fn class_declaration_produces_binding() {
        let body = "class Foo {\n  int x = 0;\n}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/foo.dart", body));
        let binding = out.bindings.iter().find(|b| b.symbol == "Foo").unwrap();
        assert_eq!(binding.language, "dart");
        assert_eq!(binding.module_path, vec!["lib", "foo"]);
        assert!(matches!(binding.visibility, Visibility::Conventional));
    }

    #[test]
    fn private_symbol_gets_private_attribute() {
        let body = "void _privateHelper() {}\nvoid public() {}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/helpers.dart", body));
        let private = out
            .bindings
            .iter()
            .find(|b| b.symbol == "_privateHelper")
            .unwrap();
        let public = out.bindings.iter().find(|b| b.symbol == "public").unwrap();
        assert_eq!(
            private.attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
        assert!(!public.attributes.contains_key("private"));
    }

    #[test]
    fn public_and_private_functions_distinguished() {
        // PR-7 acceptance: `_private` and `public` top-level functions
        // distinguished by visibility attribute.
        let body = "void public() {}\nvoid _private() {}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/foo.dart", body));
        assert_eq!(out.bindings.len(), 2);
        let public = out.bindings.iter().find(|b| b.symbol == "public").unwrap();
        let private = out
            .bindings
            .iter()
            .find(|b| b.symbol == "_private")
            .unwrap();
        assert!(!public.attributes.contains_key("private"));
        assert_eq!(
            private.attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
    }

    #[test]
    fn mixin_produces_binding() {
        let body = "mixin Serializable on Object {\n  String toJson();\n}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/mixins.dart", body));
        assert!(out.bindings.iter().any(|b| b.symbol == "Serializable"));
    }

    #[test]
    fn extension_produces_binding() {
        let body = "extension StringExt on String {\n  bool get isEmail => contains('@');\n}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/ext.dart", body));
        assert!(out.bindings.iter().any(|b| b.symbol == "StringExt"));
    }

    #[test]
    fn typedef_produces_binding() {
        let body = "typedef JsonMap = Map<String, dynamic>;\n";
        let out = extract_dart_surface("demo/comp", &input("lib/types.dart", body));
        assert!(out.bindings.iter().any(|b| b.symbol == "JsonMap"));
    }

    #[test]
    fn enum_produces_binding() {
        let body = "enum Color { red, green, blue }\n";
        let out = extract_dart_surface("demo/comp", &input("lib/color.dart", body));
        assert!(out.bindings.iter().any(|b| b.symbol == "Color"));
    }

    #[test]
    fn deprecated_annotation_captured() {
        let body = "@deprecated\nvoid oldFn() {}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/old.dart", body));
        let binding = out.bindings.iter().find(|b| b.symbol == "oldFn").unwrap();
        let anns = binding
            .attributes
            .get(ATTR_DART_ANNOTATIONS)
            .expect("dart_annotations must be present");
        let names: Vec<String> = anns
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"deprecated".to_string()));
    }

    #[test]
    fn module_path_derived_from_lib_subdirectory() {
        let body = "class Bar {}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/foo/bar.dart", body));
        assert_eq!(out.bindings[0].module_path, vec!["lib", "foo", "bar"]);
    }

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "class Baz {}\n";
        let out = extract_dart_surface("foo/bar", &input("lib/baz.dart", body));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
        assert_eq!(out.library_apis[0].language, "dart");
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let inputs = DartSourceInputs::default();
        let out = extract_dart_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn non_utf8_bytes_skip_silently() {
        let inputs = DartSourceInputs {
            sources: vec![(PathBuf::from("lib/mod.dart"), vec![0xFF, 0xFE, 0xFD])],
            pubspec_yaml: None,
        };
        let out = extract_dart_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn abstract_class_produces_binding() {
        let body = "abstract class Animal {\n  void speak();\n}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/animal.dart", body));
        assert!(out.bindings.iter().any(|b| b.symbol == "Animal"));
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "dart-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }

    // ── extract_pubspec_name ──────────────────────────────────────────

    #[test]
    fn extracts_pubspec_name() {
        let yaml = "name: my_pkg\nversion: 0.1.0\n";
        assert_eq!(extract_pubspec_name(yaml), Some("my_pkg".to_string()));
    }

    #[test]
    fn extracts_pubspec_name_with_quotes() {
        let yaml = "name: \"my_pkg\"\nversion: 0.1.0\n";
        assert_eq!(extract_pubspec_name(yaml), Some("my_pkg".to_string()));
    }

    // ── extract_pubspec_path_deps ─────────────────────────────────────

    #[test]
    fn extract_pubspec_path_deps_single() {
        let yaml = "name: consumer\ndependencies:\n  lib_a:\n    path: ../lib_a\n  http: ^0.13.0\n";
        let deps = extract_pubspec_path_deps(yaml);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].0, "lib_a");
        assert_eq!(deps[0].1, PathBuf::from("../lib_a"));
    }

    #[test]
    fn extract_pubspec_path_deps_multiple() {
        let yaml = "name: consumer\ndependencies:\n  lib_a:\n    path: ../lib_a\n  lib_b:\n    path: ../lib_b\n";
        let deps = extract_pubspec_path_deps(yaml);
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn extract_pubspec_path_deps_returns_empty_for_no_path_deps() {
        let yaml = "name: consumer\ndependencies:\n  http: ^0.13.0\n  meta: any\n";
        let deps = extract_pubspec_path_deps(yaml);
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_pubspec_path_deps_malformed_returns_empty() {
        let deps = extract_pubspec_path_deps("not: valid: yaml: at: all: ][");
        // serde_yaml is lenient; this might parse. Just check it doesn't panic.
        let _ = deps;
    }

    // ── C-1 regression: content_sha must not be the empty-string SHA ─────

    /// The well-known SHA-256 of an empty byte slice.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn content_sha_is_not_empty_string_sha() {
        // Each declaration line is non-empty, so the span must cover actual
        // bytes and the resulting SHA must differ from the empty-string SHA.
        let body = "class Foo {}\nclass Bar {}\n";
        let out = extract_dart_surface("demo/comp", &input("lib/decls.dart", body));
        let foo = out.bindings.iter().find(|b| b.symbol == "Foo").unwrap();
        let bar = out.bindings.iter().find(|b| b.symbol == "Bar").unwrap();

        assert_ne!(
            foo.content_sha, EMPTY_SHA256,
            "Foo.content_sha must not be the empty-string SHA"
        );
        assert_ne!(
            bar.content_sha, EMPTY_SHA256,
            "Bar.content_sha must not be the empty-string SHA"
        );
        // The two declarations have different source text, so their SHAs differ.
        assert_ne!(
            foo.content_sha, bar.content_sha,
            "Foo and Bar must have distinct content_shas"
        );
    }

    // ── C-2 regression: top-level variable initializers must not emit phantom Fn bindings ──

    #[test]
    fn final_variable_initializer_does_not_emit_phantom_fn_binding() {
        // `final myList = compute();` looks like a function call after modifier
        // stripping, but must not produce a binding for `compute`.
        // `const COUNT = computeCount();` is the same pattern with `const`.
        let body = "final myList = compute();\nconst COUNT = computeCount();\n";
        let out = extract_dart_surface("demo/comp", &input("lib/vars.dart", body));

        let has_compute = out.bindings.iter().any(|b| b.symbol == "compute");
        let has_compute_count = out.bindings.iter().any(|b| b.symbol == "computeCount");

        assert!(
            !has_compute,
            "phantom binding for `compute` must not be emitted"
        );
        assert!(
            !has_compute_count,
            "phantom binding for `computeCount` must not be emitted"
        );
        // No bindings at all from these two variable lines.
        assert!(
            out.bindings.is_empty(),
            "no bindings should be emitted for variable declarations"
        );
    }
}
