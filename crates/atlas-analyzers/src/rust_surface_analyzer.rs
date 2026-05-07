//! Phase 1 surface analyser for Rust components.
//!
//! Extracts contracts, bindings, and library-api items by scanning the
//! component's `src/lib.rs` and `src/main.rs` for `pub` items at the
//! top level of the file. The scan is regex-driven (rust-analyzer
//! integration is Phase 2, plan §6 risks); nested `pub` items inside
//! `pub mod foo { ... }` are missed in Phase 1 — documented at the
//! call sites.
//!
//! ## Outputs
//!
//! For every top-level `pub struct` with `#[derive(... Serialize ...
//! Deserialize ...)]` preceding it, the analyser emits a `data-format`
//! contract whose `definition_binding` covers the struct's
//! `pub`-to-closing-brace byte range. The struct also appears as a
//! `pub_item` of kind `struct` in the component's `LibraryApi`.
//!
//! For every other top-level `pub item` (struct without serde derive,
//! `pub fn`, `pub trait`, `pub enum`, `pub mod`, `pub type`, `pub const`,
//! `pub static`, `pub union`, `pub macro_rules!`), a `pub_item` is
//! recorded under the component's single Rust `LibraryApi`.
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
//! ## Why this lives in `atlas-analyzers`
//!
//! The plan §4 PR-7 stub places the analyser in `atlas-analyzers/src/`
//! so future per-language surface analysers compose under the same
//! [`crate::Analyzer`] trait. Phase 1 binding extraction is a pure
//! function of file bytes, so the analyser is materially testable in
//! isolation without standing up a database.

use std::path::PathBuf;

use atlas_index::{
    Binding, Contract, ContractKind, CostClass, LibraryApi, PubItem, PubItemKind, Stage,
};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id (matches the wire form a future
/// `analyzers.yaml` would carry; design §6.6).
pub const ANALYZER_ID: &str = "rust-surface-analyzer";

/// Bumped when the extraction algorithm changes shape (e.g. when
/// nested `pub mod` items start contributing). Phase 2's
/// rust-analyzer wire-up is the next bump.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output of a single component's Rust-surface analysis. The L5
/// driver downcasts the `Box<dyn StageOutput>` back to this struct
/// via the [`crate::StageOutput`] machinery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustSurfaceOutput {
    /// Code-derived `data-format` contracts for top-level
    /// `pub struct` items that carry `#[derive(Serialize, Deserialize)]`.
    pub contracts: Vec<Contract>,
    /// Every binding emitted (one per contract; each contract's
    /// `definition_binding` is also represented here for callers
    /// that want a flat list).
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] entry — Phase 1 emits Rust only.
    /// Empty when the component exposes no `pub` items.
    pub library_apis: Vec<LibraryApi>,
}

crate::impl_stage_output!(RustSurfaceOutput);

