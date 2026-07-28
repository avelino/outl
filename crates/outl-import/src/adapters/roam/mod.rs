//! Roam Research adapter (JSON backup format).
//!
//! Roam's "Export → JSON" produces an array of pages, each with a
//! recursive tree of blocks:
//!
//! ```jsonc
//! [
//!   {
//!     "title": "Avelino",
//!     "children": [
//!       {
//!         "string": "first block",
//!         "uid": "abcd",
//!         "children": [ { "string": "child", "uid": "efgh" } ]
//!       }
//!     ]
//!   }
//! ]
//! ```
//!
//! The adapter's whole job is JSON → typed IR. Inline dialect
//! translation lives in the `inline` submodule; everything downstream (placeholder
//! emission, handle resolution) is the shared emitter's business.

mod inline;
#[cfg(test)]
mod tests;

use crate::adapter::{ImportError, SourceAdapter};
use crate::adapters::scan::parse_whole_fence;
use crate::ir::{
    BlockContent, ComponentKind, ImportBlock, ImportGraph, ImportPage, Inline, PageBody, PageName,
    TaskState,
};
use crate::report::ImportReport;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::path::Path;

/// Top-level Roam page from the JSON backup.
#[derive(Debug, Deserialize)]
struct RoamPage {
    title: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
}

/// One block in the Roam tree.
#[derive(Debug, Deserialize)]
struct RoamBlock {
    #[serde(default)]
    string: String,
    #[serde(default)]
    uid: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
    /// `heading: 1..3` — Roam's per-block heading level.
    #[serde(default)]
    heading: Option<u8>,
    /// `open: false` marks a folded block (EDN exports always carry
    /// it; JSON backups sometimes do).
    #[serde(default)]
    open: Option<bool>,
    /// Milliseconds since epoch.
    #[serde(default, rename = "create-time")]
    create_time: Option<i64>,
    /// Milliseconds since epoch.
    #[serde(default, rename = "edit-time")]
    edit_time: Option<i64>,
}

/// The Roam adapter.
pub struct RoamAdapter;

impl SourceAdapter for RoamAdapter {
    fn id(&self) -> &'static str {
        "roam"
    }

    fn detect(src: &Path) -> bool {
        src.is_file() && src.extension().and_then(|e| e.to_str()) == Some("json")
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        let text = std::fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;
        let pages: Vec<RoamPage> = serde_json::from_str(&text)
            .map_err(|e| ImportError::Parse(format!("roam backup JSON: {e}")))?;

        let mut graph = ImportGraph::default();
        for page in &pages {
            if page.title.trim().is_empty() {
                continue; // Roam can emit empty pages on edge cases.
            }
            let name = match outl_actions::parse_flexible_date(&page.title) {
                Some(d) => PageName::Journal(d),
                None => PageName::Named(page.title.clone()),
            };
            let blocks = page
                .children
                .iter()
                .map(|b| convert_block(b, &page.title, report))
                .collect();
            graph.pages.push(ImportPage {
                name,
                props: Vec::new(),
                body: PageBody::Outline(blocks),
                stem_override: None,
            });
        }
        Ok(graph)
    }
}

/// One Roam block → IR block (recursive).
fn convert_block(b: &RoamBlock, page: &str, report: &mut ImportReport) -> ImportBlock {
    let (task, raw) = split_task_marker(&b.string);
    if let Some(state) = task {
        report.count_task(state.report_key());
    }

    ImportBlock {
        uid: (!b.uid.is_empty()).then(|| b.uid.clone()),
        content: convert_content(raw, page, report),
        children: b
            .children
            .iter()
            .map(|c| convert_block(c, page, report))
            .collect(),
        task,
        heading: b.heading.filter(|h| (1..=3).contains(h)),
        collapsed: b.open == Some(false),
        props: Vec::new(),
        created: b.create_time.and_then(ms_to_datetime),
        edited: b.edit_time.and_then(ms_to_datetime),
    }
}

/// Strip a leading `{{[[TODO]]}}` / `{{[[DONE]]}}` marker.
fn split_task_marker(raw: &str) -> (Option<TaskState>, &str) {
    let trimmed = raw.trim_start();
    for (marker, state) in [
        ("{{[[TODO]]}}", TaskState::Todo),
        ("{{TODO}}", TaskState::Todo),
        ("{{[[DONE]]}}", TaskState::Done),
        ("{{DONE}}", TaskState::Done),
    ] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return (Some(state), rest.trim_start());
        }
    }
    (None, raw)
}

