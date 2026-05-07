//! Python surface analyser logic (Atlas vNext Phase 2 PR-3).
//!
//! This crate's library form is the pure analyser: it takes
//! [`PythonSourceInputs`] (parsed from on-disk Python files +
//! `pyproject.toml`) and emits [`PythonSurfaceOutput`] containing
//! bindings, library APIs, and the path-dep edges declared in
//! `pyproject.toml`'s `[tool.poetry.dependencies]` /
//! `[project.dependencies]` tables.
//!
//! ## Sibling binary
//!
//! The companion `python-analyzer` binary at `src/main.rs` wraps this
//! library in the subprocess wire protocol from
//! [`atlas_analyzers::subprocess`]. Tests and the in-tree
//! `python_surface_analyzer` wrapper resolve it via
//! `env!("CARGO_BIN_EXE_python-analyzer")`.
//!
//! ## Binding shape
//!
//! Python has no `pub`/`priv` keyword: top-level `def foo(...)` and
//! `class Bar(...)` are library-public by convention; a leading
//! underscore (`_private`) flags the binding as conventionally-private
//! via the structured `attributes.private: true` slot. Decorators
//! attached to a binding ride `attributes.decorator_chain: [name, …]`.
//!
//! `module_path` is derived from the source file's relative path with
//! the trailing `.py` and any `__init__` segment stripped:
//!
//! - `pkg/mod.py` → `["pkg", "mod"]`
//! - `pkg/__init__.py` → `["pkg"]`
//! - `pkg/sub/mod.py` → `["pkg", "sub", "mod"]`
//!
//! The symbol's `Binding.symbol` is the bare identifier; the full
//! dotted name (e.g. `pkg.mod.foo`) is reconstructed by joining
//! `module_path + [symbol]`.
//!
//! ## Span convention
//!
//! `Binding.span` is a `(start_byte, end_byte)` half-open range over
//! the source file's bytes, matching [`atlas_index::Binding`]'s
//! contract. `rustpython-parser`'s `text-size` ranges supply the
//! start/end positions; they are byte offsets, not character offsets,
//! so they line up with the `bytes[start..end]` content-sha
//! algorithm.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_index::{Binding, ContractKind, LibraryApi, PubItem, PubItemKind, Visibility};
use rustpython_parser::ast::{self, Stmt, Suite};
use rustpython_parser::Parse;
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

/// Stable analyser id for the Python surface analyser. Matches the
/// wire form a future `analyzers.yaml` would carry; the in-tree
/// wrapper at `atlas_analyzers::python_surface_analyzer` mirrors this
/// constant.
pub const ANALYZER_ID: &str = "python-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Inputs describing one component's Python source surface.
///
/// The driver fills this in by walking the component's source tree
/// (`pkg/**/*.py`, `pkg/__init__.py`), then calls
/// [`extract_python_surface`].
#[derive(Debug, Clone, Default)]
pub struct PythonSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. The relative path
    /// must be relative to the package root (the directory containing
    /// `pyproject.toml`), so `module_path` derivation is unambiguous.
    /// Empty `bytes` are tolerated and produce no output.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `pyproject.toml` contents. When present, the analyser
    /// resolves `[project].name` / `[tool.poetry].name` into the
    /// component's [`LibraryApi`] entrypoint id.
    pub pyproject_toml: Option<Vec<u8>>,
}

/// Output of one Python surface analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PythonSurfaceOutput {
    /// Every top-level `def` / `class` in the parsed source files.
    /// One entry per binding; the symbol name is the identifier, the
    /// dotted module path lives on `Binding.module_path`.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per language (Python is the only
    /// language this analyser handles). Empty when the component
    /// exposes no top-level definitions.
    pub library_apis: Vec<LibraryApi>,
}

