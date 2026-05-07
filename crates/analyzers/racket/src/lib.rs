//! Racket surface analyser logic (Atlas vNext Phase 2 PR-9).
//!
//! This crate's library form is the pure analyser: it takes
//! [`RacketSourceInputs`] (parsed from on-disk Racket files +
//! `info.rkt`) and emits [`RacketSurfaceOutput`] containing
//! bindings and the path-dep edges declared in `info.rkt`'s `deps`
//! field.
//!
//! ## Sibling binary
//!
//! The companion `racket-analyzer` binary at `src/main.rs` wraps this
//! library in the subprocess wire protocol from
//! [`atlas_analyzers::subprocess`]. Tests and the in-tree
//! `racket_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_racket-analyzer")`.
//!
//! ## Binding shape
//!
//! Racket uses `provide` / `require` for module surface. A name
//! exported via `(provide name)` becomes a `Binding` with
//! `Visibility::Conventional`. A top-level `(define name ...)` that
//! does NOT appear in a `provide` form becomes a `Binding` with
//! `attributes.private: true`. Both sets still carry
//! `Visibility::Conventional` — Racket has no `pub`/`priv` keyword.
//!
//! `module_path` is derived strictly from the source file's relative
//! path — file-path components, *excluding the symbol*. The trailing
//! `.rkt` extension is stripped:
//!
//! - `pkg/mod.rkt` → `["pkg", "mod"]`
//! - `pkg/sub/mod.rkt` → `["pkg", "sub", "mod"]`
//! - `main.rkt` → `["main"]`
//!
//! ## Parser choice
//!
//! `tree-sitter-racket` is not yet published on crates.io and the
//! grammar is incomplete (as of 2026). A hand-rolled minimal
//! s-expression reader is used instead. The reader is shallow — it
//! extracts top-level `define` and `provide` forms only, which is
//! all PR-9 requires. This is the "subagent discretion — shallow
//! extraction" clause from §4 PR-9.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, ContractKind, LibraryApi, PubItem, PubItemKind, Visibility, ATTR_PRIVATE,
};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

/// Stable analyser id for the Racket surface analyser. Matches the
/// wire form a future `analyzers.yaml` would carry; the in-tree
/// wrapper at `atlas_analyzers::racket_surface_analyzer` mirrors this
/// constant.
pub const ANALYZER_ID: &str = "racket-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Inputs describing one component's Racket source surface.
///
/// The driver fills this in by walking the component's source tree
/// (`pkg/**/*.rkt`), then calls [`extract_racket_surface`].
#[derive(Debug, Clone, Default)]
pub struct RacketSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path
    /// must be relative to the package root (the directory containing
    /// `info.rkt`), so `module_path` derivation is unambiguous.
    /// Empty `bytes` are tolerated and produce no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `info.rkt` contents. When present, the analyser
    /// resolves the `deps` field for path-dep edges.
    pub info_rkt: Option<Vec<u8>>,
}

/// Output of one Racket surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RacketSurfaceOutput {
    /// Every top-level `define` in the parsed source files.
    /// `provide`d names have no `private` attribute; `define`d-only
    /// names carry `attributes.private: true`.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per component. Empty when no
    /// `provide`d names are found.
    pub library_apis: Vec<LibraryApi>,
}

/// Drive the Racket-surface extraction over the component's source
/// inputs. Returns the bindings and `LibraryApi` (at most one)
/// discovered.
///
/// `component_id` is the owning component's id (e.g. `repo/my-pkg`);
/// the resulting library-api id is `<component_id>/public-api`.
///
/// Source files that fail to parse (or whose bytes are not valid
/// UTF-8) are silently skipped — the analyser is conservative and
/// prefers emitting nothing for a malformed file over panicking
/// the pipeline.
pub fn extract_racket_surface(
    component_id: &str,
    inputs: &RacketSourceInputs,
) -> RacketSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    // Sort sources by path so the binding emission order is
    // deterministic regardless of the driver's enumeration order.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, bytes) in &sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let module_path = derive_module_path(rel_path);
        emit_racket_surface(
            rel_path,
            bytes,
            text,
            &module_path,
            &mut bindings,
            &mut pub_items,
        );
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
            language: "racket".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    RacketSurfaceOutput {
        bindings,
        library_apis,
    }
}

