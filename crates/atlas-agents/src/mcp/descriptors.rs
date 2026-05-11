//! Convert `Tool::json_schema()` into MCP `tools/list` descriptors.
//!
//! MCP tool-descriptor shape (per the upstream protocol):
//!
//! ```json
//! {
//!   "name": "tool_id",
//!   "description": "...",
//!   "inputSchema": <json-schema-object>
//! }
//! ```
//!
//! Atlas does not yet wire MCP `outputSchema` — the runtime treats every
//! tool result as opaque JSON forwarded back to the LLM.

use serde_json::{json, Value};

use crate::tool::ToolHandle;

/// Lift a single `Tool` into its MCP tool-descriptor JSON shape.
pub fn tool_to_descriptor(tool: &ToolHandle) -> Value {
    let schema = tool.json_schema();
    json!({
        "name": tool.id(),
        "description": schema.description.clone(),
        "inputSchema": schema.args_schema.clone(),
    })
}

/// Lift the entire registered tool catalog into the `tools/list`
/// response payload (`{"tools": [...]}`).
pub fn tools_list_response(tools: &[ToolHandle]) -> Value {
    let descriptors: Vec<Value> = tools.iter().map(tool_to_descriptor).collect();
    json!({ "tools": descriptors })
}
