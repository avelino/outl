//! One iCloud folder, two binaries: the workspace that has to survive a
//! staggered rollout.
//!
//! Devices in a single workspace never update at the same instant — the
//! mobile build sits on TestFlight while the desktop is already on the
//! next commit, and a laptop can stay closed for a week. So the sidecar
//! format is read and written **concurrently by binaries of different
//! ages**, and the rule that keeps that safe is the one documented on
//! `SIDECAR_VERSION`: an additive field does not bump the version.
//!
//! What happened when it did: the older binary version-checked the file,
//! refused it, and on every path that consumes a sidecar a refused one
//! looked exactly like a missing one. No old blocks → every block
//! matched at level 3 → a fresh ULID each, while the old ids stayed in
//! the tree. The page duplicated, every `((blk-…))` handle rotated, and
//! the newer binary did the same in reverse on its next boot: a loop
//! that doubled the workspace once per sync.
//!
//! These tests pin both halves of the contract by modelling the shipped
//! binary explicitly (`ShippedSidecar` below — the exact v2 shape, with
//! its version gate and without `text`) and running it against the
//! current one over the same files.

use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_md::reconcile::reconcile_md;
use outl_md::sidecar::{self, sidecar_path_for};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ---------------------------------------------------------------------
// The already-shipped binary (everything up to v0.9.0-beta.149)
// ---------------------------------------------------------------------

/// `SidecarBlock` as every released binary knows it: no `text`, and — as
/// in the real struct — no `deny_unknown_fields`, which is what lets it
/// tolerate keys added later at the same version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShippedBlock {
    id: NodeId,
    line: usize,
    indent: u32,
    content_hash: String,
    #[serde(default)]
    ref_handle: String,
}

/// `Sidecar` as every released binary knows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShippedSidecar {
    version: u32,
    page_id: NodeId,
    last_synced_hash: String,
    last_synced_at: String,
    blocks: Vec<ShippedBlock>,
    #[serde(default)]
    pipeline_version: u32,
}

/// The version gate the shipped binary applies, verbatim:
/// `version < 1 || version > 2` is refused.
const SHIPPED_SIDECAR_VERSION: u32 = 2;

/// `sidecar::read` as the shipped binary performs it.
///
/// Returns `Err` on the two outcomes that matter here: unparseable JSON,
/// and a version outside `[1, 2]`. Both used to collapse into "there is
/// no sidecar" further down the pipeline.
fn shipped_read(path: &Path) -> Result<ShippedSidecar, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let sc: ShippedSidecar = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if sc.version < 1 || sc.version > SHIPPED_SIDECAR_VERSION {
        return Err(format!("unsupported sidecar version: {}", sc.version));
    }
    Ok(sc)
}

/// `sidecar::write` as the shipped binary performs it: it serialises the
/// struct it knows, so **`text` is dropped from disk**. This is the
/// half of the loop the current binary has to absorb without losing an
/// id.
fn shipped_write(path: &Path, sc: &ShippedSidecar) {
    fs::write(path, serde_json::to_string_pretty(sc).unwrap()).unwrap();
}

/// A full boot of the shipped binary over `md_path`: read the sidecar,
/// refuse to proceed if the version gate rejects it, otherwise write it
/// back in its own shape.
fn shipped_binary_boot(md_path: &Path) -> Result<(), String> {
    let path = sidecar_path_for(md_path);
    let mut sc = shipped_read(&path)?;
    sc.version = SHIPPED_SIDECAR_VERSION;
    shipped_write(&path, &sc);
    Ok(())
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

const ORIGINAL: &str = "\
- buy groceries at the market
- call the plumber back
  - ask about the leak under the sink
- ship the release notes
";

fn setup() -> (TempDir, Workspace, HlcGenerator) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    (dir, ws, hlc)
}

fn write_page(dir: &Path, body: &str) -> PathBuf {
    let pages = dir.join("pages");
    fs::create_dir_all(&pages).unwrap();
    let path = pages.join("notes.md");
    fs::write(&path, body).unwrap();
    path
}

