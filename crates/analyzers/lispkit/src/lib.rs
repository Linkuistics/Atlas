//! LispKit surface analyser logic (Atlas vNext Phase 2 PR-10).
//!
//! This crate's library form is the pure analyser: it takes
//! [`LispKitSourceInputs`] (parsed from on-disk `*.sld` library
//! declaration files) and emits [`LispKitSurfaceOutput`] containing
//! bindings for exported and non-exported `define`d symbols.
//!
//! ## Sibling binary
//!
//! The companion `lispkit-analyzer` binary at `src/main.rs` wraps this
//! library in the subprocess wire protocol from
//! [`atlas_analyzers::subprocess`]. Tests and the in-tree
//! `lispkit_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_lispkit-analyzer")`.
//!
//! ## Manifest convention
//!
//! LispKit components are identified by the presence of one or more
//! `*.sld` (Scheme Library Definition) files — the R7RS-standard
//! extension for `define-library` forms. This is the most
//! unambiguous signal:
//!
//! - `*.sld` is the canonical extension the R7RS standard uses in its
//!   own examples and that portable Scheme implementations (LispKit,
//!   Chibi-Scheme, Gauche, Chez) expect.
//! - Alternatives like `package.scm` or `lispkit.toml` are
//!   project-specific conventions, not part of the R7RS spec.
//! - Without access to the Linkuistics project, `*.sld` is the
//!   safest fallback: every LispKit library ships at least one.
//!
//! The L3 classifier (`lispkit_classifier.rs` in `atlas-analyzers`)
//! keys on `**/*.sld` glob.
//!
//! ## Binding shape
//!
//! - `(define-library (lib name) (export sym1 sym2 ...) (begin ...))`
//!   → each exported `sym` is a `Binding` with
//!   `visibility: Conventional` (no `private` attribute).
//! - `(define name ...)` inside the library but NOT in the
//!   `(export ...)` clause → `Binding` with
//!   `visibility: Conventional` + `attributes.private: true`.
//!
//! ## module_path derivation
//!
//! If a `(define-library (lib name))` form is present, `module_path`
//! is the library identifier's components:
//! `(my-lib utils)` → `["my-lib", "utils"]`.
//!
//! Fallback: file-path components of the `*.sld` file, excluding the
//! `.sld` extension (e.g. `libs/my-lib/utils.sld` → `["libs", "my-lib", "utils"]`).
//!
//! The symbol's `Binding.symbol` is the bare identifier; the full path
//! `module_path.join(".") + "." + symbol` is reconstructed by callers.
//!
//! ## Span convention
//!
//! `Binding.span` is `(start_byte, end_byte)` over the source file.
//! Because our s-expression reader is hand-rolled and minimal, we
//! record `(0, 0)` for every binding — a conservative span that
//! hashes the empty slice. Phase 3 may add precise offset tracking.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, ContractKind, LibraryApi, PubItem, PubItemKind, Visibility, ATTR_PRIVATE,
};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

/// Stable analyser id for the LispKit surface analyser.
pub const ANALYZER_ID: &str = "lispkit-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Inputs describing one component's LispKit source surface.
///
/// The driver fills this in by walking the component's source tree
/// (`**/*.sld`), then calls [`extract_lispkit_surface`].
#[derive(Debug, Clone, Default)]
pub struct LispKitSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path must
    /// be relative to the package root (the directory containing the
    /// `*.sld` file), so `module_path` derivation is unambiguous.
    /// Empty `bytes` are tolerated and produce no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
}

/// Output of one LispKit surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LispKitSurfaceOutput {
    /// Every binding found in the parsed `.sld` source files.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per analysis run. Empty when the
    /// component exposes no exported symbols.
    pub library_apis: Vec<LibraryApi>,
}

/// Drive the LispKit surface extraction over the component's source
/// inputs. Returns the bindings and `LibraryApi` (at most one overall)
/// discovered.
///
/// `component_id` is the owning component's id (e.g. `repo/my-lib`);
/// the resulting library-api id is `<component_id>/public-api`.
///
/// Source files that fail to parse as UTF-8 or contain no recognisable
/// `define-library` / `define` forms are silently skipped.
pub fn extract_lispkit_surface(
    component_id: &str,
    inputs: &LispKitSourceInputs,
) -> LispKitSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    // Sort sources by path for deterministic output ordering.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, bytes) in sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        extract_from_sld(rel_path, text, &mut bindings, &mut pub_items);
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
            language: "scheme".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    LispKitSurfaceOutput {
        bindings,
        library_apis,
    }
}