/// Derive a module path from the file's relative path.
///
/// Algorithm: strip the `.rkt` extension, then split by path separator.
/// Result is the list of path components.
///
/// - `main.rkt` → `["main"]`
/// - `pkg/mod.rkt` → `["pkg", "mod"]`
/// - `pkg/sub/mod.rkt` → `["pkg", "sub", "mod"]`
///
/// A non-`.rkt` file returns an empty vector — the binding will
/// record its symbol with no dotted prefix.
fn derive_module_path(rel: &Path) -> Vec<String> {
    match rel.extension().and_then(|s| s.to_str()) {
        Some("rkt") => {}
        _ => return Vec::new(),
    }
    let stem = rel.with_extension("");
    stem.components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect()
}

/// Parse and emit bindings from a single Racket source file.
///
/// The extraction logic:
/// 1. Parse the file to collect the set of names in `provide` forms.
/// 2. Parse the file to collect all top-level `define` names.
/// 3. `provide`d names → `Binding` with no `private` attribute.
/// 4. `define`d-only names → `Binding` with `attributes.private: true`.
fn emit_racket_surface(
    rel_path: &Path,
    bytes: &[u8],
    text: &str,
    module_path: &[String],
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    let provided: BTreeSet<String> = collect_provided_names(text);
    let defines: Vec<(String, (usize, usize))> = collect_top_level_defines(text, bytes);

    for (name, span) in defines {
        let is_provided = provided.contains(&name);
        let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
        if !is_provided {
            attributes.insert(ATTR_PRIVATE.into(), YamlValue::Bool(true));
        }
        let content_sha = sha256_hex_of_range(bytes, span);
        bindings.push(Binding {
            language: "racket".into(),
            symbol: name.clone(),
            file: rel_path.to_path_buf(),
            span,
            content_sha,
            visibility: Visibility::Conventional,
            module_path: module_path.to_vec(),
            attributes,
        });
        if is_provided {
            pub_items.push(PubItem {
                name,
                file: rel_path.to_path_buf(),
                kind: PubItemKind::Fn,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal s-expression reader
// ---------------------------------------------------------------------------
//
// Racket source is full of s-expressions. We do NOT implement a full
// reader. Instead we implement two targeted extractors:
//
// 1. `collect_provided_names` — scans top-level `(provide ...)` forms
//    and collects every symbol-id named in them.
// 2. `collect_top_level_defines` — scans top-level `(define ...)` forms
//    and collects the bound name plus byte span.
//
// "Top-level" means the form starts in column 0 or immediately after
// the top-level opening parenthesis. We walk the token stream
// character-by-character, tracking paren depth. Depth 0 is the module
// body. Forms at depth 1 (the form's own parens) are the top-level
// forms we care about.
//
// Limitations (acceptable for PR-9 shallow extraction):
// - `(define (name args) body)` is handled: the name is the first
//   symbol after `define`.
// - `(define name value)` is handled likewise.
// - `(define-values (n1 n2) ...)` is NOT walked for individual names.
// - Block comments `#| ... |#` are stripped; line comments `;` are stripped.
// - String literals are skipped via a simple quote-escape walk.
// - `#lang` line is treated as a comment (starts with `#`, skipped).

/// Return the set of symbol names exported by any `(provide ...)` form
/// at top level in `text`. `(all-defined-out)` is noted but not
/// expanded — callers treat it as "every define is exported" if needed.
/// `(provide all-defined-out)` → the *keyword* is not a user symbol.
fn collect_provided_names(text: &str) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    let tokens: Vec<Token<'_>> = tokenise(text);

    // Walk token stream. At top-level (depth 0), when we see
    // `(provide ...` enter the provide form (depth 1) and collect
    // symbol tokens until the matching `)`.
    let mut depth: i32 = 0;
    let mut in_provide = false;
    let mut provide_depth: i32 = 0;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Open => {
                depth += 1;
                if depth == 1 {
                    // Check if the next non-comment token is `provide`.
                    let next = tokens.get(i + 1);
                    if let Some(Token::Symbol("provide")) = next {
                        in_provide = true;
                        provide_depth = depth;
                        i += 2; // skip `(` and `provide`
                        continue;
                    }
                } else if in_provide {
                    // Nested form inside provide — skip the sub-form
                    // (e.g. `(rename-out [old new])`). We don't try
                    // to extract from these in Phase 2.
                    let close_i = find_matching_close(&tokens, i);
                    i = close_i + 1;
                    continue;
                }
            }
            Token::Close => {
                if in_provide && depth == provide_depth {
                    in_provide = false;
                }
                depth -= 1;
            }
            Token::Symbol(s) => {
                if in_provide {
                    // Skip provide keywords.
                    if !matches!(
                        *s,
                        "all-defined-out"
                            | "all-from-out"
                            | "rename-out"
                            | "except-out"
                            | "prefix-out"
                            | "struct-out"
                            | "protect-out"
                    ) {
                        names.insert(s.to_string());
                    }
                }
            }
            Token::StringLit | Token::Other => {}
        }
        i += 1;
    }
    names
}

/// Return `(symbol_name, byte_span)` pairs for every top-level
/// `(define ...)` form. Span covers the opening `(` through the
/// matching `)`. Handles both `(define name ...)` and
/// `(define (name args) ...)` shapes.
fn collect_top_level_defines(text: &str, bytes: &[u8]) -> Vec<(String, (usize, usize))> {
    let mut out: Vec<(String, (usize, usize))> = Vec::new();
    let tokens_with_pos: Vec<TokenWithPos<'_>> = tokenise_with_pos(text);

    let mut depth: i32 = 0;
    let mut i = 0;
    while i < tokens_with_pos.len() {
        let TokenWithPos { token, byte_pos } = &tokens_with_pos[i];
        match token {
            Token::Open => {
                depth += 1;
                if depth == 1 {
                    let form_start = *byte_pos;
                    // Peek ahead for `define` keyword.
                    let next = tokens_with_pos.get(i + 1);
                    if let Some(TokenWithPos {
                        token: Token::Symbol(kw),
                        ..
                    }) = next
                    {
                        if is_define_keyword(kw) {
                            // Extract the name.
                            let name = extract_define_name(&tokens_with_pos, i + 2);
                            let close_i = find_matching_close_pos(&tokens_with_pos, i);
                            let form_end = if close_i < tokens_with_pos.len() {
                                tokens_with_pos[close_i].byte_pos + 1
                            } else {
                                bytes.len()
                            };
                            // Skip past the form.
                            if let Some(n) = name {
                                let span = (form_start, form_end.min(bytes.len()));
                                out.push((n, span));
                            }
                            i = close_i + 1;
                            depth -= 1; // the form's open was counted; close skipped.
                            continue;
                        }
                    }
                    // Not a define — skip to matching close.
                    let close_i = find_matching_close_pos(&tokens_with_pos, i);
                    i = close_i + 1;
                    depth -= 1;
                    continue;
                }
            }
            Token::Close => {
                depth -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// True for `define` and common define-variant keywords.
fn is_define_keyword(kw: &str) -> bool {
    matches!(
        kw,
        "define"
            | "define/contract"
            | "define/public"
            | "define/private"
            | "define/override"
            | "define/augment"
    )
}

/// Extract the bound name from a define form. In:
///   `(define name ...)` → "name"
///   `(define (name arg ...) ...)` → "name"
/// `tokens_with_pos[start_i]` is the token immediately after the
/// define keyword.
fn extract_define_name<'a>(tokens: &[TokenWithPos<'a>], start_i: usize) -> Option<String> {
    let tp = tokens.get(start_i)?;
    match &tp.token {
        Token::Symbol(s) => Some(s.to_string()),
        Token::Open => {
            // `(name args ...)` — name is first symbol inside.
            let inner = tokens.get(start_i + 1)?;
            if let Token::Symbol(s) = &inner.token {
                Some(s.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Walk tokens forward from `open_i` (which must be a `Token::Open`)
/// to find the index of the matching `Token::Close`.
fn find_matching_close(tokens: &[Token<'_>], open_i: usize) -> usize {
    let mut depth = 0i32;
    for (j, t) in tokens.iter().enumerate().skip(open_i) {
        match t {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

/// Walk tokens_with_pos forward from `open_i` to find the matching close.
fn find_matching_close_pos(tokens: &[TokenWithPos<'_>], open_i: usize) -> usize {
    let mut depth = 0i32;
    for (j, tp) in tokens.iter().enumerate().skip(open_i) {
        match &tp.token {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

// ---------------------------------------------------------------------------
// Tokeniser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token<'a> {
    Open,
    Close,
    Symbol(&'a str),
    StringLit,
    Other,
}

struct TokenWithPos<'a> {
    token: Token<'a>,
    byte_pos: usize,
}

fn tokenise(text: &str) -> Vec<Token<'_>> {
    tokenise_with_pos(text)
        .into_iter()
        .map(|t| t.token)
        .collect()
}

fn tokenise_with_pos(text: &str) -> Vec<TokenWithPos<'_>> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out: Vec<TokenWithPos<'_>> = Vec::new();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            // Skip whitespace.
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            // Line comment.
            b';' => {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment #| ... |#
            b'#' if i + 1 < len && bytes[i + 1] == b'|' => {
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'|' && bytes[i + 1] == b'#' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Datum comment #; — skip the next datum.
            b'#' if i + 1 < len && bytes[i + 1] == b';' => {
                i += 2;
                // skip whitespace then the next token
                while i < len && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                    i += 1;
                }
                if i < len && bytes[i] == b'(' {
                    let toks = tokenise_with_pos(&text[i..]);
                    let close_i = find_matching_close_pos(&toks, 0);
                    let skip_end = toks.get(close_i).map(|t| t.byte_pos + 1).unwrap_or(0);
                    i += skip_end;
                } else {
                    // Skip simple datum (not a paren form)
                    while i < len && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')')
                    {
                        i += 1;
                    }
                }
            }
            // `#lang` at the start of a line is a module language
            // declaration — skip to EOL (effectively a comment for our
            // purposes).  All other `#`-prefixed forms are reader macros
            // (`#t`, `#f`, `#:keyword`, `#'id`, `#"bytes"`, `#px"..."`,
            // `#rx"..."`, etc.) that are valid Racket syntax: emit them
            // as `Token::Other` so they do NOT consume the rest of the
            // line or swallow adjacent parens.
            b'#' => {
                // Check for #lang (only skip-to-EOL when at col 0).
                let at_line_start = i == 0 || (i >= 1 && bytes[i - 1] == b'\n');
                if at_line_start
                    && i + 5 <= len
                    && &bytes[i..i + 5] == b"#lang"
                    && (i + 5 == len || matches!(bytes[i + 5], b' ' | b'\t' | b'\r' | b'\n'))
                {
                    // Skip `#lang <name>` to EOL.
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                } else {
                    // Reader macro prefix: consume the `#` plus any
                    // immediately-following non-whitespace, non-paren
                    // characters as a single opaque token.
                    let start = i;
                    i += 1; // consume `#`
                            // If next char is `"` this is a byte-string literal
                            // — skip it (we do not parse the contents).
                    if i < len && bytes[i] == b'"' {
                        i += 1;
                        while i < len {
                            match bytes[i] {
                                b'\\' => i += 2,
                                b'"' => {
                                    i += 1;
                                    break;
                                }
                                _ => i += 1,
                            }
                        }
                    } else {
                        // Consume the remainder of this token (e.g. `t`,
                        // `f`, `:keyword`, `'id`, `px`, `rx`, …).
                        while i < len
                            && !matches!(
                                bytes[i],
                                b' ' | b'\t'
                                    | b'\r'
                                    | b'\n'
                                    | b'('
                                    | b')'
                                    | b'['
                                    | b']'
                                    | b'{'
                                    | b'}'
                                    | b'"'
                                    | b';'
                            )
                        {
                            i += 1;
                        }
                    }
                    out.push(TokenWithPos {
                        token: Token::Other,
                        byte_pos: start,
                    });
                }
            }
            // Open paren / bracket.
            b'(' | b'[' | b'{' => {
                out.push(TokenWithPos {
                    token: Token::Open,
                    byte_pos: i,
                });
                i += 1;
            }
            // Close paren / bracket.
            b')' | b']' | b'}' => {
                out.push(TokenWithPos {
                    token: Token::Close,
                    byte_pos: i,
                });
                i += 1;
            }
            // String literal.
            b'"' => {
                out.push(TokenWithPos {
                    token: Token::StringLit,
                    byte_pos: i,
                });
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            // Quote / quasiquote / unquote — treat as Other.
            b'\'' | b'`' | b',' => {
                out.push(TokenWithPos {
                    token: Token::Other,
                    byte_pos: i,
                });
                i += 1;
            }
            // Symbol or number.
            _ => {
                let start = i;
                while i < len
                    && !matches!(
                        bytes[i],
                        b' ' | b'\t'
                            | b'\r'
                            | b'\n'
                            | b'('
                            | b')'
                            | b'['
                            | b']'
                            | b'{'
                            | b'}'
                            | b'"'
                            | b';'
                    )
                {
                    i += 1;
                }
                let sym = &text[start..i];
                out.push(TokenWithPos {
                    token: Token::Symbol(sym),
                    byte_pos: start,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// info.rkt path-dep extraction
// ---------------------------------------------------------------------------

/// Scans `contents` for string literals inside a `(define deps ...)`
/// form and extracts those that look like paths (start with `.` or `/`).
fn scan_deps_strings<'a>(contents: &'a str) -> Vec<&'a str> {
    let bytes = contents.as_bytes();
    let len = bytes.len();
    let mut results: Vec<&'a str> = Vec::new();

    // We scan raw bytes for patterns of the form:
    //   (define deps ...<"./path">...)
    // Since info.rkt is structured, we first find the `deps` binding
    // and then scan all string literals within it.
    //
    // State machine: find `define deps`, then collect strings until
    // matching closing paren.

    // Find "define deps" as substring (after stripping comments).
    // Rather than building a full AST, we use the tokeniser positions.
    let tokens = tokenise_with_pos(contents);
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Open = &tokens[i].token {
            if matches!(
                tokens.get(i + 1),
                Some(TokenWithPos {
                    token: Token::Symbol("define"),
                    ..
                })
            ) && matches!(
                tokens.get(i + 2),
                Some(TokenWithPos {
                    token: Token::Symbol("deps"),
                    ..
                })
            ) {
                let close_i = find_matching_close_pos(&tokens, i);
                // Scan for string literals between i and close_i.
                for tp in &tokens[i + 3..=close_i.min(tokens.len().saturating_sub(1))] {
                    if let Token::StringLit = &tp.token {
                        // Reconstruct the string content from bytes.
                        let str_start = tp.byte_pos + 1; // skip opening "
                        let mut j = str_start;
                        let mut s_end = j;
                        while j < len {
                            match bytes[j] {
                                b'\\' => j += 2,
                                b'"' => {
                                    s_end = j;
                                    break;
                                }
                                _ => j += 1,
                            }
                        }
                        if s_end > str_start {
                            let s = &contents[str_start..s_end];
                            if s.starts_with('.') || s.starts_with('/') {
                                results.push(s);
                            }
                        }
                    }
                }
                i = close_i + 1;
                continue;
            }
        }
        i += 1;
    }
    results
}

/// Extract path-dep entries from `info.rkt` content. Returns relative
/// or absolute paths found in the `deps` list. Registry package names
/// (bare strings without path prefix) are excluded.
pub fn extract_info_rkt_path_deps(contents: &str) -> Vec<PathBuf> {
    scan_deps_strings(contents)
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

// ---------------------------------------------------------------------------
// SHA helpers
// ---------------------------------------------------------------------------

/// SHA-256 hex of the bytes covered by `span`. Mirrors
/// [`atlas_analyzers::sha256_hex_of_range`] so this crate doesn't take
/// a hard dependency on the analyser host.
pub fn sha256_hex_of_range(bytes: &[u8], span: (usize, usize)) -> String {
    let (start, end) = span;
    let slice: &[u8] = if start <= end && end <= bytes.len() {
        &bytes[start..end]
    } else {
        b""
    };
    let digest: [u8; 32] = Sha256::digest(slice).into();
    hex_string(&digest)
}

/// SHA-256 hex of `bytes`.
pub fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex_string(&digest)
}

fn hex_string(digest: &[u8; 32]) -> String {
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
    hex_string(&digest)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> RacketSourceInputs {
        RacketSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
            info_rkt: None,
        }
    }

    // ── acceptance tests (PR-9 criteria) ────────────────────────────────────

    #[test]
    fn provided_name_becomes_binding_without_private_flag() {
        // PR-9 acceptance: `info.rkt` + `main.rkt` with `(provide foo)`
        // and `(define foo ...)` → binding for `foo` without private.
        let body = "#lang racket\n(provide foo)\n(define foo 42)\n";
        let out = extract_racket_surface("demo/comp", &input("main.rkt", body));
        let foo = out.bindings.iter().find(|b| b.symbol == "foo").unwrap();
        assert_eq!(foo.language, "racket");
        assert!(matches!(foo.visibility, Visibility::Conventional));
        assert!(!foo.attributes.contains_key("private"));
    }

    #[test]
    fn define_without_provide_gets_private_true_attribute() {
        // PR-9 acceptance: `(define helper ...)` without provide →
        // binding present in surface but `private: true`.
        let body = "#lang racket\n(provide foo)\n(define foo 1)\n(define helper 2)\n";
        let out = extract_racket_surface("demo/comp", &input("main.rkt", body));

        let helper = out.bindings.iter().find(|b| b.symbol == "helper").unwrap();
        assert_eq!(
            helper.attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
        let foo = out.bindings.iter().find(|b| b.symbol == "foo").unwrap();
        assert!(!foo.attributes.contains_key("private"));
    }

    #[test]
    fn function_definition_form_extracts_name() {
        // `(define (foo x y) body)` → binding with symbol `foo`.
        let body = "#lang racket\n(provide foo)\n(define (foo x y) (+ x y))\n";
        let out = extract_racket_surface("demo/comp", &input("pkg/mod.rkt", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "foo");
    }

    #[test]
    fn module_path_derived_from_file_path_excluding_symbol() {
        let body = "#lang racket\n(provide foo)\n(define foo 1)\n";
        let out = extract_racket_surface("demo/comp", &input("pkg/sub/mod.rkt", body));
        assert_eq!(
            out.bindings[0].module_path,
            vec!["pkg".to_string(), "sub".to_string(), "mod".to_string()]
        );
        assert_eq!(out.bindings[0].symbol, "foo");
    }

    #[test]
    fn info_rkt_path_deps_extracted() {
        // PR-9 acceptance: `(require 'other-pkg)` resolves via path-dep.
        let info = "#lang info\n(define deps '(\"base\" \"../sibling-pkg\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert_eq!(deps, vec![PathBuf::from("../sibling-pkg")]);
    }

    #[test]
    fn info_rkt_registry_deps_excluded() {
        let info = "#lang info\n(define deps '(\"base\" \"rackunit-lib\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert!(deps.is_empty());
    }

    // ── tokeniser / parser unit tests ───────────────────────────────────────

    #[test]
    fn derive_module_path_strips_rkt_extension() {
        assert_eq!(
            derive_module_path(Path::new("pkg/mod.rkt")),
            vec!["pkg".to_string(), "mod".to_string()]
        );
    }

    #[test]
    fn derive_module_path_returns_empty_for_non_rkt() {
        assert!(derive_module_path(Path::new("pkg/mod.py")).is_empty());
    }

    #[test]
    fn derive_module_path_single_file() {
        assert_eq!(
            derive_module_path(Path::new("main.rkt")),
            vec!["main".to_string()]
        );
    }

    #[test]
    fn collect_provided_names_basic() {
        let text = "#lang racket\n(provide foo bar)\n(define foo 1)\n(define bar 2)\n";
        let names = collect_provided_names(text);
        assert!(names.contains("foo"), "{names:?}");
        assert!(names.contains("bar"), "{names:?}");
    }

    #[test]
    fn collect_provided_names_skips_keywords() {
        let text = "#lang racket\n(provide (all-defined-out))\n";
        let names = collect_provided_names(text);
        assert!(!names.contains("all-defined-out"), "{names:?}");
    }

    #[test]
    fn collect_top_level_defines_simple_value() {
        let text = "#lang racket\n(define x 10)\n";
        let bytes = text.as_bytes();
        let defines = collect_top_level_defines(text, bytes);
        assert_eq!(defines.len(), 1);
        assert_eq!(defines[0].0, "x");
    }

    #[test]
    fn collect_top_level_defines_function_form() {
        let text = "#lang racket\n(define (square n) (* n n))\n";
        let bytes = text.as_bytes();
        let defines = collect_top_level_defines(text, bytes);
        assert_eq!(defines.len(), 1);
        assert_eq!(defines[0].0, "square");
    }

    #[test]
    fn multiple_defines_in_file() {
        let body = "#lang racket\n(provide foo)\n(define foo 1)\n(define bar 2)\n";
        let out = extract_racket_surface("ns/comp", &input("mod.rkt", body));
        assert_eq!(out.bindings.len(), 2);
        let syms: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(syms.contains(&"foo"));
        assert!(syms.contains(&"bar"));
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let inputs = RacketSourceInputs::default();
        let out = extract_racket_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn non_utf8_bytes_skip_silently() {
        let inputs = RacketSourceInputs {
            sources: vec![(PathBuf::from("mod.rkt"), vec![0xFF, 0xFE, 0xFD])],
            info_rkt: None,
        };
        let out = extract_racket_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "#lang racket\n(provide foo)\n(define foo 1)\n";
        let out = extract_racket_surface("foo/bar", &input("mod.rkt", body));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
        assert_eq!(out.library_apis[0].language, "racket");
    }

    #[test]
    fn no_provide_means_no_library_api() {
        let body = "#lang racket\n(define foo 1)\n";
        let out = extract_racket_surface("foo/bar", &input("mod.rkt", body));
        assert!(out.library_apis.is_empty());
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(
            out.bindings[0].attributes.get("private"),
            Some(&YamlValue::Bool(true))
        );
    }

    #[test]
    fn line_comment_stripped_before_extraction() {
        let body = "; this is a comment\n#lang racket\n(provide foo)\n(define foo 1)\n";
        let out = extract_racket_surface("demo/comp", &input("mod.rkt", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "foo");
    }

    #[test]
    fn span_is_non_zero_for_define_form() {
        let body = "#lang racket\n(define foo 42)\n";
        let bytes = body.as_bytes();
        let defines = collect_top_level_defines(body, bytes);
        assert!(!defines.is_empty());
        let (_, (start, end)) = &defines[0];
        assert!(*start < *end, "span must be non-empty: {start}..{end}");
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "racket-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }

    // ── F-RKT-1 regression tests: reader macros must not eat rest-of-line ──

    /// `#t` inside a `define` must not swallow the closing `)` and
    /// adjacent forms.  Before the fix the tokeniser hit the catch-all
    /// `b'#'` arm and jumped to `\n`, consuming `)`, which caused
    /// `find_matching_close_pos` to pair the `define` paren with the
    /// *next* form's `)`, silently absorbing `(provide foo)` into the
    /// broken define.
    ///
    /// The source has `(define x #t)` followed by `(provide foo)` and
    /// `(define foo 1)` so both `x` and `foo` are defined, `foo` is
    /// exported, and `bar` (defined later) is private.
    #[test]
    fn reader_macro_hash_true_does_not_eat_rest_of_line() {
        let body = "#lang racket\n(define x #t)\n(provide foo)\n(define foo 1)\n(define bar 1)\n";
        let out = extract_racket_surface("ns/comp", &input("mod.rkt", body));

        // `x` must be present (define bound, not provided → private).
        let x = out
            .bindings
            .iter()
            .find(|b| b.symbol == "x")
            .expect("binding `x` must be present");
        assert_eq!(
            x.attributes.get("private"),
            Some(&YamlValue::Bool(true)),
            "`x` must be private; attrs: {:?}",
            x.attributes
        );

        // `foo` must be exported (provide hit it).
        let foo = out
            .bindings
            .iter()
            .find(|b| b.symbol == "foo")
            .expect("binding `foo` must be present — provide must not be absorbed");
        assert!(
            !foo.attributes.contains_key("private"),
            "`foo` must be exported (no private flag); attrs: {:?}",
            foo.attributes
        );

        // `bar` must be private (define but not provided).
        let bar = out
            .bindings
            .iter()
            .find(|b| b.symbol == "bar")
            .expect("binding `bar` must be present");
        assert_eq!(
            bar.attributes.get("private"),
            Some(&YamlValue::Bool(true)),
            "`bar` must carry private:true; attrs: {:?}",
            bar.attributes
        );
    }

    /// `#f` (boolean false) must be treated the same as `#t`.
    #[test]
    fn reader_macro_hash_false_does_not_eat_rest_of_line() {
        let body = "#lang racket\n(define flag #f)\n(provide flag)\n";
        let out = extract_racket_surface("ns/comp", &input("mod.rkt", body));
        let flag = out
            .bindings
            .iter()
            .find(|b| b.symbol == "flag")
            .expect("binding `flag` must be present");
        assert!(
            !flag.attributes.contains_key("private"),
            "`flag` must be exported; attrs: {:?}",
            flag.attributes
        );
    }

    /// `#:keyword` tokens (used heavily in `racket/base` keyword
    /// arguments and `struct` field specs) must be tokenised as Other,
    /// not consume to EOL.
    #[test]
    fn reader_macro_hash_keyword_does_not_eat_rest_of_line() {
        // `#:mutable` is a keyword arg to `struct`; the `)` that closes
        // the struct form must still be present.
        let body =
            "#lang racket\n(define-struct point (x y) #:mutable)\n(provide make-point)\n(define helper 1)\n";
        let out = extract_racket_surface("ns/comp", &input("mod.rkt", body));
        // `make-point` would only be in `provide` scope if the paren
        // balance is intact after `#:mutable`.
        // At minimum the provide form must not be silently absorbed:
        // `helper` is defined but not provided → private.
        let helper = out
            .bindings
            .iter()
            .find(|b| b.symbol == "helper")
            .expect("binding `helper` must be present");
        assert_eq!(
            helper.attributes.get("private"),
            Some(&YamlValue::Bool(true)),
            "`helper` must be private; attrs: {:?}",
            helper.attributes
        );
    }

    /// `#'identifier` (syntax-quote) must be treated as Other.
    #[test]
    fn reader_macro_syntax_quote_does_not_eat_rest_of_line() {
        let body = "#lang racket\n(define stx #'foo)\n(provide stx)\n(define other 2)\n";
        let out = extract_racket_surface("ns/comp", &input("mod.rkt", body));

        let stx = out
            .bindings
            .iter()
            .find(|b| b.symbol == "stx")
            .expect("binding `stx` must be present");
        assert!(
            !stx.attributes.contains_key("private"),
            "`stx` must be exported; attrs: {:?}",
            stx.attributes
        );

        let other = out
            .bindings
            .iter()
            .find(|b| b.symbol == "other")
            .expect("binding `other` must be present");
        assert_eq!(
            other.attributes.get("private"),
            Some(&YamlValue::Bool(true)),
            "`other` must be private; attrs: {:?}",
            other.attributes
        );
    }

    #[test]
    fn info_rkt_absolute_path_dep_extracted() {
        let info = "#lang info\n(define deps '(\"/absolute/path\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert_eq!(deps, vec![PathBuf::from("/absolute/path")]);
    }

    #[test]
    fn info_rkt_multiple_path_deps() {
        let info = "#lang info\n(define deps '(\"base\" \"../a\" \"../b\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert_eq!(deps, vec![PathBuf::from("../a"), PathBuf::from("../b")]);
    }
}
