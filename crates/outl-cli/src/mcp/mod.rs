//! MCP (Model Context Protocol) server shim.
//!
//! Speaks JSON-RPC 2.0 over stdio implementing the MCP protocol surface
//! Claude Desktop expects:
//!
//! - `initialize` / `initialized`
//! - `tools/list`, `tools/call`
//! - `resources/list`, `resources/read`
//! - `prompts/list`, `prompts/get`
//!
//! Every tool delegates to the same handlers used by the CLI
//! subcommands, so business logic never duplicates.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::output::{codes, ApiError, Envelope};
use crate::ws::{self, WsCtx};
use outl_md::index::WorkspaceIndex;

mod prompts;
mod protocol;
mod resources;
mod tools;

/// Protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Server identification surfaced through `initialize`.
pub const SERVER_NAME: &str = "outl";

/// Server build version (mirrors the crate version).
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the stdio MCP loop. Returns when the client closes stdin.
pub fn serve(workspace_path: PathBuf) -> anyhow::Result<()> {
    let ctx = Arc::new(ServerCtx::new(workspace_path));
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let n = stdin.read_line(&mut line)?;
        if n == 0 {
            // EOF — client closed the pipe.
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = handle_line(trimmed, &ctx);
        if let Some(resp) = response {
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Shared per-server context.
///
/// Holds the workspace open for the lifetime of the MCP session so we
/// don't re-replay the op log on every tool call, and caches the
/// `WorkspaceIndex` between read-only calls (invalidated whenever a
/// mutating tool runs).
pub(crate) struct ServerCtx {
    /// Workspace root the MCP server operates on.
    pub workspace_path: PathBuf,
    /// Lazy state guarded by a single mutex. We don't run tool calls
    /// concurrently today, so a `parking_lot::Mutex` is sufficient and
    /// cheap.
    state: Mutex<ServerState>,
}

#[derive(Default)]
struct ServerState {
    workspace: Option<WsCtx>,
    index: Option<WorkspaceIndex>,
}

impl ServerCtx {
    fn new(workspace_path: PathBuf) -> Self {
        Self {
            workspace_path,
            state: Mutex::new(ServerState::default()),
        }
    }

    /// Run `f` against the cached workspace, opening it on first use.
    ///
    /// The lock is held for the whole call — fine because the MCP
    /// stdio loop is single-threaded today. If we ever serve concurrent
    /// requests this becomes the obvious throttling point.
    pub(crate) fn with_workspace<F, R>(self: &Arc<Self>, f: F) -> Result<R, ApiError>
    where
        F: FnOnce(&mut WsCtx) -> Result<R, ApiError>,
    {
        let mut state = self.state.lock();
        if state.workspace.is_none() {
            state.workspace = Some(ws::open(&self.workspace_path)?);
        }
        let wc = state.workspace.as_mut().ok_or_else(|| {
            ApiError::new(
                codes::INTERNAL,
                "workspace failed to materialise".to_string(),
            )
        })?;
        f(wc)
    }

    /// Run `f` against the cached `WorkspaceIndex`, building it on
    /// first use. Mutating tools should call [`Self::invalidate_index`]
    /// after their `apply_page_md_with_sidecar` so the next read sees
    /// fresh blocks.
    pub(crate) fn with_index<F, R>(self: &Arc<Self>, f: F) -> R
    where
        F: FnOnce(&WorkspaceIndex) -> R,
    {
        let mut state = self.state.lock();
        if state.index.is_none() {
            state.index = Some(WorkspaceIndex::build(&self.workspace_path));
        }
        f(state.index.as_ref().expect("index just populated"))
    }

    /// Drop the cached index. The next `with_index` rebuild from disk.
    pub(crate) fn invalidate_index(self: &Arc<Self>) {
        self.state.lock().index = None;
    }
}

fn handle_line(line: &str, ctx: &Arc<ServerCtx>) -> Option<String> {
    let request: protocol::JsonRpcRequest = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(e) => {
            // Parse error — id may not be available; respond with null id.
            let resp = protocol::JsonRpcResponse::error(
                Value::Null,
                protocol::PARSE_ERROR,
                format!("invalid JSON: {e}"),
            );
            return serde_json::to_string(&resp).ok();
        }
    };

    // Notifications (no `id`) get no response.
    let is_notification = request.id.is_none();
    let id = request.id.clone().unwrap_or(Value::Null);
    let method = request.method.clone();
    let params = request.params.unwrap_or(Value::Null);

    let result = dispatch(&method, params, ctx);

    if is_notification {
        return None;
    }

    let response = match result {
        Ok(value) => protocol::JsonRpcResponse::success(id, value),
        Err(err) => protocol::JsonRpcResponse::error(id, err.code, err.message),
    };
    serde_json::to_string(&response).ok()
}

fn dispatch(
    method: &str,
    params: Value,
    ctx: &Arc<ServerCtx>,
) -> Result<Value, protocol::JsonRpcError> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false },
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION,
            },
        })),
        "initialized" | "notifications/initialized" => Ok(Value::Null),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => tools::call(params, ctx),
        "resources/list" => Ok(json!({ "resources": resources::list() })),
        "resources/read" => resources::read(params, ctx),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": resources::templates() })),
        "prompts/list" => Ok(json!({ "prompts": prompts::list() })),
        "prompts/get" => prompts::get(params, ctx),
        other => Err(protocol::JsonRpcError::method_not_found(other)),
    }
}

/// Wrap an [`ApiError`] into MCP tool output. MCP tool errors flow
/// through the response shape `{ content: [...], isError: true }`
/// rather than as JSON-RPC errors, so the client gets a recoverable
/// signal instead of a protocol-level fault.
pub(crate) fn tool_error_payload(err: &ApiError) -> Value {
    json!({
        "content": [
            { "type": "text", "text": format!("{}: {}", err.code, err.message) }
        ],
        "isError": true,
    })
}

/// Wrap a successful tool result into the MCP tool-output envelope.
///
/// `tool_name` lets us pick a more useful `text` representation than
/// "pretty-printed JSON" for the tools where the user is asking for
/// raw markdown (`export_md`, `page_render`, etc.). The
/// `structuredContent` field always carries the full envelope so
/// callers that prefer machine shape still get it.
pub(crate) fn tool_success_payload(tool_name: &str, payload: &Value) -> Value {
    let text = preferred_text_for(tool_name, payload);
    let envelope = Envelope::success(payload.clone());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": serde_json::to_value(&envelope).unwrap_or(Value::Null),
        "isError": false,
    })
}

/// Pick a text content best suited for `tool_name`.
///
/// Tools that produce a single big string (rendered markdown, summary
/// text) flatten the payload by reading its natural field. Everything
/// else stays as pretty-printed JSON so structured callers always see
/// the same shape.
fn preferred_text_for(tool_name: &str, payload: &Value) -> String {
    let take_field = |field: &str| -> Option<String> {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    match tool_name {
        // Pure-markdown surfaces: prefer the raw `md` field.
        "outl_export_md" | "outl_page_render" => take_field("md"),
        // Daily / page surfaces ship both `md` and a structured outline;
        // the host shows the markdown as the "natural" text content.
        "outl_daily_today" | "outl_daily_get" => take_field("md"),
        _ => None,
    }
    .unwrap_or_else(|| {
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    })
}
