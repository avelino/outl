//! The external edit that used to break block references: **one save
//! that both rewords a block and adds or removes another**.
//!
//! The counts disagree, so the positional fallback (level 1.5) is out
//! of play. Before level-2 similarity matching, every reworded block in
//! such a save minted a fresh ULID, its old id went to `TRASH_ROOT`,
//! and every `((blk-…))` / `!((blk-…))` pointing at it stopped
//! resolving. These run the whole pipeline — `.md` on disk → reconcile
//! → sidecar → second reconcile — because that is where the sidecar
//! version, the matcher, and the diff have to agree.

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_md::reconcile::reconcile_md;
use outl_md::sidecar::{self, sidecar_path_for};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const ORIGINAL: &str = "\
- buy groceries at the market
- call the plumber back
- ship the release notes
";

fn setup() -> (TempDir, Workspace, HlcGenerator) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    (dir, ws, hlc)
}

fn write_page(dir: &Path, body: &str) -> std::path::PathBuf {
    let pages = dir.join("pages");
    fs::create_dir_all(&pages).unwrap();
    let path = pages.join("notes.md");
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn reword_one_block_and_delete_another_keeps_the_id_and_the_ref_handle() {
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    let log_path = dir.path().join("orphans.log");

    reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();
    let before = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(before.version, sidecar::SIDECAR_VERSION);
    assert_eq!(before.blocks.len(), 3);
    assert_eq!(
        before.blocks[0].text, "buy groceries at the market",
        "the sidecar must record the text level 2 will diff against"
    );
    let (kept_id, kept_handle) = (before.blocks[0].id, before.blocks[0].ref_handle.clone());
    let deleted_id = before.blocks[1].id;

    // One external save: block 1 reworded, block 2 gone.
    fs::write(
        &md_path,
        "- buy groceries at the market today\n- ship the release notes\n",
    )
    .unwrap();
    let report = reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();

    let after = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(after.blocks.len(), 2);
    assert_eq!(
        after.blocks[0].id, kept_id,
        "the reworded block must keep its id"
    );
    assert_eq!(
        after.blocks[0].ref_handle, kept_handle,
        "…and its ((blk-…)) handle, or every reference to it dangles"
    );
    assert_eq!(
        ws.block_text(kept_id).unwrap_or_default(),
        "buy groceries at the market today",
        "the workspace must carry the new text under the same node"
    );

    // The genuinely deleted block is the only orphan, and it is logged
    // before anything moves it to the trash.
    assert_eq!(report.orphans, 1);
    let log = fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains(&deleted_id.to_string()),
        "deleted block must be in orphans.log; got:\n{log}"
    );
    assert!(
        !log.contains(&kept_id.to_string()),
        "the reworded block must NOT be logged as an orphan; got:\n{log}"
    );
}

#[test]
fn reword_one_block_and_insert_another_keeps_the_id() {
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);

    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    let before = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    let kept_id = before.blocks[2].id;
    let kept_handle = before.blocks[2].ref_handle.clone();

    fs::write(
        &md_path,
        "- fresh block at the top\n\
         - buy groceries at the market\n\
         - call the plumber back\n\
         - ship the release notes now\n",
    )
    .unwrap();
    let report = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let after = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(after.blocks.len(), 4);
    assert_eq!(after.blocks[3].id, kept_id);
    assert_eq!(after.blocks[3].ref_handle, kept_handle);
    assert_eq!(report.orphans, 0, "nothing was deleted");
}

#[test]
fn replacing_a_block_outright_orphans_it_into_the_log() {
    // Same save shape as the first test — one block rewritten, one
    // deleted — but the rewrite lands *below* the similarity threshold.
    // That is a delete plus an insert, not an edit, and invariant 2
    // says the old id must reach `orphans.log` before anything trashes
    // it. Silent deletion is a P0.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    let log_path = dir.path().join("orphans.log");

    reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();
    let before = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    let replaced_id = before.blocks[0].id;
    let deleted_id = before.blocks[1].id;

    fs::write(
        &md_path,
        "- an entirely different thought about compilers\n\
         - ship the release notes\n",
    )
    .unwrap();
    let report = reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();

    assert_eq!(report.orphans, 2, "replaced + deleted");
    let after = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_ne!(
        after.blocks[0].id, replaced_id,
        "a below-threshold replacement gets a fresh id"
    );
    let log = fs::read_to_string(&log_path).unwrap();
    for id in [replaced_id, deleted_id] {
        assert!(
            log.contains(&id.to_string()),
            "level-3 blocks must be logged before deletion; {id} missing from:\n{log}"
        );
    }
}

#[test]
fn v2_sidecar_without_text_still_loads_and_reconciles() {
    // Every workspace on disk today has a v2 sidecar. It must keep
    // loading, keep matching by hash, and gain the `text` field on the
    // next write — level 2 simply doesn't fire until then.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    let sidecar_path = sidecar_path_for(&md_path);

    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    let with_text = sidecar::read(&sidecar_path).unwrap();
    let ids: Vec<_> = with_text.blocks.iter().map(|b| b.id).collect();

    // Downgrade on disk: drop every `text` and stamp version 2.
    let mut raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    raw["version"] = serde_json::json!(2);
    for b in raw["blocks"].as_array_mut().unwrap() {
        b.as_object_mut().unwrap().remove("text");
    }
    fs::write(&sidecar_path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    let loaded = sidecar::read(&sidecar_path).unwrap();
    assert!(
        loaded.blocks.iter().all(|b| b.text.is_empty()),
        "a v2 payload has no text to load"
    );

    // A structural edit (one block appended) still matches the
    // untouched blocks by hash and preserves their ids.
    fs::write(&md_path, format!("{ORIGINAL}- a fourth item\n")).unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let after = sidecar::read(&sidecar_path).unwrap();
    assert_eq!(after.version, sidecar::SIDECAR_VERSION);
    assert_eq!(after.blocks.len(), 4);
    assert_eq!(
        after
            .blocks
            .iter()
            .take(3)
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        ids,
        "hash matching must preserve the pre-existing ids"
    );
    assert!(
        after.blocks.iter().all(|b| !b.text.is_empty()),
        "the write after the upgrade must record every block's text"
    );
}
