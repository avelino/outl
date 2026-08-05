//! Re-import safety: is this destination safe to write a graph into?
//!
//! Two questions, answered separately because the evidence differs.
//!
//! 1. **Is there content here already?** Measured against the op log's
//!    materialized tree, never against the `.md` files. A workspace
//!    paired over iroh holds every op but projects a page's file only
//!    when that page is opened, so counting files sees an empty folder
//!    on a device that holds the whole graph.
//! 2. **Is that content the user's, or the wreckage of an import that
//!    died halfway?** The second is recorded by [`IMPORT_MARKER`] and
//!    must not demand `--force`, or the only way to recover from a
//!    failed import is the flag that destroys.
//!
//! Both guards are no-ops under `--force`, and `--dry-run` never
//! reaches them.

use anyhow::Result;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use std::path::{Path, PathBuf};

/// Marker `outl import` drops in `.outl/` for the duration of a real
/// import.
///
/// The pipeline writes page by page, so a failure at page 40k of 66k
/// leaves a half-populated destination and no way back in: the guard
/// below (correctly) sees content, and the only override is `--force`,
/// the flag that destroys. The marker records that **everything** in
/// the destination came from an import that never finished, which makes
/// a plain re-run safe and lets the guard step aside — no `--force`, no
/// dead end.
const IMPORT_MARKER: &str = "import-in-progress.json";

/// A previous import that never finished, read back from the marker.
pub(super) struct UnfinishedImport {
    adapter: String,
    source: String,
    started: String,
}

/// Path of the in-progress marker inside a workspace's `.outl/`.
fn marker_path(dot_outl: &Path) -> PathBuf {
    dot_outl.join(IMPORT_MARKER)
}

/// Read the marker, if a previous import left one behind. Any unreadable
/// or malformed marker reads as "no unfinished import" — the guard then
/// stays in its safe (refusing) mode rather than waving a run through on
/// a corrupt file.
pub(super) fn read_marker(dot_outl: &Path) -> Option<UnfinishedImport> {
    let raw = std::fs::read_to_string(marker_path(dot_outl)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let field = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    };
    Some(UnfinishedImport {
        adapter: field("adapter"),
        source: field("source"),
        started: field("started"),
    })
}

/// Drop the marker before the first page is written. Best-effort: a
/// workspace we can't write a marker into is one the import is about to
/// fail on anyway, and losing the marker only costs the user a `--force`
/// on the retry — never data.
pub(super) fn write_marker(dot_outl: &Path, adapter: &str, src: &Path) {
    let body = serde_json::json!({
        "adapter": adapter,
        "source": src.display().to_string(),
        "started": chrono::Local::now().to_rfc3339(),
    });
    if let Err(e) = std::fs::write(marker_path(dot_outl), body.to_string()) {
        tracing::warn!("could not write the import marker: {e}");
    }
}

/// Clear the marker once the import completed. From here on the
/// destination holds finished content and the guard protects it again.
pub(super) fn clear_marker(dot_outl: &Path) {
    if let Err(e) = std::fs::remove_file(marker_path(dot_outl)) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("could not clear the import marker: {e}");
        }
    }
}

/// What the destination already holds.
///
/// Measured against the **op log's materialized tree**, not the `.md`
/// files. A workspace paired over iroh receives every op through sync
/// but only projects a page's `.md` when that page is opened, so a fresh
/// laptop can hold a complete 66k-block graph with zero files on disk. A
/// file-counting guard sees `0` there and waves the import through —
/// and because `reconcile_md` derives the page root from the slug
/// (`NodeId::from_slug`, the *same* deterministic id as the page already
/// in the tree) with no sidecar to match against, the imported blocks
/// land as children **under the already-populated root**. That fusion is
/// in the op log, replicated to every peer, with no undo.
#[derive(Debug)]
struct Occupancy {
    /// Pages in the tree that aren't templates.
    pages: usize,
    /// Slug of the first page found carrying real text. `Some` is the
    /// signal that the destination holds content.
    first_with_content: Option<String>,
    /// `.md` files under `pages/` + `journals/` with no sidecar next to
    /// them — dropped in by hand, never reconciled, and therefore
    /// invisible to the tree. An import would overwrite them.
    unreconciled_md: usize,
}

