//! Phase 2 surface analyser for TypeScript / JavaScript components
//! (in-process).
//!
//! Sibling of [`crate::rust_surface_analyzer`]: extracts contracts,
//! bindings, and library-API items from a TS/JS package by parsing
//! its source files with `swc_ecma_parser`. The analyser is run
//! in-process — subprocess transport is PR-2's concern.
//!
//! ## Outputs
//!
//! For every top-level `export` declaration the analyser sees, a
//! [`Binding`] is emitted under the component's library API. For
//! TypeScript-only declarations (`export type`, `export interface`,
//! `export enum`), a [`LibraryApi`] `pub_item` is emitted with a
//! `type-alias` / `enum` kind respectively. CommonJS modules
//! (`module.exports = { foo, bar }` and `exports.foo = ...`) are
//! recognised when the source is `.js` and a CommonJS-style assignment
//! pattern is present.
//!
//! `package.json#main` / `module` / `exports` is resolved to a
//! [`LibraryApi`] entrypoint when the manifest contains one.
//!
//! ## Phase 1 binding shape (PR-1 caveat)
//!
//! The plan §4 refers to a `Visibility::Explicit` enum and a
//! `Binding.attributes` map. Those fields are introduced by PR-3
//! (Python). For PR-1 the analyser uses the **current** Phase 1
//! [`Binding`] shape (`language`, `symbol`, `file`, `span`,
//! `content_sha`). Module-system metadata (`commonjs` vs `esm`) and
//! type-only flags are encoded by suffixing the `language` field —
//! `typescript`, `typescript-type`, `javascript`, `javascript-cjs` —
//! as the only field available to the current schema. PR-3 will
//! migrate to the structured `attributes` map.
//!
//! ## Span convention
//!
//! Spans are `(start_byte, end_byte)` half-open ranges over the
//! binding's source file. SWC's `BytePos` is global to the SourceMap;
//! we subtract `SourceFile.start_pos` to get file-local offsets that
//! match Phase 1's contract-content-sha algorithm.

use std::path::{Path, PathBuf};

use atlas_index::{
    Binding, Contract, ContractKind, CostClass, LibraryApi, PubItem, PubItemKind, Stage,
};
use swc_common::{sync::Lrc, BytePos, FileName, SourceMap, Spanned};
use swc_ecma_ast::{
    Decl, DefaultDecl, ExportSpecifier, Expr, Lit, ModuleDecl, ModuleExportName, ModuleItem, Pat,
    Program, PropName, PropOrSpread, Stmt,
};
use swc_ecma_parser::{lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax};

use crate::{AnalysisContext, Analyzer, AnalyzerResult, FingerprintInput, Target};

/// Stable analyser id.
pub const ANALYZER_ID: &str = "ts-js-surface-analyzer";

/// Bumped when the extraction algorithm changes shape.
pub const ANALYZER_VERSION: &str = "1.0.0";

/// Output of one TS/JS surface analysis. Mirrors
/// [`crate::rust_surface_analyzer::RustSurfaceOutput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsJsSurfaceOutput {
    /// Code-derived contracts (currently empty — TS interfaces and
    /// type aliases are exposed via `library_apis` rather than
    /// promoted to data-format contracts in PR-1; PR-3 will rework
    /// that classification once `attributes` lands).
    pub contracts: Vec<Contract>,
    /// Every top-level binding emitted. One entry per exported
    /// identifier; CommonJS `module.exports = { foo, bar }` produces
    /// one entry per key.
    pub bindings: Vec<Binding>,
    /// At most one [`LibraryApi`] per language. Empty when the
    /// component exposes no exports.
    pub library_apis: Vec<LibraryApi>,
}

crate::impl_stage_output!(TsJsSurfaceOutput);

/// The analyser itself. Stateless.
#[derive(Debug, Default)]
pub struct TsJsSurfaceAnalyzer;

impl TsJsSurfaceAnalyzer {
    pub fn new() -> Self {
        TsJsSurfaceAnalyzer
    }
}

impl Analyzer for TsJsSurfaceAnalyzer {
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
        target.manifest_by_name("package.json").is_some()
    }

    fn fingerprint_inputs(&self, _target: &Target) -> Vec<FingerprintInput> {
        // Source files are not loaded into `Target.manifests`; the
        // engine-side L5 driver contributes file shas via the
        // FingerprintBuilder directly. Mirrors
        // [`crate::rust_surface_analyzer::RustSurfaceAnalyzer::fingerprint_inputs`].
        Vec::new()
    }

    fn analyse(&self, _ctx: &AnalysisContext, _target: &Target) -> AnalyzerResult {
        // Like the Rust surface analyser, the dispatcher path is
        // reserved for a future driver that hands the analyser a
        // populated `Target` carrying source bytes. The current PR-1
        // surface is exercised through [`extract_ts_js_surface`].
        AnalyzerResult::Declines
    }
}

