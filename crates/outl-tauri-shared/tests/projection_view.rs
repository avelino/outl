//! The async projection path builds the reply view from the **tree**
//! (`build_page_view_from_tree`) instead of re-reading the `.md`
//! (`build_page_view`), because the `.md` write is deferred to the
//! background `ProjectionWriter`. This guards the load-bearing property
//! that the two produce the **same** outline — otherwise a client would
//! render differently depending on whether the projection had landed.

use outl_actions::{append_block, apply_page_md_with_sidecar, open_journal};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_tauri_shared::helpers::{build_page_view, build_page_view_from_tree};
use tempfile::TempDir;

#[test]
fn tree_view_matches_md_view() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    let day = open_journal(
        &mut ws,
        &hlc,
        chrono::NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
    )
    .unwrap();

    // A mix that exercises the shape: plain text, a TODO prefix, inline
    // markdown / refs (drives the `tokens` field), and a nested child.
    let a = append_block(&mut ws, &hlc, Some(day), Some("plain block")).unwrap();
    append_block(&mut ws, &hlc, Some(day), Some("TODO a task with [[a ref]]")).unwrap();
    append_block(&mut ws, &hlc, Some(a), Some("child with **bold**")).unwrap();

    // Project to disk so `build_page_view` (which reads the `.md`) has a
    // file to read; `build_page_view_from_tree` ignores it.
    apply_page_md_with_sidecar(&ws, &root, day).unwrap();

    let from_md = build_page_view(&ws, &root, day).unwrap();
    let from_tree = build_page_view_from_tree(&ws, day).unwrap();

    // Compare the outlines structurally (OutlineNode isn't PartialEq, so
    // go through the wire serialization both clients actually consume).
    let md_json = serde_json::to_value(&from_md.outline).unwrap();
    let tree_json = serde_json::to_value(&from_tree.outline).unwrap();
    assert_eq!(
        md_json, tree_json,
        "the tree-projected view must be byte-identical to the .md-read view"
    );

    // Same page meta, too — the frontend keys off page.id / slug.
    assert_eq!(from_md.page.id, from_tree.page.id);
    assert_eq!(from_md.page.slug, from_tree.page.slug);
}