impl Occupancy {
    /// Whether the destination is untouched as far as this guard cares.
    fn is_vacant(&self) -> bool {
        self.first_with_content.is_none() && self.unreconciled_md == 0
    }
}

/// Inspect the destination workspace.
///
/// A page counts as content when it holds at least one block with
/// non-blank text. That is deliberately **not** "the workspace has
/// pages": `outl init` seeds a `templates/journal` template page and
/// today's journal (one empty bullet, instantiated from that template),
/// so the documented `outl init ./notes && outl import roam … ./notes`
/// flow would otherwise trip the guard every single time and teach the
/// user that `--force` is routine. `--force` is the flag that destroys;
/// it must stay rare enough to make someone stop and read.
///
/// Template pages are skipped outright — they're scaffolding, and an
/// import only touches one if the source happens to carry that slug.
///
/// The text scan short-circuits on the first block it finds, so a
/// freshly-initialized workspace costs two `block_text` reads and a
/// populated one stops immediately. Nothing here forces the O(all
/// blocks) materialization pass that boot defers.
fn occupancy(ws: &Workspace, root: &Path) -> Occupancy {
    let page_roots: Vec<NodeId> = ws
        .tree()
        .iter_nodes()
        .filter(|(_, parent, _)| *parent == NodeId::root())
        .map(|(id, _, _)| id)
        .filter(|id| outl_actions::page_meta(ws, *id).is_some())
        .filter(|id| {
            ws.tree()
                .property(*id, outl_actions::TEMPLATE_KEY)
                .is_none()
        })
        .collect();

    let mut first_with_content = None;
    for page in &page_roots {
        if first_with_content.is_some() {
            break;
        }
        outl_actions::walk_subtree(ws, *page, |block| {
            let has_text = ws.block_text(block).is_some_and(|t| !t.trim().is_empty());
            if has_text {
                first_with_content = outl_actions::page_meta(ws, *page).map(|m| m.slug);
                return false;
            }
            true
        });
    }

    Occupancy {
        pages: page_roots.len(),
        first_with_content,
        unreconciled_md: unreconciled_md(root),
    }
}

/// Count `.md` files under `pages/` + `journals/` (recursively — page
/// slugs nest) that have no sidecar beside them.
///
/// Every file outl itself writes is projected together with its sidecar,
/// so a lone `.md` was put there by hand and has never been through
/// `reconcile_md`. It holds content the tree cannot see, and the import
/// would overwrite it.
fn unreconciled_md(root: &Path) -> usize {
    ["pages", "journals"]
        .iter()
        .map(|sub| {
            walkdir::WalkDir::new(root.join(sub))
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path().extension().and_then(|x| x.to_str()) == Some("md")
                        && !outl_md::sidecar::sidecar_path_for(e.path()).exists()
                })
                .count()
        })
        .sum()
}

/// Pre-open guard: the destination exists but is **not** an outl
/// workspace, so the only evidence available is what's on disk.
///
/// Once `.outl/` exists the op log is the authority and
/// [`guard_workspace`] takes over — counting `.md` files there would
/// both miss a synced-but-unprojected graph and false-positive on the
/// two files `outl init` projects.
pub(super) fn guard_foreign_destination(dst: &Path, force: bool) -> Result<()> {
    if force || !dst.exists() || dst.join(".outl").exists() {
        return Ok(());
    }
    let existing = unreconciled_md(dst);
    if existing == 0 {
        return Ok(());
    }
    anyhow::bail!(
        "{} is not an outl workspace but already holds {existing} markdown file(s) under \
         `pages/` / `journals/`.\n\
         An import overwrites every file it emits, with no undo.\n\
         Import into a fresh directory instead, or re-run with `--force` if overwriting is \
         exactly what you want. `--dry-run` is always safe.",
        dst.display()
    )
}

