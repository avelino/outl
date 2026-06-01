//! MCP resources — read-only URIs Claude Desktop can attach to context.
//!
//! We expose three categories:
//!
//! - `outl://workspace/info` — workspace summary as JSON.
//! - `outl://daily/today` — today's journal rendered as `.md`.
//! - `outl://page/<slug>` — page projection rendered as `.md`.
//!
//! The first two are concrete; the page surface is a template the
//! `resources/templates/list` endpoint advertises.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::cmd::daily as daily_cmd;
use crate::cmd::workspace_info as wi_cmd;
use crate::output::ApiError;
use crate::ws;

use super::protocol::JsonRpcError;
use super::ServerCtx;

/// Concrete (non-template) resources Claude can list.
pub fn list() -> Vec<Value> {
    vec![
        json!({
            "uri": "outl://workspace/info",
            "name": "Workspace info",
            "description": "Counts, root path, actor id.",
            "mimeType": "application/json",
        }),
        json!({
            "uri": "outl://daily/today",
            "name": "Today's journal",
            "description": "Markdown projection of today's daily note.",
            "mimeType": "text/markdown",
        }),
    ]
}

/// Resource templates — surfaced via `resources/templates/list`.
pub fn templates() -> Vec<Value> {
    vec![json!({
        "uriTemplate": "outl://page/{slug}",
        "name": "Page by slug",
        "description": "Markdown projection of a workspace page.",
        "mimeType": "text/markdown",
    })]
}

/// Handle `resources/read`.
pub fn read(params: Value, ctx: &Arc<ServerCtx>) -> Result<Value, JsonRpcError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcError::invalid_params("missing `uri`"))?;

    let outcome = resolve(uri, ctx);
    match outcome {
        Ok((mime, text)) => Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": mime,
                "text": text,
            }]
        })),
        Err(e) => Err(JsonRpcError::internal(format!("{e}"))),
    }
}

fn resolve(uri: &str, ctx: &Arc<ServerCtx>) -> Result<(String, String), ApiError> {
    if uri == "outl://workspace/info" {
        let wc = ws::open(&ctx.workspace_path)?;
        let value = wi_cmd::info(&wc);
        let text = serde_json::to_string_pretty(&value).map_err(ApiError::internal)?;
        return Ok(("application/json".to_string(), text));
    }

    if uri == "outl://daily/today" {
        let mut wc = ws::open(&ctx.workspace_path)?;
        let value = daily_cmd::today_handler(&mut wc)?;
        let text = value
            .get("md")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Ok(("text/markdown".to_string(), text));
    }

    if let Some(slug) = uri.strip_prefix("outl://page/") {
        let wc = ws::open(&ctx.workspace_path)?;
        let value = crate::cmd::page::render(&wc, slug)?;
        let text = value
            .get("md")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Ok(("text/markdown".to_string(), text));
    }

    Err(ApiError::new(
        crate::output::codes::INVALID_ARG,
        format!("unsupported resource URI `{uri}`"),
    ))
}
