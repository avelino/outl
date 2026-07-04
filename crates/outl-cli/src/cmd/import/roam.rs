//! Roam Research import (JSON backup format).
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
//!         "children": [
//!           { "string": "child block", "uid": "efgh", "children": [] }
//!         ]
//!       }
//!     ]
//!   },
//!   {
//!     "title": "May 25th, 2026",
//!     "children": [ ... ]
//!   }
//! ]
//! ```
//!
//! We walk that tree once to build a `uid → page-title` map (for
//! resolving `((uid))` block refs), then a second pass writes outl
//! markdown:
//!
//! - Pages whose title is a date go into `journals/<iso>.md`.
//! - Everything else goes into `pages/<slug>.md` with `title:: <name>`.
//! - `((uid))` becomes `[[Target Page]]` when the UID is known;
//!   otherwise it stays as `((unresolved:uid))` for manual triage.
//! - `[[Page Name]]` survives as-is (outl syntax matches Roam).
//! - `#[[Tag Name]]` becomes `[[Tag Name]]` (outl uses `#tag` only for
//!   single-token tags).

use super::common::{
    parse_journal_date, resolve_uid_ref, truncate, write_page_md, ResolvedUid, UidIndex,
};
use super::ImportReport;
use crate::workspace_layout::Paths;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Top-level Roam page from the JSON backup.
#[derive(Debug, Deserialize)]
struct RoamPage {
    title: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
}

/// One block in the Roam tree. We ignore Roam's `:create-time`,
/// `:edit-time`, and similar metadata — outl regenerates its own.
#[derive(Debug, Deserialize)]
struct RoamBlock {
    #[serde(default)]
    string: String,
    #[serde(default)]
    uid: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
}

/// Run a Roam import. `src` is the JSON backup file.
pub fn import(src: &Path, paths: &Paths) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let text = fs::read_to_string(src)
        .with_context(|| format!("reading roam backup at {}", src.display()))?;
    let pages: Vec<RoamPage> =
        serde_json::from_str(&text).with_context(|| "parsing roam backup JSON")?;

    // Pass 1: build the UID index.
    let mut uid_index: UidIndex = HashMap::new();
    for page in &pages {
        for block in &page.children {
            register_uids(&page.title, block, &mut uid_index);
        }
    }

    // Pass 2: emit files.
    for page in &pages {
        emit_page(page, &uid_index, paths, &mut report)?;
    }

    super::common::seed_sidecars(paths)?;
    Ok(report)
}

fn register_uids(page_title: &str, block: &RoamBlock, uid_index: &mut UidIndex) {
    if !block.uid.is_empty() {
        uid_index.insert(
            block.uid.clone(),
            ResolvedUid {
                page_name: page_title.to_string(),
                snippet: truncate(&block.string, 80),
            },
        );
    }
    for child in &block.children {
        register_uids(page_title, child, uid_index);
    }
}

fn emit_page(
    page: &RoamPage,
    uid_index: &UidIndex,
    paths: &Paths,
    report: &mut ImportReport,
) -> Result<()> {
    if page.title.trim().is_empty() {
        return Ok(()); // Roam can emit empty pages on edge cases.
    }

    let journal_date = parse_journal_date(&page.title);
    let mut body = String::new();
    let mut unresolved = 0usize;
    let mut artifacts = 0usize;

    for block in &page.children {
        emit_block(
            block,
            0,
            uid_index,
            &mut body,
            &mut unresolved,
            &mut artifacts,
        );
    }
    if body.is_empty() {
        body.push_str("- \n");
    }

    let (title, is_journal) = match journal_date {
        Some(d) => (d.format("%Y-%m-%d").to_string(), true),
        None => (page.title.clone(), false),
    };
    write_page_md(paths, &title, &body, is_journal)?;
    if is_journal {
        report.journals += 1;
    } else {
        report.pages += 1;
    }
    report.unresolved_block_refs += unresolved;
    report.artifacts_stripped += artifacts;
    Ok(())
}

fn emit_block(
    block: &RoamBlock,
    indent: usize,
    uid_index: &UidIndex,
    out: &mut String,
    unresolved: &mut usize,
    artifacts: &mut usize,
) {
    let pad = "  ".repeat(indent);
    let text = rewrite_inline(&block.string, uid_index, unresolved, artifacts);
    out.push_str(&pad);
    out.push_str("- ");
    out.push_str(&text);
    out.push('\n');
    for child in &block.children {
        emit_block(child, indent + 1, uid_index, out, unresolved, artifacts);
    }
}

