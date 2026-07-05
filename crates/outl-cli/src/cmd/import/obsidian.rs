//! Obsidian vault import.
//!
//! Obsidian stores a vault as a tree of plain markdown files. Out of
//! the box the bullets and inline syntax are very close to outl's, so
//! the bulk of the file body round-trips unchanged. The differences we
//! have to clean up:
//!
//! - **YAML frontmatter** between `---` fences: Obsidian's primary
//!   metadata surface. Split + parsed by [`outl_md::frontmatter`]
//!   (the generic owner); this importer supplies the Obsidian policy:
//!   `aliases`, `cssclass`, `publish`, and `scroll` are dropped (no
//!   outl equivalent — counted in the import report) and a scalar
//!   `date` is normalized to ISO. Known keys (`title`, `tags`, `date`)
//!   become outl `key:: value` properties; unknown scalar keys pass
//!   through verbatim.
//! - **Wiki-link variants** (`[[Note|alias]]`, `[[Note#heading]]`,
//!   `[[Note^block-id]]`, `[[folder/Note]]`) collapse to `[[Note]]`
//!   via [`outl_md::wikilink::rewrite_wikilinks`].
//! - **Embeds** `![[note]]` and `![[image.png]]`: outl supports
//!   block-note embeds, so those keep their `![[...]]` shape and have
//!   their target normalised like any other wiki-link. Image
//!   attachments are converted to standard CommonMark link / image
//!   syntax via [`outl_md::wikilink::convert_image_links`], folder
//!   path preserved so the link stays resolvable once the user copies
//!   the `assets/` tree alongside the imported workspace.
//! - **Daily notes**: detected by filename matching an ISO date
//!   (`YYYY-MM-DD`) or one of the long-form spellings outl already
//!   understands (e.g. `May 25th, 2026`). Either signal routes the
//!   page to `journals/<iso>.md`. Files inside the configured
//!   daily-notes folder that don't parse as a date stay as regular
//!   pages — silently re-titling "Sprint kickoff" as a journal would
//!   be a data-loss regression. Custom moment.js date formats
//!   (`DD-MM-YYYY` etc.) are out of scope for v1.
//! - **Nested folders**: outl is flat. We collapse the folder tree
//!   into a single `path::` page property (relative path inside the
//!   vault) so the original location is recoverable without polluting
//!   the page title. Pages at the vault root get no `path::`.
//! - **Slug collisions**: handled by the [`stems`] submodule — the
//!   vault is walked once up front, collisions are detected, and the
//!   non-winning file gets a slug-safe path-derived suffix.
//! - **`.obsidian/`, `.trash/`, dotfiles**: skipped wholesale. These
//!   are app metadata, not user content.
//!
//! Out of scope for v1: nested objects in frontmatter (sequences and
//! mappings other than the tags array are dropped), moment.js date
//! format parsing beyond common ISO / long-form variants, daily-notes
//! folder routing (files are classified by filename/title only),
//! template plugin folder skipping, and Obsidian Canvas (`.canvas`)
//! files.

use super::common::parse_journal_date;
use super::ImportReport;
use crate::workspace_layout::Paths;
use anyhow::{Context, Result};
use outl_md::frontmatter::{extract_leading_h1, split_frontmatter, Frontmatter};
use outl_md::wikilink::{convert_image_links, rewrite_wikilinks};
use std::fs;
use std::path::Path;

mod stems;
#[cfg(test)]
mod tests;