/// Inputs describing one component's TS/JS source surface. The driver
/// fills this in by walking the component's source tree (`src/**.ts`,
/// `src/**.tsx`, `src/**.js`, `src/**.jsx`), then calls
/// [`extract_ts_js_surface`].
#[derive(Debug, Clone, Default)]
pub struct TsJsSourceInputs {
    /// `(relative_file_path, file_bytes)` pairs. Empty `bytes` are
    /// tolerated and produce no output. The analyser keys on the file
    /// extension to pick TS vs JS parser syntax — `.ts` / `.tsx` →
    /// TypeScript; `.js` / `.jsx` / `.mjs` / `.cjs` → JavaScript.
    pub sources: Vec<(PathBuf, Vec<u8>)>,
    /// Optional `package.json` contents. When present the analyser
    /// resolves the `main` / `module` / `exports` field into the
    /// component's [`LibraryApi`] entrypoint. Failing to parse the
    /// manifest is a non-fatal warning — the surface still includes
    /// every parsed export.
    pub package_json: Option<Vec<u8>>,
    /// Whether the component is classified as a `typescript-package`
    /// (true) or a `javascript-package` (false). Drives the language
    /// label on the emitted [`LibraryApi`]. Files with explicit
    /// extensions still parse with the correct syntax regardless of
    /// this flag.
    pub is_typescript: bool,
}

/// Drive the TS/JS-surface extraction over the component's source
/// inputs. Returns the contracts, bindings, and `LibraryApi` (at most
/// one per language) discovered.
///
/// `component_id` is the owning component's id (e.g.
/// `repo/my-package`); the resulting library-api id is
/// `<component_id>/public-api`.
pub fn extract_ts_js_surface(component_id: &str, inputs: &TsJsSourceInputs) -> TsJsSurfaceOutput {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut pub_items: Vec<PubItem> = Vec::new();

    for (rel_path, bytes) in &inputs.sources {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue; // non-UTF-8 source is rejected silently
        };
        let kind = source_kind_for(rel_path);
        let mut file_results = match kind {
            SourceKind::TypeScript => parse_typescript(rel_path, text),
            SourceKind::TypeScriptJsx => parse_typescript_jsx(rel_path, text),
            SourceKind::JavaScript => parse_javascript(rel_path, text),
            SourceKind::JavaScriptJsx => parse_javascript_jsx(rel_path, text),
            SourceKind::Unknown => continue,
        };
        bindings.append(&mut file_results.bindings);
        pub_items.append(&mut file_results.pub_items);
    }

    // Resolve package.json#main / #module / #exports into a synthetic
    // entrypoint pub_item — gives the LibraryApi an unambiguous
    // entrypoint name even when the source files do not carry one
    // directly. The entry is recorded as a `pub_item` of kind `mod`
    // (the closest existing variant) with name `entrypoint`.
    if let Some(pkg_bytes) = &inputs.package_json {
        if let Ok(pkg_text) = std::str::from_utf8(pkg_bytes) {
            if let Some(entry) = resolve_package_entrypoint(pkg_text) {
                pub_items.push(PubItem {
                    name: "entrypoint".into(),
                    file: PathBuf::from(entry),
                    kind: PubItemKind::Mod,
                });
            }
        }
    }

    let language_label = if inputs.is_typescript {
        "typescript"
    } else {
        "javascript"
    };

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
            language: language_label.into(),
            fingerprint: api_fp,
            pub_items: sorted,
        };
        api.validate()
            .expect("constructed LibraryApi has kind=LibraryApi by construction");
        vec![api]
    };

    TsJsSurfaceOutput {
        contracts: Vec::new(),
        bindings,
        library_apis,
    }
}

/// Per-file extraction result.
#[derive(Debug, Default)]
struct FileResult {
    bindings: Vec<Binding>,
    pub_items: Vec<PubItem>,
}

#[derive(Debug, Clone, Copy)]
enum SourceKind {
    TypeScript,
    TypeScriptJsx,
    JavaScript,
    JavaScriptJsx,
    Unknown,
}

fn source_kind_for(path: &std::path::Path) -> SourceKind {
    match path.extension().and_then(|s| s.to_str()) {
        Some("ts") => SourceKind::TypeScript,
        Some("tsx") => SourceKind::TypeScriptJsx,
        Some("mts") | Some("cts") => SourceKind::TypeScript,
        Some("js") | Some("mjs") | Some("cjs") => SourceKind::JavaScript,
        Some("jsx") => SourceKind::JavaScriptJsx,
        _ => SourceKind::Unknown,
    }
}