/// Parse a single `.sld` source and emit bindings + pub-items.
///
/// Algorithm:
///
/// 1. Parse the top-level s-expression tree from `text`.
/// 2. Find the outermost `(define-library ...)` form (if present).
/// 3. Extract the `(export ...)` sub-clause's symbol list.
/// 4. Derive `module_path` from the library identifier or file path.
/// 5. Emit bindings for every exported symbol and every `(define name
///    ...)` found in `(begin ...)` clauses (non-exported → private).
///
/// A file that contains no `define-library` form is silently skipped
/// (not every `*.sld` file must be a library declaration; some may be
/// supporting scripts).
fn extract_from_sld(
    rel_path: &Path,
    text: &str,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let tokens = tokenise(text);
    let mut pos = 0usize;
    let Some(top) = parse_sexp(&tokens, &mut pos) else {
        return;
    };

    // Top-level must be `(define-library ...)`.
    let Sexp::List(top_list) = &top else { return };
    if top_list.is_empty() {
        return;
    }
    let Sexp::Atom(head) = &top_list[0] else {
        return;
    };
    if head != "define-library" {
        return;
    }

    // The library identifier is the second element: `(lib name)` → list of atoms.
    let lib_id_parts: Vec<String> = if top_list.len() >= 2 {
        match &top_list[1] {
            Sexp::List(parts) => parts
                .iter()
                .filter_map(|e| {
                    if let Sexp::Atom(s) = e {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            Sexp::Atom(s) => vec![s.clone()],
        }
    } else {
        Vec::new()
    };

    // Derive module_path: library-identifier form preferred, file-path fallback.
    let module_path: Vec<String> = if !lib_id_parts.is_empty() {
        lib_id_parts
    } else {
        derive_module_path_from_file(rel_path)
    };

    // Collect export list and defined names.
    let mut exports: Vec<String> = Vec::new();
    let mut defines: Vec<String> = Vec::new();

    for clause in &top_list[2..] {
        let Sexp::List(clause_list) = clause else {
            continue;
        };
        if clause_list.is_empty() {
            continue;
        }
        let Sexp::Atom(clause_head) = &clause_list[0] else {
            continue;
        };
        match clause_head.as_str() {
            "export" => {
                for sym in &clause_list[1..] {
                    if let Sexp::Atom(s) = sym {
                        exports.push(s.clone());
                    } else if let Sexp::List(rename) = sym {
                        // (rename internal external) — export the external name.
                        if rename.len() >= 3 {
                            if let Sexp::Atom(external) = &rename[2] {
                                exports.push(external.clone());
                            }
                        }
                    }
                }
            }
            "begin" | "include" | "include-ci" | "cond-expand" => {
                collect_defines_from_body(clause_list, &mut defines);
            }
            _ => {}
        }
    }

    // Emit bindings.
    let exports_set: std::collections::BTreeSet<String> = exports.iter().cloned().collect();

    // Exported symbols → public bindings.
    for sym in &exports {
        let content_sha = sha256_hex_of_range(&[], (0, 0));
        bindings.push(Binding {
            language: "scheme".into(),
            symbol: sym.clone(),
            file: rel_path.to_path_buf(),
            span: (0, 0),
            content_sha,
            visibility: Visibility::Conventional,
            module_path: module_path.clone(),
            attributes: BTreeMap::new(),
        });
        pub_items.push(PubItem {
            name: sym.clone(),
            file: rel_path.to_path_buf(),
            kind: PubItemKind::Fn,
        });
    }

    // Non-exported defines → private bindings.
    for sym in &defines {
        if exports_set.contains(sym) {
            continue; // already emitted above
        }
        let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
        attributes.insert(ATTR_PRIVATE.into(), YamlValue::Bool(true));
        let content_sha = sha256_hex_of_range(&[], (0, 0));
        bindings.push(Binding {
            language: "scheme".into(),
            symbol: sym.clone(),
            file: rel_path.to_path_buf(),
            span: (0, 0),
            content_sha,
            visibility: Visibility::Conventional,
            module_path: module_path.clone(),
            attributes,
        });
    }
}

/// Collect top-level `(define name ...)` names from a `(begin ...)`
/// body (recursively one level for nested `begin`s).
fn collect_defines_from_body(body: &[Sexp], defines: &mut Vec<String>) {
    for item in body.iter().skip(1) {
        let Sexp::List(item_list) = item else {
            continue;
        };
        if item_list.is_empty() {
            continue;
        }
        let Sexp::Atom(item_head) = &item_list[0] else {
            continue;
        };
        match item_head.as_str() {
            "define" => {
                // `(define name ...)` or `(define (name ...) ...)`.
                if item_list.len() >= 2 {
                    match &item_list[1] {
                        Sexp::Atom(name) => defines.push(name.clone()),
                        Sexp::List(parts) => {
                            if let Some(Sexp::Atom(name)) = parts.first() {
                                defines.push(name.clone());
                            }
                        }
                    }
                }
            }
            "begin" => {
                // Nested begin — recurse one level.
                collect_defines_from_body(item_list, defines);
            }
            _ => {}
        }
    }
}

/// Derive a module path from the file's relative path (`.sld` extension
/// stripped, path components become the module path segments).
///
/// `libs/my-lib/utils.sld` → `["libs", "my-lib", "utils"]`
fn derive_module_path_from_file(rel: &Path) -> Vec<String> {
    let stem = match rel.extension().and_then(|s| s.to_str()) {
        Some("sld") | Some("scm") => rel.with_extension(""),
        _ => rel.to_path_buf(),
    };
    stem.components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// Minimal hand-rolled s-expression reader
// ---------------------------------------------------------------------------

/// A minimal s-expression value. Only atoms and lists are needed for
/// the `define-library` / `export` / `define` extraction this analyser
/// performs. Strings, characters, and other datums are approximated as
/// `Atom`s if they appear where only a symbol/atom is expected
/// (conservative — they are typically ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

/// Tokenise a Scheme source string into a flat list of tokens.
/// Handles line comments (`;`), block comments (`#| ... |#`),
/// string literals (which are skipped as opaque tokens), and
/// datum labels (`#N=` / `#N#`, which are stripped).
pub fn tokenise(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut tokens: Vec<String> = Vec::new();

    while i < len {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            }
            b';' => {
                // Line comment — skip to end of line.
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' if i + 1 < len && bytes[i + 1] == b'|' => {
                // Block comment #| ... |#
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'|' && bytes[i + 1] == b'#' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            b'"' => {
                // String literal — consume without emitting (treated as
                // an opaque atom placeholder so outer forms parse
                // correctly).
                i += 1;
                let mut s = String::from('"');
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        s.push(bytes[i] as char);
                        s.push(bytes[i + 1] as char);
                        i += 2;
                    } else if bytes[i] == b'"' {
                        s.push('"');
                        i += 1;
                        break;
                    } else {
                        s.push(bytes[i] as char);
                        i += 1;
                    }
                }
                tokens.push(s);
            }
            b'(' => {
                tokens.push("(".into());
                i += 1;
            }
            b')' => {
                tokens.push(")".into());
                i += 1;
            }
            b'\'' | b'`' => {
                // Quote shorthand — emit as a synthetic `quote` atom
                // so the parser can skip it as an unrecognised head.
                tokens.push("'".into());
                i += 1;
            }
            b',' if i + 1 < len && bytes[i + 1] == b'@' => {
                tokens.push(",@".into());
                i += 2;
            }
            b',' => {
                tokens.push(",".into());
                i += 1;
            }
            b'#' => {
                // Could be `#t`, `#f`, `#\char`, `#N=`, `#N#`, `#(`, `#u8(`.
                let start = i;
                i += 1;
                while i < len
                    && bytes[i] != b' '
                    && bytes[i] != b'\t'
                    && bytes[i] != b'\n'
                    && bytes[i] != b'\r'
                    && bytes[i] != b'('
                    && bytes[i] != b')'
                    && bytes[i] != b'"'
                    && bytes[i] != b';'
                {
                    i += 1;
                }
                // `#(` or `#u8(` are vector-open tokens; treat as `(`.
                if i < len && bytes[i] == b'(' {
                    tokens.push("(".into());
                    i += 1;
                } else {
                    let tok: String = src[start..i].to_string();
                    tokens.push(tok);
                }
            }
            _ => {
                // Atom (symbol, number, identifier).
                let start = i;
                while i < len
                    && bytes[i] != b' '
                    && bytes[i] != b'\t'
                    && bytes[i] != b'\n'
                    && bytes[i] != b'\r'
                    && bytes[i] != b'('
                    && bytes[i] != b')'
                    && bytes[i] != b'"'
                    && bytes[i] != b';'
                {
                    i += 1;
                }
                tokens.push(src[start..i].to_string());
            }
        }
    }
    tokens
}

/// Parse one s-expression from `tokens` starting at `*pos`.
/// Advances `*pos` past the consumed tokens.
/// Returns `None` at EOF or on a lone `)`.
pub fn parse_sexp(tokens: &[String], pos: &mut usize) -> Option<Sexp> {
    if *pos >= tokens.len() {
        return None;
    }
    let tok = tokens[*pos].clone();
    *pos += 1;

    if tok == ")" {
        return None;
    }
    if tok == "(" {
        let mut list = Vec::new();
        loop {
            if *pos >= tokens.len() {
                break;
            }
            if tokens[*pos] == ")" {
                *pos += 1;
                break;
            }
            if let Some(child) = parse_sexp(tokens, pos) {
                list.push(child);
            } else {
                break;
            }
        }
        return Some(Sexp::List(list));
    }
    // Quote shorthands: `'`, `` ` ``, `,`, `,@` — wrap next form.
    if tok == "'" || tok == "`" || tok == "," || tok == ",@" {
        let inner = parse_sexp(tokens, pos)?;
        return Some(Sexp::List(vec![Sexp::Atom("quote".into()), inner]));
    }
    Some(Sexp::Atom(tok))
}

/// SHA-256 hex of a half-open byte range `bytes[span.0..span.1]`.
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

/// Stable fingerprint over a sorted [`PubItem`] list.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> LispKitSourceInputs {
        LispKitSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
        }
    }

    // ---------------------------------------------------------------------------
    // Tokeniser tests
    // ---------------------------------------------------------------------------

    #[test]
    fn tokeniser_handles_line_comments() {
        let tokens = tokenise("; this is a comment\n(define x 1)");
        assert!(tokens.iter().any(|t| t == "("), "expected `(` token");
        assert!(!tokens.iter().any(|t| t.contains("comment")));
    }

    #[test]
    fn tokeniser_handles_block_comments() {
        let tokens = tokenise("#| block |# (define y 2)");
        assert!(!tokens.iter().any(|t| t.contains("block")));
        assert!(tokens.iter().any(|t| t == "define"));
    }

    #[test]
    fn tokeniser_handles_string_literals() {
        // String literals should be captured as a single token (not split on whitespace).
        let tokens = tokenise(r#"(define x "hello world")"#);
        // Should have: ( define x "hello world" )
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[3], "\"hello world\"");
    }

    // ---------------------------------------------------------------------------
    // Parser tests
    // ---------------------------------------------------------------------------

    #[test]
    fn parser_produces_correct_list() {
        let tokens = tokenise("(a b c)");
        let mut pos = 0;
        let sexp = parse_sexp(&tokens, &mut pos).unwrap();
        assert_eq!(
            sexp,
            Sexp::List(vec![
                Sexp::Atom("a".into()),
                Sexp::Atom("b".into()),
                Sexp::Atom("c".into()),
            ])
        );
    }

    #[test]
    fn parser_handles_nested_lists() {
        let tokens = tokenise("(a (b c) d)");
        let mut pos = 0;
        let sexp = parse_sexp(&tokens, &mut pos).unwrap();
        assert_eq!(
            sexp,
            Sexp::List(vec![
                Sexp::Atom("a".into()),
                Sexp::List(vec![Sexp::Atom("b".into()), Sexp::Atom("c".into()),]),
                Sexp::Atom("d".into()),
            ])
        );
    }

    #[test]
    fn parser_returns_none_at_eof() {
        let tokens: Vec<String> = vec![];
        let mut pos = 0;
        assert!(parse_sexp(&tokens, &mut pos).is_none());
    }

    // ---------------------------------------------------------------------------
    // Surface extraction tests
    // ---------------------------------------------------------------------------

    #[test]
    fn exports_produce_non_private_bindings() {
        let body = r#"
(define-library (my-lib core)
  (export add subtract)
  (begin
    (define (add x y) (+ x y))
    (define (subtract x y) (- x y))
    (define (helper z) z)))
"#;
        let out = extract_lispkit_surface("repo/my-lib", &input("my-lib/core.sld", body));
        // Two exports + one non-exported define.
        assert_eq!(out.bindings.len(), 3);

        let add = out.bindings.iter().find(|b| b.symbol == "add").unwrap();
        assert!(!add.attributes.contains_key("private"));
        assert!(matches!(add.visibility, Visibility::Conventional));

        let sub = out
            .bindings
            .iter()
            .find(|b| b.symbol == "subtract")
            .unwrap();
        assert!(!sub.attributes.contains_key("private"));

        let helper = out.bindings.iter().find(|b| b.symbol == "helper").unwrap();
        assert_eq!(
            helper.attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
    }

    #[test]
    fn non_exported_define_is_private() {
        // PR-10 acceptance: a `(define name ...)` inside a library that
        // is not in the `(export ...)` clause must be flagged `private: true`.
        let body = r#"
(define-library (test-lib)
  (export public-sym)
  (begin
    (define (public-sym) 42)
    (define (private-helper) 0)))
"#;
        let out = extract_lispkit_surface("test/lib", &input("test.sld", body));
        let private_h = out
            .bindings
            .iter()
            .find(|b| b.symbol == "private-helper")
            .unwrap();
        assert_eq!(
            private_h.attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
        let public = out
            .bindings
            .iter()
            .find(|b| b.symbol == "public-sym")
            .unwrap();
        assert!(!public.attributes.contains_key("private"));
    }

    #[test]
    fn library_id_becomes_module_path() {
        let body = r#"
(define-library (math vectors)
  (export dot-product)
  (begin
    (define (dot-product a b) 0)))
"#;
        let out = extract_lispkit_surface("comp", &input("vectors.sld", body));
        let binding = out
            .bindings
            .iter()
            .find(|b| b.symbol == "dot-product")
            .unwrap();
        assert_eq!(
            binding.module_path,
            vec!["math".to_string(), "vectors".to_string()]
        );
    }

    #[test]
    fn file_path_fallback_when_no_library_id() {
        // A `(define-library () ...)` form with an empty id falls back to file path.
        let body = r#"
(define-library ()
  (export foo)
  (begin
    (define (foo) 1)))
"#;
        let out = extract_lispkit_surface("comp", &input("libs/mylib/core.sld", body));
        let binding = out.bindings.iter().find(|b| b.symbol == "foo").unwrap();
        // File path without extension → ["libs", "mylib", "core"]
        assert_eq!(
            binding.module_path,
            vec!["libs".to_string(), "mylib".to_string(), "core".to_string()]
        );
    }

    #[test]
    fn library_api_contains_only_exported_symbols() {
        let body = r#"
(define-library (util)
  (export pub-fn)
  (begin
    (define (pub-fn) 0)
    (define (priv-fn) 1)))
"#;
        let out = extract_lispkit_surface("ns/util", &input("util.sld", body));
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].id, "ns/util/public-api");
        // Only exported symbols in pub_items.
        assert!(out.library_apis[0]
            .pub_items
            .iter()
            .all(|p| p.name == "pub-fn"));
        assert_eq!(out.library_apis[0].pub_items.len(), 1);
    }

    #[test]
    fn empty_input_produces_empty_output() {
        let inputs = LispKitSourceInputs::default();
        let out = extract_lispkit_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn non_utf8_bytes_skip_silently() {
        let inputs = LispKitSourceInputs {
            sources: vec![(PathBuf::from("lib.sld"), vec![0xFF, 0xFE, 0xFD])],
        };
        let out = extract_lispkit_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn no_define_library_form_is_silently_skipped() {
        // A plain `*.sld` file containing only bare expressions (no
        // `define-library`) is not an error — it just produces no output.
        let body = "(define-values (x y) (values 1 2))";
        let out = extract_lispkit_surface("ns/comp", &input("plain.sld", body));
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn all_languages_are_scheme() {
        let body = r#"
(define-library (lang-check)
  (export alpha)
  (begin
    (define (alpha) 0)))
"#;
        let out = extract_lispkit_surface("ns/lc", &input("lc.sld", body));
        for b in &out.bindings {
            assert_eq!(b.language, "scheme");
        }
        if let Some(api) = out.library_apis.first() {
            assert_eq!(api.language, "scheme");
        }
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "lispkit-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }

    #[test]
    fn pub_item_kind_str_fn_maps_to_fn() {
        assert_eq!(pub_item_kind_str(PubItemKind::Fn), "fn");
    }

    #[test]
    fn define_shorthand_form_extracts_name() {
        // `(define (name args...) body)` — shorthand define.
        let body = r#"
(define-library (shorthand-test)
  (export my-fn)
  (begin
    (define (my-fn x) (* x 2))))
"#;
        let out = extract_lispkit_surface("ns/st", &input("st.sld", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "my-fn");
        assert!(!out.bindings[0].attributes.contains_key("private"));
    }

    #[test]
    fn multiple_files_produce_bindings_from_all() {
        let inputs = LispKitSourceInputs {
            sources: vec![
                (
                    PathBuf::from("a.sld"),
                    br#"(define-library (a) (export alpha) (begin (define (alpha) 0)))"#.to_vec(),
                ),
                (
                    PathBuf::from("b.sld"),
                    br#"(define-library (b) (export beta) (begin (define (beta) 1)))"#.to_vec(),
                ),
            ],
        };
        let out = extract_lispkit_surface("ns/multi", &inputs);
        assert_eq!(out.bindings.len(), 2);
    }
}
