//! Classifier tool wrappers.
//!
//! Each module exposes one `Tool` impl that wraps the corresponding
//! `atlas_analyzers` L3 classifier as a pure pass-through. No new
//! analysis logic lives here — all decisions come from the underlying
//! analyser.

pub mod cargo;
pub mod csharp;
pub mod dart;
pub mod python;
pub mod ts_js;

pub use cargo::CargoClassifyTool;
pub use csharp::CsharpClassifyTool;
pub use dart::DartClassifyTool;
pub use python::PythonClassifyTool;
pub use ts_js::TsJsClassifyTool;
