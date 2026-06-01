//! Logseq import.
//!
//! Logseq stores graphs as plain markdown — pages in `pages/` and
//! journal entries in `journals/`. The bullets are outl-compatible
//! out of the box. The differences we have to clean up:
//!
//! - **`id:: <uuid>`** lines: Logseq writes a UUID on every block
//!   that's ever been referenced. Stripped — outl's IDs live in the
//!   sidecar, not the markdown.
//! - **`((<uuid>))`** block refs: we don't have block-level embeds yet
//!   (phase 3). We try to resolve the UID to its page; if found, the
//!   ref becomes `[[Page Title]]`. If not found, we leave it as plain
//!   text and log a warning.
//! - **`#+...`** directives: Logseq's per-file frontmatter style. We
//!   strip these (outl reads `title::` etc.).
//! - **Underscore-encoded names**: Logseq uses `___` and `%2F` for
//!   `/` in filenames. We canonicalize via [`super::slugify`].
//!
//! Journals in Logseq are filenames like `2026_05_25.md` (default) or
//! `2026-05-25.md` (configurable). Both work.

use super::{parse_journal_date, write_page_md, ImportReport, ResolvedUid, UidIndex};
use crate::workspace_layout::Paths;
use anyhow::{Context, Result};
use outl_md::reconcile::reconcile_md;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Run a Logseq import.
pub fn import(src: &Path, paths: &Paths) -> Result<ImportReport> {
    let mut report = ImportReport::default();

    if !src.is_dir() {
        anyhow::bail!(
            "logseq source must be the graph directory (got {})",
            src.display()
        );
    }

    // Pass 1: scan all .md files and build a uid → page-name index so
    // we can resolve `((uid))` block refs.
    let pages_dir = src.join("pages");
    let journals_dir = src.join("journals");
    let mut uid_index: UidIndex = HashMap::new();
    if pages_dir.is_dir() {
        scan_uids(&pages_dir, false, &mut uid_index)?;
    }
    if journals_dir.is_dir() {
        scan_uids(&journals_dir, true, &mut uid_index)?;
    }

    // Pass 2: convert.
    if pages_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&pages_dir).max_depth(1) {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            convert_file(entry.path(), false, &uid_index, paths, &mut report)?;
        }
    }
    if journals_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&journals_dir).max_depth(1) {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            convert_file(entry.path(), true, &uid_index, paths, &mut report)?;
        }
    }

    // Reconcile each imported file so sidecars get fresh IDs.
    seed_sidecars(paths)?;

    Ok(report)
}

/// Walk a directory and populate `uid_index` with every block's
/// UID → page name mapping (so `((uid))` refs can be resolved).
fn scan_uids(dir: &Path, is_journal: bool, uid_index: &mut UidIndex) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir).max_depth(1) {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;
        let page_name = logseq_page_name(entry.path(), is_journal);

        // For each `id:: <uid>` line, look back at the preceding block
        // content as the snippet. (Logseq writes id:: directly after
        // the block's content.)
        let mut last_block_text: String = String::new();
        for line in text.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("- ") {
                last_block_text = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("id:: ") {
                let uid = rest.trim().to_string();
                if !uid.is_empty() {
                    uid_index.insert(
                        uid,
                        ResolvedUid {
                            page_name: page_name.clone(),
                            snippet: truncate(&last_block_text, 80),
                        },
                    );
                }
            }
        }
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Convert a single Logseq `.md` file into outl format and write it
/// to `paths`. Updates `report` counts.
fn convert_file(
    src: &Path,
    is_journal: bool,
    uid_index: &UidIndex,
    paths: &Paths,
    report: &mut ImportReport,
) -> Result<()> {
    let text = fs::read_to_string(src).with_context(|| format!("reading {}", src.display()))?;
    let page_name = logseq_page_name(src, is_journal);

    let mut artifacts = 0usize;
    let mut unresolved = 0usize;
    let mut out_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_start();

        // Drop `id::` lines entirely — outl's IDs live in the sidecar.
        if trimmed.starts_with("id:: ") {
            artifacts += 1;
            continue;
        }

        // Drop Logseq frontmatter directives (#+title, #+date, etc.).
        if trimmed.starts_with("#+") {
            artifacts += 1;
            continue;
        }

        // Resolve `((uid))` block refs in-place.
        let mut converted = String::with_capacity(line.len());
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '(' && line[i..].starts_with("((") {
                if let Some(close_rel) = line[i + 2..].find("))") {
                    let uid = &line[i + 2..i + 2 + close_rel];
                    if let Some(resolved) = uid_index.get(uid) {
                        converted.push_str(&format!("[[{}]]", resolved.page_name));
                        artifacts += 1;
                    } else {
                        // Leave the original `((uid))` so the user can
                        // hunt it down post-import.
                        converted.push_str(&format!("((unresolved:{uid}))"));
                        unresolved += 1;
                    }
                    // Skip ahead past `))`.
                    for _ in 0..(2 + close_rel + 2 - 1) {
                        chars.next();
                    }
                    continue;
                }
            }
            converted.push(c);
        }
        out_lines.push(converted);
    }

    // Strip trailing blank lines so the file ends cleanly.
    while out_lines
        .last()
        .map(|l| l.trim().is_empty())
        .unwrap_or(false)
    {
        out_lines.pop();
    }
    let body = if out_lines.is_empty() {
        "- \n".to_string()
    } else {
        let mut s = out_lines.join("\n");
        s.push('\n');
        s
    };

    write_page_md(paths, &page_name, &body, is_journal)?;
    if is_journal {
        report.journals += 1;
    } else {
        report.pages += 1;
    }
    report.artifacts_stripped += artifacts;
    report.unresolved_block_refs += unresolved;
    Ok(())
}

