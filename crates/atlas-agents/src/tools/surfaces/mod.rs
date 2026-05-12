//! Surface analyser tool wrappers (PR-3b: Python, C#, Dart).
//!
//! Each module exposes one `Tool` impl that wraps the corresponding
//! subprocess surface analyser. All three follow Pattern B (subprocess
//! proxy): they locate the analyser binary at runtime, construct a
//! `SubprocessAnalyzerProxy` via the process-wide cache, call
//! `proxy.analyse()` inside `spawn_blocking`, and forward the raw JSON
//! payload to the caller unchanged.

pub mod csharp;
pub mod dart;
pub mod python;

pub use csharp::CsharpSurfaceTool;
pub use dart::DartSurfaceTool;
pub use python::PythonSurfaceTool;
