//! Ingest an existing `.md` file as a real page.
//!
//! The bare [`reconcile_md`](outl_md::reconcile::reconcile_md) primitive
//! creates the *blocks* of a file but never the page node itself: it has
//! no notion of `page-slug` / `page-kind`, so the blocks hang off a
//! `page_id` that is never linked under root and `page::list_all` never
//! sees them. Every bulk-ingest path (Logseq/Roam import, `serve`
//! initial scan, the mobile/TUI orphan scanners) needs the *page* to
//! materialise, not just its blocks.
//!
//! [`ingest_md_file`] is the shared fix: it derives the slug from the
//! filename and the kind from the directory, creates the page node via
//! [`open_or_create`] (deterministic id, `page-slug` + `page-kind`),
//! then reconciles the file's blocks under that exact node with
//! [`reconcile_md_with_page_id`].

use std::collections::HashSet;
use std::path::Path;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::parse;
use outl_md::reconcile::{reconcile_md_with_page_id, ReconcileReport};
use outl_md::slug::slugify;

use crate::backlinks::extract_refs;
use crate::error::ActionError;
use crate::page::{
    date_from_slug, find_by_slug, is_valid_slug, open_or_create, page_id_from_slug, PageKind,
};
use crate::tree::children_of;

/// Ingest a single `.md` file as a page (or journal), creating the page
/// node if it doesn't exist yet and reconciling its blocks underneath.
///
/// - The **slug** is the file stem (`pages/foo.md` → `foo`,
///   `journals/2026-05-24.md` → `2026-05-24`).
/// - The **kind** is [`PageKind::Journal`] when the file sits directly
///   under a `journals/` directory, [`PageKind::Page`] otherwise.
/// - The **title** is the file's `title::` page property, falling back
///   to the slug when absent.
///
/// Idempotent: [`open_or_create`] returns the existing node when the
/// page already exists, and the deterministic `page_id` keeps reconcile
/// attaching blocks to the same node across runs and across devices.
///
/// `orphan_log` receives one line per orphan id surfaced during
/// matching; pass `None` to suppress it.
pub fn ingest_md_file(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    orphan_log: Option<&Path>,
) -> Result<ReconcileReport, ActionError> {
    let slug = slug_from_path(md_path)
        .ok_or_else(|| ActionError::InvalidSlug(md_path.display().to_string()))?;
    let kind = kind_from_path(md_path);
    let title = read_title(md_path).unwrap_or_else(|| slug.clone());

    // Create (or find) the page node under root with page-slug / page-kind.
    open_or_create(workspace, hlc, &slug, &title, kind)?;

    // Reconcile blocks under that exact node. The id must match the one
    // open_or_create used, hence page_id_from_slug rather than letting
    // reconcile mint a fresh random id when no sidecar exists yet.
    let page_id = page_id_from_slug(&slug);
    let report = reconcile_md_with_page_id(workspace, hlc, md_path, page_id, orphan_log)?;
    Ok(report)
}

/// File stem as the page slug, or `None` for a stem that isn't valid
/// UTF-8 or is empty.
fn slug_from_path(md_path: &Path) -> Option<String> {
    md_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// `journals/` parent → journal; anything else → page.
fn kind_from_path(md_path: &Path) -> PageKind {
    match md_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
    {
        Some("journals") => PageKind::Journal,
        _ => PageKind::Page,
    }
}

/// Read the `title::` page property from the file, if present and
/// non-empty.
fn read_title(md_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(md_path).ok()?;
    parse(&text)
        .properties
        .into_iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v)
        .filter(|t| !t.is_empty())
}

/// Ingest every `.md` directly inside `dir` (non-recursive), returning
/// one `(path, result)` per file so callers can log per-file failures
/// without aborting the whole sweep. Missing / unreadable directories
/// yield an empty vector.
pub fn ingest_dir(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    dir: &Path,
    orphan_log: Option<&Path>,
) -> Vec<(std::path::PathBuf, Result<ReconcileReport, ActionError>)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.starts_with('.'))
        })
        .collect();
    paths.sort();
    for path in paths {
        let res = ingest_md_file(workspace, hlc, &path, orphan_log);
        out.push((path, res));
    }
    out
}