/// Recover the user-visible page name from a Logseq filename.
///
/// Logseq encodes `/` in titles as `%2F` and (in some configs)
/// spaces as `___`. We decode both. Journals are returned as the
/// ISO date string if recognized; otherwise as-is.
fn logseq_page_name(path: &Path, is_journal: bool) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if is_journal {
        // Logseq's default filename is `2026_05_25.md`. Normalize to
        // ISO so `parse_journal_date` can recognize it downstream.
        let candidate = stem.replace('_', "-");
        if let Some(d) = parse_journal_date(&candidate) {
            return d.to_string();
        }
        return stem;
    }
    stem.replace("%2F", "/").replace("___", " ")
}

/// Run `reconcile_md` on every imported file so the sidecar JSON is
/// stamped with stable IDs. Without this, the user has to open the
/// TUI once to seed sidecars — which works but is surprising.
fn seed_sidecars(paths: &Paths) -> Result<()> {
    use outl_core::hlc::HlcGenerator;
    use outl_core::storage::JsonlStorage;
    use outl_core::workspace::Workspace;

    let cfg = crate::workspace_layout::read_config(paths)?;
    let actor = cfg.actor()?;
    std::fs::create_dir_all(&paths.ops)?;
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
            let _ = reconcile_md(&mut ws, &hlc, p, Some(&paths.orphans));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn page_name_decodes_logseq_encoding() {
        let p = std::path::PathBuf::from("/x/pages/foo%2Fbar.md");
        assert_eq!(logseq_page_name(&p, false), "foo/bar");
        let p = std::path::PathBuf::from("/x/pages/meu___projeto.md");
        assert_eq!(logseq_page_name(&p, false), "meu projeto");
    }

    #[test]
    fn journal_name_normalizes_underscores() {
        let p = std::path::PathBuf::from("/x/journals/2026_05_25.md");
        assert_eq!(logseq_page_name(&p, true), "2026-05-25");
    }

    #[test]
    fn id_lines_get_stripped() {
        let src_dir = TempDir::new().unwrap();
        let pages_dir = src_dir.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::write(
            pages_dir.join("foo.md"),
            "- first\n  id:: 6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90\n- second\n",
        )
        .unwrap();

        let dst_dir = TempDir::new().unwrap();
        let dst = dst_dir.path().join("ws");
        crate::cmd::init::run(&dst).unwrap();
        let paths = Paths::at(&dst);
        let report = import(src_dir.path(), &paths).unwrap();

        assert_eq!(report.pages, 1);
        assert!(report.artifacts_stripped >= 1);

        let out = fs::read_to_string(paths.pages.join("foo.md")).unwrap();
        assert!(!out.contains("id::"), "id:: line not stripped:\n{out}");
        assert!(out.contains("- first"));
        assert!(out.contains("- second"));
    }

    #[test]
    fn block_refs_resolve_to_page_links_when_uid_known() {
        let src_dir = TempDir::new().unwrap();
        let pages_dir = src_dir.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::write(
            pages_dir.join("source.md"),
            "- the original block\n  id:: 6601-source\n",
        )
        .unwrap();
        fs::write(
            pages_dir.join("referrer.md"),
            "- see ((6601-source)) for context\n",
        )
        .unwrap();

        let dst_dir = TempDir::new().unwrap();
        let dst = dst_dir.path().join("ws");
        crate::cmd::init::run(&dst).unwrap();
        let paths = Paths::at(&dst);
        let _ = import(src_dir.path(), &paths).unwrap();

        let out = fs::read_to_string(paths.pages.join("referrer.md")).unwrap();
        assert!(
            out.contains("[[source]]"),
            "block ref not rewritten to page link:\n{out}"
        );
    }

    #[test]
    fn unknown_block_refs_are_marked() {
        let src_dir = TempDir::new().unwrap();
        let pages_dir = src_dir.path().join("pages");
        fs::create_dir_all(&pages_dir).unwrap();
        fs::write(
            pages_dir.join("only.md"),
            "- refers to ((deadbeef-no-match))\n",
        )
        .unwrap();

        let dst_dir = TempDir::new().unwrap();
        let dst = dst_dir.path().join("ws");
        crate::cmd::init::run(&dst).unwrap();
        let paths = Paths::at(&dst);
        let report = import(src_dir.path(), &paths).unwrap();

        assert!(report.unresolved_block_refs >= 1);
        let out = fs::read_to_string(paths.pages.join("only.md")).unwrap();
        assert!(out.contains("unresolved:"));
    }
}
