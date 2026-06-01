//! Static schema list returned by `tools/list`.
//!
//! Adding a tool: append a `tool_def(...)` entry here AND wire the
//! handler in `dispatch::run_tool`. Schema-only changes never need to
//! touch the dispatcher.

use serde_json::{json, Value};

use super::tool_def;

/// Every MCP tool we expose, with its JSON Schema input.
pub fn list() -> Vec<Value> {
    vec![
        // Page
        tool_def(
            "outl_page_get",
            "Get a page's meta and outline tree.",
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_page_create",
            "Create a new page (idempotent on slug).",
            json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string" },
                    "title": { "type": "string" },
                    "icon": { "type": "string" }
                },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_page_update",
            "Update a page's title and/or icon.",
            json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string" },
                    "title": { "type": "string" },
                    "icon": { "type": "string" }
                },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_page_delete",
            "Delete a page (move root to trash). Requires confirm:true.",
            json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string" },
                    "confirm": { "type": "boolean" }
                },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_page_list",
            "List pages, optionally filtered (`tag:foo` or `kind:journal|page`).",
            json!({
                "type": "object",
                "properties": { "filter": { "type": "string" } }
            }),
        ),
        tool_def(
            "outl_page_rename",
            "Rename a page slug. Does not rewrite `[[old_slug]]` references — they appear in `affected_refs`.",
            json!({
                "type": "object",
                "properties": {
                    "old_slug": { "type": "string" },
                    "new_slug": { "type": "string" }
                },
                "required": ["old_slug", "new_slug"]
            }),
        ),
        tool_def(
            "outl_page_render",
            "Return the page rendered as clean markdown.",
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }),
        ),
        // Block
        tool_def(
            "outl_block_get",
            "Get a single block by id.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_append",
            "Append a new block as the last child of a page or block.",
            json!({
                "type": "object",
                "properties": {
                    "page": { "type": "string" },
                    "parent": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }),
        ),
        tool_def(
            "outl_block_insert",
            "Insert a sibling immediately after `after`.",
            json!({
                "type": "object",
                "properties": {
                    "after": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["after", "text"]
            }),
        ),
        tool_def(
            "outl_block_update",
            "Replace a block's text body.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["id", "text"]
            }),
        ),
        tool_def(
            "outl_block_move",
            "Move a block to a new parent / position.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "parent": { "type": "string" },
                    "after": { "type": "string" }
                },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_delete",
            "Delete a block (move to trash). Requires confirm:true.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "confirm": { "type": "boolean" }
                },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_toggle_todo",
            "Cycle the block's TODO state: None → TODO → DONE → None.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_tree",
            "Return the block plus its descendants.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        // Daily / Journal
        tool_def(
            "outl_daily_today",
            "Get today's journal.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "outl_daily_get",
            "Get a journal by date (ISO, `today/yesterday/tomorrow`, or `April 22nd, 2026`).",
            json!({
                "type": "object",
                "properties": { "date": { "type": "string" } },
                "required": ["date"]
            }),
        ),
        tool_def(
            "outl_daily_append",
            "Append a block to a journal (today by default).",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "date": { "type": "string" }
                },
                "required": ["text"]
            }),
        ),
        tool_def(
            "outl_daily_range",
            "List journals between two inclusive dates.",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" }
                },
                "required": ["from", "to"]
            }),
        ),
        // Search / Query
        tool_def(
            "outl_search",
            "Full-text search across blocks and/or page titles.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "in": { "type": "string", "enum": ["blocks", "pages", "all"] },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }),
        ),
        tool_def(
            "outl_query",
            "Structured filter over pages (tag, property, date range, kind).",
            json!({
                "type": "object",
                "properties": {
                    "tag": { "type": "string" },
                    "priority": { "type": "string" },
                    "since": { "type": "string" },
                    "kind": { "type": "string", "enum": ["page", "journal"] },
                    "props": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }),
        ),
        // Backlinks / Refs
        tool_def(
            "outl_backlinks",
            "Pages and blocks that mention `[[<slug>]]`.",
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_block_refs",
            "Blocks that cite `((blk-XXXXXX))`.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        tool_def(
            "outl_block_embed",
            "Resolve `!((…))` recursively (source block + children).",
            json!({
                "type": "object",
                "properties": { "id_or_handle": { "type": "string" } },
                "required": ["id_or_handle"]
            }),
        ),
        // Tag
        tool_def(
            "outl_tag_list",
            "List every tag in the workspace with usage counts.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "outl_tag_pages",
            "Pages that mention `#<tag>` somewhere in their subtree.",
            json!({
                "type": "object",
                "properties": { "tag": { "type": "string" } },
                "required": ["tag"]
            }),
        ),
        // Page properties
        tool_def(
            "outl_page_prop_set",
            "Set a page-level `key:: value` property.",
            json!({
                "type": "object",
                "properties": {
                    "page": { "type": "string" },
                    "key": { "type": "string" },
                    "value": { "type": "string" }
                },
                "required": ["page", "key", "value"]
            }),
        ),
        tool_def(
            "outl_page_prop_get",
            "Read a page property by key.",
            json!({
                "type": "object",
                "properties": {
                    "page": { "type": "string" },
                    "key": { "type": "string" }
                },
                "required": ["page", "key"]
            }),
        ),
        tool_def(
            "outl_page_prop_list",
            "List every property on a page.",
            json!({
                "type": "object",
                "properties": { "page": { "type": "string" } },
                "required": ["page"]
            }),
        ),
        // Export
        tool_def(
            "outl_export_hugo",
            "Render a page as a Hugo Markdown file.",
            json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string" },
                    "out_dir": { "type": "string" }
                },
                "required": ["slug", "out_dir"]
            }),
        ),
        tool_def(
            "outl_export_md",
            "Return the page as clean markdown.",
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }),
        ),
        tool_def(
            "outl_export_json",
            "Return the page's AST + sidecar as JSON.",
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }),
        ),
        // Workspace
        tool_def(
            "outl_workspace_info",
            "Workspace summary: path, actor, counts, ops.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "outl_workspace_doctor",
            "Run integrity checks (op log, sidecars, block refs, lock). Returns a structured report.",
            json!({ "type": "object", "properties": {} }),
        ),
    ]
}