/// Rewrite Roam-specific inline syntax to outl conventions.
fn rewrite_inline(
    text: &str,
    uid_index: &UidIndex,
    unresolved: &mut usize,
    artifacts: &mut usize,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        // `((uid))` block ref.
        if c == '(' && text[i..].starts_with("((") {
            if let Some(close_rel) = text[i + 2..].find("))") {
                let uid = &text[i + 2..i + 2 + close_rel];
                out.push_str(&resolve_uid_ref(uid, uid_index, artifacts, unresolved));
                for _ in 0..(2 + close_rel + 2 - 1) {
                    chars.next();
                }
                continue;
            }
        }
        // `#[[Multi Word Tag]]` → `[[Multi Word Tag]]`. outl tags
        // are single-token; multi-word tags become page refs.
        if c == '#' && text[i + 1..].starts_with("[[") {
            if let Some(close_rel) = text[i + 3..].find("]]") {
                let inner = &text[i + 3..i + 3 + close_rel];
                out.push_str(&format!("[[{inner}]]"));
                *artifacts += 1;
                for _ in 0..(1 + 2 + close_rel + 2 - 1) {
                    chars.next();
                }
                continue;
            }
        }
        // Roam's `{{TODO}}` / `{{DONE}}` markers — convert to outl
        // prefix style on the same block.
        if c == '{' && text[i..].starts_with("{{[[TODO]]}}") {
            out.push_str("TODO");
            *artifacts += 1;
            for _ in 0..("{{[[TODO]]}}".len() - 1) {
                chars.next();
            }
            continue;
        }
        if c == '{' && text[i..].starts_with("{{[[DONE]]}}") {
            out.push_str("DONE");
            *artifacts += 1;
            for _ in 0..("{{[[DONE]]}}".len() - 1) {
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn run_import(json: &str) -> (TempDir, Paths, ImportReport) {
        let src_dir = TempDir::new().unwrap();
        let src = src_dir.path().join("backup.json");
        fs::write(&src, json).unwrap();

        let dst_dir = TempDir::new().unwrap();
        let dst = dst_dir.path().join("ws");
        crate::cmd::init::run(&dst).unwrap();
        let paths = Paths::at(&dst);
        let report = import(&src, &paths).unwrap();
        // We return dst_dir to keep the tempdir alive.
        (dst_dir, paths, report)
    }

    #[test]
    fn basic_page_with_blocks() {
        let json = r#"[
            {"title": "Avelino", "children": [
                {"string": "first block", "uid": "a1", "children": []},
                {"string": "second block", "uid": "a2", "children": [
                    {"string": "child", "uid": "a3", "children": []}
                ]}
            ]}
        ]"#;
        let (_d, paths, report) = run_import(json);
        assert_eq!(report.pages, 1);
        assert_eq!(report.journals, 0);
        let out = fs::read_to_string(paths.pages.join("avelino.md")).unwrap();
        assert!(out.contains("title:: Avelino"));
        assert!(out.contains("- first block"));
        assert!(out.contains("- second block"));
        assert!(out.contains("  - child"));
    }

    #[test]
    fn journal_dates_become_journals() {
        let json = r#"[
            {"title": "May 25th, 2026", "children": [
                {"string": "morning thought", "uid": "j1", "children": []}
            ]}
        ]"#;
        let (_d, paths, report) = run_import(json);
        assert_eq!(report.journals, 1);
        let p = paths.journals.join("2026-05-25.md");
        assert!(p.exists(), "journal file missing at {}", p.display());
        let out = fs::read_to_string(p).unwrap();
        assert!(out.contains("- morning thought"));
        // Journals don't get `title::`.
        assert!(!out.starts_with("title::"));
    }

    #[test]
    fn block_refs_resolve_to_page_links() {
        let json = r#"[
            {"title": "Source", "children": [
                {"string": "the original", "uid": "src-uid", "children": []}
            ]},
            {"title": "Referrer", "children": [
                {"string": "see ((src-uid)) please", "uid": "ref", "children": []}
            ]}
        ]"#;
        let (_d, paths, _) = run_import(json);
        let out = fs::read_to_string(paths.pages.join("referrer.md")).unwrap();
        assert!(out.contains("[[Source]]"), "unresolved:\n{out}");
    }

    #[test]
    fn multiword_tags_become_page_refs() {
        let json = r#"[
            {"title": "Note", "children": [
                {"string": "see #[[My Project]] today", "uid": "x", "children": []}
            ]}
        ]"#;
        let (_d, paths, _) = run_import(json);
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[My Project]]"));
        assert!(!out.contains("#[[My Project]]"));
    }

    #[test]
    fn todo_markers_become_prefix() {
        let json = r#"[
            {"title": "Tasks", "children": [
                {"string": "{{[[TODO]]}} buy milk", "uid": "t1", "children": []},
                {"string": "{{[[DONE]]}} laundry", "uid": "t2", "children": []}
            ]}
        ]"#;
        let (_d, paths, _) = run_import(json);
        let out = fs::read_to_string(paths.pages.join("tasks.md")).unwrap();
        assert!(out.contains("- TODO buy milk"));
        assert!(out.contains("- DONE laundry"));
    }
}
