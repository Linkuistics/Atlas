//! Two concurrent in-process clients connecting to one `McpServer`,
//! issuing interleaved `tools/call` requests. Verifies isolation +
//! correctness under concurrency: each client's JSON-RPC `id` value
//! round-trips back on that client's socket without crossing wires,
//! and each client's `arguments` payload is echoed back verbatim.
//!
//! The test is the cornerstone acceptance probe for PR-1's
//! multi-client multiplexing requirement, preserved through PR-A's
//! migration of the underlying JSON-RPC framing onto `rmcp`. Test
//! logic (concurrency setup, isolation assertions, id round-trip,
//! payload-per-client) is unchanged; the wire-shape assertions
//! adapt to the standard MCP envelope `rmcp` emits (initialize
//! handshake first; `structuredContent` for tool-call payloads).

use std::sync::Arc;
use std::sync::OnceLock;

use atlas_agents::mcp::server::McpServer;
use atlas_agents::mcp::ClientId;
use atlas_agents::{
    FingerprintInput, Tool, ToolArgs, ToolContext, ToolError, ToolResult, ToolSchema,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Minimal `Tool` impl that echoes its args back as the result. The
/// MCP server emits this via `CallToolResult::structured(...)`, which
/// places the args in `structuredContent` (and a textual rendering in
/// `content[0]`); the test extracts from `structuredContent`.
struct EchoTool;

fn echo_schema() -> &'static ToolSchema {
    static SCHEMA: OnceLock<ToolSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| ToolSchema {
        args_schema: json!({
            "type": "object",
            "properties": {
                "payload": { "type": "string" }
            },
            "required": ["payload"],
            "additionalProperties": true
        }),
        description: "Echo the args payload back verbatim. Test-only tool.".to_string(),
    })
}

#[async_trait::async_trait]
impl Tool for EchoTool {
    fn id(&self) -> &'static str {
        "echo"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn json_schema(&self) -> &ToolSchema {
        echo_schema()
    }
    async fn invoke(&self, args: ToolArgs, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let bytes = serde_json::to_vec(&args.0)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        Ok(ToolResult {
            output: args.0,
            bytes,
        })
    }
    fn fingerprint_inputs(&self, _args: &ToolArgs) -> Vec<FingerprintInput> {
        vec![]
    }
}

/// Spawn a `serve_client` task wired to one half of a `duplex` pipe.
/// Returns the other half (suitable for the test to read/write as the
/// "client" side) plus the task handle.
fn spawn_client(
    server: Arc<McpServer>,
    client_id: ClientId,
) -> (
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (client_side, server_side) = tokio::io::duplex(4096);
    let (server_reader, server_writer) = tokio::io::split(server_side);
    let handle = tokio::spawn(async move {
        server
            .serve_client(client_id, server_reader, server_writer)
            .await
    });
    (client_side, handle)
}

/// Send one JSON-RPC request and read exactly one JSON-RPC response.
async fn round_trip(pipe: &mut tokio::io::DuplexStream, req: Value) -> Value {
    let mut bytes = serde_json::to_vec(&req).unwrap();
    bytes.push(b'\n');
    pipe.write_all(&bytes).await.unwrap();
    pipe.flush().await.unwrap();

    let (read_half, _write_half) = tokio::io::split(pipe);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

/// Perform the MCP initialize handshake. Returns the server's
/// InitializeResult response so callers can assert on it when they
/// care. rmcp's `serve_server` enters the dispatch loop immediately
/// after sending InitializeResult — no `notifications/initialized`
/// is required.
async fn initialize_handshake(pipe: &mut tokio::io::DuplexStream) -> Value {
    round_trip(
        pipe,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "atlas-mcp-multiplex-test", "version": "0"}
            }
        }),
    )
    .await
}

