//! Tests for the journal projection family (render + apply + paths).

use std::path::PathBuf;

use super::*;
use crate::block::append_block;
use crate::page::{open_journal, open_or_create, page_meta, PageKind};
use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

#[test]
fn render_page_md_outputs_title_prop_then_children() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("second")).unwrap();

    // The title lives in the `title::` property (not the root's text),
    // so it renders as a page property above the children.
    let md = render_page_md(&ws, page);
    assert_eq!(md, "title:: Ideas\n\n- first\n- second\n");
}

/// Build a page projected while it held `initial`, then return the
/// `(workspace, hlc, root, page_id, md_path)` so a test can drive the
/// stale-projection scenarios. The `TempDir` is returned to keep it alive.
fn projected_page(initial: &str) -> (TempDir, Workspace, HlcGenerator, NodeId, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some(initial)).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    (tmp, ws, hlc, page, md_path)
}

/// The reported bug: a peer's op lands in the TREE, but the already-present
/// `.md` the view reads is never re-projected, so the page renders empty.
/// `_if_stale` must detect the tree ran ahead and re-project.
#[test]
fn if_stale_reprojects_when_tree_ran_ahead_of_the_md() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));

    // A synced-in block enters the tree; nothing re-projects the `.md`.
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();
    assert!(!std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(
        wrote.is_some(),
        "a tree ahead of its .md must be re-projected"
    );
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        md.contains("first") && md.contains("synced-in"),
        "re-projection must carry the synced-in block: {md:?}"
    );
}

/// An in-sync page must NOT be re-projected — otherwise every nav churns the
/// sidecar's `last_synced_at` and floods sync (the reason `_if_absent`
/// existed in the first place).
#[test]
fn if_stale_is_a_noop_when_the_md_matches_the_tree() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    let before = std::fs::read_to_string(&md_path).unwrap();
    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(wrote.is_none(), "an in-sync page must not be re-projected");
    assert_eq!(std::fs::read_to_string(&md_path).unwrap(), before);
}

/// Absent `.md` (a peer synced the page into the tree but it was never
/// projected here) → project it. Subsumes `_if_absent` (issue #120).
#[test]
fn if_stale_projects_a_page_whose_md_is_absent() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    assert!(!md_path.exists());

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(wrote.is_some());
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));
}

/// A `.md` whose hash no longer matches its sidecar carries an unreconciled
/// external edit — `_if_stale` must leave it for the `.md → tree` reconcile,
/// never clobber it with a tree re-projection.
#[test]
fn if_stale_never_clobbers_an_external_edit() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, "- hand edited externally\n").unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(
        wrote.is_none(),
        "an externally-edited .md must not be clobbered"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- hand edited externally\n"
    );
}

/// Re-stamp `path`'s sidecar so it declares the bytes currently on disk as
/// the last faithful projection, without changing its block entries. This
/// reproduces the state a `reconcile_md` leaves behind when it rewrites the
/// sidecar to agree with a `.md` whose content never became ops.
fn restamp_sidecar_as_faithful(md_path: &PathBuf) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

/// The `.md` is a *faithful* projection by the hash gate — its sidecar
/// agrees with the bytes on disk — yet it carries content that exists in no
/// op. Measured on a real 2.5k-page workspace: 616 pages in that exact
/// state, ~3.7k lines of content the log had never seen.
///
/// The hash gate cannot tell this apart from a genuinely stale projection,
/// and the old code called both "stale" and re-projected, which deletes
/// the on-disk content and reports success. Every GUI open path
/// (`open_page_by_slug`, `open_journal_for`, …) runs through here, so the
/// loss fires on a plain page open, not just on `doctor --repair`.
///
/// Refuse, and say which lines would have been lost.
#[test]
fn if_stale_refuses_when_the_md_carries_content_the_log_lacks() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);

    match result {
        Err(crate::ActionError::PageMarkdownAheadOfLog { sample, .. }) => assert!(
            sample.contains("only ever on disk"),
            "the error must name the content at risk, got {sample:?}"
        ),
        other => panic!("expected PageMarkdownAheadOfLog, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- first\n- only ever on disk\n",
        "the bytes must survive untouched"
    );
}

