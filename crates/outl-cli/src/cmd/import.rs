//! `outl import` — bring an existing graph in from Logseq or Roam.
//!
//! Each source format has its own quirks (Logseq stores `id::` lines
//! inline; Roam ships a JSON backup). The shared output is the same:
//! a populated `pages/` and `journals/` directory in an outl
//! workspace, plus an initial reconcile so sidecars are stamped.

use crate::workspace_layout::{ensure_ops_dir, read_config, Paths};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_md::slug::slugify;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub mod logseq;
pub mod roam;

/// Summary of what an import produced.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Pages copied / converted.
    pub pages: usize,
    /// Journals copied / converted.
    pub journals: usize,
    /// Block-level references that couldn't be resolved
    /// (block UID with no matching block).
    pub unresolved_block_refs: usize,
    /// `id::` / `((uid))` / similar artifacts stripped from output.
    pub artifacts_stripped: usize,
}

impl ImportReport {
    /// Pretty-print the report to stdout.
    pub fn print(&self) {
        println!();
        println!("Import summary:");
        println!("  pages:                {}", self.pages);
        println!("  journals:             {}", self.journals);
        println!(
            "  artifacts stripped:   {} (id::, block refs, embed comments)",
            self.artifacts_stripped
        );
        if self.unresolved_block_refs > 0 {
            println!(
                "  unresolved block refs: {} (left as plain text)",
                self.unresolved_block_refs
            );
        }
    }
}

/// Dispatch on the source format chosen by the user.
pub fn run(source: &str, src: &Path, dst: &Path) -> Result<()> {
    let dst = dst.to_path_buf();
    if !dst.exists() {
        crate::cmd::init::run(&dst)?;
    }
    let paths = Paths::at(dst.clone());

    let report = match source {
        "logseq" => logseq::import(src, &paths)
            .with_context(|| format!("logseq import from {}", src.display()))?,
        "roam" => roam::import(src, &paths)
            .with_context(|| format!("roam import from {}", src.display()))?,
        other => anyhow::bail!("unknown import source: {other} (expected: logseq, roam)"),
    };

    report.print();
    println!();
    println!(
        "Next: run `outl --path {}` to open the imported workspace.",
        dst.display()
    );
    Ok(())
}

// --- helpers shared between Logseq and Roam imports ----------------------

