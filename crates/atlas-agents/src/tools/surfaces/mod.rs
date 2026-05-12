//! Surface analyser tool wrappers.
//!
//! In-process wrappers for Rust + TS/JS (call `extract_*_surface`
//! directly because their `Analyzer::analyse` impls intentionally
//! return `Declines`); subprocess-proxy wrappers for the others
//! locate the analyser binary at runtime, construct a
//! `SubprocessAnalyzerProxy` via the process-wide cache, call
//! `proxy.analyse()` inside `spawn_blocking`, and forward the raw JSON
//! payload to the caller unchanged.

pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod lispkit;
pub mod python;
pub mod racket;
pub mod rust;
pub mod ts_js;

pub use csharp::CsharpSurfaceTool;
pub use dart::DartSurfaceTool;
pub use elixir::ElixirSurfaceTool;
pub use lispkit::LispKitSurfaceTool;
pub use python::PythonSurfaceTool;
pub use racket::RacketSurfaceTool;
pub use rust::RustSurfaceTool;
pub use ts_js::TsJsSurfaceTool;