/// A peer edited a block. The `.md` still holds the pre-edit text, the
/// tree holds the edit, and the sidecar is hash-faithful to the file.
///
/// The old line is on disk and absent from the render, which is exactly
/// what a disk-versus-render comparison measures — so the first version
/// of this guard refused, and the page froze showing the pre-edit text
/// with nothing surfaced to the user. That is issue #166 reintroduced
/// for the most ordinary sync case there is.
///
/// The question the guard has to ask is "does the op log know this
/// line", not "do disk and tree disagree". The sidecar answers the
/// first: its blocks are what the log held at the last agreement.
#[test]
fn if_stale_reprojects_a_page_a_peer_edited() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("original text");
    let block = crate::tree::children_of(&ws, page)[0].0;
    crate::block::edit_text(&mut ws, &hlc, block, "original text edited by peer").unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a remote edit must reach the .md, not freeze the page"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("edited by peer"));
}

/// Same shape, remote delete: the deleted block's text is on disk and
/// gone from the render. It is not unlogged content, it is content the
/// log deliberately removed, and refusing here resurrects it on the next
/// forced reconcile.
#[test]
fn if_stale_reprojects_a_page_a_peer_deleted_from() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("kept");
    let doomed = append_block(&mut ws, &hlc, Some(page), Some("to be deleted")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    crate::block::delete(&mut ws, &hlc, doomed).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(wrote.is_some(), "a remote delete must reach the .md");
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert!(md.contains("kept") && !md.contains("to be deleted"));
}

/// Indent with no text change moved the line but changed nothing about
/// what the log knows. `trim_end` alone left the indent in the
/// comparison, so `- child` and `  - child` read as different lines and
/// a pure indent counted as unlogged content.
#[test]
fn if_stale_reprojects_after_a_pure_indent() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("parent");
    let child = append_block(&mut ws, &hlc, Some(page), Some("child")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    crate::block::indent(&mut ws, &hlc, child).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(wrote.is_some(), "a pure indent is not unlogged content");
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("child"));
}

/// A sidecar written before `SidecarBlock::text` existed (v1, and every
/// v2 written by a pre-0.11 binary) carries `text: ""` on every block.
///
/// Measured on a real workspace: 7,400 blocks, **zero** with text. With
/// no text to compare against, every line on disk reads as unknown, and
/// the guard flagged 615 pages holding 35,261 lines — against the 233 /
/// 1,426 that are genuinely unlogged. Refusing there would freeze most
/// of the workspace instead of protecting it.
///
/// So a sidecar that cannot answer the question does not get to veto the
/// write. The `CURRENT_PIPELINE_VERSION` bump re-reconciles every page on
/// first boot, which rewrites the sidecars with text, and the guard turns
/// itself on from there.
#[test]
fn if_stale_does_not_block_on_a_sidecar_with_no_text() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    // Strip `text` from every block, as a pre-0.11 sidecar has it.
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    for b in &mut sc.blocks {
        b.text = String::new();
    }
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a sidecar with no text cannot veto the write"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// The counterpart that must keep working: the tree genuinely ran ahead
/// (a peer's ops landed), the `.md` holds a strict subset of what the log
/// renders, so nothing on disk is at risk. This is issue #166's case and
/// the guard above must not regress it.
#[test]
fn if_stale_still_reprojects_when_the_md_holds_no_unlogged_content() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a .md that lost nothing must still be re-projected"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// Whitespace-only drift must not trip the guard. The renderer's trailing
/// newline changed between releases, so on a real workspace a large share
/// of "stale" pages differ from the log by exactly that — refusing those
/// would strand the genuine re-projections behind noise.
#[test]
fn if_stale_ignores_whitespace_only_differences_when_deciding() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    // Trailing spaces + a missing final newline: no content is unique to
    // disk once trimmed, so the tree's new block still wins.
    std::fs::write(&md_path, "- first   ").unwrap();
    restamp_sidecar_as_faithful(&md_path);
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "whitespace-only drift is not unlogged content"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// A present-but-unreadable `.md` (non-UTF8 bytes here — `read_to_string`
/// fails with `InvalidData`, not `NotFound`) must NOT be treated as absent
/// and re-projected; that would clobber a file that may hold real content.
/// It surfaces the I/O error and leaves the bytes untouched.
#[test]
fn if_stale_does_not_clobber_an_unreadable_md() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, [0xff, 0xfe, 0x00]).unwrap();

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);
    assert!(
        result.is_err(),
        "an unreadable .md must surface an error, not be clobbered"
    );
    assert_eq!(std::fs::read(&md_path).unwrap(), vec![0xff, 0xfe, 0x00]);
}

