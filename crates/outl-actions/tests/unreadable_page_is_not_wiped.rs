//! The silent page wipe: a failed read must never become a write.
//!
//! Every read-parse-mutate-render-write path in this crate used to open
//! the page with `fs::read_to_string(..).unwrap_or_default()`. When the
//! read failed for any reason other than "the file isn't there" — a raw
//! `EIO`, a permissions change, invalid UTF-8, or an iCloud placeholder
//! whose bytes hadn't been downloaded to this device yet — the page
//! parsed as an empty AST, got rendered back, and `write_atomic`
//! faithfully replaced a full page with nothing.
//!
//! What made it unrecoverable in practice is the step after: the sidecar
//! was rebuilt from the same empty AST, so its hashes agreed with the
//! file on disk and every later consistency scan (`scan_for_orphans`,
//! `scan_for_desynced_projections`, `outl doctor`) saw a page that was
//! perfectly in sync — and empty.
//!
//! These tests use a **directory** where the `.md` belongs. Reading it
//! fails with a non-`NotFound` error on every platform, which is exactly
//! the shape of failure the old code swallowed, and needs no fault
//! injection layer to reproduce.

use outl_actions::error::ActionError;
use outl_actions::journal::{mutate_page_md, page_md_path};
use outl_actions::page::{open_or_create, page_meta, PageKind};
use outl_actions::{append_block, read_page_outline};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// Make the page's `.md` path unreadable-but-present by putting a
/// directory there. Returns the meta the callers need.
fn workspace_with_unreadable_page(
    tmp: &TempDir,
) -> (Workspace, HlcGenerator, outl_actions::page::PageMeta) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("something important")).unwrap();
    let meta = page_meta(&ws, page).unwrap();

    let path = page_md_path(tmp.path(), &meta);
    std::fs::create_dir_all(&path).unwrap();

    (ws, hlc, meta)
}

/// `mutate_page_md` must refuse rather than render an empty AST over a
/// page it could not read.
#[test]
fn mutate_page_md_errors_instead_of_wiping_an_unreadable_page() {
    let tmp = TempDir::new().unwrap();
    let (_ws, _hlc, meta) = workspace_with_unreadable_page(&tmp);

    let result = mutate_page_md(tmp.path(), &meta, |parsed, _| {
        // A mutation that would be perfectly ordinary on a healthy page.
        parsed.blocks.push(Default::default());
        Ok(())
    });

    assert!(
        result.is_err(),
        "an unreadable page must surface an error, not be silently replaced \
         with the mutation applied to an empty document"
    );

    // And nothing was written over it.
    let path = page_md_path(tmp.path(), &meta);
    assert!(
        path.is_dir(),
        "the unreadable path must be left exactly as it was found"
    );
    let sidecar = outl_md::resolve_sidecar_path(&path);
    assert!(
        !sidecar.exists(),
        "a sidecar rebuilt from the empty parse is what hid this bug from \
         every later consistency scan — it must not be written"
    );
}

/// The read path has the same rule for a different reason: rendering an
/// empty outline for a page that *does* have content invites the user to
/// retype it, and the next commit writes that emptiness back.
#[test]
fn read_page_outline_errors_instead_of_reporting_an_unreadable_page_as_empty() {
    let tmp = TempDir::new().unwrap();
    let (_ws, _hlc, meta) = workspace_with_unreadable_page(&tmp);

    assert!(
        read_page_outline(tmp.path(), &meta).is_err(),
        "an unreadable page must not be indistinguishable from an empty one"
    );
}