/// The analyser itself. Stateless; the only knob is the `ANALYZER_VERSION`
/// constant, which the registry picks up via the [`Analyzer::version`]
/// trait method.
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
        // The dispatcher path is reserved for a future Phase 2 driver
        // that hands a populated `Target` carrying `src/lib.rs`/
        // `src/main.rs` bytes. Phase 1's L5 driver invokes
        // [`extract_rust_surface`] directly rather than going through
        // `analyse`, so this branch returns `Declines` to remain a
        // well-behaved registry citizen without duplicating the
        // engine's file-loading logic. Plan §4 PR-7 explicitly favours
        // option (b): keep deterministic binding-extraction inside the
        // analyser, but let the L5 driver call into it.
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
pub fn extract_rust_surface(component_id: &str, inputs: &RustSourceInputs) -> RustSurfaceOutput {
    let mut contracts: Vec<Contract> = Vec::new();
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    for (rel_path, bytes) in &inputs.sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue; // non-UTF-8 source is rejected silently — Phase 2's parser will be stricter
        };
        for item in scan_pub_items(text) {
            // Build the binding span as half-open byte range.
            let span = (item.start_byte, item.end_byte);
            let content_sha = crate::sha256_hex_of_range(bytes, span);

            // Every `pub item` shows up in `pub_items` for the
            // library-api. `pub mod` is recorded too — design §6.3
            // example shows nested mods are valid library-api members.
            pub_items.push(PubItem {
                name: item.name.clone(),
                file: rel_path.clone(),
                kind: item.kind,
            });

            // For `pub struct` with `#[derive(Serialize, Deserialize)]`,
            // emit a `data-format` contract.
            if matches!(item.kind, PubItemKind::Struct) && item.has_serde_derive {
                let local = kebabify_struct_name(&item.name);
                let contract_id = format!("{component_id}/{local}");
                let binding = Binding {
                    language: "rust".into(),
                    symbol: item.name.clone(),
                    file: rel_path.clone(),
                    span,
                    content_sha: content_sha.clone(),
                };
                let contract = Contract {
                    id: contract_id,
                    kind: ContractKind::DataFormat,
                    // Phase 1 §2.1 reduction: contract sha == binding sha.
                    fingerprint: content_sha.clone(),
                    definition_binding: binding.clone(),
                    description: String::new(),
                };
                bindings.push(binding);
                contracts.push(contract);
            }
        }
    }

    let library_apis: Vec<LibraryApi> = if pub_items.is_empty() {
        Vec::new()
    } else {
        // Sort pub_items deterministically: by file then by name. The
        // scan order (file order in `sources`, declaration order
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
        // Validate before emitting (PR-1 status note).
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

/// One `pub` item discovered by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PubItemRecord {
    name: String,
    kind: PubItemKind,
    /// Byte offset of the first character of `pub`.
    start_byte: usize,
    /// Byte offset one past the last character of the item (the
    /// byte immediately after `}` for block items, after `;` for
    /// statement items).
    end_byte: usize,
    /// True when a `#[derive(...)]` attribute *immediately* preceding
    /// the `pub` keyword (modulo whitespace and doc-comments) names
    /// both `Serialize` and `Deserialize`. Phase 1 detects only the
    /// adjacent-attribute form — the cfg_attr-wrapped form is Phase 2.
    has_serde_derive: bool,
}