/// Drive the Python-surface extraction over the component's source
/// inputs. Returns the bindings and `LibraryApi` (at most one)
/// discovered.
///
/// `component_id` is the owning component's id (e.g. `repo/my-pkg`);
/// the resulting library-api id is `<component_id>/public-api`.
///
/// Source files that fail to parse as Python (or whose bytes are not
/// valid UTF-8) are silently skipped — the analyser is conservative
/// and prefers emitting nothing for a malformed file over panicking
/// the pipeline.
pub fn extract_python_surface(
    component_id: &str,
    inputs: &PythonSourceInputs,
) -> PythonSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    // Sort sources by path so the binding emission order is
    // deterministic regardless of the driver's enumeration order.
    let mut sorted_sources: Vec<&(PathBuf, Vec<u8>)> = inputs.sources.iter().collect();
    sorted_sources.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel_path, bytes) in sorted_sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let module_path = derive_module_path(rel_path);
        let Some(suite) = parse_python_source(text, rel_path) else {
            continue;
        };
        for stmt in &suite {
            emit_top_level_stmt(
                rel_path,
                bytes,
                &module_path,
                stmt,
                &mut bindings,
                &mut pub_items,
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
            language: "python".into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    PythonSurfaceOutput {
        bindings,
        library_apis,
    }
}

/// Parse a single Python source string into a `Suite` (the body of a
/// `Module` shape — `Vec<Stmt>`). Returns `None` on parse failure
/// (the caller skips that file).
fn parse_python_source(text: &str, rel: &Path) -> Option<Suite> {
    let path_str = rel.to_string_lossy();
    Suite::parse(text, &path_str).ok()
}

/// Derive a dotted module path from the file's relative path.
///
/// Algorithm:
///
/// 1. Drop the file's `.py` (or `.pyi`) extension.
/// 2. Drop a trailing `__init__` segment so `pkg/__init__.py` becomes
///    `["pkg"]` rather than `["pkg", "__init__"]`.
/// 3. The remaining path components become the dotted module path.
///
/// A non-`.py` file or a path that resolves to an empty list (e.g. a
/// bare top-level `__init__.py`) returns an empty vector — the
/// binding will record its symbol with no dotted prefix.
fn derive_module_path(rel: &Path) -> Vec<String> {
    let stem = match rel.extension().and_then(|s| s.to_str()) {
        Some("py") | Some("pyi") => rel.with_extension(""),
        _ => return Vec::new(),
    };
    let mut segments: Vec<String> = stem
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect();
    if matches!(segments.last().map(String::as_str), Some("__init__")) {
        segments.pop();
    }
    segments
}

fn emit_top_level_stmt(
    rel_path: &Path,
    bytes: &[u8],
    module_path: &[String],
    stmt: &Stmt,
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            let name = f.name.as_str().to_string();
            let span = stmt_byte_span(stmt, bytes);
            let decorators = decorator_chain(&f.decorator_list);
            push_binding(
                bindings,
                pub_items,
                rel_path,
                bytes,
                module_path,
                &name,
                span,
                PubItemKind::Fn,
                decorators,
            );
        }
        Stmt::AsyncFunctionDef(f) => {
            let name = f.name.as_str().to_string();
            let span = stmt_byte_span(stmt, bytes);
            let decorators = decorator_chain(&f.decorator_list);
            push_binding(
                bindings,
                pub_items,
                rel_path,
                bytes,
                module_path,
                &name,
                span,
                PubItemKind::Fn,
                decorators,
            );
        }
        Stmt::ClassDef(c) => {
            let name = c.name.as_str().to_string();
            let span = stmt_byte_span(stmt, bytes);
            let decorators = decorator_chain(&c.decorator_list);
            push_binding(
                bindings,
                pub_items,
                rel_path,
                bytes,
                module_path,
                &name,
                span,
                PubItemKind::Struct,
                decorators,
            );
        }
        // Top-level `name = …` assignments (constants like `__all__`,
        // module-level singletons) are NOT promoted to bindings —
        // Python's library-API surface is canonically the set of
        // top-level `def`/`class` names. Phase 3 may revisit if
        // dataclass-style constants need to participate.
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn push_binding(
    bindings: &mut Vec<Binding>,
    pub_items: &mut Vec<PubItem>,
    rel_path: &Path,
    bytes: &[u8],
    module_path: &[String],
    name: &str,
    span: (usize, usize),
    pub_item_kind: PubItemKind,
    decorators: Vec<String>,
) {
    let mut attributes: BTreeMap<String, YamlValue> = BTreeMap::new();
    if name.starts_with('_') && !name.starts_with("__") {
        // Conventionally-private: a single leading underscore. The
        // `__dunder__` form (Python's reserved sandwich) is excluded
        // here because top-level dunders are typically protocol-shaped
        // (e.g. `__all__`) and deserve their own attribute treatment
        // if they ever surface.
        attributes.insert("private".into(), YamlValue::Bool(true));
    }
    if !decorators.is_empty() {
        let chain: Vec<YamlValue> = decorators
            .iter()
            .map(|d| YamlValue::String(d.clone()))
            .collect();
        attributes.insert("decorator_chain".into(), YamlValue::Sequence(chain));
    }

    let mut full_path: Vec<String> = module_path.to_vec();
    full_path.push(name.to_string());

    let content_sha = sha256_hex_of_range(bytes, span);
    bindings.push(Binding {
        language: "python".into(),
        symbol: name.to_string(),
        file: rel_path.to_path_buf(),
        span,
        content_sha,
        // Python has no `pub`/`priv` keyword: a top-level `def`/`class`
        // is library-public by convention. Leading-underscore names
        // remain `Visibility::Conventional` here; the
        // `attributes.private: true` flag set above is the carrier of
        // the private signal.
        visibility: Visibility::Conventional,
        module_path: full_path,
        attributes,
    });
    pub_items.push(PubItem {
        name: name.to_string(),
        file: rel_path.to_path_buf(),
        kind: pub_item_kind,
    });
}

/// Extract the decorator names for a `def`/`class` body. The chain is
/// recorded in source order; each entry is a dotted path of
/// identifiers (`dataclass`, `dataclasses.dataclass`, `staticmethod`).
/// Decorators with arguments (`@app.route("/")`) record only the
/// callee path — argument values do not contribute.
fn decorator_chain(decorators: &[ast::Expr]) -> Vec<String> {
    decorators.iter().filter_map(decorator_name_of).collect()
}

fn decorator_name_of(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Name(n) => Some(n.id.as_str().to_string()),
        ast::Expr::Attribute(a) => {
            // `pkg.mod.deco` — recurse for the prefix, then append
            // the trailing attr name.
            let prefix = decorator_name_of(&a.value)?;
            Some(format!("{prefix}.{}", a.attr.as_str()))
        }
        ast::Expr::Call(c) => {
            // `@route(...)` — the callee is what we name.
            decorator_name_of(&c.func)
        }
        _ => None,
    }
}

