//! Helpers shared by every importer (Logseq, Roam, Obsidian, future).
//!
//! Anything two or more importers genuinely use the same way lives
//! here: the import report, page writing, journal-date parsing, the
//! `((uid))` resolution machinery, the shallow `.md` walk, and the
//! final sidecar-seeding pass. Source-specific quirks stay in each
//! importer file — see `CLAUDE.md` in this directory.

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
            "  artifacts stripped:   {} (source-specific metadata: id::, block refs, embed markers, dropped frontmatter keys)",
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

/// Write a page `.md` to disk. Filename comes from `slugify(name)`.
/// Returns the file path that was written.
pub(in crate::cmd::import) fn write_page_md(
    paths: &Paths,
    title: &str,
    body: &str,
    is_journal: bool,
) -> Result<PathBuf> {
    write_page_md_with_stem(paths, title, body, is_journal, None)
}

/// Same as [`write_page_md`] but the caller can override the on-disk
/// filename stem. Used by importers that need to disambiguate slug
/// collisions (e.g. two source files with the same H1 title) before
/// writing — they compute a unique stem themselves and pass it here.
/// `title` is still what gets written into the `title::` property, so
/// the user-visible page name is unaffected.
pub(in crate::cmd::import) fn write_page_md_with_stem(
    paths: &Paths,
    title: &str,
    body: &str,
    is_journal: bool,
    stem_override: Option<&str>,
) -> Result<PathBuf> {
    let dir = if is_journal {
        &paths.journals
    } else {
        &paths.pages
    };
    fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let stem = if let Some(s) = stem_override {
        s.to_string()
    } else if is_journal {
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
/// source tools use (ISO 8601, Logseq's `Mar 5th, 2026`, Roam's
/// `January 5th, 2026`, slashed and day-first variants).
///
/// Thin wrapper over [`outl_actions::parse_flexible_date`] — the one
/// owner of "human-typed date → `NaiveDate`" across the workspace.
pub(in crate::cmd::import) fn parse_journal_date(name: &str) -> Option<NaiveDate> {
    outl_actions::parse_flexible_date(name)
}

/// Build a `block_uid → page slug` map for resolving `((uid))` refs
/// against pages that exist somewhere else in the source.
pub(in crate::cmd::import) type UidIndex = HashMap<String, ResolvedUid>;

/// Where a UID lives in the target workspace.
#[derive(Debug, Clone)]
pub(in crate::cmd::import) struct ResolvedUid {
    /// User-visible page name (used to render `[[Title]]`).
    pub page_name: String,
    /// First ~80 chars of the block's text — kept for future surfaces
    /// (a `--verbose` import report, or future block-embed rendering).
    /// Populated by importers but unread for now.
    #[allow(dead_code)]
    pub snippet: String,
}

/// Truncate `s` to at most `max` chars, appending `…` when trimmed.
/// Used to build the [`ResolvedUid::snippet`] previews.
pub(in crate::cmd::import) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Resolve one `((uid))` reference: `[[Page Title]]` when the UID is
/// known (counted as a stripped artifact), `((unresolved:uid))`
/// otherwise (counted as unresolved so the user can hunt it down
/// post-import).
pub(in crate::cmd::import) fn resolve_uid_ref(
    uid: &str,
    uid_index: &UidIndex,
    artifacts: &mut usize,
    unresolved: &mut usize,
) -> String {
    if let Some(resolved) = uid_index.get(uid) {
        *artifacts += 1;
        format!("[[{}]]", resolved.page_name)
    } else {
        *unresolved += 1;
        format!("((unresolved:{uid}))")
    }
}

/// Rewrite every `((uid))` reference in `text` via
/// [`resolve_uid_ref`]. An unbalanced `((` with no closing `))` is
/// left verbatim.
pub(in crate::cmd::import) fn rewrite_uid_refs(
    text: &str,
    uid_index: &UidIndex,
    artifacts: &mut usize,
    unresolved: &mut usize,
) -> String {
    if !text.contains("((") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(open_rel) = text[cursor..].find("((") {
        let abs_open = cursor + open_rel;
        out.push_str(&text[cursor..abs_open]);
        let after_open = abs_open + 2;
        let Some(close_rel) = text[after_open..].find("))") else {
            // Unbalanced — copy the rest verbatim and stop.
            out.push_str(&text[abs_open..]);
            return out;
        };
        let uid = &text[after_open..after_open + close_rel];
        out.push_str(&resolve_uid_ref(uid, uid_index, artifacts, unresolved));
        cursor = after_open + close_rel + 2;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Collect the `.md` files sitting directly inside `dir` (depth 1, no
/// recursion). Missing directories yield an empty list.
pub(in crate::cmd::import) fn md_files_shallow(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect()
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
pub(in crate::cmd::import) fn seed_sidecars(paths: &Paths) -> Result<()> {
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
        for p in md_files_shallow(dir) {
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            let _ = outl_md::reconcile::reconcile_md(&mut ws, &hlc, &p, Some(&paths.orphans));
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

    fn index_with(uid: &str, page: &str) -> UidIndex {
        let mut idx = UidIndex::new();
        idx.insert(
            uid.to_string(),
            ResolvedUid {
                page_name: page.to_string(),
                snippet: String::new(),
            },
        );
        idx
    }

    #[test]
    fn known_uid_becomes_page_link() {
        let idx = index_with("abc", "Source");
        let (mut a, mut u) = (0, 0);
        let out = rewrite_uid_refs("see ((abc)) here", &idx, &mut a, &mut u);
        assert_eq!(out, "see [[Source]] here");
        assert_eq!((a, u), (1, 0));
    }

    #[test]
    fn unknown_uid_is_marked_unresolved() {
        let idx = UidIndex::new();
        let (mut a, mut u) = (0, 0);
        let out = rewrite_uid_refs("((nope))", &idx, &mut a, &mut u);
        assert_eq!(out, "((unresolved:nope))");
        assert_eq!((a, u), (0, 1));
    }

    #[test]
    fn unbalanced_ref_passes_through() {
        let idx = UidIndex::new();
        let (mut a, mut u) = (0, 0);
        let out = rewrite_uid_refs("open ((abc", &idx, &mut a, &mut u);
        assert_eq!(out, "open ((abc");
        assert_eq!((a, u), (0, 0));
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate("short", 80), "short");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}