/// Run an Obsidian import. `src` is the vault root directory.
pub fn import(src: &Path, paths: &Paths) -> Result<ImportReport> {
    let mut report = ImportReport::default();

    if !src.is_dir() {
        anyhow::bail!(
            "obsidian source must be the vault directory (got {})",
            src.display()
        );
    }

    // Pass 1: walk + compute base slug for every file, so collisions
    // can be detected before any file is written.
    let discovered = stems::discover(src)?;

    // Pass 2: assign unique stems. The lex-smallest relative path wins
    // the bare slug; colliding siblings get a folder / filename-stem
    // suffix so nothing is silently overwritten.
    let stem_map = stems::assign_unique_stems(&discovered, src);
    let collisions = discovered
        .iter()
        .filter(|d| stem_map.get(&d.path).is_some_and(|s| s != &d.base_slug))
        .count();
    if collisions > 0 {
        eprintln!("warning: {collisions} source file(s) produced colliding outl slugs;");
        eprintln!(
            "         each was disambiguated with a path-derived suffix (see the import report)."
        );
    }

    // Pass 3: convert each file using its assigned stem.
    for discovered in &discovered {
        let stem = stem_map
            .get(&discovered.path)
            .cloned()
            .unwrap_or_else(|| discovered.base_slug.clone());
        convert_file(&discovered.path, src, paths, &stem, &mut report)?;
    }

    super::common::seed_sidecars(paths)?;
    Ok(report)
}

// --- frontmatter policy ----------------------------------------------------

/// Frontmatter keys that are Obsidian-only app metadata with no outl
/// equivalent. Dropped (and counted in the import report).
const OBSIDIAN_DROPPED_KEYS: &[&str] = &["aliases", "cssclass", "publish", "scroll"];

/// Parse an Obsidian YAML frontmatter block: the generic
/// [`outl_md::frontmatter::parse_frontmatter`] does the structural
/// work; this wrapper supplies the Obsidian drop-list and normalizes a
/// scalar `date` value to ISO via the shared
/// [`outl_actions::parse_date_label`] (the one owner of flexible date
/// parsing). Unrecognized date spellings keep the original string so
/// the value isn't lost.
///
/// Returns `None` when the YAML fails to parse. Callers should
/// restore the original fenced block verbatim into the body so the
/// user's content isn't silently lost.
fn parse_obsidian_frontmatter(yaml: &str) -> Option<Frontmatter> {
    let mut fm = outl_md::frontmatter::parse_frontmatter(yaml, OBSIDIAN_DROPPED_KEYS)?;
    for (key, value) in fm.props.iter_mut() {
        if key == "date" {
            if let Some(iso) = outl_actions::parse_date_label(value) {
                *value = iso;
            }
        }
    }
    Some(fm)
}

// --- per-file conversion -------------------------------------------------