/// Best-effort byte-range computation for a top-level statement.
///
/// `rustpython-parser`'s `Located` trait carries `range` returning a
/// `text_size::TextRange`; the `start()` / `end()` accessors yield
/// byte offsets that line up with `bytes[start..end]`. Python text is
/// canonically UTF-8, so the byte offsets are valid `usize`s without
/// the codepoint conversion JS-style parsers require.
///
/// On a degenerate range (start past end-of-bytes, or start > end) we
/// fall back to `(0, 0)` so the content-sha computation hashes the
/// empty slice and the binding is recorded with a zero-width span.
/// This is conservative — a malformed-shape binding cannot affect
/// downstream contracts beyond the known-empty content sha.
fn stmt_byte_span(stmt: &Stmt, bytes: &[u8]) -> (usize, usize) {
    use rustpython_parser::ast::Ranged;
    let range = stmt.range();
    let start: usize = range.start().to_usize();
    let end: usize = range.end().to_usize();
    if start > end || end > bytes.len() {
        return (0, 0);
    }
    (start, end)
}

/// SHA-256 hex of the bytes covered by `span`. Mirrors
/// [`atlas_analyzers::sha256_hex_of_range`] so this crate doesn't take
/// a hard dependency on the analyser host (the binary uses both the
/// host's wire types and this helper).
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

/// Stable canonical fingerprint over a sorted [`PubItem`] list. The
/// canonical form is the api id followed by one line per pub_item
/// (`<file>\t<kind-str>\t<name>`), terminated by `\n`. Identical
/// shape to the Rust analyser's helper so a polyglot component's
/// library APIs hash uniformly.
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

/// Extract `(name, path)` pairs from a `pyproject.toml` for path-deps.
/// Recognised forms:
///
/// - `[tool.poetry.dependencies]` `name = { path = "..." }` (Poetry).
/// - `[project]` table with `[tool.poetry.source]` listing relative
///   paths is uncommon enough we don't synthesise from it; the
///   canonical Poetry form is `dependencies` plus `develop = true`.
/// - `[project.dependencies]` PEP-621 form is a list of strings, not
///   a table; PEP 621 itself does not standardise local-path
///   dependencies, so this branch returns nothing for the standard
///   form. Tools layered on top (`uv`, Hatch) commonly extend with
///   `[tool.uv.sources]` `name = { path = "..." }`; that table is
///   recognised here as a sibling of the Poetry one.
///
/// On a malformed manifest the function returns an empty Vec — the
/// degrade-to-default policy mirrors
/// [`atlas_engine::manifest_parse::extract_path_deps`] for Cargo.
pub fn extract_pyproject_path_deps(contents: &str) -> Vec<PathBuf> {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(tool) = table.get("tool").and_then(toml::Value::as_table) {
        if let Some(poetry) = tool.get("poetry").and_then(toml::Value::as_table) {
            collect_path_deps_from_table(poetry.get("dependencies"), &mut out);
            collect_path_deps_from_table(poetry.get("dev-dependencies"), &mut out);
            // Poetry 1.2+ optional dependency groups: `[tool.poetry.group.<name>.dependencies]`.
            if let Some(group_table) = poetry.get("group").and_then(toml::Value::as_table) {
                for (_group_name, group_value) in group_table {
                    if let Some(group_inner) = group_value.as_table() {
                        collect_path_deps_from_table(group_inner.get("dependencies"), &mut out);
                    }
                }
            }
        }
        // `[tool.uv.sources]` and `[tool.hatch.metadata.hooks.fancy-pypi-readme]`
        // are convention-driven; uv's source map is the most-widely-deployed
        // form for path-deps in PEP 621 projects.
        if let Some(uv) = tool.get("uv").and_then(toml::Value::as_table) {
            collect_path_deps_from_table(uv.get("sources"), &mut out);
        }
    }
    out
}