/// Refuse to import into a workspace that already holds content.
///
/// An import overwrites every `.md` it emits and then reconciles them,
/// so a second run against a live workspace doesn't just duplicate
/// content — it **erases** whatever the user wrote in outl since the
/// first import (the op log replays the overwritten file as an edit),
/// and it fuses the imported blocks under page roots that already hold
/// synced content. There is no undo at the op-log level, so the
/// destructive path is opt-in: `--force`. `--dry-run` never reaches this
/// guard.
///
/// An `unfinished` import (see [`IMPORT_MARKER`]) waves the run through:
/// everything in the destination came from a run that never completed,
/// so there is nothing of the user's to protect and demanding `--force`
/// would only train them to reach for it.
pub(super) fn guard_workspace(
    dst: &Path,
    ws: &Workspace,
    force: bool,
    unfinished: Option<&UnfinishedImport>,
) -> Result<()> {
    if force {
        return Ok(());
    }
    if let Some(prev) = unfinished {
        eprintln!(
            "note: {} holds the leftovers of a {} import from {} started {} that never finished.\n\
             Importing again over it (no `--force` needed — nothing here is yours to lose).",
            dst.display(),
            prev.adapter,
            prev.source,
            prev.started
        );
        return Ok(());
    }
    let occ = occupancy(ws, dst);
    if occ.is_vacant() {
        return Ok(());
    }
    let what = match (&occ.first_with_content, occ.unreconciled_md) {
        (Some(slug), 0) => format!(
            "{} page(s), starting with `{slug}` which holds content",
            occ.pages
        ),
        (Some(slug), n) => format!(
            "{} page(s), starting with `{slug}` which holds content, plus {n} markdown file(s) \
             that were never reconciled",
            occ.pages
        ),
        (None, n) => format!("{n} markdown file(s) that were never reconciled"),
    };
    anyhow::bail!(
        "{} already holds {what}.\n\
         Importing again overwrites the pages it emits and reconciles the result through the \
         op log — anything you wrote in outl, or received from a paired device, would be \
         merged into the imported tree with no undo.\n\
         This count comes from the op log, not from the `.md` files on disk: a device that \
         synced but hasn't opened its pages yet holds the whole graph with nothing projected.\n\
         Import into a fresh directory instead, or re-run with `--force` if overwriting is \
         exactly what you want. `--dry-run` is always safe.",
        dst.display()
    )
}

