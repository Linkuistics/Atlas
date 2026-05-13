# rmcp maturity verification (PR-A plan-time gate)

Verified: 2026-05-13

The Atlas vNext production-prompt sprint plan §4 Task 6 Step A.1 gates
PR-A behind a four-criterion maturity check on `rmcp` (Rust MCP SDK).
This note records the verification probes run, the observed values, and
the decision.

| Criterion | Threshold | Observed | Pass? |
|---|---|---|---|
| Last crates.io publish | within 12 months (≥ 2025-05-13) | 2026-05-01 v1.6.0 (12 days ago); active monthly cadence — v1.2.0 (2026-03-11), v1.3.0 (2026-03-26), v1.4.0 (2026-04-10), v1.5.0 (2026-04-16), v1.6.0 (2026-05-01) | ✅ |
| Repo activity (default branch) | within 6 months (≥ 2025-11-13) | 2026-05-12 on `main` — `fix(rmcp): flatten Resource variant of PromptMessageContent (#843)`; commits within last week include stdio-parse-error robustness fixes (#833 on 2026-05-07) | ✅ |
| Multi-client server abstraction | documented | `serve_server<S, T, E, A>(service: S, transport: T) -> Result<RunningService<RoleServer, S>, ServerInitializeError>` — per-transport server task; `ServerHandler` impls are `Clone`-required; multi-client = spawn `serve_server` per duplex transport with handlers sharing `Arc<Inner>` state | ✅ |
| Transitive deps (cargo tree -p rmcp -e normal --depth 1, default features = [base64, macros, server]) | ≤ 30 direct; no WS/TLS/HTTP-server crates | 14 direct deps (`async-trait, base64, chrono, futures, pastey, pin-project-lite, rmcp-macros, schemars, serde, serde_json, thiserror, tokio, tokio-util, tracing`); zero hits for tungstenite/hyper/rustls/tokio-rustls/axum/warp/actix/reqwest in full tree at default features | ✅ |

**Decision:** PASS → PR-A proceeds with `rmcp`.

**Targeted upstream version:** `rmcp = "1.6"` (pinned in workspace
`Cargo.toml` to a major.minor; patch updates flow via Renovate /
Dependabot on a deliberate cadence). The non-default feature flags
(`auth`, `__reqwest`, `transport-streamable-http-*`, etc.) that pull
TLS/HTTP-server crates are explicitly NOT enabled — Atlas's MCP server
is in-process stdio-only per recast §5.5 ("no external surfaces").

**Verification probe artefacts:**

```
$ cargo search rmcp --limit 1
rmcp = "1.6.0"                    # Rust SDK for Model Context Protocol

$ cargo info rmcp | head -10
rmcp 1.6.0
license: Apache-2.0
repository: https://github.com/modelcontextprotocol/rust-sdk/
default features: [base64, macros, server]

$ cargo tree -p rmcp -e normal --depth 1
rmcp v1.6.0
├── async-trait v0.1.89 (proc-macro)
├── base64 v0.22.1
├── chrono v0.4.44
├── futures v0.3.32
├── pastey v0.2.2 (proc-macro)
├── pin-project-lite v0.2.17
├── rmcp-macros v1.6.0 (proc-macro)
├── schemars v1.2.1
├── serde v1.0.228
├── serde_json v1.0.149
├── thiserror v2.0.18
├── tokio v1.52.3
├── tokio-util v0.7.18
└── tracing v0.1.44
```

Many of the 14 direct deps overlap with Atlas's existing
`[workspace.dependencies]` (tokio, serde, serde_json, async-trait,
thiserror, tracing, chrono, base64, futures via reqwest-transitive,
pin-project-lite via tokio-transitive); the net Cargo.lock growth is
sub-dozen crates dominated by `rmcp-macros`, `schemars`, `pastey`,
`tokio-util`.

**Multi-client architecture mapping (Step A.3 preview):**

The current `crates/atlas-agents/src/mcp/server.rs::McpServer`
hand-rolls a JSON-RPC dispatch loop reading newline-delimited frames
from an `AsyncRead` + `AsyncWrite` pair, with per-`ClientId` transcript
recording behind a `Mutex<HashMap<ClientId, Vec<Value>>>`. The rmcp-
based replacement preserves the public surface:

- `McpServer::new(tools, ctx)` constructor unchanged.
- `McpServer::serve_client(self: Arc<Self>, client_id, reader, writer)`
  signature unchanged (caller-facing). Internally builds an
  `rmcp::transport::AsyncRwTransport::new((reader, writer))`, clones
  the shared handler state, embeds the `client_id`, and awaits
  `rmcp::serve_server(handler, transport)`.
- `drain_client_transcript(client_id)` unchanged. Recording lives on
  the handler's tool-dispatch path inside the `ServerHandler` impl.
- `tool_count()` unchanged.

The handler internal type implements `rmcp::ServerHandler` (derived
via the `#[tool_handler]` macro or by hand) and exposes its tool list
via the `tools/list` MCP method; `tools/call` dispatches to the
registered `Arc<dyn Tool>` impls by `id()` lookup. Multi-client
isolation is structural: each `serve_client` invocation gets its own
Tokio task with its own transport; the underlying `Arc<ToolCatalog>`
is read-only and shared.

**One risk surfaced for Step A.3 (does not affect the verification
decision):** The current `dispatch_tool_call` emits a non-standard MCP
content block `{"type":"json","json":<result>}` to preserve structured
output. Standard MCP content types are `text` / `image` /
`embedded_resource` only. If rmcp's tool-result serialiser enforces
the standard shape, two paths:

1. **Custom `Content` variant via rmcp's API** — preferred; preserves
   `mcp_multiplex.rs` test assertions verbatim ("no test logic
   changes" goal from plan line 2737).
2. **Test-assertion update + JSON-string-in-text content** — fallback;
   the test's `content_a["json"]["payload"]` assertion adapts to
   `serde_json::from_str(content_a["text"])["payload"]`. This is a
   minimal wire-shape adaptation, not a "test logic change" (the
   multi-client isolation + id round-trip assertions remain intact).

Step A.3 attempts path 1 first; on failure escalates to path 2 with a
note in the per-PR status file. Either way, the structural
multi-client regression detection (interleaved concurrent clients +
isolated id round-trip) is preserved — that's the test's load-bearing
contract.
