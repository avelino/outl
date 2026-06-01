//! `outl workspace info` — workspace summary (path, actor, counts).

use std::path::Path;

use clap::Args;
use serde_json::{json, Value};

use outl_actions::{list_pages, PageKind};

use crate::output::emit;
use crate::ws::{self, WsCtx};

/// Args for `outl workspace info`.
#[derive(Args, Debug)]
pub struct WorkspaceInfoArgs {
    /// Force JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Run `outl workspace info`.
pub fn run(args: &WorkspaceInfoArgs, path: &Path) -> i32 {
    let result = ws::open(path).map(|ctx| info(&ctx));
    emit(args.json, result, print_info)
}

/// Pure handler for the MCP shim.
pub fn info(ctx: &WsCtx) -> Value {
    let pages = list_pages(&ctx.workspace);
    let mut journals = 0usize;
    let mut regular = 0usize;
    for p in &pages {
        match p.kind {
            PageKind::Journal => journals += 1,
            PageKind::Page => regular += 1,
        }
    }
    json!({
        "root": ctx.root.display().to_string(),
        "actor": ctx.actor.to_string(),
        "log_db": ctx.paths.db.display().to_string(),
        "pages_dir": ctx.paths.pages.display().to_string(),
        "journals_dir": ctx.paths.journals.display().to_string(),
        "pages": regular,
        "journals": journals,
        "ops": ctx.workspace.log().len(),
        "tree_nodes": ctx.workspace.tree().node_count(),
    })
}

fn print_info(v: &Value) {
    let root = v.get("root").and_then(Value::as_str).unwrap_or("?");
    let actor = v.get("actor").and_then(Value::as_str).unwrap_or("?");
    let pages = v.get("pages").and_then(Value::as_u64).unwrap_or(0);
    let journals = v.get("journals").and_then(Value::as_u64).unwrap_or(0);
    let ops = v.get("ops").and_then(Value::as_u64).unwrap_or(0);
    println!("root:     {root}");
    println!("actor:    {actor}");
    println!("pages:    {pages}");
    println!("journals: {journals}");
    println!("ops:      {ops}");
}