/// `block text → (id, ref_handle)` for every entry in the sidecar.
fn identity_map(md_path: &Path) -> BTreeMap<String, (NodeId, String)> {
    let sc = sidecar::read(&sidecar_path_for(md_path)).unwrap();
    // Keyed by `content_hash`, not by `text`: a sidecar the shipped
    // binary rewrote has no `text` field to key on, and the hash is the
    // one identifier of a block's content present in every shape.
    sc.blocks
        .iter()
        .map(|b| (b.content_hash.clone(), (b.id, b.ref_handle.clone())))
        .collect()
}

/// Number of live (non-trashed) descendants of `page_id` in the tree.
///
/// The duplication symptom is invisible in the sidecar — it only lists
/// what the last write produced. It shows up here: the orphaned copies
/// stay parented under the page because nothing ever moved them to
/// `TRASH_ROOT`.
fn live_descendants(ws: &Workspace, page_id: NodeId) -> usize {
    let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (node, parent, _) in ws.tree().iter_nodes() {
        children.entry(parent).or_default().push(node);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![page_id];
    while let Some(n) = stack.pop() {
        for &c in children.get(&n).into_iter().flatten() {
            if seen.insert(c) {
                stack.push(c);
            }
        }
    }
    seen.len()
}

fn md_block_count(md: &str) -> usize {
    md.lines()
        .filter(|l| l.trim_start().starts_with("- "))
        .count()
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[test]
fn the_shipped_binary_can_read_what_this_one_writes() {
    // The single assertion that would have caught the regression: take
    // a sidecar this binary just wrote and run it through the version
    // gate of the one already in users' hands.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let sc = shipped_read(&sidecar_path_for(&md_path))
        .expect("a released binary must still accept this sidecar");

    assert_eq!(sc.version, SHIPPED_SIDECAR_VERSION);
    assert_eq!(sc.blocks.len(), md_block_count(ORIGINAL));
    assert!(
        sc.blocks.iter().all(|b| b.ref_handle.starts_with("blk-")),
        "the fields the shipped binary does know must survive intact"
    );

    // …and the field it does not know is present on disk regardless.
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(sidecar_path_for(&md_path)).unwrap()).unwrap();
    assert_eq!(raw["blocks"][0]["text"], "buy groceries at the market");
}

#[test]
fn alternating_binaries_never_rotate_an_id_or_a_ref_handle() {
    // The loop, run for real: shipped binary boots, rewrites the
    // sidecar without `text`; current binary boots, reconciles, writes
    // it back with `text`. Ten round trips. Nothing may move.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let page_id = sidecar::read(&sidecar_path_for(&md_path)).unwrap().page_id;
    let expected = md_block_count(ORIGINAL);
    let baseline = identity_map(&md_path);
    assert_eq!(baseline.len(), expected);

    for round in 0..10 {
        shipped_binary_boot(&md_path)
            .unwrap_or_else(|e| panic!("round {round}: shipped binary refused the sidecar: {e}"));

        // Proof the downgrade actually happened — otherwise the loop
        // would be testing nothing.
        let downgraded = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
        assert!(
            downgraded.blocks.iter().all(|b| b.text.is_empty()),
            "round {round}: the shipped binary drops `text` on write"
        );

        reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

        assert_eq!(
            identity_map(&md_path),
            baseline,
            "round {round}: every block must keep its id AND its ((blk-…)) handle"
        );
        assert_eq!(
            live_descendants(&ws, page_id),
            expected,
            "round {round}: the tree gained a duplicate set of blocks"
        );
    }
}

#[test]
fn an_edit_by_each_binary_in_turn_keeps_the_untouched_blocks_stable() {
    // The realistic version of the loop: both devices are *used*, not
    // just booted. Each side appends a block and hands the workspace
    // back. Ids of everything it did not touch must not move, and the
    // tree must not accumulate copies.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let page_id = sidecar::read(&sidecar_path_for(&md_path)).unwrap().page_id;
    let baseline = identity_map(&md_path);
    let mut body = ORIGINAL.to_string();

    for round in 0..4 {
        // The device running the shipped binary edits the file, then
        // reconciles with a sidecar that has no `text` (level 2 off —
        // exactly the matching the shipped binary performs) and writes
        // the sidecar back in its own shape.
        body.push_str(&format!("- note from the old binary, round {round}\n"));
        fs::write(&md_path, &body).unwrap();
        reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
        shipped_binary_boot(&md_path).unwrap();

        // The device running the current binary edits and reconciles.
        body.push_str(&format!("- note from the new binary, round {round}\n"));
        fs::write(&md_path, &body).unwrap();
        reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

        let now = identity_map(&md_path);
        for (hash, id_and_handle) in &baseline {
            assert_eq!(
                now.get(hash),
                Some(id_and_handle),
                "round {round}: an untouched block lost its id or its handle"
            );
        }
        assert_eq!(
            live_descendants(&ws, page_id),
            md_block_count(&body),
            "round {round}: the tree must hold exactly the blocks in the .md"
        );
    }
}

#[test]
fn a_sidecar_from_the_future_is_refused_instead_of_rebuilt() {
    // Defence in depth for the day a bump is genuinely necessary. This
    // binary cannot patch the ones already shipped, but it can refuse to
    // be the one that corrupts the page: an unreadable-because-newer
    // sidecar must not be mistaken for a missing one.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let sidecar_path = sidecar_path_for(&md_path);
    let before = sidecar::read(&sidecar_path).unwrap();
    let page_id = before.page_id;
    let ids_before: Vec<NodeId> = before.blocks.iter().map(|b| b.id).collect();
    let live_before = live_descendants(&ws, page_id);

    // A peer on a future format writes the page.
    let mut raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    raw["version"] = serde_json::json!(SHIPPED_SIDECAR_VERSION + 7);
    fs::write(&sidecar_path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    // And the `.md` changes, so the short-circuit cannot mask the path.
    fs::write(&md_path, format!("{ORIGINAL}- a block added by the peer\n")).unwrap();

    let err = reconcile_md(&mut ws, &hlc, &md_path, None)
        .expect_err("a future sidecar must stop the reconcile, not restart the page");
    assert!(
        err.to_string().contains("unsupported sidecar version"),
        "the refusal must name the cause; got: {err}"
    );

    assert_eq!(
        live_descendants(&ws, page_id),
        live_before,
        "a refused reconcile must not touch the tree"
    );
    // The sidecar is left exactly as the peer wrote it — not rewritten
    // with a fresh set of ULIDs, which is what "rebuild from scratch"
    // would have produced.
    let on_disk: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    let ids_after: Vec<NodeId> = on_disk["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| serde_json::from_value(b["id"].clone()).unwrap())
        .collect();
    assert_eq!(ids_after, ids_before, "the sidecar must be left untouched");
    assert_eq!(on_disk["version"], SHIPPED_SIDECAR_VERSION + 7);
}

#[test]
fn v1_sidecar_still_loads_and_keeps_its_ids() {
    // Backward compatibility is unchanged by the rule: the oldest shape
    // in the wild (no `ref_handle`, no `text`) still reconciles, and the
    // handles it never stored are re-derived to the same values.
    let (dir, mut ws, hlc) = setup();
    let md_path = write_page(dir.path(), ORIGINAL);
    let sidecar_path = sidecar_path_for(&md_path);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let baseline = identity_map(&md_path);

    // Downgrade on disk to the v1 shape.
    let mut raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    raw["version"] = serde_json::json!(1);
    for b in raw["blocks"].as_array_mut().unwrap() {
        let b = b.as_object_mut().unwrap();
        b.remove("text");
        b.remove("ref_handle");
    }
    fs::write(&sidecar_path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    let loaded = sidecar::read(&sidecar_path).unwrap();
    assert_eq!(loaded.version, sidecar::SIDECAR_VERSION);
    assert!(loaded.blocks.iter().all(|b| !b.ref_handle.is_empty()));

    // A structural edit still matches the untouched blocks by hash.
    fs::write(&md_path, format!("{ORIGINAL}- a fourth item\n")).unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let now = identity_map(&md_path);
    for (hash, id_and_handle) in &baseline {
        assert_eq!(
            now.get(hash),
            Some(id_and_handle),
            "a v1 sidecar must not cost a block its id"
        );
    }
}