/// What to tell the user when the pipeline dies mid-import.
pub(super) fn recovery_hint(dst: &Path, dot_outl: &Path) -> String {
    format!(
        "the import stopped partway and {} is left half-populated.\n\
         `{}` records that, so re-running the exact same command imports again without \
         `--force` — the guard recognises the marker.\n\
         To start clean instead, delete {} and import into a fresh directory.",
        dst.display(),
        marker_path(dot_outl).display(),
        dst.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::import::{run, ImportFlags};

    /// Build a destination holding `rel` page files.
    fn dst_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for rel in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(&p, "- hi\n").expect("write");
        }
        dir
    }

    /// A workspace at `<tmp>/notes`, exactly as `outl init` leaves it.
    fn init_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("notes");
        crate::cmd::init::run(&root, "global").expect("init");
        (dir, root)
    }

    /// A roam backup holding one page with one block.
    fn roam_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("backup.json");
        std::fs::write(
            &src,
            r#"[{"title": "A", "children": [{"string": "x", "uid": "a1", "children": []}]}]"#,
        )
        .expect("write fixture");
        (dir, src)
    }

    #[test]
    fn guard_allows_a_missing_or_empty_destination() {
        let empty = dst_with(&["pages/.keep"]);
        assert!(guard_foreign_destination(&empty.path().join("nope"), false).is_ok());
        assert!(guard_foreign_destination(empty.path(), false).is_ok());
    }

    /// A directory that isn't an outl workspace has no op log to consult,
    /// so the `.md` files on disk are the only evidence — and an import
    /// would overwrite every one of them.
    #[test]
    fn guard_refuses_a_non_workspace_directory_holding_markdown() {
        let dst = dst_with(&[
            "pages/notes.md",
            "pages/work/deep.md",
            "journals/2026-05-25.md",
        ]);
        let err = guard_foreign_destination(dst.path(), false).expect_err("must refuse");
        let msg = err.to_string();
        assert!(msg.contains("3 markdown file(s)"), "counts files: {msg}");
        assert!(msg.contains("--force"), "names the escape hatch: {msg}");
        assert!(guard_foreign_destination(dst.path(), true).is_ok());
    }

    /// The documented migration flow is `outl init ./notes && outl import
    /// roam backup.json ./notes`. `init` seeds a journal template page
    /// and today's (empty) journal, so a guard that counts pages or `.md`
    /// files refuses here — every time — and teaches the user that
    /// `--force`, the flag that destroys, is routine.
    #[test]
    fn guard_allows_a_freshly_initialized_workspace() {
        let (_tmp, root) = init_workspace();
        let ctx = crate::ws::open(&root).expect("open");
        let occ = occupancy(&ctx.workspace, &root);
        assert!(
            occ.is_vacant(),
            "`outl init` output is not user content: {occ:?}"
        );
        assert!(guard_workspace(&root, &ctx.workspace, false, None).is_ok());
    }

    /// The one that costs the graph: a laptop paired over iroh has every
    /// op but projects a page's `.md` only when that page is opened. A
    /// file-counting guard sees an empty directory and waves the import
    /// through, which fuses 66k imported blocks under page roots that
    /// already hold the synced content — in the op log, on every peer,
    /// with no undo.
    #[test]
    fn guard_refuses_a_workspace_that_exists_only_in_the_op_log() {
        let (_tmp, root) = init_workspace();
        {
            let mut ctx = crate::ws::open(&root).expect("open");
            let page = outl_actions::open_or_create_by_name(
                &mut ctx.workspace,
                &ctx.hlc,
                "meeting-notes",
                outl_actions::PageKind::Page,
            )
            .expect("create page");
            outl_actions::append_block(
                &mut ctx.workspace,
                &ctx.hlc,
                Some(page),
                Some("decision we cannot lose"),
            )
            .expect("append");
        }
        // Nothing was ever projected: no `.md`, no sidecar, ops only.
        std::fs::remove_dir_all(root.join("pages")).expect("drop pages/");
        std::fs::remove_dir_all(root.join("journals")).expect("drop journals/");
        assert_eq!(unreconciled_md(&root), 0, "no files left to count");

        let ctx = crate::ws::open(&root).expect("reopen");
        let err = guard_workspace(&root, &ctx.workspace, false, None)
            .expect_err("must refuse the import");
        let msg = err.to_string();
        assert!(msg.contains("meeting-notes"), "names the page: {msg}");
        assert!(msg.contains("op log"), "explains the evidence: {msg}");
        assert!(msg.contains("--force"), "names the escape hatch: {msg}");
    }

    /// Content the user typed into today's journal is still content, even
    /// though `init` created that page.
    #[test]
    fn guard_refuses_a_journal_the_user_wrote_in() {
        let (_tmp, root) = init_workspace();
        {
            let mut ctx = crate::ws::open(&root).expect("open");
            let today = outl_actions::open_today(&mut ctx.workspace, &ctx.hlc).expect("today");
            outl_actions::append_block(
                &mut ctx.workspace,
                &ctx.hlc,
                Some(today),
                Some("wrote this before importing"),
            )
            .expect("append");
        }
        let ctx = crate::ws::open(&root).expect("reopen");
        assert!(guard_workspace(&root, &ctx.workspace, false, None).is_err());
    }

    /// Markdown dropped into `pages/` by hand never went through
    /// `reconcile_md`, so it has no sidecar and the tree cannot see it —
    /// but an import would overwrite it all the same.
    #[test]
    fn guard_refuses_markdown_dropped_in_without_a_sidecar() {
        let (_tmp, root) = init_workspace();
        std::fs::write(root.join("pages").join("dropped.md"), "- by hand\n").expect("write");
        let ctx = crate::ws::open(&root).expect("open");
        let err = guard_workspace(&root, &ctx.workspace, false, None).expect_err("must refuse");
        assert!(
            err.to_string().contains("never reconciled"),
            "{}",
            err.to_string()
        );
    }

    #[test]
    fn guard_yields_to_an_explicit_force() {
        let (_tmp, root) = init_workspace();
        std::fs::write(root.join("pages").join("dropped.md"), "- by hand\n").expect("write");
        let ctx = crate::ws::open(&root).expect("open");
        assert!(guard_workspace(&root, &ctx.workspace, true, None).is_ok());
    }

    /// An import that died at page 40k of 66k leaves content behind, so
    /// the guard would demand `--force` on the retry — the destructive
    /// flag, as the only way to recover from a failure. The marker breaks
    /// that dead end: everything in the destination came from the failed
    /// run, so there is nothing of the user's to protect.
    #[test]
    fn an_unfinished_import_is_resumable_without_force() {
        let (_tmp, root) = init_workspace();
        let dot_outl = root.join(".outl");
        {
            let mut ctx = crate::ws::open(&root).expect("open");
            let page = outl_actions::open_or_create_by_name(
                &mut ctx.workspace,
                &ctx.hlc,
                "half-written",
                outl_actions::PageKind::Page,
            )
            .expect("create page");
            outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some("partial"))
                .expect("append");
        }
        let ctx = crate::ws::open(&root).expect("reopen");
        assert!(
            guard_workspace(&root, &ctx.workspace, false, None).is_err(),
            "without the marker this is indistinguishable from real content"
        );

        write_marker(&dot_outl, "roam", Path::new("/tmp/backup.json"));
        let unfinished = read_marker(&dot_outl).expect("marker round-trips");
        assert_eq!(unfinished.adapter, "roam");
        assert!(guard_workspace(&root, &ctx.workspace, false, Some(&unfinished)).is_ok());

        clear_marker(&dot_outl);
        assert!(read_marker(&dot_outl).is_none());
        assert!(guard_workspace(&root, &ctx.workspace, false, None).is_err());
    }

    /// The whole point of the marker, end to end: an import that dies
    /// partway leaves content behind, so without it the retry would have
    /// to be `--force` — the destructive flag as the only exit from a
    /// failure. A directory sitting where a page's `.md` must be written
    /// is the cheapest real mid-pipeline failure.
    #[test]
    fn a_failed_import_leaves_a_marker_and_the_retry_needs_no_force() {
        let (_tmp, root) = init_workspace();
        let (_src_dir, src) = roam_fixture();
        std::fs::create_dir_all(root.join("pages").join("a.md")).expect("block the write");

        let err = run("roam", &src, &root, ImportFlags::default()).expect_err("write must fail");
        let msg = err.to_string();
        assert!(msg.contains("half-populated"), "{msg}");
        assert!(msg.contains("import-in-progress.json"), "{msg}");
        assert!(
            marker_path(&root.join(".outl")).exists(),
            "the marker is what makes the retry possible — it must survive"
        );

        std::fs::remove_dir(root.join("pages").join("a.md")).expect("unblock");
        run("roam", &src, &root, ImportFlags::default()).expect("retry resumes without --force");
        assert!(
            !marker_path(&root.join(".outl")).exists(),
            "the completed retry clears it"
        );
    }

    /// A corrupt marker must not be a free pass — it reads as "no
    /// unfinished import" and the guard stays in its refusing mode.
    #[test]
    fn a_corrupt_marker_is_not_a_free_pass() {
        let (_tmp, root) = init_workspace();
        let dot_outl = root.join(".outl");
        std::fs::write(marker_path(&dot_outl), "{not json").expect("write");
        assert!(read_marker(&dot_outl).is_none());
    }

    /// End to end over the documented flow: `init` then `import` works
    /// with no flags, the marker is gone afterwards, and the *second*
    /// import — the destructive one — is the one that gets refused.
    #[test]
    fn init_then_import_needs_no_force_but_the_second_import_does() {
        let (_tmp, root) = init_workspace();
        let (_src_dir, src) = roam_fixture();

        run("roam", &src, &root, ImportFlags::default()).expect("init + import is the happy path");
        assert!(
            !marker_path(&root.join(".outl")).exists(),
            "a completed import clears its marker"
        );

        let err = run("roam", &src, &root, ImportFlags::default())
            .expect_err("re-importing over the result is destructive");
        assert!(err.to_string().contains("--force"), "{}", err);

        run(
            "roam",
            &src,
            &root,
            ImportFlags {
                force: true,
                ..Default::default()
            },
        )
        .expect("--force is the explicit opt-in");
    }

    /// `--dry-run` writes nothing, so it never reaches the guard.
    #[test]
    fn dry_run_never_touches_the_guard() {
        let dst = dst_with(&["pages/notes.md"]);
        let src_dir = tempfile::tempdir().expect("tempdir");
        let src = src_dir.path().join("backup.json");
        std::fs::write(
            &src,
            r#"[{"title": "A", "children": [{"string": "x", "uid": "a1", "children": []}]}]"#,
        )
        .expect("write fixture");
        let flags = ImportFlags {
            dry_run: true,
            json: true,
            ..Default::default()
        };
        run("roam", &src, dst.path(), flags).expect("dry-run stays allowed");
    }
}
