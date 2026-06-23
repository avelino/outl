//! Obsidian vault import.
//!
//! Obsidian stores a vault as a tree of plain markdown files. Out of
//! the box the bullets and inline syntax are very close to outl's, so
//! the bulk of the file body round-trips unchanged. The differences we
//! have to clean up:
//!
//! - **YAML frontmatter** between `---` fences: Obsidian's primary
//!   metadata surface. We parse it with [`serde_yaml_ng`] and re-emit
//!   known keys (`title`, `tags`, `date`) as outl `key:: value`
//!   properties. Unknown scalar keys pass through verbatim.
//!   `aliases`, `cssclass`, `publish`, and `scroll` are dropped (no
//!   outl equivalent — counted in the import report).
//! - **Wiki-link variants**:
//!   - `[[Note|alias]]` → `[[Note]]` (outl has no alias syntax).
//!   - `[[Note#heading]]` → `[[Note]]` (outl has no heading refs).
//!   - `[[Note^block-id]]` → `[[Note]]` (outl uses `((blk-XXXXXX))`).
//!   - `[[folder/Note]]` → `[[Note]]` (outl is flat — no folders).
//! - **Embeds** `![[note]]` and `![[image.png]]`: outl supports
//!   block-note embeds, so those keep their `![[...]]` shape and have
//!   their target normalised like any other wiki-link. Image
//!   attachments (`.png`, `.jpg`, `.jpeg`, …) are converted to
//!   standard CommonMark link / image syntax — `[alias](assets/foo.jpeg)`
//!   or `![caption](assets/foo.jpeg)` for embeds — because outl has no
//!   notion of image-as-page and a bare `[[bar.jpeg]]` would be a
//!   dangling ref. The folder path is preserved so the link stays
//!   resolvable once the user copies the `assets/` tree alongside the
//!   imported workspace.
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
//! - **Slug collisions**: outl is flat, so two source files whose H1
//!   (or frontmatter `title`, or filename stem) produce the same slug
//!   would silently overwrite each other. The importer walks the
//!   vault once up front, detects collisions, and disambiguates the
//!   non-winning file by appending a slug-safe suffix derived from
//!   the source path (`ideas` + `Docs/Ideas/Ideas.md` →
//!   `ideas-ideas`). The lex-smallest relative path wins the bare
//!   slug. The user-visible `title::` is unaffected — only the
//!   on-disk filename changes.
//! - **`.obsidian/`, `.trash/`, dotfiles**: skipped wholesale. These
//!   are app metadata, not user content.
//!
//! Out of scope for v1: nested objects in frontmatter (sequences and
//! mappings other than the tags array are dropped), moment.js date
//! format parsing beyond common ISO / long-form variants, daily-notes
//! folder routing (files are classified by filename/title only),
//! template plugin folder skipping, and Obsidian Canvas (`.canvas`)
//! files.

use super::{parse_journal_date, ImportReport};
use crate::workspace_layout::Paths;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde_yaml_ng::Value as YamlValue;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// Run an Obsidian import. `src` is the vault root directory.
pub fn import(src: &Path, paths: &Paths) -> Result<ImportReport> {
    let mut report = ImportReport::default();

    if !src.is_dir() {
        anyhow::bail!(
            "obsidian source must be the vault directory (got {})",
            src.display()
        );
    }

    // Pass 1: walk + compute base slug for every file. We don't need
    // the resolved title here (convert_file re-derives it) — only the
    // slug, so collisions can be detected before any file is written.
    let mut discovered: Vec<DiscoveredFile> = Vec::new();
    for entry in walkdir::WalkDir::new(src)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let title = derive_title(path, &text);
        let slug = outl_md::slug::slugify(&title);
        discovered.push(DiscoveredFile {
            path: path.to_path_buf(),
            base_slug: slug,
        });
    }

    // Pass 2: assign unique stems. The lex-smallest relative path wins
    // the bare slug; colliding siblings get a folder / filename-stem
    // suffix so nothing is silently overwritten.
    let stem_map = assign_unique_stems(&discovered, src);
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

    super::seed_sidecars(paths)?;
    Ok(report)
}

/// One source file we discovered during the walk, plus the slug we'd
/// assign it in the absence of collisions.
struct DiscoveredFile {
    path: std::path::PathBuf,
    base_slug: String,
}