fn parse_typescript(rel: &Path, text: &str) -> FileResult {
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: false,
        decorators: false,
        dts: rel.to_string_lossy().ends_with(".d.ts"),
        no_early_errors: true,
        disallow_ambiguous_jsx_like: false,
    });
    parse_with_syntax(rel, text, syntax, "typescript")
}

fn parse_typescript_jsx(rel: &Path, text: &str) -> FileResult {
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: true,
        decorators: false,
        dts: false,
        no_early_errors: true,
        disallow_ambiguous_jsx_like: false,
    });
    parse_with_syntax(rel, text, syntax, "typescript")
}

fn parse_javascript(rel: &Path, text: &str) -> FileResult {
    let syntax = Syntax::Es(EsSyntax::default());
    parse_with_syntax(rel, text, syntax, "javascript")
}

fn parse_javascript_jsx(rel: &Path, text: &str) -> FileResult {
    let syntax = Syntax::Es(EsSyntax {
        jsx: true,
        ..EsSyntax::default()
    });
    parse_with_syntax(rel, text, syntax, "javascript")
}

/// Run swc against `text` with the given `syntax`. Best-effort: parse
/// failures (recoverable errors) are tolerated — the analyser emits
/// whatever items it could recognise. A *catastrophic* parser failure
/// (the parser returns `Err`) yields an empty FileResult so the
/// caller still gets a deterministic empty output for that file.
fn parse_with_syntax(
    rel: &Path,
    text: &str,
    syntax: Syntax,
    language_label: &'static str,
) -> FileResult {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        Lrc::new(FileName::Real(rel.to_path_buf())),
        text.to_string(),
    );
    let lexer = Lexer::new(syntax, Default::default(), StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(_) => return FileResult::default(),
    };
    let start = fm.start_pos;

    let mut result = FileResult::default();
    let bytes = text.as_bytes();

    match program {
        Program::Module(module) => {
            for item in &module.body {
                if let ModuleItem::ModuleDecl(decl) = item {
                    extract_module_decl(decl, rel, bytes, start, language_label, &mut result);
                } else if let ModuleItem::Stmt(stmt) = item {
                    // CommonJS detection runs on script bodies too,
                    // but a module body containing `module.exports`
                    // is unusual; tolerate it for completeness.
                    extract_commonjs_stmt(stmt, rel, bytes, start, language_label, &mut result);
                }
            }
        }
        Program::Script(script) => {
            for stmt in &script.body {
                extract_commonjs_stmt(stmt, rel, bytes, start, language_label, &mut result);
            }
        }
    }

    result
}

