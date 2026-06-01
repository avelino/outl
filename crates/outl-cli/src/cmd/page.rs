//! `outl page …` — page-level operations.
//!
//! Each handler returns a `serde_json::Value` so the same body powers
//! both the CLI (`emit` → human or `--json`) and the MCP shim (raw
//! payload returned as the tool result).

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::page::SLUG_KEY;
use outl_actions::{
    apply_page_md_with_sidecar, find_by_slug, list_pages, open_or_create_page, page_meta,
    project_outline, read_text_prop, render_page_md, set_property, walk_subtree, PageKind,
};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use crate::human::print_outline_tree;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl page …` subcommands.
#[derive(Subcommand, Debug)]
pub enum PageCommand {
    /// Get a page (meta + outline).
    Get {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Create a new page.
    Create {
        /// Page slug (filename-safe, e.g. `ideas`).
        slug: String,
        /// Page title (defaults to slug when omitted).
        #[arg(long)]
        title: Option<String>,
        /// Page icon (single emoji or short string).
        #[arg(long)]
        icon: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Update a page's metadata (title and/or icon).
    Update {
        /// Page slug to update.
        slug: String,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New icon (use `--icon=""` to clear).
        #[arg(long)]
        icon: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Delete a page (moves it to the trash root; op stays in the log).
    Delete {
        /// Page slug.
        slug: String,
        /// Required to actually delete.
        #[arg(long)]
        confirm: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List every page in the workspace.
    List {
        /// Optional filter expression: `tag:foo` or `kind:journal|page`.
        #[arg(long)]
        filter: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Rename a page slug. Does not rewrite `[[old_slug]]` references —
    /// they appear in `affected_refs` so the caller can decide.
    Rename {
        /// Current slug.
        old_slug: String,
        /// New slug.
        new_slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Render a page to clean `.md`.
    Render {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Page-level property reads/writes.
    Prop {
        #[command(subcommand)]
        sub: super::prop::PropCommand,
    },
}

/// Run a `outl page …` invocation.
pub fn run(cmd: &PageCommand, path: &Path) -> i32 {
    match cmd {
        PageCommand::Get { slug, json } => {
            let result = ws::open(path).and_then(|mut ctx| get(&mut ctx, slug));
            emit(*json, result, print_page_get)
        }
        PageCommand::Create {
            slug,
            title,
            icon,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| create(&mut ctx, slug, title.as_deref(), icon.as_deref()));
            emit(*json, result, |v| print_page_meta("created", v))
        }
        PageCommand::Update {
            slug,
            title,
            icon,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| update(&mut ctx, slug, title.as_deref(), icon.as_deref()));
            emit(*json, result, |v| print_page_meta("updated", v))
        }
        PageCommand::Delete {
            slug,
            confirm,
            json,
        } => {
            if !*confirm {
                let err = ApiError::new(
                    codes::CONFIRM_REQUIRED,
                    format!("refusing to delete page `{slug}` without --confirm"),
                );
                return emit::<Value, _>(*json, Err(err), |_| {});
            }
            let result = ws::open(path).and_then(|mut ctx| delete(&mut ctx, slug));
            emit(*json, result, |v| {
                println!(
                    "deleted: {}",
                    v.get("slug").and_then(Value::as_str).unwrap_or("?")
                );
            })
        }
        PageCommand::List { filter, json } => {
            let result = ws::open(path).and_then(|ctx| list(&ctx, filter.as_deref()));
            emit(*json, result, print_page_list)
        }
        PageCommand::Rename {
            old_slug,
            new_slug,
            json,
        } => {
            let result = ws::open(path).and_then(|mut ctx| rename(&mut ctx, old_slug, new_slug));
            emit(*json, result, |v| print_page_meta("renamed", v))
        }
        PageCommand::Render { slug, json } => {
            let result = ws::open(path).and_then(|ctx| render(&ctx, slug));
            emit(*json, result, |v| {
                if let Some(md) = v.get("md").and_then(Value::as_str) {
                    println!("{md}");
                }
            })
        }
        PageCommand::Prop { sub } => super::prop::run(sub, path),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Resolve a slug to a page node id or return [`codes::PAGE_NOT_FOUND`].
fn resolve(ctx: &WsCtx, slug: &str) -> Result<NodeId, ApiError> {
    find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))
}

/// Page meta + outline tree.
pub fn get(ctx: &mut WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, slug)?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    let outline = project_outline(&ctx.workspace, id);
    let icon = page_property(&ctx.workspace, id, "icon");
    Ok(json!({
        "meta": serde_json::to_value(&meta).map_err(ApiError::internal)?,
        "icon": icon,
        "outline": serde_json::to_value(&outline).map_err(ApiError::internal)?,
    }))
}

/// Create a new page (idempotent on slug — re-creating returns the same
/// meta).
pub fn create(
    ctx: &mut WsCtx,
    slug: &str,
    title: Option<&str>,
    icon: Option<&str>,
) -> Result<Value, ApiError> {
    let display_title = title.unwrap_or(slug);
    let id = open_or_create_page(
        &mut ctx.workspace,
        &ctx.hlc,
        slug,
        display_title,
        PageKind::Page,
    )
    .map_err(ApiError::internal)?;

    if let Some(value) = icon {
        if !value.is_empty() {
            write_page_property(ctx, id, "icon", Some(value))?;
        }
    }

    write_projection(ctx, id)?;
    page_meta_value(ctx, id)
}

/// Update a page's title and/or icon. At least one of `--title` /
/// `--icon` must be provided.
pub fn update(
    ctx: &mut WsCtx,
    slug: &str,
    title: Option<&str>,
    icon: Option<&str>,
) -> Result<Value, ApiError> {
    if title.is_none() && icon.is_none() {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "update requires at least one of --title or --icon".to_string(),
        ));
    }
    let id = resolve(ctx, slug)?;

    if let Some(t) = title {
        outl_actions::block::edit_text(&mut ctx.workspace, &ctx.hlc, id, t)
            .map_err(ApiError::internal)?;
    }
    if let Some(icon_value) = icon {
        if icon_value.is_empty() {
            // Setting the property to None clears it.
            write_page_property(ctx, id, "icon", None)?;
        } else {
            write_page_property(ctx, id, "icon", Some(icon_value))?;
        }
    }

    write_projection(ctx, id)?;
    page_meta_value(ctx, id)
}

/// Delete a page by moving its root to the trash. The op stays in the
/// log so peers converge.
pub fn delete(ctx: &mut WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, slug)?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;

    outl_actions::block::delete(&mut ctx.workspace, &ctx.hlc, id).map_err(ApiError::internal)?;

    // Remove the on-disk projection so peers don't see a stale page.
    let md_path = outl_actions::page_md_path(&ctx.root, &meta);
    let _ = std::fs::remove_file(&md_path);
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    let _ = std::fs::remove_file(&sidecar_path);

    Ok(json!({
        "slug": meta.slug,
        "id": meta.id,
    }))
}

/// List every page, optionally filtered.
pub fn list(ctx: &WsCtx, filter: Option<&str>) -> Result<Value, ApiError> {
    let metas = list_pages(&ctx.workspace);
    let kept: Vec<_> = match filter {
        None => metas,
        Some(expr) => {
            let (key, value) = expr.split_once(':').ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARG,
                    format!("filter must be `key:value`, got `{expr}`"),
                )
            })?;
            let key = key.trim();
            let value = value.trim();
            metas
                .into_iter()
                .filter(|m| match key {
                    "kind" => m.kind.as_str() == value,
                    "tag" => {
                        let node = ulid::Ulid::from_string(&m.id).ok().map(NodeId);
                        node.map(|n| page_has_tag(&ctx.workspace, n, value))
                            .unwrap_or(false)
                    }
                    _ => true,
                })
                .collect()
        }
    };
    Ok(json!({ "pages": kept }))
}

/// Rename a page's slug. Updates the `page-slug` property, renames
/// the on-disk `.md`, and returns the list of blocks still referencing
/// the old slug so the caller can decide whether to rewrite them.
pub fn rename(ctx: &mut WsCtx, old_slug: &str, new_slug: &str) -> Result<Value, ApiError> {
    if find_by_slug(&ctx.workspace, new_slug).is_some() {
        return Err(ApiError::new(
            codes::SLUG_CONFLICT,
            format!("page `{new_slug}` already exists"),
        ));
    }
    let id = resolve(ctx, old_slug)?;
    let old_meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;

    write_page_property(ctx, id, SLUG_KEY, Some(new_slug))?;
    write_projection(ctx, id)?;

    // Remove the old md/sidecar so the workspace doesn't keep a stale copy.
    let old_md = outl_actions::page_md_path(&ctx.root, &old_meta);
    let _ = std::fs::remove_file(&old_md);
    let old_sidecar = outl_md::resolve_sidecar_path(&old_md);
    let _ = std::fs::remove_file(&old_sidecar);

    // Affected backlinks: keep the old textual form so the caller can
    // grep / rewrite.
    let affected = outl_actions::backlinks_for_target(&ctx.workspace, old_slug);
    Ok(json!({
        "meta": page_meta_inner(ctx, id)?,
        "old_slug": old_slug,
        "affected_refs": affected
            .iter()
            .map(|b| json!({
                "block_id": b.block_id,
                "block_text": b.block_text,
                "source_page": b.source_page,
            }))
            .collect::<Vec<_>>(),
    }))
}

/// Render a page's outline to clean markdown.
pub fn render(ctx: &WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, slug)?;
    let md = render_page_md(&ctx.workspace, id);
    Ok(json!({ "slug": slug, "md": md }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn page_meta_value(ctx: &WsCtx, id: NodeId) -> Result<Value, ApiError> {
    Ok(json!({ "meta": page_meta_inner(ctx, id)? }))
}

fn page_meta_inner(ctx: &WsCtx, id: NodeId) -> Result<Value, ApiError> {
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    serde_json::to_value(&meta).map_err(ApiError::internal)
}

/// Read a page property into a JSON value (string when text; structured
/// form for `List`/`Tag`/`PageRef`).
fn page_property(workspace: &outl_core::workspace::Workspace, id: NodeId, key: &str) -> Value {
    match read_text_prop(workspace, id, key) {
        Some(s) => Value::String(s),
        None => Value::Null,
    }
}

/// Wrap `outl_actions::set_property` with `ApiError` mapping. `None`
/// clears the property.
fn write_page_property(
    ctx: &mut WsCtx,
    id: NodeId,
    key: &str,
    value: Option<&str>,
) -> Result<(), ApiError> {
    let val = value.map(|v| PropValue::Text(v.to_string()));
    set_property(&mut ctx.workspace, &ctx.hlc, id, key, val).map_err(ApiError::internal)
}

fn write_projection(ctx: &mut WsCtx, id: NodeId) -> Result<(), ApiError> {
    apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, id).map_err(ApiError::internal)?;
    Ok(())
}

fn page_has_tag(workspace: &outl_core::workspace::Workspace, page: NodeId, tag: &str) -> bool {
    let needle = format!("#{tag}");
    let mut found = false;
    walk_subtree(workspace, page, |id| {
        if let Some(text) = workspace.block_text(id) {
            if text.contains(&needle) {
                found = true;
                return false; // early-stop
            }
        }
        true
    });
    found
}

// ---------------------------------------------------------------------------
// Human-readable formatters
// ---------------------------------------------------------------------------

fn print_page_get(v: &Value) {
    let title = v
        .pointer("/meta/title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)");
    let slug = v
        .pointer("/meta/slug")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = v
        .pointer("/meta/kind")
        .and_then(Value::as_str)
        .unwrap_or("page");
    println!("{title}  ({slug}, {kind})");
    if let Some(outline) = v.get("outline").and_then(Value::as_array) {
        print_outline_tree(outline, 0);
    }
}

fn print_page_meta(verb: &str, v: &Value) {
    let slug = v
        .pointer("/meta/slug")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let title = v
        .pointer("/meta/title")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("{verb}: {slug} ({title})");
}

fn print_page_list(v: &Value) {
    let pages = v.get("pages").and_then(Value::as_array);
    let Some(pages) = pages else {
        return;
    };
    if pages.is_empty() {
        println!("no pages");
        return;
    }
    for page in pages {
        let slug = page.get("slug").and_then(Value::as_str).unwrap_or("?");
        let title = page.get("title").and_then(Value::as_str).unwrap_or("?");
        let kind = page.get("kind").and_then(Value::as_str).unwrap_or("page");
        println!("{kind:8}  {slug:30}  {title}");
    }
}
