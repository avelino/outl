//! `outl block …` — block-level operations.
//!
//! Every mutation routes through `outl-actions` so the op log stays
//! source of truth. `block move` is the one user-visible name for
//! `Op::Move`; cycle rejection bubbles up as `CYCLE_REJECTED`.

use std::path::Path;
use std::str::FromStr;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{
    append_block, apply_page_md_with_sidecar, children_of, create_after, enclosing_page_id,
    page_meta, position_after, position_for_new_last_child, project_outline, split_todo, PageMeta,
};
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};

use crate::human::{print_outline_node, todo_prefix};
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl block …` subcommands.
#[derive(Subcommand, Debug)]
pub enum BlockCommand {
    /// Get a single block by id.
    Get {
        /// Block id (ULID string, e.g. `01HX...`).
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Append a new block as the last child of a page (or another block).
    Append {
        /// Target page slug. Required unless `--parent` is given.
        #[arg(long)]
        page: Option<String>,
        /// Parent block id. Mutually exclusive with `--page`.
        #[arg(long)]
        parent: Option<String>,
        /// Block text body.
        #[arg(long)]
        text: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Insert a new sibling immediately after another block.
    Insert {
        /// Sibling that the new block lands after.
        #[arg(long)]
        after: String,
        /// Block text body.
        #[arg(long)]
        text: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Replace a block's text.
    Update {
        /// Block id to mutate.
        id: String,
        /// New text body (write the TODO/DONE prefix yourself if needed).
        #[arg(long)]
        text: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Move a block to a new parent and/or position.
    Move {
        /// Block to move.
        id: String,
        /// New parent (defaults to current parent when omitted).
        #[arg(long)]
        parent: Option<String>,
        /// Sibling to land after. Without `--after`, the block becomes
        /// the last child of `parent`.
        #[arg(long)]
        after: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Delete a block (moves it to the trash root).
    Delete {
        /// Block id.
        id: String,
        /// Required to actually delete.
        #[arg(long)]
        confirm: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Cycle the block's TODO state: `None → TODO → DONE → None`.
    ToggleTodo {
        /// Block id.
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Return the block and its descendant subtree.
    Tree {
        /// Block id.
        id: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl block …` invocation.