/// Extract any exports from one ESM module-level declaration.
fn extract_module_decl(
    decl: &ModuleDecl,
    rel: &Path,
    bytes: &[u8],
    start: BytePos,
    language_label: &'static str,
    out: &mut FileResult,
) {
    match decl {
        ModuleDecl::ExportDecl(export) => {
            // `export <decl>` covering function / class / var / type /
            // interface / enum.
            extract_decl_for_export(&export.decl, rel, bytes, start, language_label, out);
        }
        ModuleDecl::ExportNamed(named) => {
            // `export { foo, bar }` and `export { foo } from 'mod'`.
            // The latter is a re-export; we still record the symbol
            // because surface analysis cares about the public-API
            // shape regardless of whether the implementation is
            // local. The `is_type_only` flag flows through to the
            // language label so PR-3's `attributes` upgrade preserves
            // the distinction.
            for spec in &named.specifiers {
                if let ExportSpecifier::Named(named_spec) = spec {
                    let exported_name = match &named_spec.exported {
                        Some(ModuleExportName::Ident(id)) => id.sym.to_string(),
                        Some(ModuleExportName::Str(s)) => s.value.to_string_lossy().into_owned(),
                        None => match &named_spec.orig {
                            ModuleExportName::Ident(id) => id.sym.to_string(),
                            ModuleExportName::Str(s) => s.value.to_string_lossy().into_owned(),
                        },
                    };
                    let span = (
                        byte_offset(named_spec.span.lo, start),
                        byte_offset(named_spec.span.hi, start),
                    );
                    let lang = language_label_for(
                        language_label,
                        named.type_only || named_spec.is_type_only,
                    );
                    push_binding(out, lang.clone(), exported_name.clone(), rel, span, bytes);
                    push_pub_item(
                        out,
                        exported_name,
                        rel,
                        if named.type_only || named_spec.is_type_only {
                            PubItemKind::TypeAlias
                        } else {
                            PubItemKind::Fn
                        },
                    );
                }
            }
        }
        ModuleDecl::ExportDefaultDecl(default_decl) => {
            // `export default function Foo() {}` / `export default class Foo {}`.
            let (name, kind, span_owner) = match &default_decl.decl {
                DefaultDecl::Fn(fn_expr) => (
                    fn_expr
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    PubItemKind::Fn,
                    default_decl.span,
                ),
                DefaultDecl::Class(class_expr) => (
                    class_expr
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    PubItemKind::Struct,
                    default_decl.span,
                ),
                DefaultDecl::TsInterfaceDecl(iface) => (
                    iface.id.sym.to_string(),
                    PubItemKind::TypeAlias,
                    default_decl.span,
                ),
            };
            let span = (
                byte_offset(span_owner.lo, start),
                byte_offset(span_owner.hi, start),
            );
            push_binding(
                out,
                language_label.to_string(),
                name.clone(),
                rel,
                span,
                bytes,
            );
            push_pub_item(out, name, rel, kind);
        }
        ModuleDecl::ExportDefaultExpr(default_expr) => {
            // `export default <expression>`. We do not attempt to
            // recover a name from arbitrary expressions; emit a
            // binding with symbol `default`.
            let span = (
                byte_offset(default_expr.span.lo, start),
                byte_offset(default_expr.span.hi, start),
            );
            push_binding(
                out,
                language_label.to_string(),
                "default".into(),
                rel,
                span,
                bytes,
            );
            push_pub_item(out, "default".into(), rel, PubItemKind::Fn);
        }
        ModuleDecl::ExportAll(_)
        | ModuleDecl::Import(_)
        | ModuleDecl::TsImportEquals(_)
        | ModuleDecl::TsExportAssignment(_)
        | ModuleDecl::TsNamespaceExport(_) => {
            // PR-1 ignores `export *`, imports, and TS-specific
            // import/export forms. PR-3 may revisit; for now they do
            // not contribute to the library API.
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// Extract from `export <decl>`'s inner declaration.
fn extract_decl_for_export(
    decl: &Decl,
    rel: &Path,
    bytes: &[u8],
    start: BytePos,
    language_label: &'static str,
    out: &mut FileResult,
) {
    match decl {
        Decl::Fn(fn_decl) => {
            let name = fn_decl.ident.sym.to_string();
            let span_owner = fn_decl.function.span;
            let span = (
                byte_offset(span_owner.lo, start),
                byte_offset(span_owner.hi, start),
            );
            push_binding(
                out,
                language_label.to_string(),
                name.clone(),
                rel,
                span,
                bytes,
            );
            push_pub_item(out, name, rel, PubItemKind::Fn);
        }
        Decl::Class(class_decl) => {
            let name = class_decl.ident.sym.to_string();
            let span_owner = class_decl.class.span;
            let span = (
                byte_offset(span_owner.lo, start),
                byte_offset(span_owner.hi, start),
            );
            push_binding(
                out,
                language_label.to_string(),
                name.clone(),
                rel,
                span,
                bytes,
            );
            push_pub_item(out, name, rel, PubItemKind::Struct);
        }
        Decl::Var(var_decl) => {
            // `export const foo = ...` / `export let bar = ...` /
            // `export var baz = ...`. Each declarator becomes its
            // own binding.
            for declarator in &var_decl.decls {
                if let Pat::Ident(binding_ident) = &declarator.name {
                    let name = binding_ident.id.sym.to_string();
                    let span_owner = declarator.span;
                    let span = (
                        byte_offset(span_owner.lo, start),
                        byte_offset(span_owner.hi, start),
                    );
                    push_binding(
                        out,
                        language_label.to_string(),
                        name.clone(),
                        rel,
                        span,
                        bytes,
                    );
                    push_pub_item(out, name, rel, PubItemKind::Const);
                }
                // Destructuring patterns (`export const { a, b } = x`)
                // are not yet recovered. PR-3 may revisit.
            }
        }
        Decl::TsTypeAlias(alias) => {
            let name = alias.id.sym.to_string();
            let span = (
                byte_offset(alias.span.lo, start),
                byte_offset(alias.span.hi, start),
            );
            // Type-only export: encode in the language label until
            // PR-3 lands `attributes`.
            let lang = language_label_for(language_label, true);
            push_binding(out, lang, name.clone(), rel, span, bytes);
            push_pub_item(out, name, rel, PubItemKind::TypeAlias);
        }
        Decl::TsInterface(iface) => {
            let name = iface.id.sym.to_string();
            let span = (
                byte_offset(iface.span.lo, start),
                byte_offset(iface.span.hi, start),
            );
            let lang = language_label_for(language_label, true);
            push_binding(out, lang, name.clone(), rel, span, bytes);
            push_pub_item(out, name, rel, PubItemKind::TypeAlias);
        }
        Decl::TsEnum(ts_enum) => {
            let name = ts_enum.id.sym.to_string();
            let span = (
                byte_offset(ts_enum.span.lo, start),
                byte_offset(ts_enum.span.hi, start),
            );
            push_binding(
                out,
                language_label.to_string(),
                name.clone(),
                rel,
                span,
                bytes,
            );
            push_pub_item(out, name, rel, PubItemKind::Enum);
        }
        Decl::TsModule(_) | Decl::Using(_) => {
            // Phase 2 PR-1 ignores `module` and `using` declarations.
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// CommonJS export extraction. Walks script-level statements looking for:
///
/// - `module.exports = { foo, bar }` — emit one binding per key.
/// - `module.exports = identifier` — emit a binding for `identifier`.
/// - `module.exports.foo = ...` / `exports.foo = ...` — emit a
///   binding for `foo`.
fn extract_commonjs_stmt(
    stmt: &Stmt,
    rel: &Path,
    bytes: &[u8],
    start: BytePos,
    language_label: &'static str,
    out: &mut FileResult,
) {
    let Stmt::Expr(expr_stmt) = stmt else { return };
    let Expr::Assign(assign) = &*expr_stmt.expr else {
        return;
    };
    // `module.exports = ...` / `exports.foo = ...` etc.
    let target = match &assign.left {
        swc_ecma_ast::AssignTarget::Simple(simple) => simple,
        _ => return,
    };

    let cjs_lang = format!("{language_label}-cjs");
    if is_module_exports_target(target) {
        // `module.exports = <rhs>`. RHS may be an object literal or
        // an identifier; both shapes contribute.
        match &*assign.right {
            Expr::Object(obj) => {
                for prop in &obj.props {
                    if let PropOrSpread::Prop(p) = prop {
                        let prop_name = property_name(p);
                        if let Some(name) = prop_name {
                            let span = (
                                byte_offset(prop.span().lo, start),
                                byte_offset(prop.span().hi, start),
                            );
                            push_binding(out, cjs_lang.clone(), name.clone(), rel, span, bytes);
                            push_pub_item(out, name, rel, PubItemKind::Fn);
                        }
                    }
                }
            }
            Expr::Ident(ident) => {
                let name = ident.sym.to_string();
                let span = (
                    byte_offset(assign.span.lo, start),
                    byte_offset(assign.span.hi, start),
                );
                push_binding(out, cjs_lang.clone(), name.clone(), rel, span, bytes);
                push_pub_item(out, name, rel, PubItemKind::Fn);
            }
            _ => {}
        }
        return;
    }
    // `module.exports.foo = ...` / `exports.foo = ...`.
    if let Some(name) = property_assign_name(target) {
        let span = (
            byte_offset(assign.span.lo, start),
            byte_offset(assign.span.hi, start),
        );
        push_binding(out, cjs_lang, name.clone(), rel, span, bytes);
        push_pub_item(out, name, rel, PubItemKind::Fn);
    }
}

/// True iff `target` is the bare `module.exports` member expression.
fn is_module_exports_target(target: &swc_ecma_ast::SimpleAssignTarget) -> bool {
    let swc_ecma_ast::SimpleAssignTarget::Member(member) = target else {
        return false;
    };
    let Expr::Ident(obj) = &*member.obj else {
        return false;
    };
    if obj.sym.as_ref() != "module" {
        return false;
    }
    matches!(&member.prop, swc_ecma_ast::MemberProp::Ident(p) if p.sym.as_ref() == "exports")
}

/// Recover the property name from `module.exports.foo = ...` or
/// `exports.foo = ...`.
fn property_assign_name(target: &swc_ecma_ast::SimpleAssignTarget) -> Option<String> {
    let swc_ecma_ast::SimpleAssignTarget::Member(member) = target else {
        return None;
    };
    // Case A: `exports.foo = ...`.
    if let Expr::Ident(obj) = &*member.obj {
        if obj.sym.as_ref() == "exports" {
            if let swc_ecma_ast::MemberProp::Ident(p) = &member.prop {
                return Some(p.sym.to_string());
            }
        }
    }
    // Case B: `module.exports.foo = ...`. The outer `member` is
    // `module.exports.foo`; its `obj` is the inner `module.exports`.
    if let Expr::Member(inner) = &*member.obj {
        if let Expr::Ident(obj) = &*inner.obj {
            if obj.sym.as_ref() == "module" {
                if let swc_ecma_ast::MemberProp::Ident(inner_prop) = &inner.prop {
                    if inner_prop.sym.as_ref() == "exports" {
                        if let swc_ecma_ast::MemberProp::Ident(p) = &member.prop {
                            return Some(p.sym.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Recover the textual property name from one object-literal entry.
/// Returns `None` for spread / private / computed properties whose
/// names are not statically known.
fn property_name(prop: &swc_ecma_ast::Prop) -> Option<String> {
    match prop {
        swc_ecma_ast::Prop::Shorthand(ident) => Some(ident.sym.to_string()),
        swc_ecma_ast::Prop::KeyValue(kv) => prop_name_to_string(&kv.key),
        swc_ecma_ast::Prop::Method(m) => prop_name_to_string(&m.key),
        swc_ecma_ast::Prop::Getter(g) => prop_name_to_string(&g.key),
        swc_ecma_ast::Prop::Setter(s) => prop_name_to_string(&s.key),
        swc_ecma_ast::Prop::Assign(_) => None,
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

fn prop_name_to_string(name: &PropName) -> Option<String> {
    match name {
        PropName::Ident(id) => Some(id.sym.to_string()),
        PropName::Str(s) => Some(s.value.to_string_lossy().into_owned()),
        PropName::Num(n) => Some(n.value.to_string()),
        PropName::BigInt(b) => Some(b.value.to_string()),
        PropName::Computed(_) => None,
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Compute file-local byte offset from an SWC global `BytePos`. SWC
/// `BytePos` is 1-indexed within the SourceMap; subtract the source
/// file's `start_pos` to get the 0-indexed file-local offset that
/// matches the `bytes[start..end]` content-sha algorithm.
fn byte_offset(pos: BytePos, start: BytePos) -> usize {
    (pos.0.saturating_sub(start.0)) as usize
}

fn push_binding(
    out: &mut FileResult,
    language: String,
    symbol: String,
    rel: &Path,
    span: (usize, usize),
    bytes: &[u8],
) {
    let content_sha = crate::sha256_hex_of_range(bytes, span);
    out.bindings.push(Binding {
        language,
        symbol,
        file: rel.to_path_buf(),
        span,
        content_sha,
    });
}

fn push_pub_item(out: &mut FileResult, name: String, rel: &Path, kind: PubItemKind) {
    out.pub_items.push(PubItem {
        name,
        file: rel.to_path_buf(),
        kind,
    });
}

/// Encode the type-only flag into the language label, since PR-1 has
/// no `attributes` map. PR-3 will refactor to use the structured
/// attribute slot.
fn language_label_for(base: &'static str, type_only: bool) -> String {
    if type_only {
        format!("{base}-type")
    } else {
        base.to_string()
    }
}

/// Resolve `package.json#main` / `module` / `exports` to a single
/// entrypoint path string. Returns `None` if no recognised field is
/// present. The resolution order matches Node.js's:
///
/// 1. `exports` (string form, or `"."` key in object form).
/// 2. `module` (the ESM convention).
/// 3. `main` (the CommonJS convention).
fn resolve_package_entrypoint(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;

    if let Some(exports) = obj.get("exports") {
        if let Some(s) = exports.as_str() {
            return Some(s.to_string());
        }
        if let Some(map) = exports.as_object() {
            // Prefer the "." key, then any string entry.
            if let Some(dot) = map.get(".") {
                if let Some(s) = dot.as_str() {
                    return Some(s.to_string());
                }
                if let Some(inner) = dot.as_object() {
                    if let Some(import) = inner.get("import").and_then(|v| v.as_str()) {
                        return Some(import.to_string());
                    }
                    if let Some(require) = inner.get("require").and_then(|v| v.as_str()) {
                        return Some(require.to_string());
                    }
                    if let Some(default) = inner.get("default").and_then(|v| v.as_str()) {
                        return Some(default.to_string());
                    }
                }
            }
        }
    }
    if let Some(module) = obj.get("module").and_then(|v| v.as_str()) {
        return Some(module.to_string());
    }
    if let Some(main) = obj.get("main").and_then(|v| v.as_str()) {
        return Some(main.to_string());
    }
    None
}

/// SHA-256 hex of the canonicalised public-API surface — same form as
/// the Rust analyser, so a polyglot component's library APIs hash
/// uniformly.
fn library_api_fingerprint(api_id: &str, items: &[PubItem]) -> String {
    use sha2::{Digest, Sha256};
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

/// Pull a string literal out of an `Expr` if it is one.
#[allow(dead_code)] // Reserved for future entrypoint-resolution logic.
fn string_lit(expr: &Expr) -> Option<String> {
    if let Expr::Lit(Lit::Str(s)) = expr {
        Some(s.value.to_string_lossy().into_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rel: &str, body: &str) -> TsJsSourceInputs {
        TsJsSourceInputs {
            sources: vec![(PathBuf::from(rel), body.as_bytes().to_vec())],
            package_json: None,
            is_typescript: rel.ends_with(".ts") || rel.ends_with(".tsx"),
        }
    }

    #[test]
    fn ts_extracts_named_exports() {
        // Acceptance criterion: `export function foo() {}` and
        // `export class Bar {}` produce two `Binding` records.
        let body = "export function foo() {}\nexport class Bar {}\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.ts", body));
        assert_eq!(out.bindings.len(), 2, "got: {:?}", out.bindings);
        let names: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"Bar"));
        assert_eq!(out.library_apis.len(), 1);
        assert_eq!(out.library_apis[0].language, "typescript");
        assert_eq!(out.library_apis[0].pub_items.len(), 2);
    }

    #[test]
    fn ts_extracts_type_only_export() {
        // Acceptance criterion: `export type Foo = string` produces a
        // `Binding` with the type-only attribute (encoded in the
        // language label until PR-3's `attributes` lands).
        let body = "export type Foo = string;\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/types.ts", body));
        assert_eq!(out.bindings.len(), 1, "got: {:?}", out.bindings);
        let b = &out.bindings[0];
        assert_eq!(b.symbol, "Foo");
        assert_eq!(
            b.language, "typescript-type",
            "type-only flag must be encoded in language label"
        );
        assert_eq!(
            out.library_apis[0].pub_items[0].kind,
            PubItemKind::TypeAlias
        );
    }

    #[test]
    fn ts_extracts_interface_as_type_alias_pub_item() {
        let body = "export interface Foo { a: number; }\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/types.ts", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].language, "typescript-type");
        assert_eq!(
            out.library_apis[0].pub_items[0].kind,
            PubItemKind::TypeAlias
        );
    }

    #[test]
    fn ts_extracts_default_export() {
        let body = "export default function answer() { return 42; }\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/main.ts", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "answer");
    }

    #[test]
    fn ts_extracts_const_export() {
        let body = "export const NAME = \"example\";\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.ts", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].symbol, "NAME");
        assert_eq!(out.library_apis[0].pub_items[0].kind, PubItemKind::Const);
    }

    #[test]
    fn ts_named_re_export_is_recognised() {
        // `export { foo } from './other'` re-exports `foo` from a
        // sibling module. The surface analyser records the name even
        // though the implementation lives elsewhere.
        let body = "export { foo } from './other';\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.ts", body));
        let names: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(names.contains(&"foo"), "got: {names:?}");
    }

    #[test]
    fn js_extracts_commonjs_exports() {
        // Acceptance criterion: `module.exports = { foo, bar }`
        // produces two `Binding` records.
        let body = "function foo() {}\nfunction bar() {}\nmodule.exports = { foo, bar };\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.js", body));
        let cjs_bindings: Vec<&Binding> = out
            .bindings
            .iter()
            .filter(|b| b.language == "javascript-cjs")
            .collect();
        assert_eq!(
            cjs_bindings.len(),
            2,
            "expected 2 CJS bindings, got: {:?}",
            out.bindings
        );
        let names: Vec<&str> = cjs_bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
    }

    #[test]
    fn js_extracts_commonjs_property_assignments() {
        let body = "exports.alpha = function() {};\nmodule.exports.beta = 42;\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.js", body));
        let names: Vec<&str> = out.bindings.iter().map(|b| b.symbol.as_str()).collect();
        assert!(names.contains(&"alpha"), "got: {names:?}");
        assert!(names.contains(&"beta"), "got: {names:?}");
        // Both bindings carry the `-cjs` language label so PR-3's
        // attribute-aware schema can tell them apart from ESM.
        for b in &out.bindings {
            assert_eq!(b.language, "javascript-cjs");
        }
    }

    #[test]
    fn js_esm_export_uses_plain_javascript_label() {
        // When a `.js` file uses ESM `export`, the language label is
        // `javascript` (not `javascript-cjs`).
        let body = "export function foo() {}\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.js", body));
        assert_eq!(out.bindings.len(), 1);
        assert_eq!(out.bindings[0].language, "javascript");
    }

    #[test]
    fn empty_inputs_emit_empty_output() {
        let inputs = TsJsSourceInputs::default();
        let out = extract_ts_js_surface("demo/comp", &inputs);
        assert!(out.contracts.is_empty());
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn no_exports_emit_no_library_api() {
        let body = "function privateFn() {}\nclass Hidden {}\n";
        let out = extract_ts_js_surface("demo/comp", &input("src/index.ts", body));
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn package_json_main_resolves_to_entrypoint_pub_item() {
        let pkg = "{\"name\":\"x\",\"main\":\"dist/index.js\"}";
        let inputs = TsJsSourceInputs {
            sources: vec![(
                PathBuf::from("src/index.ts"),
                b"export const X = 1;\n".to_vec(),
            )],
            package_json: Some(pkg.as_bytes().to_vec()),
            is_typescript: true,
        };
        let out = extract_ts_js_surface("demo/comp", &inputs);
        let entrypoint_items: Vec<&PubItem> = out
            .library_apis
            .iter()
            .flat_map(|api| api.pub_items.iter())
            .filter(|p| p.name == "entrypoint")
            .collect();
        assert_eq!(entrypoint_items.len(), 1);
        assert_eq!(entrypoint_items[0].file, PathBuf::from("dist/index.js"));
    }

    #[test]
    fn package_json_module_takes_precedence_over_main() {
        let pkg = "{\"main\":\"dist/index.cjs\",\"module\":\"dist/index.mjs\"}";
        let entry = resolve_package_entrypoint(pkg).unwrap();
        assert_eq!(entry, "dist/index.mjs");
    }

    #[test]
    fn package_json_exports_dot_key_takes_precedence_over_module() {
        let pkg = r#"{"exports":{"." :"dist/main.mjs"}, "module":"dist/index.mjs"}"#;
        let entry = resolve_package_entrypoint(pkg).unwrap();
        assert_eq!(entry, "dist/main.mjs");
    }

    #[test]
    fn package_json_exports_string_form_resolves() {
        let pkg = r#"{"exports":"dist/index.mjs"}"#;
        let entry = resolve_package_entrypoint(pkg).unwrap();
        assert_eq!(entry, "dist/index.mjs");
    }

    #[test]
    fn library_api_pub_items_are_sorted_by_file_then_name() {
        let inputs = TsJsSourceInputs {
            sources: vec![
                (
                    PathBuf::from("src/secondary.ts"),
                    b"export function zeta() {}\n".to_vec(),
                ),
                (
                    PathBuf::from("src/index.ts"),
                    b"export function alpha() {}\nexport function beta() {}\n".to_vec(),
                ),
            ],
            package_json: None,
            is_typescript: true,
        };
        let out = extract_ts_js_surface("ns/comp", &inputs);
        let api = &out.library_apis[0];
        let names: Vec<&str> = api.pub_items.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "zeta"]);
    }

    #[test]
    fn library_api_id_follows_component_id_convention() {
        let body = "export function alpha() {}\n";
        let out = extract_ts_js_surface("foo/bar", &input("src/index.ts", body));
        assert_eq!(out.library_apis[0].id, "foo/bar/public-api");
    }

    #[test]
    fn ignores_non_utf8_source_bytes() {
        let inputs = TsJsSourceInputs {
            sources: vec![(PathBuf::from("src/index.ts"), vec![0xFF, 0xFE, 0xFD])],
            package_json: None,
            is_typescript: true,
        };
        let out = extract_ts_js_surface("ns/comp", &inputs);
        assert!(out.bindings.is_empty());
        assert!(out.library_apis.is_empty());
    }

    #[test]
    fn analyzer_trait_metadata_is_stable() {
        let an = TsJsSurfaceAnalyzer::new();
        assert_eq!(an.id(), ANALYZER_ID);
        assert_eq!(an.stage(), Stage::L5);
        assert_eq!(an.cost_class(), CostClass::DeterministicCheap);
        assert_eq!(an.version(), ANALYZER_VERSION);
    }

    #[test]
    fn applies_is_true_when_package_json_present() {
        let target = Target {
            dir: PathBuf::from("/tmp/x"),
            languages: std::collections::BTreeSet::new(),
            manifests: vec![crate::TargetFile {
                name: "package.json".into(),
                relpath: PathBuf::from("package.json"),
                bytes: b"{}".to_vec(),
                content_sha: "abc".into(),
            }],
            top_level_files: Vec::new(),
        };
        assert!(TsJsSurfaceAnalyzer::new().applies(&target));
    }

    #[test]
    fn pub_struct_with_serde_style_jsdoc_is_irrelevant_to_binding_sha() {
        // Sanity check that doc-comments OUTSIDE the binding span do
        // not affect the content sha (the binding span starts at the
        // `function` keyword in swc).
        let body1 = "export function alpha() { return 1; }\n";
        let body2 = "/** doc */\nexport function alpha() { return 1; }\n";
        let o1 = extract_ts_js_surface("c", &input("src/index.ts", body1));
        let o2 = extract_ts_js_surface("c", &input("src/index.ts", body2));
        assert_eq!(o1.bindings.len(), 1);
        assert_eq!(o2.bindings.len(), 1);
        // The function span runs from `function` to the closing brace
        // — invariant under leading doc-comments.
        assert_eq!(
            o1.bindings[0].content_sha, o2.bindings[0].content_sha,
            "doc comments outside the span must not affect the binding sha"
        );
    }
}