/// Write a page `.md` to disk. Filename comes from `slugify(name)`.
/// Returns the file path that was written.
pub(super) fn write_page_md(
    paths: &Paths,
    title: &str,
    body: &str,
    is_journal: bool,
) -> Result<PathBuf> {
    let dir = if is_journal {
        &paths.journals
    } else {
        &paths.pages
    };
    fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let stem = if is_journal {
        // For journals the title is itself the ISO date — keep as-is.
        title.to_string()
    } else {
        slugify(title)
    };
    let path = dir.join(format!("{stem}.md"));
    let full = if is_journal {
        // Journals don't need the `title::` header — the filename is
        // the date.
        body.to_string()
    } else {
        // Pages always carry the original human name in `title::`.
        format!("title:: {title}\n\n{body}")
    };
    outl_md::write_atomic(&path, full.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Try to parse `name` as a journal date in any of the formats the
/// source tools use. Returns `Some(NaiveDate)` if it looks like one of:
///
/// - ISO 8601 (`2026-05-25`)
/// - Logseq's local format depending on locale (we accept the most
///   common: `Mar 5th, 2026`)
/// - Roam's `January 5th, 2026`
pub(super) fn parse_journal_date(name: &str) -> Option<NaiveDate> {
    if let Ok(d) = NaiveDate::parse_from_str(name, "%Y-%m-%d") {
        return Some(d);
    }
    // Try Roam-style: "January 5th, 2026", "May 25th, 2026" etc.
    // chrono's parser doesn't accept "5th" suffixes natively; strip
    // them and retry.
    let stripped = strip_ordinal_suffixes(name);
    if let Ok(d) = NaiveDate::parse_from_str(&stripped, "%B %d, %Y") {
        return Some(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(&stripped, "%b %d, %Y") {
        return Some(d);
    }
    None
}

/// Remove `1st`, `2nd`, `3rd`, `4th`...`31st` ordinal suffixes from
/// "January 5th, 2026" style strings so chrono can parse them.
fn strip_ordinal_suffixes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            out.push(c);
            // Look ahead for a 2-letter ordinal suffix immediately
            // following.
            let mut clone = chars.clone();
            let a = clone.next();
            let b = clone.next();
            let is_suffix = matches!(
                (a, b),
                (Some('s'), Some('t'))
                    | (Some('n'), Some('d'))
                    | (Some('r'), Some('d'))
                    | (Some('t'), Some('h'))
            );
            if is_suffix {
                // skip both letters
                chars.next();
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Build a `block_uid → page slug` map for resolving `((uid))` refs
/// against pages that exist somewhere else in the source.
pub(super) type UidIndex = HashMap<String, ResolvedUid>;

/// Where a UID lives in the target workspace.
#[derive(Debug, Clone)]
pub(super) struct ResolvedUid {
    /// User-visible page name (used to render `[[Title]]`).
    pub page_name: String,
    /// First ~80 chars of the block's text — kept for future surfaces
    /// (a `--verbose` import report, or future block-embed rendering).
    /// Populated by importers but unread for now.
    #[allow(dead_code)]
    pub snippet: String,
}

/// Run `reconcile_md` on every imported file so the sidecar JSON is
/// stamped with stable IDs. Without this, the user has to open the
/// TUI once to seed sidecars — which works but is surprising.
///
/// Acquires the same locks every other workspace opener takes
/// (shared `WorkspaceLock` + per-actor `ActorWriteLock` via
/// `resolve_write_actor`), so a concurrent TUI / MCP server / serve
/// against the destination is safe. The importer normally lands in
/// a fresh workspace, but supporting "import while attached" is a
/// no-cost consequence of routing through the standard flow.
pub(super) fn seed_sidecars(paths: &Paths) -> Result<()> {
    let cfg = read_config(paths)?;
    let config_actor = cfg.actor()?;

    let _lock = outl_core::WorkspaceLock::acquire(&paths.root).with_context(|| {
        format!(
            "could not acquire workspace lock at {}",
            paths.root.display()
        )
    })?;
    ensure_ops_dir(paths)?;
    let (_actor_lock, actor) = outl_core::resolve_write_actor(&paths.ops, config_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", paths.ops.display()))?;

    let storage = JsonlStorage::open(paths.ops.clone(), actor)?;
    let mut ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))?;
    let hlc = HlcGenerator::new(actor);

    for dir in [&paths.pages, &paths.journals] {
        for entry in walkdir::WalkDir::new(dir).max_depth(1) {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            let _ = outl_md::reconcile::reconcile_md(&mut ws, &hlc, p, Some(&paths.orphans));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_parse() {
        assert_eq!(
            parse_journal_date("2026-05-25").unwrap().to_string(),
            "2026-05-25"
        );
    }

    #[test]
    fn roam_long_dates_parse() {
        assert_eq!(
            parse_journal_date("January 5th, 2026").unwrap().to_string(),
            "2026-01-05"
        );
        assert_eq!(
            parse_journal_date("May 25th, 2026").unwrap().to_string(),
            "2026-05-25"
        );
        assert_eq!(
            parse_journal_date("March 2nd, 2026").unwrap().to_string(),
            "2026-03-02"
        );
    }

    #[test]
    fn non_dates_return_none() {
        assert!(parse_journal_date("Avelino").is_none());
        assert!(parse_journal_date("Project X").is_none());
    }

    #[test]
    fn ordinal_stripper_handles_edge_cases() {
        assert_eq!(strip_ordinal_suffixes("1st"), "1");
        assert_eq!(strip_ordinal_suffixes("22nd"), "22");
        assert_eq!(strip_ordinal_suffixes("3rd"), "3");
        assert_eq!(strip_ordinal_suffixes("4th"), "4");
        assert_eq!(strip_ordinal_suffixes("plain text"), "plain text");
    }
}