/// Create a stub page for every `[[ref]]` that doesn't resolve to an
/// existing page — the Logseq "implicit page" model.
///
/// In Logseq a reference like `[[Acme]]` or `[[@Jane Doe]]`
/// brings a page into existence even if you never gave it its own file:
/// you can open it and see every block that mentions it. A plain
/// `.md` import only creates pages for files, so those references land
/// nowhere and their backlinks can't be listed. This walks every
/// block, collects the `[[ref]]` targets, and creates a stub page for
/// each one still missing.
///
/// The stub's **title is the reference text verbatim** (`@Jane Doe`,
/// `Acme`), and its slug is `slugify(ref)`. Backlinks match on slug
/// **and** title, so `[[@Jane Doe]]` then resolves to the stub even
/// though the slug is `jane-doe`. Date-shaped refs (`[[2026-05-30]]`)
/// become journals so they merge with any real journal of that day.
///
/// Returns the number of stubs created. Idempotent: a second run finds
/// every page already present and creates nothing.
pub fn create_missing_ref_pages(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<usize, ActionError> {
    // Collect every distinct ref target first; mutating the tree while
    // walking it would be a borrow hazard.
    let mut targets: Vec<String> = Vec::new();
    let mut seen_text: HashSet<String> = HashSet::new();
    collect_refs(workspace, NodeId::root(), &mut |text| {
        for r in extract_refs(text) {
            if seen_text.insert(r.clone()) {
                targets.push(r);
            }
        }
    });

    let mut created_ids: Vec<NodeId> = Vec::new();
    let mut seen_slug: HashSet<String> = HashSet::new();
    for target in targets {
        let slug = slugify(&target);
        if !is_valid_slug(&slug) {
            continue;
        }
        if !seen_slug.insert(slug.clone()) {
            continue;
        }
        if find_by_slug(workspace, &slug).is_some() {
            continue;
        }
        let kind = if date_from_slug(&slug).is_some() {
            PageKind::Journal
        } else {
            PageKind::Page
        };
        let id = open_or_create(workspace, hlc, &slug, &target, kind)?;
        created_ids.push(id);
    }

    // Project each new stub to its `.md` + sidecar. The op log is the
    // source of truth (mobile/CLI list pages from it), but the TUI
    // builds its page list by scanning `pages/`/`journals/` on disk —
    // without a projection the implicit page is invisible there, and
    // it has no `.md` for the TUI to open/edit. Skip when the workspace
    // isn't disk-backed (in-memory tests).
    if let Some(root) = workspace.root.clone() {
        for id in &created_ids {
            let Some(meta) = crate::page::page_meta(workspace, *id) else {
                continue;
            };
            // The stub has no blocks yet, so `render_page_md` would emit
            // an empty file and the TUI would label the page by slug
            // (`jane-doe`) instead of its title (`@Jane Doe`). Write a
            // `title::` header for pages; journals are
            // labelled by their date filename, so an empty file is fine.
            let md = if meta.kind == PageKind::Journal {
                String::new()
            } else {
                format!("title:: {}\n", meta.title)
            };
            let path = crate::journal::page_md_path(&root, &meta);
            crate::journal::write_md_atomic(&path, &md)?;
            let sidecar =
                outl_md::sidecar::Sidecar::new_for_page(*id, &outl_md::sidecar::file_hash(&md));
            outl_md::sidecar::write(&outl_md::sidecar::sidecar_path_for(&path), &sidecar)?;
        }
    }

    Ok(created_ids.len())
}

/// Walk the whole tree, invoking `f` with each block's text.
fn collect_refs<F: FnMut(&str)>(workspace: &Workspace, parent: NodeId, f: &mut F) {
    for (id, _) in children_of(workspace, parent) {
        if let Some(text) = workspace.block_text(id) {
            f(&text);
        }
        collect_refs(workspace, id, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::list_all;
    use outl_core::id::ActorId;
    use std::fs;
    use tempfile::TempDir;

    fn workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        (ws, hlc)
    }

    #[test]
    fn ingest_creates_a_visible_page_with_blocks() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        let md = pages.join("my-note.md");
        fs::write(&md, "title:: My Note\n\n- alpha\n- beta\n").unwrap();

        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        let pages_listed = list_all(&ws);
        assert_eq!(pages_listed.len(), 1);
        assert_eq!(pages_listed[0].slug, "my-note");
        assert_eq!(pages_listed[0].title, "My Note");
        assert_eq!(pages_listed[0].kind, PageKind::Page);
    }

    #[test]
    fn page_id_is_deterministic_so_reconcile_attaches_to_the_page_node() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        let md = pages.join("foo.md");
        fs::write(&md, "title:: Foo\n\n- one\n").unwrap();

        let page = open_or_create(&mut ws, &hlc, "foo", "Foo", PageKind::Page).unwrap();
        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        // The block created by reconcile must be a descendant of the page
        // node open_or_create made — i.e. the page now has children.
        assert_eq!(page, page_id_from_slug("foo"));
        let kids = crate::tree::children_of(&ws, page);
        assert!(!kids.is_empty(), "blocks should attach under the page node");
        // Still exactly one page, not a duplicate phantom.
        assert_eq!(list_all(&ws).len(), 1);
    }

    #[test]
    fn journal_dir_yields_journal_kind() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let journals = dir.path().join("journals");
        fs::create_dir_all(&journals).unwrap();
        let md = journals.join("2026-05-24.md");
        fs::write(&md, "- woke up\n").unwrap();

        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        let listed = list_all(&ws);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "2026-05-24");
        assert_eq!(listed[0].kind, PageKind::Journal);
    }

    #[test]
    fn ingest_is_idempotent() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        let md = pages.join("bar.md");
        fs::write(&md, "title:: Bar\n\n- x\n- y\n").unwrap();

        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();
        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        assert_eq!(list_all(&ws).len(), 1);
    }

    #[test]
    fn implicit_pages_created_for_unresolved_refs() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        // A page mentioning two entities with no file of their own.
        let md = pages.join("note.md");
        fs::write(
            &md,
            "title:: Note\n\n- talked to [[@Jane Doe]] about [[Acme]]\n",
        )
        .unwrap();
        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        let created = create_missing_ref_pages(&mut ws, &hlc).unwrap();
        assert_eq!(created, 2, "two implicit pages: @Jane Doe and Acme");

        // The stubs exist with the reference text as their title, so
        // backlinks resolve against the title form.
        let acme = find_by_slug(&ws, "acme").expect("acme stub exists");
        let meta = crate::page::page_meta(&ws, acme).unwrap();
        assert_eq!(meta.title, "Acme");
        let links = crate::backlinks::backlinks_for_page(&ws, &meta);
        assert_eq!(links.len(), 1);

        let jane = find_by_slug(&ws, "jane-doe").expect("jane-doe stub exists");
        let jmeta = crate::page::page_meta(&ws, jane).unwrap();
        assert_eq!(jmeta.title, "@Jane Doe");

        // Idempotent: a second pass creates nothing.
        assert_eq!(create_missing_ref_pages(&mut ws, &hlc).unwrap(), 0);
    }

    #[test]
    fn implicit_date_ref_becomes_a_journal() {
        let (mut ws, hlc) = workspace();
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        let md = pages.join("plan.md");
        fs::write(&md, "title:: Plan\n\n- follow up [[2026-05-30]]\n").unwrap();
        ingest_md_file(&mut ws, &hlc, &md, None).unwrap();

        create_missing_ref_pages(&mut ws, &hlc).unwrap();
        let day = find_by_slug(&ws, "2026-05-30").expect("journal stub exists");
        assert_eq!(
            crate::page::page_meta(&ws, day).unwrap().kind,
            PageKind::Journal
        );
    }
}
