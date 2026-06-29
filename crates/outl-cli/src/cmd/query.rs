//! `outl query …` — structured filter over pages and blocks.
//!
//! v1 surface: `--tag=foo`, `--priority=p1`, `--since=Nd`,
//! `--kind=page|journal`. Filters are AND-composed. `--raw` is reserved
//! for the query DSL (not yet implemented) and currently returns `INVALID_ARG`.

use std::path::Path;

use chrono::{Duration, NaiveDate};
use clap::Args;
use serde_json::{json, Value};

use outl_actions::{today, walk_subtree, PageMeta};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// Args for `outl query`.
#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Page must mention `#<tag>` somewhere in its subtree.
    #[arg(long)]
    pub tag: Option<String>,
    /// Page must carry property `priority::` matching this value.
    #[arg(long)]
    pub priority: Option<String>,
    /// Generic property filter: `--prop key=value`. Repeatable.
    #[arg(long = "prop", value_name = "KEY=VALUE")]
    pub props: Vec<String>,
    /// Only return journals whose date is within the last N days
    /// (`7d`, `30d`, …) or after an explicit ISO date.
    #[arg(long)]
    pub since: Option<String>,
    /// Restrict to a single page kind: `page` | `journal`.
    #[arg(long)]
    pub kind: Option<String>,
    /// Reserved for the query DSL (not yet implemented) — currently rejected.
    #[arg(long)]
    pub raw: Option<String>,
    /// Force JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Run a `outl query` invocation.
pub fn run(args: &QueryArgs, path: &Path) -> i32 {
    let result = ws::open(path).and_then(|ctx| handler(&ctx, args));
    emit(args.json, result, |v| {
        if let Some(items) = v.get("results").and_then(Value::as_array) {
            for item in items {
                let slug = item.get("slug").and_then(Value::as_str).unwrap_or("?");
                let kind = item.get("kind").and_then(Value::as_str).unwrap_or("page");
                let title = item.get("title").and_then(Value::as_str).unwrap_or("?");
                println!("{kind:8}  {slug:30}  {title}");
            }
        }
    })
}

/// Pure handler — used by both CLI and MCP shim.
pub fn handler(ctx: &WsCtx, args: &QueryArgs) -> Result<Value, ApiError> {
    if args.raw.is_some() {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "--raw is reserved for the query DSL and not yet implemented".to_string(),
        ));
    }

    let cutoff = args.since.as_deref().map(parse_since).transpose()?;
    let parsed_props = parse_prop_filters(&args.props, args.priority.as_deref())?;

    let mut matches: Vec<Value> = Vec::new();
    for meta in outl_actions::list_pages(&ctx.workspace) {
        if let Some(kind) = &args.kind {
            if meta.kind.as_str() != kind {
                continue;
            }
        }
        if let Some(start) = cutoff {
            if !journal_after(&meta, start) {
                continue;
            }
        }
        let id = match ulid::Ulid::from_string(&meta.id) {
            Ok(u) => NodeId(u),
            Err(_) => continue,
        };

        if let Some(tag) = &args.tag {
            if !subtree_contains(&ctx.workspace, id, &format!("#{tag}")) {
                continue;
            }
        }

        let mut props_ok = true;
        for (key, value) in &parsed_props {
            if !page_property_matches(&ctx.workspace, id, key, value) {
                props_ok = false;
                break;
            }
        }
        if !props_ok {
            continue;
        }

        matches.push(json!({
            "id": meta.id,
            "slug": meta.slug,
            "title": meta.title,
            "kind": meta.kind,
        }));
    }
    Ok(json!({
        "count": matches.len(),
        "results": matches,
    }))
}

fn parse_since(s: &str) -> Result<NaiveDate, ApiError> {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_suffix('d') {
        let days: i64 = rest.parse().map_err(|_| {
            ApiError::new(
                codes::INVALID_ARG,
                format!("--since `{s}` is not a valid `Nd` form"),
            )
        })?;
        return Ok(today() - Duration::days(days));
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d").map_err(|_| {
        ApiError::new(
            codes::INVALID_ARG,
            format!("--since `{s}` is neither `Nd` nor ISO YYYY-MM-DD"),
        )
    })
}

fn parse_prop_filters(
    props: &[String],
    priority: Option<&str>,
) -> Result<Vec<(String, String)>, ApiError> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(p) = priority {
        out.push(("priority".to_string(), p.to_string()));
    }
    for raw in props {
        let (k, v) = raw.split_once('=').ok_or_else(|| {
            ApiError::new(
                codes::INVALID_ARG,
                format!("--prop must be `KEY=VALUE`, got `{raw}`"),
            )
        })?;
        out.push((k.trim().to_string(), v.trim().to_string()));
    }
    Ok(out)
}

fn journal_after(meta: &PageMeta, cutoff: NaiveDate) -> bool {
    if let Some(d) = outl_actions::date_from_slug(&meta.slug) {
        return d >= cutoff;
    }
    // Non-journals are kept; date filter only narrows down dated pages.
    !matches!(meta.kind, outl_actions::PageKind::Journal)
}

fn page_property_matches(
    workspace: &outl_core::workspace::Workspace,
    page: NodeId,
    key: &str,
    expected: &str,
) -> bool {
    match workspace.tree().property(page, key) {
        Some(PropValue::Text(s)) => s == expected,
        Some(PropValue::Tag(s)) => s == expected,
        Some(PropValue::PageRef(s)) => s == expected,
        Some(PropValue::List(items)) => items.iter().any(|v| match v {
            PropValue::Text(s) | PropValue::Tag(s) | PropValue::PageRef(s) => s == expected,
            _ => false,
        }),
        None => false,
    }
}

fn subtree_contains(
    workspace: &outl_core::workspace::Workspace,
    parent: NodeId,
    needle: &str,
) -> bool {
    let mut found = false;
    walk_subtree(workspace, parent, |id| {
        if let Some(text) = workspace.block_text(id) {
            if text.contains(needle) {
                found = true;
                return false;
            }
        }
        true
    });
    found
}