/// Copy (`Cmd+C` in view mode) snapshots a block via
/// `render_block_md`. It must capture the **whole** subtree — every
/// descendant at every depth — so a paste reproduces the block in
/// full. This pins the "are we grabbing all the sub-blocks?" review
/// concern: the renderer walks `build_outline` recursively, so a
/// four-level-deep subtree round-trips with its indentation intact.
#[test]
fn render_block_md_captures_the_full_deep_subtree() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let src = append_block(&mut ws, &hlc, Some(page), Some("src")).unwrap();
    let c1 = append_block(&mut ws, &hlc, Some(src), Some("c1")).unwrap();
    let c1a = append_block(&mut ws, &hlc, Some(c1), Some("c1a")).unwrap();
    append_block(&mut ws, &hlc, Some(c1a), Some("c1a_i")).unwrap();
    append_block(&mut ws, &hlc, Some(c1), Some("c1b")).unwrap();
    append_block(&mut ws, &hlc, Some(src), Some("c2")).unwrap();

    let md = render_block_md(&ws, src);
    assert_eq!(
        md,
        "- src\n  - c1\n    - c1a\n      - c1a_i\n    - c1b\n  - c2\n"
    );
}

#[test]
fn render_page_md_emits_page_level_properties() {
    // Regression for the silent divergence between the op log
    // and the rendered `.md`. Page-level properties (`type::`,
    // `icon::`, etc.) used to be dropped on render because
    // `render_page_md` always passed `properties: Vec::new()`.
    // Result: a person page created via `@` autocomplete in the
    // TUI carried `Op::SetProp { type: person }` in the log but
    // its `.md` had only the blocks — the `WorkspaceIndex`
    // (which parses `.md`) didn't list it under `pages_by_type`,
    // so the next `@` mention never surfaced it.
    use crate::page::set_property;
    use outl_core::property::PropValue;

    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
    set_property(
        &mut ws,
        &hlc,
        page,
        crate::person::TYPE_KEY,
        Some(PropValue::Text(crate::person::PERSON_TYPE.to_string())),
    )
    .unwrap();
    set_property(
        &mut ws,
        &hlc,
        page,
        "icon",
        Some(PropValue::Text("🦀".to_string())),
    )
    .unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("bio")).unwrap();

    let md = render_page_md(&ws, page);
    assert!(
        md.contains("type:: person"),
        "rendered .md must carry the type:: person property; got:\n{md}"
    );
    assert!(
        md.contains("icon:: 🦀"),
        "rendered .md must carry the icon property; got:\n{md}"
    );
    // `page-slug` / `page-kind` stay internal — they're owned by
    // the page-model layer, not by the rendered `.md`.
    assert!(
        !md.contains("page-slug"),
        "internal book-keeping property leaked into rendered .md:\n{md}"
    );
    assert!(
        !md.contains("page-kind"),
        "internal book-keeping property leaked into rendered .md:\n{md}"
    );
    // Body still renders the block.
    assert!(md.contains("- bio"));
}