/// Top-level `pub` item scanner. Operates on the raw source bytes
/// (UTF-8). Phase 1 limitations (documented on the analyser's module
/// docs):
///
/// - Nested `pub mod foo { ... pub struct Bar; }` inner items are
///   missed (we only consider items at brace depth 0).
/// - Items inside `cfg_attr(..., derive(...))` are recorded as
///   non-serde even when one of the conditional branches would
///   compile to `#[derive(Serialize, Deserialize)]`. This is the
///   intentionally conservative branch.
/// - Doc-comments and attributes preceding the `pub` keyword are
///   *not* part of the item's span; the span starts at `pub`.
fn scan_pub_items(text: &str) -> Vec<PubItemRecord> {
    let bytes = text.as_bytes();
    let mut out: Vec<PubItemRecord> = Vec::new();

    // We walk byte-by-byte tracking brace depth and string/char/
    // line-comment/block-comment state. At depth 0, when we see the
    // start of a token that could be `pub`, we attempt to recognise
    // the item.
    let mut i: usize = 0;
    let mut depth: i32 = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // Skip line comments (`// ...\n`).
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip block comments (`/* ... */`). Rust nests block comments
        // syntactically; track the depth.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            let mut bdepth = 1;
            while i < bytes.len() && bdepth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    bdepth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    bdepth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Skip raw strings and ordinary strings — the scanner ignores
        // their contents for brace tracking.
        if b == b'r' && i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            // Raw string: `r###"..."###`. Count leading `#`s.
            let mut j = i + 1;
            let mut hash_count = 0usize;
            while j < bytes.len() && bytes[j] == b'#' {
                hash_count += 1;
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                // Skip past the closing `"###` matching this prefix.
                j += 1;
                while j < bytes.len() {
                    if bytes[j] == b'"' {
                        let end = j + 1 + hash_count;
                        if end <= bytes.len() && bytes[j + 1..end].iter().all(|&c| c == b'#') {
                            j = end;
                            break;
                        }
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        if b == b'"' {
            // Ordinary string: skip until the next unescaped `"`.
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            // Char literal or lifetime — distinguish by lookahead.
            // A char literal starts `'<char>'` or `'\<esc>'`; a
            // lifetime is `'ident`. We try to skip a char literal
            // first, falling back to a single-byte advance if the
            // lookahead doesn't match.
            if i + 2 < bytes.len() && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            if i + 3 < bytes.len() && bytes[i + 1] == b'\\' {
                // Escape sequence — find the closing `'`.
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\'' {
                    j += 1;
                }
                if j < bytes.len() {
                    i = j + 1;
                    continue;
                }
            }
            // Lifetime or unrecognised — advance one byte.
            i += 1;
            continue;
        }

        if b == b'{' {
            depth += 1;
            i += 1;
            continue;
        }
        if b == b'}' {
            depth -= 1;
            i += 1;
            continue;
        }

        // Top-level (depth == 0) only: try to recognise a `pub` item.
        if depth == 0 && is_pub_keyword_at(bytes, i) {
            if let Some(record) = parse_pub_item(bytes, i) {
                let advance = record.end_byte;
                out.push(record);
                i = advance;
                continue;
            }
        }

        i += 1;
    }

    out
}

/// True iff `bytes[i..]` starts with the `pub` keyword followed by a
/// non-identifier byte (whitespace, `(`, `;`, etc.).
fn is_pub_keyword_at(bytes: &[u8], i: usize) -> bool {
    if i + 3 > bytes.len() {
        return false;
    }
    if &bytes[i..i + 3] != b"pub" {
        return false;
    }
    // Ensure `pub` is a whole word (the previous byte must not be an
    // identifier byte; the next byte must not be either).
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return false;
    }
    let next = bytes.get(i + 3).copied().unwrap_or(b' ');
    !is_ident_byte(next)
}

fn is_ident_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

/// Parse one top-level `pub` item starting at `start` (the `p` of
/// `pub`). Returns `None` if the item shape is not recognised
/// (Phase 1 conservative — we'd rather miss an item than emit wrong
/// data).
fn parse_pub_item(bytes: &[u8], start: usize) -> Option<PubItemRecord> {
    // Detect a `#[derive(... Serialize ... Deserialize ...)]`
    // attribute attached to this item. The attribute precedes the
    // `pub` keyword, separated only by whitespace and doc-comments.
    let has_serde_derive = preceding_block_has_serde_derive(bytes, start);

    // Skip `pub` and any restriction (`pub(crate)`, `pub(super)`).
    let mut i = start + 3;
    i = skip_ws_and_comments(bytes, i);
    if i < bytes.len() && bytes[i] == b'(' {
        // Skip until the matching `)`. `pub(restriction)` is single-
        // depth so a naive scan suffices.
        let mut paren = 1;
        i += 1;
        while i < bytes.len() && paren > 0 {
            match bytes[i] {
                b'(' => paren += 1,
                b')' => paren -= 1,
                _ => {}
            }
            i += 1;
        }
        i = skip_ws_and_comments(bytes, i);
    }
    if i >= bytes.len() {
        return None;
    }

    // Read the item-kind keyword (struct, fn, enum, trait, mod,
    // type, const, static, union, async, unsafe, default, macro_rules!, ...).
    // Async/unsafe/default modifiers may precede the actual kind
    // keyword (e.g. `pub unsafe fn ...`, `pub async fn ...`,
    // `pub default fn ...`). Skip them.
    loop {
        let word = read_ident(bytes, i)?;
        if matches!(word.as_str(), "async" | "unsafe" | "default" | "extern") {
            i += word.len();
            i = skip_ws_and_comments(bytes, i);
            // `extern "C" fn ...` — skip the optional ABI string.
            if word == "extern" && i < bytes.len() && bytes[i] == b'"' {
                // Skip ABI string literal.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() {
                    i += 1;
                }
                i = skip_ws_and_comments(bytes, i);
            }
            continue;
        }
        break;
    }

    let kind_word = read_ident(bytes, i)?;
    // Special-case `macro_rules!` — read the `!` and continue as a
    // macro item.
    let (kind, kind_keyword_len) = match kind_word.as_str() {
        "struct" => (PubItemKind::Struct, kind_word.len()),
        "enum" => (PubItemKind::Enum, kind_word.len()),
        "fn" => (PubItemKind::Fn, kind_word.len()),
        "trait" => (PubItemKind::Trait, kind_word.len()),
        "mod" => (PubItemKind::Mod, kind_word.len()),
        "type" => (PubItemKind::TypeAlias, kind_word.len()),
        "const" => (PubItemKind::Const, kind_word.len()),
        "static" => (PubItemKind::Static, kind_word.len()),
        "union" => (PubItemKind::Union, kind_word.len()),
        "use" => return None, // `pub use` is a re-export, not a declaration site
        _ => return None,
    };
    i += kind_keyword_len;
    i = skip_ws_and_comments(bytes, i);

    // Read the item's name (or the macro's name; for macros the
    // identifier follows `macro_rules!`).
    let name = read_ident(bytes, i)?;
    if name.is_empty() {
        return None;
    }

    // Find the item's terminator. For block items (`struct { }`,
    // `enum { }`, `fn { }`, `trait { }`, `mod { }`, `union { }`),
    // the span ends at the matching `}`. For statement items
    // (`type X = Y;`, `const X: T = ...;`, `static X: T = ...;`,
    // `struct X;`, `struct X(...);`), the span ends at the first
    // `;` outside any nested braces / parens / strings.
    let end_byte = match kind {
        PubItemKind::TypeAlias | PubItemKind::Const | PubItemKind::Static => {
            // Statement items always end at `;`.
            scan_to_semicolon(bytes, i)?
        }
        PubItemKind::Mod => {
            // `pub mod foo;` (file-style) or `pub mod foo { ... }`
            // (inline). Look for `{` or `;`.
            scan_to_brace_or_semi(bytes, i)?
        }
        PubItemKind::Struct | PubItemKind::Union | PubItemKind::Enum | PubItemKind::Trait => {
            // Block-bodied. `pub struct X;` (unit-struct) and
            // `pub struct X(...);` (tuple-struct) end at `;` instead;
            // probe for that first.
            scan_to_brace_or_semi(bytes, i)?
        }
        PubItemKind::Fn => {
            // Functions always end at `}` (Rust does not allow `pub fn foo();`
            // outside a trait body, and `applies` already filters out trait
            // bodies via brace depth).
            scan_to_close_brace(bytes, i)?
        }
        PubItemKind::Macro => unreachable!(),
    };

    Some(PubItemRecord {
        name,
        kind,
        start_byte: start,
        end_byte,
        has_serde_derive,
    })
}