/// Send one `tools/call` request from the test-side end of a client
/// pipe and return the parsed JSON-RPC response.
async fn send_tools_call(pipe: &mut tokio::io::DuplexStream, id: u64, payload: &str) -> Value {
    round_trip(
        pipe,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "echo",
                "arguments": { "payload": payload }
            }
        }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_concurrent_clients_isolated_dispatch() {
    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap(),
    };
    let server = Arc::new(McpServer::new(
        vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ctx,
    ));

    let (mut pipe_a, handle_a) = spawn_client(Arc::clone(&server), ClientId(1));
    let (mut pipe_b, handle_b) = spawn_client(Arc::clone(&server), ClientId(2));

    // Each client must complete the MCP initialize handshake before
    // issuing tool calls (rmcp enforces initialize-first lifecycle).
    let init_a = initialize_handshake(&mut pipe_a).await;
    let init_b = initialize_handshake(&mut pipe_b).await;
    assert_eq!(init_a["id"], 0);
    assert_eq!(init_b["id"], 0);
    assert_eq!(init_a["result"]["serverInfo"]["name"], "atlas-agents");
    assert_eq!(init_b["result"]["serverInfo"]["name"], "atlas-agents");

    // Interleave: client A id=100 fires first, client B id=200 fires
    // concurrently. Both clients use the same JSON-RPC `id` semantics
    // (numeric) but the values do not collide across the two sockets
    // — and even if they did, response demultiplexing is per-socket.
    let req_a = send_tools_call(&mut pipe_a, 100, "from-client-a");
    let req_b = send_tools_call(&mut pipe_b, 200, "from-client-b");
    let (resp_a, resp_b) = tokio::join!(req_a, req_b);

    // Each response must carry its originating client's id (proving
    // no cross-wire), and each response must echo its own payload
    // (via rmcp's `structuredContent` field — the canonical MCP
    // place for structured tool outputs).
    assert_eq!(resp_a["jsonrpc"], "2.0");
    assert_eq!(resp_a["id"], 100, "client A response id must round-trip");
    assert_eq!(
        resp_a["result"]["structuredContent"]["payload"], "from-client-a",
        "client A must receive its own payload, not B's"
    );
    assert_eq!(resp_a["result"]["isError"], false);

    assert_eq!(resp_b["jsonrpc"], "2.0");
    assert_eq!(resp_b["id"], 200, "client B response id must round-trip");
    assert_eq!(
        resp_b["result"]["structuredContent"]["payload"], "from-client-b",
        "client B must receive its own payload, not A's"
    );
    assert_eq!(resp_b["result"]["isError"], false);

    // Close pipes so serve_client tasks see EOF and exit cleanly.
    drop(pipe_a);
    drop(pipe_b);
    handle_a.await.unwrap().unwrap();
    handle_b.await.unwrap().unwrap();
}

#[tokio::test]
async fn initialize_returns_protocol_handshake() {
    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap(),
    };
    let server = Arc::new(McpServer::new(
        vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ctx,
    ));
    let (mut pipe, handle) = spawn_client(Arc::clone(&server), ClientId(7));

    let resp = initialize_handshake(&mut pipe).await;
    assert_eq!(resp["id"], 0);
    assert!(resp["result"]["protocolVersion"].is_string());
    assert_eq!(resp["result"]["serverInfo"]["name"], "atlas-agents");
    // ServerCapabilities.tools is set (we advertise tool support).
    assert!(resp["result"]["capabilities"]["tools"].is_object());

    drop(pipe);
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn tools_list_returns_registered_tool_catalog() {
    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap(),
    };
    let server = Arc::new(McpServer::new(
        vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ctx,
    ));
    let (mut pipe, handle) = spawn_client(Arc::clone(&server), ClientId(11));

    initialize_handshake(&mut pipe).await;

    let resp = round_trip(
        &mut pipe,
        json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/list"
        }),
    )
    .await;
    assert_eq!(resp["id"], 42);
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");
    assert!(tools[0]["description"].as_str().unwrap().contains("Echo"));
    assert!(tools[0]["inputSchema"].is_object());

    drop(pipe);
    handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn unknown_method_returns_method_not_found_error() {
    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap(),
    };
    let server = Arc::new(McpServer::new(
        vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ctx,
    ));
    let (mut pipe, handle) = spawn_client(Arc::clone(&server), ClientId(13));

    initialize_handshake(&mut pipe).await;

    // Send a non-standard MCP method (rmcp routes anything outside its
    // built-in set to `ServerHandler::on_custom_request`, which we leave
    // at the default — returns METHOD_NOT_FOUND).
    let resp = round_trip(
        &mut pipe,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "atlas/this_method_does_not_exist"
        }),
    )
    .await;
    assert_eq!(resp["id"], 9);
    assert!(resp.get("result").is_none() || resp["result"].is_null());
    assert_eq!(resp["error"]["code"], -32601);

    drop(pipe);
    handle.await.unwrap().unwrap();
}
