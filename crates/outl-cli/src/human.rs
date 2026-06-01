//! Human-readable formatters shared by every CLI subcommand.
//!
//! The JSON envelope is for machines; this module is the
//! terminal-friendly view that runs when `--json` is *not* set. Each
//! formatter consumes the same `serde_json::Value` shape the handler
//! returns, so adding a new printer never forces a change in the
//! business path.

use serde_json::Value;

/// Prefix used in front of a block body to surface its TODO state.
///
/// Mirrors what the user sees in the TUI today: `[ ]` for open,
/// `[x]` for done, empty for a plain bullet. Lives in one place so
/// every CLI surface (page, block, daily, backlinks, embed) renders
/// the same.
pub fn todo_prefix(todo: Option<&str>) -> &'static str {
    match todo {
        Some("TODO") => "[ ] ",
        Some("DONE") => "[x] ",
        _ => "",
    }
}

/// Print an outline tree starting at depth `depth`. Each node is the
/// shape produced by `outl_actions::project_outline` (after JSON
/// serialization): `{ "text": "...", "todo": "Todo|Done|null",
/// "children": [...] }`.
///
/// `depth = 0` puts the first level flush left. Children render with
/// 2-space indent per level.
pub fn print_outline_tree(nodes: &[Value], depth: usize) {
    for node in nodes {
        print_outline_node(node, depth);
    }
}

/// Print one outline node and recurse into its children.
pub fn print_outline_node(node: &Value, depth: usize) {
    let text = node.get("text").and_then(Value::as_str).unwrap_or("");
    let todo = node.get("todo").and_then(Value::as_str);
    let prefix = todo_prefix(todo);
    println!("{:indent$}- {}{}", "", prefix, text, indent = depth * 2);
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        print_outline_tree(children, depth + 1);
    }
}