/// Derive the page title for a discovered file, using the same
/// resolution order as `convert_file`: frontmatter `title` → leading
/// H1 → filename stem. We re-read the frontmatter here so the
/// collision pre-pass doesn't need to carry it around.
fn derive_title(src: &Path, text: &str) -> String {
    let (frontmatter, body_after_fm) = split_frontmatter(text);
    if let Some(yaml) = frontmatter.as_deref() {
        if let Ok((Some(title), _, _)) = parse_frontmatter(yaml) {
            return title;
        }
    }
    if let (Some(h1), _) = extract_leading_h1(&body_after_fm) {
        return h1;
    }
    src.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

/// Build a `source_path → final_stem` map. For each collision group
/// the entry whose relative path is lex-smallest keeps the bare slug;
/// every other entry gets a path-derived suffix until its stem is
/// unique inside the map.
fn assign_unique_stems(
    files: &[DiscoveredFile],
    vault_root: &Path,
) -> HashMap<std::path::PathBuf, String> {
    let mut by_slug: HashMap<String, Vec<&DiscoveredFile>> = HashMap::new();
    for f in files {
        by_slug.entry(f.base_slug.clone()).or_default().push(f);
    }

    let mut assigned: HashMap<std::path::PathBuf, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();

    // Process slugs in deterministic order so the output is stable
    // across runs (and across platforms with different FS enumeration
    // orders).
    let mut slug_keys: Vec<String> = by_slug.keys().cloned().collect();
    slug_keys.sort();

    for slug in &slug_keys {
        let group = by_slug.get(slug).unwrap();
        // Sort each collision group by relative path. The lex-smallest
        // path wins the bare slug; ties fall through to suffixing.
        let mut sorted: Vec<&DiscoveredFile> = group.clone();
        sorted.sort_by(|a, b| {
            a.path
                .strip_prefix(vault_root)
                .unwrap_or(&a.path)
                .cmp(b.path.strip_prefix(vault_root).unwrap_or(&b.path))
        });

        for (i, f) in sorted.iter().enumerate() {
            let stem = if i == 0 && !used.contains(slug) {
                slug.clone()
            } else {
                disambiguate_stem(slug, &f.path, vault_root, &used)
            };
            used.insert(stem.clone());
            assigned.insert(f.path.clone(), stem);
        }
    }

    assigned
}

/// Compute a path-derived alternative stem for `path` whose base slug
/// collided. Tries, in order: `<base>-<immediate-folder>`,
/// `<base>-<filename-stem>`, `<base>-<folder>-<filename-stem>`,
/// `<base>-<N>` (numeric fallback).
fn disambiguate_stem(base: &str, path: &Path, vault_root: &Path, used: &HashSet<String>) -> String {
    let rel = path.strip_prefix(vault_root).unwrap_or(path);
    let parent_name = rel
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| outl_md::slug::slugify(&n.to_string_lossy()))
        .unwrap_or_default();
    let file_stem = rel
        .file_stem()
        .map(|s| outl_md::slug::slugify(&s.to_string_lossy()))
        .unwrap_or_default();

    let candidates: [String; 3] = [
        format!("{base}-{parent_name}"),
        format!("{base}-{file_stem}"),
        format!("{base}-{parent_name}-{file_stem}"),
    ];
    for candidate in &candidates {
        if !candidate.ends_with('-') && !used.contains(candidate) {
            return candidate.clone();
        }
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !used.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

// --- walk filter ---------------------------------------------------------

/// True for entries that should not be visited at all (and not
/// descended into, when the entry is a directory). Anything starting
/// with a dot at depth > 0 is treated as app metadata — covers
/// `.obsidian/`, `.trash/`, `.git/`, per-file dotfiles, etc.
fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.') && entry.depth() > 0)
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
        Some(yaml) => match parse_frontmatter(yaml) {
            Ok((title, props, dropped)) => {
                report.artifacts_stripped += dropped;
                (title, props, body_after_fm)
            }
            Err(()) => {
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
    super::write_page_md_with_stem(
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

// --- frontmatter ---------------------------------------------------------

/// Split a leading `---\n...\n---\n` block from the file. Returns
/// `(Some(yaml_text), body)` when present and well-formed; otherwise
/// `(None, original_text)`. YAML's `...` document-end marker is also
/// honoured as a closing fence.
fn split_frontmatter(text: &str) -> (Option<String>, String) {
    let normalized: &str = if text.starts_with("---\r\n") {
        return split_frontmatter(&text.replace("\r\n", "\n"));
    } else if text.starts_with("---\n") {
        text
    } else {
        return (None, text.to_string());
    };
    let after_open = &normalized["---\n".len()..];

    let mut cursor = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = after_open[..cursor].to_string();
            let yaml = yaml.strip_suffix('\n').unwrap_or(&yaml).to_string();
            let body_offset = "---\n".len() + cursor + line.len();
            let body = if body_offset >= normalized.len() {
                String::new()
            } else {
                normalized[body_offset..].to_string()
            };
            return (Some(yaml), body);
        }
        cursor += line.len();
    }
    // No closing fence — treat whole file as body so we don't drop
    // user content.
    (None, normalized.to_string())
}

/// Parsed frontmatter: optional `title`, additional properties to
/// emit as `key:: value` lines, and a count of dropped keys (either
/// Obsidian-specific or non-scalar values we couldn't represent).
type Frontmatter = (Option<String>, Vec<(String, String)>, usize);

/// Parse a YAML frontmatter block. On success returns the title,
/// additional properties, and dropped-key count.
///
/// Returns `Err(())` when the YAML itself fails to parse. Callers
/// should restore the original fenced block verbatim into the body so
/// the user's content isn't silently lost.
fn parse_frontmatter(yaml: &str) -> Result<Frontmatter, ()> {
    let parsed: YamlValue = serde_yaml_ng::from_str(yaml).map_err(|_| ())?;
    let map = match parsed {
        YamlValue::Mapping(m) => m,
        _ => return Ok((None, Vec::new(), 0)),
    };

    let mut title: Option<String> = None;
    let mut props: Vec<(String, String)> = Vec::new();
    let mut dropped = 0;

    for (k, v) in map.into_iter() {
        let YamlValue::String(key) = k else {
            continue;
        };
        match key.as_str() {
            "title" => {
                if let Some(s) = scalar_string(&v) {
                    title = Some(s);
                } else {
                    dropped += 1;
                }
            }
            "tags" => {
                let tags = tags_from_yaml(&v);
                if !tags.is_empty() {
                    props.push(("tags".to_string(), tags.join(" ")));
                } else {
                    dropped += 1;
                }
            }
            // Obsidian-only, no outl equivalent.
            "aliases" | "cssclass" | "publish" | "scroll" => {
                dropped += 1;
            }
            "date" => {
                if let Some(s) = scalar_string(&v) {
                    let iso = normalize_date(&s).unwrap_or(s);
                    props.push(("date".to_string(), iso));
                } else {
                    dropped += 1;
                }
            }
            _ => {
                if let Some(s) = scalar_string(&v) {
                    props.push((key, s));
                } else {
                    // Non-scalar value we can't represent as `key:: v`.
                    dropped += 1;
                }
            }
        }
    }

    Ok((title, props, dropped))
}

/// Render a scalar YAML value (string / number / bool) to a String.
/// Returns `None` for sequences and mappings.
fn scalar_string(v: &YamlValue) -> Option<String> {
    match v {
        YamlValue::String(s) => Some(s.clone()),
        YamlValue::Number(n) => Some(n.to_string()),
        YamlValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract tags from a YAML value. Obsidian allows three shapes:
/// - scalar: `tags: foo`
/// - inline list: `tags: [foo, bar]`
/// - block list: `tags:\n  - foo\n  - bar`
///
/// Returned tags are normalized to `#name` form (no leading `#` in
/// the YAML, but `#`-prefixed in outl's `tags::` property).
fn tags_from_yaml(v: &YamlValue) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match v {
        YamlValue::String(s) => {
            // Obsidian also accepts inline comma / space separated.
            for t in s.split([',', ' ']) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(tag_form(t));
                }
            }
        }
        YamlValue::Sequence(seq) => {
            for item in seq {
                if let Some(s) = scalar_string(item) {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(tag_form(s));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Normalize a tag to `#name` form. Strips any leading `#` the user
/// might have written (Obsidian accepts both `foo` and `#foo`).
fn tag_form(raw: &str) -> String {
    let stripped = raw.trim_start_matches('#');
    format!("#{stripped}")
}

/// Try to render a date string as ISO `YYYY-MM-DD`. Accepts a handful
/// of common Obsidian / moment.js formats. Returns `None` for anything
/// unrecognized — callers should fall back to the original string so
/// the value isn't lost.
fn normalize_date(s: &str) -> Option<String> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    for fmt in &[
        "%Y/%m/%d",
        "%d/%m/%Y",
        "%B %d, %Y",
        "%b %d, %Y",
        "%d %B %Y",
        "%d %b %Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.format("%Y-%m-%d").to_string());
        }
    }
    None
}

// --- inline rewrites: image links then wiki-link variants --------

/// File extensions Obsidian treats as image attachments. When a
/// wiki-link target ends in one of these, the link is rewritten as a
/// standard CommonMark link / image rather than a `[[ref]]`, because
/// outl has no notion of image-as-page and a bare `[[bar.jpeg]]` would
/// be a dangling ref.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "ico", "tiff", "tif",
];

fn is_image_target(target: &str) -> bool {
    // Obsidian allows `#heading` and `^block-id` suffixes on any wiki
    // link, including images (e.g. `![[image.png#crop]]` for image
    // cropping). Strip them before checking the extension.
    let stripped = target.split_once('#').map(|(t, _)| t).unwrap_or(target);
    let stripped = stripped.split_once('^').map(|(t, _)| t).unwrap_or(stripped);
    let lower = stripped.to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// Convert Obsidian wiki-link / embed syntax for image assets into
/// standard CommonMark links, preserving the original folder path so
/// the link stays resolvable. Two shapes:
///
/// - `![[assets/foo/bar.jpeg]]`              → `![bar.jpeg](assets/foo/bar.jpeg)`
/// - `![[assets/foo/bar.jpeg|caption]]`      → `![caption](assets/foo/bar.jpeg)`
/// - `[[assets/foo/bar.jpeg|Open: x.png]]`   → `[Open: x.png](assets/foo/bar.jpeg)`
/// - `[[assets/foo/bar.jpeg]]`               → `[bar.jpeg](assets/foo/bar.jpeg)`
///
/// Non-image wiki-links pass through untouched (the regular
/// [`rewrite_wikilinks`] pass handles them afterwards).
fn convert_image_links(text: &str) -> String {
    if !text.contains("[[") {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(open_rel) = text[cursor..].find("[[") {
        let abs_open = cursor + open_rel;
        // Flush preceding text, including a possible leading '!'.
        out.push_str(&text[cursor..abs_open]);
        let embed = abs_open > 0 && bytes[abs_open - 1] == b'!';
        if embed {
            // Drop the '!' we already flushed; we'll re-emit it inside
            // the rewritten image token.
            out.pop();
        }
        let after_open = abs_open + 2;
        let Some(close_rel) = text[after_open..].find("]]") else {
            // Unbalanced — flush rest verbatim and stop.
            out.push_str(&text[abs_open..]);
            return out;
        };
        let close = after_open + close_rel;
        let inner = &text[after_open..close];

        // Split target / alias on the first '|'.
        let (target, alias) = match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a.trim())),
            None => (inner.trim(), None),
        };

        if is_image_target(target) {
            // Preserve folder path in the URL so the link resolves once
            // the user copies the `assets/` tree alongside the workspace.
            // Heading / block-ref suffixes are kept in the URL (they
            // may carry meaning, e.g. image-crop fragments) but
            // dropped from the caption so the alt text stays readable.
            let link_target = target.trim();
            let caption_target = link_target
                .split_once('#')
                .map(|(t, _)| t)
                .unwrap_or(link_target);
            let caption_target = caption_target
                .split_once('^')
                .map(|(t, _)| t)
                .unwrap_or(caption_target);
            let leaf = caption_target
                .rsplit('/')
                .next()
                .unwrap_or(caption_target)
                .trim();
            let caption = alias.unwrap_or(leaf);
            if embed {
                out.push('!'); // image embed: `![caption](target)`
            }
            out.push('[');
            out.push_str(caption);
            out.push_str("](");
            out.push_str(link_target);
            out.push(')');
        } else {
            // Non-image — re-emit verbatim (including the leading '!'
            // if we popped one) so the downstream wiki-link rewriter
            // sees the original token.
            if embed {
                out.push('!');
            }
            out.push_str("[[");
            out.push_str(inner);
            out.push_str("]]");
        }
        cursor = close + 2;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Rewrite Obsidian wiki-link variants to canonical `[[Note]]` form.
///
/// - `[[Note|alias]]` → `[[Note]]`
/// - `[[Note#heading]]` → `[[Note]]`
/// - `[[Note^block-id]]` → `[[Note]]`
/// - `[[folder/Note]]` → `[[Note]]`
/// - `[[Note]]` → unchanged
///
/// Embeds (`![[...]]`) are passed through unchanged — outl supports
/// both block embeds and image references in that shape, so the `!`
/// prefix is preserved by leaving the whole token alone.
fn rewrite_wikilinks(text: &str) -> String {
    if !text.contains("[[") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(open_rel) = text[cursor..].find("[[") {
        let abs_open = cursor + open_rel;
        out.push_str(&text[cursor..abs_open]);
        let after_open = abs_open + 2;
        if let Some(close_rel) = text[after_open..].find("]]") {
            let close = after_open + close_rel;
            let inner = &text[after_open..close];
            out.push_str("[[");
            out.push_str(&clean_wikilink_target(inner));
            out.push_str("]]");
            cursor = close + 2;
        } else {
            // Unbalanced — copy the rest verbatim and stop.
            out.push_str(&text[abs_open..]);
            return out;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

/// Strip alias / heading / block-ref markers and folder prefixes from
/// a wiki-link target. The `|` alias marker binds tightest (Obsidian
/// forbids `|` inside the target itself), then `#` heading, then `^`
/// block ref. Folder prefixes (`folder/Note`) collapse to the last
/// path segment because outl pages are flat.
fn clean_wikilink_target(inner: &str) -> String {
    let target = inner.split_once('|').map(|(t, _)| t).unwrap_or(inner);
    let target = target.split_once('#').map(|(t, _)| t).unwrap_or(target);
    let target = target.split_once('^').map(|(t, _)| t).unwrap_or(target);
    let target = target.rsplit_once('/').map(|(_, n)| n).unwrap_or(target);
    target.trim().to_string()
}

// --- title extraction ----------------------------------------------------

/// If `body` opens (after optional blank lines) with a single H1 line
/// (`# Heading`), return `(title, rest_of_body)` with the H1 line
/// stripped. Otherwise return `(None, body_unchanged)`. Only the very
/// first non-blank line is considered — a heading buried inside the
/// body stays as content.
fn extract_leading_h1(body: &str) -> (Option<String>, String) {
    let lines: Vec<&str> = body.lines().collect();
    let mut idx = 0;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() {
        return (None, body.to_string());
    }
    let trimmed = lines[idx].trim_start();
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return (None, body.to_string());
    };
    let title = rest.trim().to_string();
    if title.is_empty() {
        return (None, body.to_string());
    }
    let remaining = if lines.len() > idx + 1 {
        lines[idx + 1..].join("\n")
    } else {
        String::new()
    };
    (Some(title), remaining)
}

// --- body composition ----------------------------------------------------

/// Compose the final body that gets handed to `write_page_md_with_stem`.
///
/// `write_page_md_with_stem` prepends `title:: <title>\n\n` for
/// non-journals, so we never include `title` in `fm_props` here (it's
/// already filtered out by [`parse_frontmatter`]). Other properties
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Run an import against `vault`, returning the workspace TempDir
    /// (kept alive so the caller can read the produced files), the
    /// workspace [`Paths`], and the import report.
    fn run_import(vault: &Path) -> (TempDir, Paths, ImportReport) {
        let dst_dir = TempDir::new().unwrap();
        let dst = dst_dir.path().join("ws");
        crate::cmd::init::run(&dst).unwrap();
        let paths = Paths::at(dst);
        let report = import(vault, &paths).unwrap();
        (dst_dir, paths, report)
    }

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, content) in files {
            let p = dir.path().join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, content).unwrap();
        }
        dir
    }

    // --- pipeline-level tests -------------------------------------------

    #[test]
    fn basic_page_with_bullets_round_trips() {
        let vault = vault_with(&[("Project.md", "- goal A\n- goal B\n")]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 1);
        assert_eq!(report.journals, 0);

        let out = fs::read_to_string(paths.pages.join("project.md")).unwrap();
        assert!(
            out.starts_with("title:: Project\n"),
            "title missing:\n{out}"
        );
        assert!(out.contains("- goal A"));
        assert!(out.contains("- goal B"));
    }

    #[test]
    fn iso_filename_routes_to_journals() {
        let vault = vault_with(&[("2026-05-25.md", "- morning note\n")]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.journals, 1);
        assert_eq!(report.pages, 0);

        let p = paths.journals.join("2026-05-25.md");
        assert!(p.exists(), "journal file missing at {}", p.display());
        let out = fs::read_to_string(p).unwrap();
        // Journals don't get title:: — the filename is the date.
        assert!(!out.starts_with("title::"));
        assert!(out.contains("- morning note"));
    }

    #[test]
    fn iso_date_filename_routes_to_journals_regardless_of_folder() {
        // File lives in a `daily/` folder with `.obsidian/daily-notes.json`
        // pointing at it — but routing fires because of the ISO filename,
        // not the folder. We keep this test as a sanity check that the
        // common Obsidian setup imports cleanly.
        let vault = vault_with(&[
            (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
            ("daily/2026-06-01.md", "- standup\n"),
            ("pages/Project.md", "- work\n"),
        ]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.journals, 1);
        assert_eq!(report.pages, 1);
        assert!(paths.journals.join("2026-06-01.md").exists());
        assert!(paths.pages.join("project.md").exists());
    }

    #[test]
    fn non_date_file_in_daily_folder_stays_a_page() {
        // HIGH regression guard: a file inside the configured daily
        // notes folder whose filename isn't a date must stay a regular
        // page (with `path::` recording the origin), not be force
        // routed to journals/.
        let vault = vault_with(&[
            (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
            ("daily/sprint-kickoff.md", "- agenda\n"),
        ]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 1);
        assert_eq!(report.journals, 0);
        let out = fs::read_to_string(paths.pages.join("sprint-kickoff.md")).unwrap();
        assert!(out.contains("title:: sprint-kickoff"), "title lost:\n{out}");
        assert!(out.contains("path:: daily"), "path missing:\n{out}");
    }

    #[test]
    fn skips_obsidian_and_trash_dirs() {
        let vault = vault_with(&[
            (".obsidian/app.json", "{}"),
            (".obsidian/workspace.json", "{}"),
            (".trash/Deleted.md", "- should not import\n"),
            ("Note.md", "- keep\n"),
        ]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 1);
        // No file produced for the dotfile entries.
        assert!(!paths.pages.join("app.md").exists());
        assert!(!paths.pages.join("workspace.md").exists());
        assert!(!paths.pages.join("deleted.md").exists());
        assert!(paths.pages.join("note.md").exists());
    }

    #[test]
    fn nested_folder_emits_path_property() {
        let vault = vault_with(&[("projects/work/Q4.md", "- quarter plan\n")]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 1);

        let out = fs::read_to_string(paths.pages.join("q4.md")).unwrap();
        assert!(out.contains("path:: projects/work"), "path missing:\n{out}");
        assert!(out.contains("- quarter plan"));
    }

    #[test]
    fn vault_root_file_has_no_path_property() {
        let vault = vault_with(&[("Flat.md", "- hi\n")]);
        let (_hold, paths, _report) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("flat.md")).unwrap();
        assert!(!out.contains("path::"), "unexpected path:: in:\n{out}");
    }

    #[test]
    fn journal_in_daily_folder_has_no_path_property() {
        let vault = vault_with(&[
            (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
            ("daily/2026-06-01.md", "- standup\n"),
        ]);
        let (_hold, paths, _report) = run_import(vault.path());
        let out = fs::read_to_string(paths.journals.join("2026-06-01.md")).unwrap();
        assert!(!out.contains("path::"), "unexpected path:: in:\n{out}");
    }

    // --- wiki-link variants ---------------------------------------------

    #[test]
    fn wikilink_alias_is_stripped() {
        let vault = vault_with(&[("Note.md", "- see [[Target|the alias]] here\n")]);
        let (_hold, paths, _report) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[Target]]"), "alias not stripped:\n{out}");
        assert!(!out.contains("the alias"));
    }

    #[test]
    fn wikilink_heading_is_stripped() {
        let vault = vault_with(&[("Note.md", "- jump to [[Target#section]] now\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[Target]]"), "heading not stripped:\n{out}");
        assert!(!out.contains("section"));
    }

    #[test]
    fn wikilink_block_ref_is_stripped() {
        let vault = vault_with(&[("Note.md", "- see [[Target^abc123]]\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[Target]]"), "block ref not stripped:\n{out}");
        assert!(!out.contains("abc123"));
    }

    #[test]
    fn wikilink_folder_prefix_is_stripped() {
        let vault = vault_with(&[("Note.md", "- link [[folder/sub/Target]] now\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(
            out.contains("[[Target]]"),
            "folder prefix not stripped:\n{out}"
        );
    }

    #[test]
    fn wikilink_combined_variants_collapse_to_target() {
        // [[folder/Note|alias#section]] → [[Note]]
        let vault = vault_with(&[("Note.md", "- x [[folder/Note|alias#section]] y\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[Note]]"), "combined not collapsed:\n{out}");
    }

    #[test]
    fn note_embeds_are_preserved_image_embeds_become_md_links() {
        // Outl supports `![[note]]` block-note embeds natively, so a
        // non-image embed round-trips unchanged. Image attachments
        // (`![[foo.jpeg]]`) are converted to standard CommonMark image
        // syntax because outl has no image-as-page.
        let vault = vault_with(&[("Note.md", "- see ![[other-note]] and ![[image.png]]\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("![[other-note]]"), "note embed lost:\n{out}");
        assert!(
            out.contains("![image.png](image.png)"),
            "image embed not converted:\n{out}"
        );
    }

    #[test]
    fn image_wiki_link_with_alias_becomes_md_link() {
        let vault = vault_with(&[(
            "Note.md",
            "- [[assets/foo/bar.jpeg|Open: pasted.png]] here\n",
        )]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(
            out.contains("[Open: pasted.png](assets/foo/bar.jpeg)"),
            "image wiki-link not converted:\n{out}"
        );
        assert!(
            !out.contains("[[assets/foo/bar.jpeg"),
            "original token should be gone:\n{out}"
        );
    }

    #[test]
    fn image_wiki_link_without_alias_uses_leaf_name_as_caption() {
        let vault = vault_with(&[("Note.md", "- see [[folder/deep/photo.jpeg]] now\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        // Folder path preserved so the link stays resolvable; caption
        // falls back to the leaf filename.
        assert!(
            out.contains("[photo.jpeg](folder/deep/photo.jpeg)"),
            "image link without alias wrong:\n{out}"
        );
    }

    #[test]
    fn note_wiki_link_is_not_mistaken_for_image() {
        // A note whose name happens to contain a dot but isn't an
        // image extension stays a wiki-link.
        let vault = vault_with(&[("Note.md", "- [[Spec.v3]] and [[image-notes]]\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("[[Spec.v3]]"), "note link broken:\n{out}");
        assert!(out.contains("[[image-notes]]"), "note link broken:\n{out}");
    }

    #[test]
    fn image_target_with_heading_suffix_is_still_recognised() {
        // Obsidian allows `#crop` and `^block` suffixes on image links.
        // The extension check must look past them.
        let vault = vault_with(&[(
            "Note.md",
            "- ![[image.png#crop]] and [[photo.jpeg^meta|cap]]\n",
        )]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        // Embed form keeps the `#crop` as a URL fragment.
        assert!(
            out.contains("![image.png](image.png#crop)"),
            "image embed with # suffix wrong:\n{out}"
        );
        assert!(
            out.contains("[cap](photo.jpeg^meta)"),
            "image link with ^ suffix wrong:\n{out}"
        );
    }

    // --- slug collision disambiguation ----------------------------------

    #[test]
    fn colliding_titles_get_path_derived_suffix() {
        // Two source files with the same H1 "Ideas" but different
        // folders. The lex-smallest relative path wins the bare slug;
        // the other gets a folder-derived suffix.
        //   "Docs/HL Game Design/Ideas.md"  (H...)
        //   "Docs/Ideas/Ideas.md"           (I...)
        // 'H' < 'I' lexicographically, so HL Game Design wins the bare
        // slug and Docs/Ideas gets the suffix.
        let vault = vault_with(&[
            ("Docs/Ideas/Ideas.md", "# Ideas\n- a\n"),
            ("Docs/HL Game Design/Ideas.md", "# Ideas\n- b\n"),
        ]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 2);

        let winner = fs::read_to_string(paths.pages.join("ideas.md")).unwrap();
        assert!(
            winner.contains("- b"),
            "wrong winner content (expected HL Game Design's '- b'):\n{winner}"
        );
        let suffixed = fs::read_to_string(paths.pages.join("ideas-ideas.md"))
            .expect("expected suffixed file `ideas-ideas.md`");
        assert!(suffixed.contains("- a"));
        // title:: is unaffected by the disambiguation.
        assert!(suffixed.contains("title:: Ideas"));
    }

    #[test]
    fn same_folder_collision_uses_folder_suffix() {
        // Two files in the same folder with the same H1. The folder
        // suffix alone produces a unique stem (because the winner has
        // no suffix), so the disambiguator stops there. Filename-stem
        // is only tried if folder-suffix also collides.
        let vault = vault_with(&[
            ("Docs/Prompt A.md", "# Same Title\n- a\n"),
            ("Docs/Prompt B.md", "# Same Title\n- b\n"),
        ]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 2);

        // "Docs/Prompt A.md" < "Docs/Prompt B.md" lex, so A wins.
        let winner = fs::read_to_string(paths.pages.join("same-title.md")).unwrap();
        assert!(winner.contains("- a"), "wrong winner:\n{winner}");
        let suffixed = fs::read_to_string(paths.pages.join("same-title-docs.md"))
            .expect("expected `same-title-docs.md`");
        assert!(suffixed.contains("- b"));
    }

    // --- frontmatter -----------------------------------------------------

    #[test]
    fn frontmatter_title_and_tags_become_properties() {
        let vault = vault_with(&[(
            "Note.md",
            "---\ntitle: Real Title\ntags: [foo, bar]\n---\n- body bullet\n",
        )]);
        let (_hold, paths, report) = run_import(vault.path());
        assert_eq!(report.pages, 1);

        // Filename is `Note.md` but frontmatter title is `Real Title`,
        // so the slug comes from `Real Title`.
        let out = fs::read_to_string(paths.pages.join("real-title.md")).unwrap();
        assert!(out.contains("title:: Real Title"), "title wrong:\n{out}");
        assert!(out.contains("tags:: #foo #bar"), "tags wrong:\n{out}");
        assert!(out.contains("- body bullet"));
    }

    #[test]
    fn frontmatter_tags_block_list_form() {
        let vault = vault_with(&[("Note.md", "---\ntags:\n  - alpha\n  - beta\n---\n- x\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(
            out.contains("tags:: #alpha #beta"),
            "tags block wrong:\n{out}"
        );
    }

    #[test]
    fn frontmatter_unknown_scalar_keys_pass_through() {
        let vault = vault_with(&[("Note.md", "---\nauthor: jane\nrating: 7\n---\n- x\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("author:: jane"), "missing author prop:\n{out}");
        assert!(out.contains("rating:: 7"), "missing rating prop:\n{out}");
    }

    #[test]
    fn frontmatter_dropped_keys_are_counted() {
        let vault = vault_with(&[(
            "Note.md",
            "---\naliases: [foo, bar]\ncssclass: wide\npublish: false\n---\n- x\n",
        )]);
        let (_hold, _paths, report) = run_import(vault.path());
        assert!(
            report.artifacts_stripped >= 3,
            "dropped frontmatter keys not counted: {:?}",
            report
        );
    }

    #[test]
    fn frontmatter_date_is_normalized() {
        let vault = vault_with(&[("Note.md", "---\ndate: 2026/04/22\n---\n- x\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(
            out.contains("date:: 2026-04-22"),
            "date not normalized:\n{out}"
        );
    }

    #[test]
    fn frontmatter_no_closing_fence_passes_through() {
        // Malformed frontmatter should not eat the file.
        let vault = vault_with(&[("Note.md", "---\ntitle: half\n- bullet\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
        assert!(out.contains("- bullet"), "body lost on bad fm:\n{out}");
    }

    // --- title fallbacks -------------------------------------------------

    #[test]
    fn leading_h1_becomes_title_and_is_stripped_from_body() {
        let vault = vault_with(&[("Note.md", "# Real Heading\n- under h1\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        // Slug derived from H1, not from filename.
        let out = fs::read_to_string(paths.pages.join("real-heading.md")).unwrap();
        assert!(out.contains("title:: Real Heading"), "title wrong:\n{out}");
        // H1 line itself is gone.
        assert!(!out.contains("# Real Heading"));
        assert!(out.contains("- under h1"));
    }

    #[test]
    fn frontmatter_title_beats_h1() {
        let vault = vault_with(&[("Note.md", "---\ntitle: FM Title\n---\n# H1 Title\n- body\n")]);
        let (_hold, paths, _) = run_import(vault.path());
        let out = fs::read_to_string(paths.pages.join("fm-title.md")).unwrap();
        assert!(out.contains("title:: FM Title"));
        // H1 stays in body since title came from frontmatter.
        assert!(out.contains("# H1 Title"));
    }

    // --- idempotency -----------------------------------------------------

    #[test]
    fn reimport_produces_same_files() {
        let vault = vault_with(&[
            ("Note.md", "- x\n- y\n"),
            ("projects/Sub.md", "- nested\n"),
            ("2026-05-25.md", "- journal\n"),
        ]);

        let dst1 = TempDir::new().unwrap();
        let dst1_path = dst1.path().join("ws");
        crate::cmd::init::run(&dst1_path).unwrap();
        let paths1 = Paths::at(dst1_path);
        import(vault.path(), &paths1).unwrap();

        let dst2 = TempDir::new().unwrap();
        let dst2_path = dst2.path().join("ws");
        crate::cmd::init::run(&dst2_path).unwrap();
        let paths2 = Paths::at(dst2_path);
        import(vault.path(), &paths2).unwrap();

        for name in &["note.md", "sub.md"] {
            let a = fs::read_to_string(paths1.pages.join(name)).unwrap();
            let b = fs::read_to_string(paths2.pages.join(name)).unwrap();
            assert_eq!(a, b, "non-idempotent page {name}:\nA:\n{a}\nB:\n{b}");
        }
        let a = fs::read_to_string(paths1.journals.join("2026-05-25.md")).unwrap();
        let b = fs::read_to_string(paths2.journals.join("2026-05-25.md")).unwrap();
        assert_eq!(a, b, "non-idempotent journal:\nA:\n{a}\nB:\n{b}");
    }

    #[test]
    fn reimport_into_same_destination_is_idempotent() {
        // Same-destination re-import is the more failure-prone case
        // (overwrite semantics, stale sidecars, path collisions).
        let vault = vault_with(&[
            ("Note.md", "- a\n"),
            ("projects/Sub.md", "- b\n"),
            ("2026-05-25.md", "- j\n"),
        ]);

        let dst = TempDir::new().unwrap();
        let dst_path = dst.path().join("ws");
        crate::cmd::init::run(&dst_path).unwrap();
        let paths = Paths::at(dst_path.clone());

        import(vault.path(), &paths).unwrap();
        let snap: Vec<(String, String)> = walkdir::WalkDir::new(&dst_path)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .map(|e| {
                (
                    e.path()
                        .strip_prefix(&dst_path)
                        .unwrap()
                        .display()
                        .to_string(),
                    fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();

        import(vault.path(), &paths).unwrap();
        for (rel, before) in &snap {
            let after = fs::read_to_string(dst_path.join(rel)).unwrap();
            assert_eq!(before, &after, "non-idempotent re-import of {rel}");
        }
    }
}