/// The gap the "unreadable" fix left open, and the reason it matters
/// more than the case above.
///
/// On iOS and legacy iCloud Drive an un-downloaded file is
/// `.foo.md.icloud` and **the real name does not exist**, so the read is
/// `NotFound` — the one error `read_for_rewrite` answers with `Ok("")`.
/// A sidecar sitting next to it is proof the page existed: rewriting
/// there projects one block over N, rebuilds the sidecar to agree, and
/// the next `reconcile_md` emits `Move`→`TRASH_ROOT` for every id that
/// just vanished.
#[test]
fn a_missing_md_with_a_sidecar_present_is_not_a_new_page() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("something important")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("and a second block")).unwrap();
    let meta = page_meta(&ws, page).unwrap();

    // Project it, then lose only the `.md` — exactly what a half-synced
    // folder (or an editor that deletes-and-recreates) leaves behind.
    outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &meta);
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    let sidecar_before = std::fs::read_to_string(&sidecar_path).unwrap();
    std::fs::remove_file(&md_path).unwrap();

    let result = mutate_page_md(tmp.path(), &meta, |parsed, _| {
        parsed.blocks.push(Default::default());
        Ok(())
    });

    assert!(
        matches!(result, Err(ActionError::PageMarkdownVanished(_))),
        "a missing .md beside a live sidecar is a lost file, not a new page: {result:?}"
    );
    assert!(
        !md_path.exists(),
        "the refused rewrite must not create the page it declined to touch"
    );
    assert_eq!(
        std::fs::read_to_string(&sidecar_path).unwrap(),
        sidecar_before,
        "rebuilding the sidecar from a one-block parse is what orphans every \
         block on the next reconcile — it must not be rewritten"
    );
}

/// The iCloud shape of the same thing, with no sidecar on this device
/// yet: the file is not lost at all, it just hasn't landed.
#[test]
fn an_icloud_placeholder_is_not_a_new_page() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let meta = page_meta(&ws, page).unwrap();

    let md_path = page_md_path(tmp.path(), &meta);
    std::fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    // What iCloud leaves in place of a file it has not downloaded.
    let placeholder = md_path.with_file_name(".notes.md.icloud");
    std::fs::write(&placeholder, "").unwrap();

    let result = mutate_page_md(tmp.path(), &meta, |parsed, _| {
        parsed.blocks.push(Default::default());
        Ok(())
    });

    assert!(
        matches!(result, Err(ActionError::PageMarkdownNotDownloaded(_))),
        "a file iCloud hasn't downloaded must not be replaced: {result:?}"
    );
    assert!(!md_path.exists(), "nothing must be written over the page");
}

/// The sidecar half of the same class of bug: `sidecar::read(..).ok()`
/// turned an *unreadable* sidecar into "no sidecar", which makes
/// `build_sidecar_from_ast` mint a fresh ULID for every block and
/// overwrite the mapping. Losing the `.md` is recoverable from the op
/// log; losing id ↔ text breaks every `((blk-…))` into the page.
#[test]
fn an_unreadable_sidecar_is_not_treated_as_a_missing_one() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("keeps its id")).unwrap();
    let meta = page_meta(&ws, page).unwrap();

    outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &meta);
    let md_before = std::fs::read_to_string(&md_path).unwrap();
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);

    // Unreadable, not absent: a directory in its place fails the read on
    // every platform without a fault-injection layer.
    std::fs::remove_file(&sidecar_path).unwrap();
    std::fs::create_dir_all(&sidecar_path).unwrap();

    let result = mutate_page_md(tmp.path(), &meta, |parsed, _| {
        parsed.blocks.push(Default::default());
        Ok(())
    });

    assert!(
        result.is_err(),
        "an unreadable sidecar must surface an error, not silently re-mint \
         every block id: {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        md_before,
        "the page must be left exactly as it was found"
    );
}

/// The one case that legitimately reads as empty stays working: a page
/// that exists in the tree but whose `.md` this device has never written
/// (a peer shipped only the ops). Regression guard for issue #120 — the
/// fix above must not turn "no file yet" into an error.
#[test]
fn a_page_with_no_md_on_disk_still_reads_as_empty() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
    let meta = page_meta(&ws, page).unwrap();
    assert!(!page_md_path(tmp.path(), &meta).exists());

    let outline = read_page_outline(tmp.path(), &meta)
        .expect("a page with no projection yet is legitimately empty, not an error");
    assert!(outline.nodes.is_empty());
}

/// …and the write side of that same case: no `.md`, no sidecar, no
/// placeholder is a genuinely new page and must still project.
#[test]
fn a_page_with_neither_md_nor_sidecar_is_still_created() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create(&mut ws, &hlc, "fresh", "Fresh", PageKind::Page).unwrap();
    let meta = page_meta(&ws, page).unwrap();
    std::fs::create_dir_all(page_md_path(tmp.path(), &meta).parent().unwrap()).unwrap();

    let path = mutate_page_md(tmp.path(), &meta, |parsed, _| {
        parsed.blocks.push(outl_md::parse::OutlineNode {
            text: "first block".to_string(),
            ..Default::default()
        });
        Ok(())
    })
    .expect("a page that never existed is legitimately created here");

    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("first block"));
}