fn collect_path_deps_from_table(block: Option<&toml::Value>, out: &mut Vec<PathBuf>) {
    let Some(deps) = block.and_then(toml::Value::as_table) else {
        return;
    };
    for (_name, spec) in deps {
        let Some(spec_table) = spec.as_table() else {
            continue;
        };
        if let Some(toml::Value::String(p)) = spec_table.get("path") {
            out.push(PathBuf::from(p));
        }
    }
}

/// Read the project name out of a `pyproject.toml`. Tries
/// `[project].name` (PEP 621) first, then `[tool.poetry].name` for
/// Poetry projects. Returns `None` on a malformed manifest or one
/// that doesn't declare a name.
pub fn extract_pyproject_project_name(contents: &str) -> Option<String> {
    let table = contents.parse::<toml::Table>().ok()?;
    if let Some(project) = table.get("project").and_then(toml::Value::as_table) {
        if let Some(toml::Value::String(name)) = project.get("name") {
            return Some(name.clone());
        }
    }
    if let Some(tool) = table.get("tool").and_then(toml::Value::as_table) {
        if let Some(poetry) = tool.get("poetry").and_then(toml::Value::as_table) {
            if let Some(toml::Value::String(name)) = poetry.get("name") {
                return Some(name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> PythonSourceInputs {
        PythonSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
            pyproject_toml: None,
        }
    }

    #[test]
    fn extracts_top_level_def_as_binding() {
        let body = "def foo():\n    return 1\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "foo");
        assert_eq!(out.bindings[0].language, "python");
        assert_eq!(
            out.bindings[0].module_path,
            vec!["pkg".to_string(), "mod".to_string(), "foo".to_string()]
        );
        assert!(matches!(
            out.bindings[0].visibility,
            Visibility::Conventional
        ));
    }

    #[test]
    fn extracts_top_level_class_as_binding() {
        let body = "class Bar:\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "Bar");
    }

    #[test]
    fn underscore_prefix_records_conventional_private_attribute() {
        // PR-3 acceptance: a Python file with `def _private()` and
        // `def public()` produces two bindings, distinguished by the
        // conventional-private attribute.
        let body = "def public():\n    pass\n\ndef _private():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
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
    fn dataclass_decorator_recorded_in_decorator_chain() {
        // PR-3 acceptance: a `@dataclass` decorator on a class
        // produces a binding whose
        // `attributes.decorator_chain` includes `dataclass`.
        let body = "from dataclasses import dataclass\n\n@dataclass\nclass Bar:\n    a: int = 0\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        let bar = out.bindings.iter().find(|b| b.symbol == "Bar").unwrap();
        let chain = bar
            .attributes
            .get("decorator_chain")
            .expect("decorator_chain must be present");
        let names: Vec<String> = chain
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"dataclass".to_string()), "got: {names:?}");
    }

    #[test]
    fn decorator_chain_preserves_order_and_names_dotted_paths() {
        let body = "@first\n@pkg.second\n@third(\"arg\")\ndef f():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        let chain = out.bindings[0]
            .attributes
            .get("decorator_chain")
            .unwrap()
            .as_sequence()
            .unwrap();
        let names: Vec<String> = chain
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert_eq!(names, vec!["first", "pkg.second", "third"]);
    }

    #[test]
    fn module_path_drops_init_segment() {
        let body = "def hello():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/__init__.py", body));
        assert_eq!(
            out.bindings[0].module_path,
            vec!["pkg".to_string(), "hello".to_string()]
        );
    }

    #[test]
    fn module_path_handles_nested_subpackage() {
        let body = "def deep():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/sub/mod.py", body));
        assert_eq!(
            out.bindings[0].module_path,
            vec![
                "pkg".to_string(),
                "sub".to_string(),
                "mod".to_string(),
                "deep".to_string()
            ]
        );
    }

    #[test]
    fn malformed_python_skips_silently() {
        let body = "def foo(:\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn non_utf8_bytes_skip_silently() {
        let inputs = PythonSourceInputs {
            sources: vec![(PathBuf::from("pkg/mod.py"), vec![0xFF, 0xFE, 0xFD])],
            pyproject_toml: None,
        };
        let out = extract_python_surface("demo/comp", &inputs);
        assert!(out.bindings.is_empty());
    }

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "def hello():\n    pass\n";
        let out = extract_python_surface("foo/bar", &input("pkg/mod.py", body));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
        assert_eq!(out.library_apis[0].language, "python");
    }

    #[test]
    fn library_api_pub_items_are_sorted_by_file_then_name() {
        let inputs = PythonSourceInputs {
            sources: vec![
                (
                    PathBuf::from("pkg/zeta.py"),
                    b"def zeta():\n    pass\n".to_vec(),
                ),
                (
                    PathBuf::from("pkg/alpha.py"),
                    b"def alpha():\n    pass\nclass Beta:\n    pass\n".to_vec(),
                ),
            ],
            pyproject_toml: None,
        };
        let out = extract_python_surface("ns/comp", &inputs);
        let names: Vec<&str> = out.library_apis[0]
            .pub_items
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["Beta", "alpha", "zeta"]);
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let inputs = PythonSourceInputs::default();
        let out = extract_python_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn async_function_is_extracted() {
        let body = "async def fetch():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "fetch");
    }

    #[test]
    fn dunder_name_does_not_get_private_attribute() {
        // `__init__`, `__all__` etc. are dunders, not single-leading
        // underscore conventional-private names.
        let body = "def __init__():\n    pass\n";
        let out = extract_python_surface("demo/comp", &input("pkg/mod.py", body));
        assert_eq!(out.bindings.len(), 1);
        assert!(!out.bindings[0].attributes.contains_key("private"));
    }

    #[test]
    fn extract_pyproject_project_name_pep621() {
        let toml = r#"
[project]
name = "my-pkg"
version = "0.1.0"
"#;
        assert_eq!(
            extract_pyproject_project_name(toml),
            Some("my-pkg".to_string())
        );
    }

    #[test]
    fn extract_pyproject_project_name_poetry() {
        let toml = r#"
[tool.poetry]
name = "my-pkg"
version = "0.1.0"
"#;
        assert_eq!(
            extract_pyproject_project_name(toml),
            Some("my-pkg".to_string())
        );
    }

    #[test]
    fn extract_pyproject_path_deps_poetry() {
        let toml = r#"
[tool.poetry]
name = "consumer"

[tool.poetry.dependencies]
python = "^3.11"
sibling = { path = "../sibling" }
serde = "^1.0"
"#;
        let deps = extract_pyproject_path_deps(toml);
        assert_eq!(deps, vec![PathBuf::from("../sibling")]);
    }

    #[test]
    fn extract_pyproject_path_deps_poetry_groups() {
        let toml = r#"
[tool.poetry]
name = "consumer"

[tool.poetry.dependencies]
python = "^3.11"

[tool.poetry.group.dev.dependencies]
test-helpers = { path = "../test-helpers" }
"#;
        let deps = extract_pyproject_path_deps(toml);
        assert_eq!(deps, vec![PathBuf::from("../test-helpers")]);
    }

    #[test]
    fn extract_pyproject_path_deps_uv_sources() {
        let toml = r#"
[project]
name = "consumer"

[tool.uv.sources]
sibling = { path = "../sibling" }
"#;
        let deps = extract_pyproject_path_deps(toml);
        assert_eq!(deps, vec![PathBuf::from("../sibling")]);
    }

    #[test]
    fn extract_pyproject_path_deps_returns_empty_for_no_path_deps() {
        let toml = r#"
[tool.poetry]
name = "consumer"

[tool.poetry.dependencies]
python = "^3.11"
serde = "^1.0"
"#;
        let deps = extract_pyproject_path_deps(toml);
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_pyproject_path_deps_malformed_input_returns_empty() {
        let deps = extract_pyproject_path_deps("this is not valid toml at all][");
        assert!(deps.is_empty());
    }

    #[test]
    fn analyzer_id_and_version_constants_are_stable() {
        assert_eq!(ANALYZER_ID, "python-surface-analyzer");
        assert_eq!(ANALYZER_VERSION, "1.0.0");
    }
}