pub fn run(cmd: &BlockCommand, path: &Path) -> i32 {
    match cmd {
        BlockCommand::Get { id, json } => {
            let result = ws::open(path).and_then(|ctx| get(&ctx, id));
            emit(*json, result, print_block)
        }
        BlockCommand::Append {
            page,
            parent,
            text,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| append(&mut ctx, page.as_deref(), parent.as_deref(), text));
            emit(*json, result, |v| print_block_created("appended", v))
        }
        BlockCommand::Insert { after, text, json } => {
            let result = ws::open(path).and_then(|mut ctx| insert(&mut ctx, after, text));
            emit(*json, result, |v| print_block_created("inserted", v))
        }
        BlockCommand::Update { id, text, json } => {
            let result = ws::open(path).and_then(|mut ctx| update(&mut ctx, id, text));
            emit(*json, result, |v| print_block_simple("updated", v))
        }
        BlockCommand::Move {
            id,
            parent,
            after,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| move_block(&mut ctx, id, parent.as_deref(), after.as_deref()));
            emit(*json, result, |v| print_block_simple("moved", v))
        }
        BlockCommand::Delete { id, confirm, json } => {
            if !*confirm {
                let err = ApiError::new(
                    codes::CONFIRM_REQUIRED,
                    format!("refusing to delete block `{id}` without --confirm"),
                );
                return emit::<Value, _>(*json, Err(err), |_| {});
            }
            let result = ws::open(path).and_then(|mut ctx| delete(&mut ctx, id));
            emit(*json, result, |v| print_block_simple("deleted", v))
        }
        BlockCommand::ToggleTodo { id, json } => {
            let result = ws::open(path).and_then(|mut ctx| toggle_todo(&mut ctx, id));
            emit(*json, result, |v| print_block_simple("toggled", v))
        }
        BlockCommand::Tree { id, json } => {
            let result = ws::open(path).and_then(|ctx| tree(&ctx, id));
            emit(*json, result, print_block)
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Get block meta (text, todo state, parent, children count).
pub fn get(ctx: &WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    let text = ctx.workspace.block_text(id).ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id_str}` not found"),
        )
    })?;
    let parent = ctx.workspace.tree().parent(id);
    let children = children_of(&ctx.workspace, id);
    let (todo, body) = split_todo(&text);
    Ok(json!({
        "id": id.to_string(),
        "text": body,
        "todo": todo.map(|t| t.as_str().to_string()),
        "parent": parent.map(|p| p.to_string()),
        "child_count": children.len(),
        "raw_text": text,
    }))
}

/// Append a new block as the last child of `page` or `parent`.
pub fn append(
    ctx: &mut WsCtx,
    page: Option<&str>,
    parent: Option<&str>,
    text: &str,
) -> Result<Value, ApiError> {
    let parent_id = match (page, parent) {
        (Some(_), Some(_)) => {
            return Err(ApiError::new(
                codes::INVALID_ARG,
                "use either --page or --parent, not both".to_string(),
            ));
        }
        (Some(slug), None) => {
            outl_actions::find_by_slug(&ctx.workspace, slug).ok_or_else(|| {
                ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found"))
            })?
        }
        (None, Some(pid)) => parse_id(pid)?,
        (None, None) => {
            return Err(ApiError::new(
                codes::INVALID_ARG,
                "append requires --page or --parent".to_string(),
            ));
        }
    };

    let new_id = append_block(&mut ctx.workspace, &ctx.hlc, Some(parent_id), Some(text))
        .map_err(ApiError::internal)?;

    write_enclosing_page(ctx, new_id)?;

    Ok(json!({
        "id": new_id.to_string(),
        "parent": parent_id.to_string(),
        "text": text,
    }))
}

/// Insert a new sibling right after `after`.
pub fn insert(ctx: &mut WsCtx, after: &str, text: &str) -> Result<Value, ApiError> {
    let after_id = parse_id(after)?;
    let new_id = create_after(&mut ctx.workspace, &ctx.hlc, after_id, Some(text))
        .map_err(ApiError::internal)?;

    write_enclosing_page(ctx, new_id)?;

    Ok(json!({
        "id": new_id.to_string(),
        "after": after_id.to_string(),
        "text": text,
    }))
}

/// Replace a block's text.
pub fn update(ctx: &mut WsCtx, id_str: &str, text: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    outl_actions::block::edit_text(&mut ctx.workspace, &ctx.hlc, id, text)
        .map_err(ApiError::internal)?;
    write_enclosing_page(ctx, id)?;
    Ok(json!({ "id": id.to_string(), "text": text }))
}

/// Move a block to a new parent / position.
pub fn move_block(
    ctx: &mut WsCtx,
    id_str: &str,
    parent: Option<&str>,
    after: Option<&str>,
) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    let current_parent = ctx.workspace.tree().parent(id).ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id_str}` not in tree"),
        )
    })?;
    let current_position = ctx
        .workspace
        .tree()
        .position(id)
        .cloned()
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "block has no position".to_string()))?;

    let new_parent = match parent {
        Some(p) => parse_id(p)?,
        None => current_parent,
    };

    let new_position = match after {
        Some(a) => {
            let after_id = parse_id(a)?;
            let after_parent = ctx.workspace.tree().parent(after_id).ok_or_else(|| {
                ApiError::new(
                    codes::BLOCK_NOT_FOUND,
                    format!("--after block `{a}` not in tree"),
                )
            })?;
            if after_parent != new_parent {
                return Err(ApiError::new(
                    codes::INVALID_ARG,
                    "--after block has a different parent than --parent".to_string(),
                ));
            }
            position_after(&ctx.workspace, after_id).ok_or_else(|| {
                ApiError::new(codes::INTERNAL, "could not derive position".to_string())
            })?
        }
        None => position_for_new_last_child(&ctx.workspace, new_parent),
    };

    // Reject cycles loudly. The CRDT also rejects on the materialised
    // tree, but it does so silently — surface it as a stable error so
    // scripts know the op was a no-op.
    if ctx.workspace.tree().creates_cycle(id, new_parent) {
        return Err(ApiError::new(
            codes::CYCLE_REJECTED,
            format!("move {id_str} → {new_parent} would create a cycle (op recorded as no-op)"),
        ));
    }

    let ts = ctx.hlc.next();
    ctx.workspace
        .apply(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Move {
                node: id,
                new_parent,
                position: new_position,
                old_parent: current_parent,
                old_position: current_position,
            },
        })
        .map_err(ApiError::internal)?;

    write_enclosing_page(ctx, id)?;
    if new_parent != current_parent {
        if let Some(old_page) = enclosing_page_id(&ctx.workspace, current_parent) {
            apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, old_page)
                .map_err(ApiError::internal)?;
        }
    }

    Ok(json!({
        "id": id.to_string(),
        "parent": new_parent.to_string(),
    }))
}