fn convert_file(
    src: &Path,
    vault_root: &Path,
    paths: &Paths,
    assigned_stem: &str,
    report: &mut ImportReport,
) -> Result<()> {
    let text = fs::read_to_string(src).with_context(|| format!("reading {}", src.display()))?;

    let rel = src.strip_prefix(vault_root).unwrap_or(src);
    let parent_rel: Option<String> = rel
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty() && s != ".");

    let file_stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // 1. Split frontmatter (if any) from body.
    let (frontmatter, body_after_fm) = split_frontmatter(&text);

    // 2. Parse frontmatter, if present. On YAML parse failure, fall
    //    back to restoring the fenced block verbatim into the body so
    //    no user content is silently dropped.
    let (fm_title, fm_props, body_after_fm) = match frontmatter.as_deref() {
        Some(yaml) => match parse_obsidian_frontmatter(yaml) {
            Some(fm) => {
                report.artifacts_stripped += fm.dropped;
                (fm.title, fm.props, body_after_fm)
            }
            None => {
                // Malformed YAML — keep the original fenced block as
                // literal text and report one artifact so the user
                // has a breadcrumb to grep for.
                report.artifacts_stripped += 1;
                let restored = format!("---\n{yaml}---\n\n{body_after_fm}");
                (None, Vec::new(), restored)
            }
        },
        None => (None, Vec::new(), body_after_fm),
    };

    // 3. Resolve page title: frontmatter `title` → leading H1 → filename.
    //    The H1 is only stripped from the body when it's actually used
    //    as the title (frontmatter wins, H1 stays as content).
    let (title, body_after_title) = if let Some(t) = fm_title {
        (t, body_after_fm)
    } else {
        match extract_leading_h1(&body_after_fm) {
            (Some(h1), rest) => (h1, rest),
            (None, _) => (file_stem.clone(), body_after_fm),
        }
    };

    // 4. Classify journal vs page. Primary signal is the filename stem
    //    (canonical Obsidian daily-note shape); title is a fallback.
    //    Files inside the configured daily-notes folder that don't
    //    parse as a date stay as regular pages so the user-facing
    //    title is preserved.
    let (final_title, is_journal) = classify(&file_stem, &title);

    // 5. Rewrite wiki-link variants in body. Image-target wiki-links
    //    are first converted to standard CommonMark links (so they
    //    resolve once the user copies the `assets/` tree alongside the
    //    workspace); then the remaining `[[Note...]]` tokens get their
    //    alias / heading / block-ref / folder-prefix stripped.
    let body_after_images = convert_image_links(&body_after_title);
    let body_rewritten = rewrite_wikilinks(&body_after_images);

    // 6. Compose body: prepend non-title properties + path::, leave
    //    bullets intact.
    let body_out = compose_body(
        &body_rewritten,
        &fm_props,
        parent_rel.as_deref(),
        is_journal,
    );

    // 7. Write. We always pass the caller-assigned stem so collisions
    //    detected in the pre-pass are honoured — `write_page_md_with_stem`
    //    still writes `title:: <title>` from the resolved title, so the
    //    user-visible name is unaffected by the on-disk filename.
    super::common::write_page_md_with_stem(
        paths,
        &final_title,
        &body_out,
        is_journal,
        if is_journal {
            None
        } else {
            Some(assigned_stem)
        },
    )?;
    if is_journal {
        report.journals += 1;
    } else {
        report.pages += 1;
    }
    Ok(())
}

/// Decide whether a file becomes a journal or a regular page.
///
/// Journal routing triggers when **either** the original filename stem
/// **or** the resolved title parses as a date (ISO or one of the
/// long-form spellings `parse_journal_date` already understands). A
/// non-date file inside the configured daily-notes folder stays a
/// regular page so the user-facing title survives — silently
/// re-titling "Sprint kickoff" as `journals/sprint-kickoff.md` would
/// be a data-loss regression.
///
/// On journal classification the returned title is always the ISO date
/// string so the slug lands at `journals/<iso>.md`.
fn classify(file_stem: &str, title: &str) -> (String, bool) {
    if let Some(d) = parse_journal_date(file_stem) {
        return (d.format("%Y-%m-%d").to_string(), true);
    }
    if let Some(d) = parse_journal_date(title) {
        return (d.format("%Y-%m-%d").to_string(), true);
    }
    (title.to_string(), false)
}

// --- body composition ----------------------------------------------------

/// Compose the final body that gets handed to `write_page_md_with_stem`.
///
/// `write_page_md_with_stem` prepends `title:: <title>\n\n` for
/// non-journals, so we never include `title` in `fm_props` here (it's
/// already lifted out by the frontmatter parser). Other properties
/// and the `path::` hint are prepended to the body; bullets follow.
fn compose_body(
    body: &str,
    fm_props: &[(String, String)],
    folder_path: Option<&str>,
    is_journal: bool,
) -> String {
    let mut props: Vec<(String, String)> = fm_props.to_vec();
    // Journals don't need path:: — they're already in `journals/`.
    if let Some(p) = folder_path {
        if !is_journal {
            props.push(("path".to_string(), p.to_string()));
        }
    }

    let body_clean = body.trim();
    let body_final = if body_clean.is_empty() {
        "- \n".to_string()
    } else {
        let mut s = body_clean.to_string();
        s.push('\n');
        s
    };

    if props.is_empty() {
        return body_final;
    }
    let mut out = String::new();
    for (k, v) in &props {
        out.push_str(k);
        out.push_str(":: ");
        out.push_str(v);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&body_final);
    out
}
