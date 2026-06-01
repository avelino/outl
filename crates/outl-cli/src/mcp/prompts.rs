//! MCP prompts — slash-shaped shortcuts Claude Desktop renders in the
//! prompt picker. Each prompt expands to a message the user can submit
//! to the model.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::cmd::daily as daily_cmd;
use crate::output::ApiError;
use crate::ws;

use super::protocol::JsonRpcError;
use super::ServerCtx;

/// List the prompts we expose.
pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "outl-summarize-day",
            "description": "Summarize a day's journal. Optional `date` argument (ISO).",
            "arguments": [
                { "name": "date", "description": "Journal date (defaults to today).", "required": false }
            ],
        }),
        json!({
            "name": "outl-blog-from-block",
            "description": "Expand a block into a blog post draft.",
            "arguments": [
                { "name": "block_id", "description": "Block id (ULID).", "required": true }
            ],
        }),
    ]
}

/// Handle `prompts/get`.
pub fn get(params: Value, ctx: &Arc<ServerCtx>) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcError::invalid_params("missing `name`"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "outl-summarize-day" => summarize_day(&args, ctx),
        "outl-blog-from-block" => blog_from_block(&args, ctx),
        other => Err(ApiError::new(
            crate::output::codes::INVALID_ARG,
            format!("unknown prompt `{other}`"),
        )),
    };

    result.map_err(|e| JsonRpcError::internal(format!("{e}")))
}

fn summarize_day(args: &Value, ctx: &Arc<ServerCtx>) -> Result<Value, ApiError> {
    let date = args.get("date").and_then(Value::as_str);
    let mut wc = ws::open(&ctx.workspace_path)?;
    let journal = match date {
        Some(d) => daily_cmd::get(&mut wc, d)?,
        None => daily_cmd::today_handler(&mut wc)?,
    };
    let date_slug = journal
        .get("date")
        .and_then(Value::as_str)
        .unwrap_or("today");
    let md = journal.get("md").and_then(Value::as_str).unwrap_or("");
    let body = format!(
        "Summarize the journal for {date_slug}. Pull out decisions, open questions, and next actions.\n\n---\n\n{md}"
    );
    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": { "type": "text", "text": body }
            }
        ]
    }))
}

fn blog_from_block(args: &Value, ctx: &Arc<ServerCtx>) -> Result<Value, ApiError> {
    let id = args
        .get("block_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::new(
                crate::output::codes::INVALID_ARG,
                "missing `block_id` argument".to_string(),
            )
        })?;
    let wc = ws::open(&ctx.workspace_path)?;
    let block = crate::cmd::block::tree(&wc, id)?;
    let body = format!(
        "Expand the following outline into a blog post draft in the user's voice (direct, pt-BR informal, English code).\n\n---\n\n{}",
        serde_json::to_string_pretty(&block).unwrap_or_default(),
    );
    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": { "type": "text", "text": body }
            }
        ]
    }))
}