/// Delete a block — moves it to the trash.
pub fn delete(ctx: &mut WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    let page = enclosing_page_id(&ctx.workspace, id);
    outl_actions::block::delete(&mut ctx.workspace, &ctx.hlc, id).map_err(ApiError::internal)?;
    if let Some(p) = page {
        apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, p).map_err(ApiError::internal)?;
    }
    Ok(json!({ "id": id.to_string() }))
}

/// Cycle the block's TODO state.
pub fn toggle_todo(ctx: &mut WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    outl_actions::block::toggle_todo(&mut ctx.workspace, &ctx.hlc, id)
        .map_err(ApiError::internal)?;
    write_enclosing_page(ctx, id)?;
    let text = ctx.workspace.block_text(id).unwrap_or_default();
    let (todo, body) = split_todo(&text);
    Ok(json!({
        "id": id.to_string(),
        "text": body,
        "todo": todo.map(|t| t.as_str().to_string()),
    }))
}

/// Return the block plus the recursive outline of its descendants.
pub fn tree(ctx: &WsCtx, id_str: &str) -> Result<Value, ApiError> {
    let id = parse_id(id_str)?;
    let text = ctx.workspace.block_text(id).ok_or_else(|| {
        ApiError::new(
            codes::BLOCK_NOT_FOUND,
            format!("block `{id_str}` not found"),
        )
    })?;
    let (todo, body) = split_todo(&text);
    let children = project_outline(&ctx.workspace, id);
    Ok(json!({
        "id": id.to_string(),
        "text": body,
        "todo": todo.map(|t| t.as_str().to_string()),
        "children": serde_json::to_value(&children).map_err(ApiError::internal)?,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a `NodeId` from its string ULID form.
pub fn parse_id(s: &str) -> Result<NodeId, ApiError> {
    ulid::Ulid::from_str(s).map(NodeId).map_err(|e| {
        ApiError::new(
            codes::INVALID_BLOCK_ID,
            format!("invalid block id `{s}`: {e}"),
        )
    })
}

/// Rewrite the enclosing page's `.md` + sidecar after a mutation.
fn write_enclosing_page(ctx: &mut WsCtx, node: NodeId) -> Result<Option<PageMeta>, ApiError> {
    let Some(page) = enclosing_page_id(&ctx.workspace, node) else {
        return Ok(None);
    };
    let meta = page_meta(&ctx.workspace, page);
    apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, page).map_err(ApiError::internal)?;
    Ok(meta)
}

// ---------------------------------------------------------------------------
// Human formatters
// ---------------------------------------------------------------------------

fn print_block(v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    let todo = v.get("todo").and_then(Value::as_str);
    println!("{id}");
    println!("  {}{}", todo_prefix(todo), text);
    if let Some(children) = v.get("children").and_then(Value::as_array) {
        for child in children {
            print_outline_node(child, 1);
        }
    }
}

fn print_block_created(verb: &str, v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    println!("{verb}: {id}  {text}");
}

fn print_block_simple(verb: &str, v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    println!("{verb}: {id}");
}