#[test]
fn page_md_path_routes_journals_and_pages_separately() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let regular = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let journal =
        open_journal(&mut ws, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();

    let r_meta = page_meta(&ws, regular).unwrap();
    let j_meta = page_meta(&ws, journal).unwrap();

    assert!(page_md_path(tmp.path(), &r_meta).ends_with("pages/ideas.md"));
    assert!(page_md_path(tmp.path(), &j_meta).ends_with("journals/2026-05-27.md"));
}

#[test]
fn apply_all_pages_writes_each_to_disk() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first idea")).unwrap();

    let written = apply_all_pages_md(&ws, tmp.path()).unwrap();
    assert_eq!(written.len(), 1);
    let body = std::fs::read_to_string(&written[0]).unwrap();
    // In-app pages store their title in the `title::` property (not the
    // root's Yrs text — see `open_or_create`), so it renders at the top.
    assert_eq!(body, "title:: Ideas\n\n- first idea\n");
}

/// Regression for https://github.com/avelino/outl/issues/120 —
/// a page synced from a peer exists in the CRDT tree but has no
/// `.md` on this device's disk. `open_page_by_slug` calls
/// `apply_page_md_with_sidecar_if_absent`; without the projection
/// `read_page_outline` returns an empty outline and the page opens
/// blank. This test models that scenario: page in workspace, no
/// file on disk → helper writes the projection → outline is populated.
#[test]
fn apply_if_absent_projects_when_md_is_missing() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    // Simulate a synced page: exists in the CRDT tree, no .md on disk.
    let page = open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("peer block")).unwrap();

    // Pre-condition: no .md on disk yet.
    let meta = page_meta(&ws, page).unwrap();
    let path = page_md_path(tmp.path(), &meta);
    assert!(!path.exists(), "test setup error: .md should not exist yet");

    // Call the guarded helper — should project because the file is absent.
    let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
    assert!(
        result.is_some(),
        "expected Some(path) when .md was absent, got None"
    );
    assert!(path.exists(), ".md must be on disk after projection");

    // The projected content must match the CRDT tree (not be empty).
    let body = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        body, "title:: Synced\n\n- peer block\n",
        "projected .md must contain the peer's block, not be blank"
    );

    // `read_page_outline` (the path `open_page_by_slug` takes after
    // the projection) must now return populated content.
    let outline = crate::outline::read_page_outline(tmp.path(), &meta).unwrap();
    assert_eq!(outline.nodes.len(), 1, "outline must have the peer's block");
    assert_eq!(outline.nodes[0].text, "peer block");
}

/// Guard against sync churn: calling `apply_page_md_with_sidecar_if_absent`
/// on a page whose `.md` is already on disk must be a **no-op** — it must
/// not rewrite the `.outl` sidecar. `build_sidecar` stamps
/// `last_synced_at: now()`, so an unconditional call would rewrite
/// the sidecar bytes on every page open, generating noise for every
/// file-transport peer (iCloud / Syncthing) even when nothing changed.
#[test]
fn apply_if_absent_is_noop_when_md_already_exists() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("a block")).unwrap();

    // First projection: write the .md and .outl to disk.
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let meta = page_meta(&ws, page).unwrap();
    let md_path = page_md_path(tmp.path(), &meta);
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);

    // Capture the sidecar bytes before the guarded call.
    let sidecar_before = std::fs::read(&sidecar_path).unwrap();

    // Give the clock a chance to tick so a second `now()` stamp
    // would differ if the sidecar were rewritten.
    std::thread::sleep(std::time::Duration::from_millis(5));

    // Guarded call — file exists, must be a no-op.
    let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
    assert!(
        result.is_none(),
        "expected None when .md already exists, got Some"
    );

    // Sidecar bytes must be unchanged (no `last_synced_at: now()` rewrite).
    let sidecar_after = std::fs::read(&sidecar_path).unwrap();
    assert_eq!(
        sidecar_before, sidecar_after,
        ".outl sidecar must not be rewritten when .md already existed"
    );
}