/// Walk backward from `start` over whitespace and doc comments and
/// collect any preceding `#[derive(...)]` attribute. Returns true
/// iff one names both `Serialize` and `Deserialize`.
fn preceding_block_has_serde_derive(bytes: &[u8], start: usize) -> bool {
    // Walk back over whitespace, line-comments, and block-comments
    // line-by-line. We collect the textual content of every
    // `#[derive(...)]` attribute encountered immediately before the
    // `pub` keyword (with only whitespace / doc-comments between).
    let mut i: isize = start as isize - 1;
    // Skip any trailing whitespace immediately before `pub`.
    while i >= 0
        && (bytes[i as usize] == b' '
            || bytes[i as usize] == b'\t'
            || bytes[i as usize] == b'\n'
            || bytes[i as usize] == b'\r')
    {
        i -= 1;
    }
    // Walk backward collecting attribute lines (each `#[ ... ]`) and
    // doc comments (`/// ...`, `//! ...`). We accept these in any
    // order — the precise grouping is irrelevant; we only need to
    // scan their joined text for the serde derives.
    let mut accumulated = String::new();
    while i >= 0 {
        // Skip whitespace.
        while i >= 0
            && (bytes[i as usize] == b' '
                || bytes[i as usize] == b'\t'
                || bytes[i as usize] == b'\n'
                || bytes[i as usize] == b'\r')
        {
            i -= 1;
        }
        if i < 0 {
            break;
        }
        let here = bytes[i as usize];
        if here == b']' {
            // Walk back to the matching `[`.
            let end_excl = i as usize + 1;
            let mut depth = 1;
            i -= 1;
            while i >= 0 && depth > 0 {
                match bytes[i as usize] {
                    b']' => depth += 1,
                    b'[' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                i -= 1;
            }
            if i < 0 || bytes[i as usize] != b'[' {
                break;
            }
            // Now expect `#` immediately before the `[`.
            let bracket_open = i as usize;
            i -= 1;
            // Tolerate `#!` (inner attribute) too — for bindings on
            // a struct the outer form `#[derive(...)]` is what we
            // care about.
            if i < 0 || bytes[i as usize] != b'#' {
                break;
            }
            let hash_pos = i as usize;
            // Slurp the entire attribute (`#[...]`), prepend with
            // newline so split-by-newline below sees it as a discrete
            // attr line. We start at `hash_pos` so the leading `#`
            // is included; `bracket_open` was the `[` byte position.
            let _ = bracket_open; // keep the local for clarity in the diff
            if let Ok(attr) = std::str::from_utf8(&bytes[hash_pos..end_excl]) {
                accumulated.push_str(attr);
                accumulated.push('\n');
            }
            i -= 1;
            continue;
        }
        // Could be the trailing newline of a line-comment or a
        // doc-comment, or the start of a non-attribute line.
        // Conservative: stop walking back the moment we hit a non-
        // attribute, non-whitespace, non-comment byte.
        // To detect a line-comment, walk backward to the start of
        // the current line and check for `//`.
        let line_start = {
            let mut j = i;
            while j >= 0 && bytes[j as usize] != b'\n' {
                j -= 1;
            }
            (j + 1) as usize
        };
        // If the line starts with `///` or `//!` or `//` (doc / line
        // comment), continue walking back. Otherwise we've hit
        // non-attribute content — stop.
        let line_first_nonws = {
            let mut j = line_start;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            j
        };
        if line_first_nonws + 2 <= bytes.len()
            && &bytes[line_first_nonws..line_first_nonws + 2] == b"//"
        {
            // Line comment — skip past its start.
            i = line_start as isize - 1;
            continue;
        }
        // Non-attribute non-comment content: stop.
        break;
    }
    if accumulated.is_empty() {
        return false;
    }
    // We accumulated each attribute as a separate `#[...]` block.
    // Look for one that contains a `derive(` group naming both
    // `Serialize` and `Deserialize`.
    for attr in accumulated.split('\n') {
        let lower = attr.trim();
        if !lower.starts_with("#[") {
            continue;
        }
        // Find a `derive(...)` substring. Inside the parentheses
        // identifiers are comma-separated; we accept paths like
        // `serde::Serialize` too.
        if let Some(derive_pos) = lower.find("derive(") {
            // Carve out the inner argument list (up to the matching
            // `)`). Brittle but sufficient for the conservative
            // Phase 1 form; nested parens don't appear in derive
            // arguments in practice.
            let after = &lower[derive_pos + "derive(".len()..];
            if let Some(close) = after.find(')') {
                let args = &after[..close];
                let names: Vec<&str> = args
                    .split(',')
                    .map(|s| s.trim())
                    .map(|s| s.rsplit("::").next().unwrap_or(s))
                    .collect();
                if names.iter().any(|n| n == &"Serialize")
                    && names.iter().any(|n| n == &"Deserialize")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Skip whitespace and `// ...` line comments / `/* ... */` block
/// comments at `i`. Returns the new index.
fn skip_ws_and_comments(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        break;
    }
    i
}

/// Read a Rust identifier starting at `i`. Returns the identifier
/// string (empty if `i` does not point at an identifier byte).
fn read_ident(bytes: &[u8], i: usize) -> Option<String> {
    if i >= bytes.len() {
        return None;
    }
    let first = bytes[i];
    if !(first == b'_' || first.is_ascii_alphabetic()) {
        return None;
    }
    let mut j = i;
    while j < bytes.len() && is_ident_byte(bytes[j]) {
        j += 1;
    }
    std::str::from_utf8(&bytes[i..j])
        .ok()
        .map(|s| s.to_string())
}

/// Scan forward from `i` to the first `;` outside any nested braces,
/// brackets, parens, or strings. Returns the index *after* the
/// terminating `;`.
fn scan_to_semicolon(bytes: &[u8], i: usize) -> Option<usize> {
    scan_to_marker(bytes, i, |b| b == b';').map(|p| p + 1)
}

/// Scan forward from `i` to either a matching `{` (followed by a
/// brace-balanced scan to its `}`) or a `;` at top depth, whichever
/// comes first. Returns the index *after* the closing `}` or the `;`.
fn scan_to_brace_or_semi(bytes: &[u8], i: usize) -> Option<usize> {
    let pos = scan_to_marker(bytes, i, |b| b == b'{' || b == b';')?;
    if bytes[pos] == b';' {
        Some(pos + 1)
    } else {
        // pos is the byte position of `{`; advance past matching `}`.
        scan_brace_body(bytes, pos)
    }
}

/// Like [`scan_to_brace_or_semi`] but always expects `{`. Used for
/// `pub fn` whose body must be a block.
fn scan_to_close_brace(bytes: &[u8], i: usize) -> Option<usize> {
    let pos = scan_to_marker(bytes, i, |b| b == b'{')?;
    scan_brace_body(bytes, pos)
}

/// Given `bytes[pos] == b'{'`, return the index *after* the matching
/// `}`. Tracks brace depth and ignores braces inside strings, char
/// literals, and comments.
fn scan_brace_body(bytes: &[u8], pos: usize) -> Option<usize> {
    debug_assert_eq!(bytes[pos], b'{');
    let mut i = pos + 1;
    let mut depth = 1i32;
    while i < bytes.len() && depth > 0 {
        let b = bytes[i];
        // Comments first so braces inside `// {` don't count.
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut bdepth = 1;
            while i < bytes.len() && bdepth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    bdepth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    bdepth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if b == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            // Char literal vs lifetime — skip a char if pattern matches.
            if i + 2 < bytes.len() && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            if i + 3 < bytes.len() && bytes[i + 1] == b'\\' {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\'' {
                    j += 1;
                }
                if j < bytes.len() {
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// Scan forward from `i` until `pred(bytes[k])` is true at brace/
/// paren/bracket depth 0 and outside any string/char/comment.
/// Returns the byte index `k`. Does not advance past `k`.
fn scan_to_marker<F: Fn(u8) -> bool>(bytes: &[u8], mut i: usize, pred: F) -> Option<usize> {
    let mut depth = 0i32; // {, [, ( all share one depth — sufficient for Phase 1
    while i < bytes.len() {
        let b = bytes[i];
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && b == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut bdepth = 1;
            while i < bytes.len() && bdepth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    bdepth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    bdepth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if b == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            if i + 2 < bytes.len() && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            if i + 3 < bytes.len() && bytes[i + 1] == b'\\' {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\'' {
                    j += 1;
                }
                if j < bytes.len() {
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if depth == 0 && pred(b) {
            return Some(i);
        }
        match b {
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
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
        // pub(crate) is technically not exported but Phase 1 records it
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
    fn nested_pub_inside_pub_mod_is_phase1_known_limitation() {
        // Plan §6 risks: nested `pub mod foo { pub struct Bar; }` is
        // missed in Phase 1. This test pins the limitation so a
        // future change that fixes it has to update this test, not
        // surprise the reviewer.
        let body = "pub mod outer { pub struct Hidden; }\n";
        let out = extract_rust_surface("c", &input("src/lib.rs", body));
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(names.contains(&"outer"), "outer mod must be recorded");
        assert!(
            !names.contains(&"Hidden"),
            "Phase 1 misses nested `pub` items (plan §6 risks): got {names:?}"
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
}