/// Roam block string → IR content.
fn convert_content(raw: &str, page: &str, report: &mut ImportReport) -> BlockContent {
    if let Some((lang, body)) = parse_whole_fence(raw) {
        return BlockContent::Code { lang, body };
    }
    if raw.contains("```") {
        // Mixed text + fence in one block string — preserve verbatim
        // rather than guessing at a split. Never drop content.
        return BlockContent::Verbatim(raw.to_string());
    }

    let pre = convert_org_dates(raw, report);
    let toks = inline::tokenize(&pre, page, report);

    // A block that IS a single query component becomes a ` ```query `
    // fence when the Roam query translates; otherwise it stays a
    // verbatim component (counted here — tokenize pushes queries
    // silently so this decision point owns the counting).
    if let Some(raw_query) = whole_block_query(&toks) {
        if let Some(body) = translate_query(raw_query) {
            report.queries_translated += 1;
            return BlockContent::Code {
                lang: Some("query".to_string()),
                body,
            };
        }
    }
    for t in &toks {
        if matches!(
            t,
            Inline::Component {
                kind: ComponentKind::Query,
                ..
            }
        ) {
            report.count_component("query");
        }
    }

    BlockContent::Inline(toks)
}

/// `Some(raw)` when the token list is exactly one query component
/// (plus whitespace).
fn whole_block_query(toks: &[Inline]) -> Option<&str> {
    let mut query: Option<&str> = None;
    for t in toks {
        match t {
            Inline::Component {
                kind: ComponentKind::Query,
                raw,
            } if query.is_none() => query = Some(raw),
            Inline::Text(s) if s.trim().is_empty() => {}
            _ => return None,
        }
    }
    query
}

/// Best-effort `{{[[query]]: {and: …}}}` → ` ```query ` DSL body.
///
/// Only the flat `{and: [[ref]] …}` form translates — `[[TODO]]` /
/// `[[DONE]]` become `status:` directives, every other ref becomes a
/// `text:` substring match (which is exactly "blocks referencing this
/// page"). Nested clauses (`{or:}`, `{between:}`, `{not:}`) have no
/// DSL equivalent and fall back to the verbatim component.
fn translate_query(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix("{{")?.strip_suffix("}}")?;
    let tail = inner[inner.find(':')? + 1..].trim();
    let and_body = tail.strip_prefix("{and:")?.strip_suffix('}')?.trim();
    if and_body.contains('{') {
        return None; // Nested clause — not translatable.
    }

    let mut lines: Vec<String> = Vec::new();
    let mut rest = and_body;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let close = after.find("]]")?;
        let name = after[..close].trim();
        match name {
            "TODO" => lines.push("status: todo".to_string()),
            "DONE" => lines.push("status: done".to_string()),
            _ => {
                if outl_actions::parse_flexible_date(name).is_some() {
                    // Absolute dates have no DSL equivalent (`since:`
                    // is relative-only) — don't mistranslate.
                    return None;
                }
                lines.push(format!("text: [[{name}]]"));
            }
        }
        rest = &after[close + 2..];
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// `DEADLINE: <2025-11-12 Wed>` / `SCHEDULED: <…>` → `[[2025-11-12]]`.
///
/// Follows the issue #63 model: a date in the block text is a
/// `[[date]]` link, surfacing the block in that day's journal via
/// backlinks; notification scheduling is a separate opt-in property.
fn convert_org_dates(raw: &str, report: &mut ImportReport) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        let hit = ["DEADLINE:", "SCHEDULED:"]
            .iter()
            .filter_map(|kw| rest.find(kw).map(|p| (p, *kw)))
            .min_by_key(|(p, _)| *p);
        let Some((pos, kw)) = hit else {
            out.push_str(rest);
            return out;
        };
        let after_kw = &rest[pos + kw.len()..];
        let after_sp = after_kw.trim_start();
        let converted = after_sp
            .strip_prefix('<')
            .and_then(|inner| inner.find('>').map(|close| &inner[..close]))
            .and_then(|stamp| {
                let date_part = stamp.get(..10)?;
                chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
            });
        match converted {
            Some(date) => {
                out.push_str(&rest[..pos]);
                out.push_str(&format!("[[{date}]]"));
                report.org_dates_converted += 1;
                // Skip: keyword + whitespace + `<…>`.
                let ws = after_kw.len() - after_sp.len();
                let angle_len = after_sp.find('>').expect("checked above") + 1;
                rest = &rest[pos + kw.len() + ws + angle_len..];
            }
            None => {
                out.push_str(&rest[..pos + kw.len()]);
                rest = &rest[pos + kw.len()..];
            }
        }
    }
}

/// Roam timestamps are milliseconds since epoch.
fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}
