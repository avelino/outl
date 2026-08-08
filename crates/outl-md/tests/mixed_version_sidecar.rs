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
//!
//! # The withheld `last_synced_hash` in a mixed fleet
//!
//! Invariant 8 made `reconcile_md` write an **empty** `last_synced_hash`
//! for a page whose `.md` holds content the pass could not turn into an
//! op. That value is perfectly readable by every already-shipped binary,
//! and the second half of this file pins what they do with it, because
//! the answer is not free.
//!
//! Measured against v0.11.0-beta.151 (the binary in users' hands when
//! this landed), run from a `git worktree` over the same files:
//!
//! - `last_synced_hash: ""` → the shipped `reconcile_md` short-circuit
//!   misses, so it **reconciles the page with its own parser** — the one
//!   whose three #210 defects this release fixed. On a page carrying a
//!   multi-line block it applied 3 ops and rewrote the block's text from
//!   `"head\ndetail\n  deeper detail"` to `"head\ndetail"`: an `Op::Edit`
//!   truncating content that *was* correctly in the log, replicated to
//!   every device. It then stamped a real hash and `pipeline_version: 2`.
//! - The same page with a real `last_synced_hash` → the shipped binary
//!   short-circuits (0 ops) and the tree is untouched.
//!
//! So the withholding does have a cost on old peers, and it is real. It
//! is still the right value, for two reasons this file pins as tests:
//!
//! 1. A real hash is **worse**, not better. The shipped
//!    `apply_page_md_with_sidecar_if_stale` gates re-projection on
//!    nothing but `last_synced_hash == file_hash(disk)` — it has no
//!    `content_lines_missing_from`. A real hash authorises it to render
//!    the tree straight over the `.md` and delete the unlogged content
//!    outright, which is issue #210 with no guard anywhere in the fleet.
//!    An empty hash is the one value that disarms it.
//! 2. No third value exists. The shipped binary reconciles when the hash
//!    does **not** match and re-projects when it **does**; the two gates
//!    are complementary, so every possible `last_synced_hash` arms one of
//!    them. Moving the signal into `version` would arm something far
//!    worse — binaries older than the "refuse, don't rebuild" fix treat
//!    an unreadable sidecar as a missing one and mint a fresh ULID per
//!    block (the loop the top of this file describes).
//!
//! What is *not* true is the part that would have justified a riskier
//! fix: the shipped binary cannot leave the page looking healthy to this
//! one. Every old write path stamps its own (lower) `pipeline_version`,
//! and its sidecar text can only be a subset of what the `.md` holds, so
//! both the re-reconcile trigger and the content guard re-arm on the next
//! boot here. That is pinned below too.
//!
//! Exposure at the time of writing: **0 of 2,827 pages** on the workspace
//! that reported #210 trip the withholding, so nothing is in this state
//! today. It is a guard for the next parser gap, and the next parser gap
//! is exactly when a fleet is most mixed. Tracked in issue #210.

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

/// `CURRENT_PIPELINE_VERSION` as v0.11.0-beta.151 defines it.
///
/// Load-bearing that this is *lower* than the current constant: every
/// write path in that binary (`reconcile_md`, `build_sidecar`,
/// `build_sidecar_from_ast`) stamps its own value rather than preserving
/// what it read, so an old peer touching a page necessarily re-queues it
/// here.
const SHIPPED_PIPELINE_VERSION: u32 = 2;

/// The `reconcile_md` short-circuit as the shipped binary evaluates it:
///
/// ```text
/// existing.last_synced_hash == md_hash
///     && existing.pipeline_version >= CURRENT_PIPELINE_VERSION
/// ```
///
/// `true` here means the shipped binary **reads the `.md` with its own
/// parser** — the one that drops an over-indented continuation line and
/// a blank line inside a block's text.
fn shipped_would_reconcile(md_path: &Path) -> bool {
    let Ok(sc) = shipped_read(&sidecar_path_for(md_path)) else {
        // Unreadable or absent → the shipped binary reconciles from
        // scratch, which is the worst arm of all.
        return true;
    };
    let disk = fs::read_to_string(md_path).expect("read md");
    !(sc.last_synced_hash == sidecar::file_hash(&disk)
        && sc.pipeline_version >= SHIPPED_PIPELINE_VERSION)
}

/// The `apply_page_md_with_sidecar_if_stale` gate as the shipped binary
/// evaluates it — the whole gate, verbatim:
///
/// ```text
/// let faithful = sidecar::read(..).map(|sc| sc.last_synced_hash == disk_hash).unwrap_or(false);
/// if !faithful { return Ok(None); }
/// ```
///
/// There is no `content_lines_missing_from` on that side. `true` means
/// the shipped binary is authorised to render the tree over the `.md`
/// and drop whatever the tree does not hold.
fn shipped_would_reproject(md_path: &Path) -> bool {
    let Ok(sc) = shipped_read(&sidecar_path_for(md_path)) else {
        return false;
    };
    let disk = fs::read_to_string(md_path).expect("read md");
    sc.last_synced_hash == sidecar::file_hash(&disk)
}

/// The shipped binary finishing a reconcile: it stamps the hash of the
/// bytes it just read and **its own** pipeline version, then writes the
/// fields it knows.
///
/// Deliberately leaves the block texts alone. Modelling its parser here
/// would add a second opinion about a defect already measured in a
/// worktree (see the module doc), and every claim below is stronger for
/// holding even when the old pass changed nothing at all.
fn shipped_reconcile(md_path: &Path, keeps_text: bool) {
    let path = sidecar_path_for(md_path);
    let disk = fs::read_to_string(md_path).expect("read md");
    let mut raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read sidecar")).expect("json");
    raw["last_synced_hash"] = serde_json::json!(sidecar::file_hash(&disk));
    raw["pipeline_version"] = serde_json::json!(SHIPPED_PIPELINE_VERSION);
    if !keeps_text {
        // A binary older than `SidecarBlock::text` serialises the struct
        // it knows, so the field is gone from disk entirely.
        for b in raw["blocks"].as_array_mut().expect("blocks") {
            b.as_object_mut().expect("block").remove("text");
        }
    }
    fs::write(&path, serde_json::to_string_pretty(&raw).expect("json")).expect("write sidecar");
}

/// Overwrite the sidecar's `last_synced_hash` in place.
///
/// `""` is exactly what `reconcile_md` writes when it read content it
/// could not turn into an op — the hash is the *only* field that differs
/// between a withheld pass and a clean one, since `blocks` and
/// `pipeline_version` both come from the same plan either way.
fn set_last_synced_hash(md_path: &Path, value: &str) {
    let path = sidecar_path_for(md_path);
    let mut raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read sidecar")).expect("json");
    raw["last_synced_hash"] = serde_json::json!(value);
    fs::write(&path, serde_json::to_string_pretty(&raw).expect("json")).expect("write sidecar");
}

/// Put the page into the state invariant 8 leaves behind: the `.md`
/// carries a content line no block the log knows accounts for, and the
/// hash was withheld rather than advanced over it.
///
/// Built with real APIs, not asserted into existence: the extra line goes
/// on disk without a reconcile, and `content_lines_missing_from` is asked
/// to confirm the page really is in the producer's condition before the
/// hash is withheld.
fn make_page_holding_unlogged_content(dir: &Path) -> PathBuf {
    let md_path = write_page(dir, ORIGINAL);
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");
    let hlc = HlcGenerator::new(actor);
    reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");

    let disk = format!("{ORIGINAL}- a line that exists in no op\n");
    fs::write(&md_path, &disk).expect("write md");

    let blocks = sidecar::read(&sidecar_path_for(&md_path))
        .expect("sidecar")
        .blocks;
    assert!(
        !outl_md::unlogged::content_lines_missing_from(&disk, &blocks).is_empty(),
        "fixture is not in the producer's condition — nothing would be withheld"
    );
    set_last_synced_hash(&md_path, "");
    md_path
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

// ---------------------------------------------------------------------
// The withheld `last_synced_hash` (invariant 8) in a mixed fleet
// ---------------------------------------------------------------------

/// **The cost, stated out loud.** An empty `last_synced_hash` puts the
/// page back in front of the shipped binary's parser.
///
/// This is not a bug to fix by changing the value — see the sibling test
/// for why every other value is worse — it is a property to keep visible.
/// A future change that makes the withheld state common (a parser gap, a
/// new guard reusing the same signal) is also making old peers re-read
/// those pages with a parser that loses content, and it needs to know
/// that before it ships, not after.
///
/// Measured consequence on v0.11.0-beta.151: 3 ops, a block's text
/// rewritten from `"head\ndetail\n  deeper detail"` to `"head\ndetail"`.
#[test]
fn a_withheld_hash_hands_the_page_back_to_the_shipped_binarys_parser() {
    let dir = TempDir::new().unwrap();
    let md_path = make_page_holding_unlogged_content(dir.path());

    assert!(
        shipped_would_reconcile(&md_path),
        "a withheld hash makes the shipped binary re-read the .md with its own parser"
    );

    // The counterfactual, in the same test so the trade is legible: the
    // value this code wrote before invariant 8 was enforced.
    let disk = fs::read_to_string(&md_path).unwrap();
    set_last_synced_hash(&md_path, &sidecar::file_hash(&disk));
    assert!(
        !shipped_would_reconcile(&md_path),
        "with a real hash the shipped binary short-circuits — this is what \
         the withholding gives up"
    );
}

/// **Why it is still the right value.** The shipped binary's
/// re-projection is gated on the hash and nothing else, so a real hash
/// authorises it to render the tree over the `.md` and delete the
/// unlogged content outright.
///
/// The two gates are complementary — reconcile fires when the hash does
/// not match, re-projection when it does — so there is no third value
/// that disarms both. This test pins the half that decides the choice:
/// the withheld hash is the only one that stops a binary with no content
/// guard from writing over the file.
#[test]
fn a_withheld_hash_is_what_stops_the_shipped_binary_overwriting_the_md() {
    let dir = TempDir::new().unwrap();
    let md_path = make_page_holding_unlogged_content(dir.path());

    assert!(
        !shipped_would_reproject(&md_path),
        "a withheld hash must leave the shipped binary unauthorised to \
         re-render the tree over a .md holding unlogged content"
    );

    let disk = fs::read_to_string(&md_path).unwrap();
    set_last_synced_hash(&md_path, &sidecar::file_hash(&disk));
    assert!(
        shipped_would_reproject(&md_path),
        "with a real hash the shipped binary is authorised to overwrite the \
         page — issue #210 with no guard anywhere in the fleet"
    );
}

/// The claim that would have justified a riskier fix, refuted.
///
/// "The old peer stamps a real hash and this binary never sees the page as
/// suspicious again" is false in both of its halves, and neither depends
/// on what the old parser did:
///
/// - it stamps its **own** `pipeline_version`, which is lower, so the
///   short-circuit here cannot fire and the page is reconciled again;
/// - the sidecar text it leaves can only account for lines its parser
///   read, so `content_lines_missing_from` still flags the page.
///
/// Asserted against the *best* case for the old binary — a pass that
/// changed no block text at all. A lossier pass only makes both stronger.
#[test]
fn a_shipped_reconcile_cannot_leave_the_page_looking_healthy_to_this_binary() {
    let dir = TempDir::new().unwrap();
    let md_path = make_page_holding_unlogged_content(dir.path());
    let disk = fs::read_to_string(&md_path).unwrap();

    shipped_reconcile(&md_path, true);

    let sc = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(sc.last_synced_hash, sidecar::file_hash(&disk));
    assert!(
        sc.pipeline_version < sidecar::CURRENT_PIPELINE_VERSION,
        "an old write path stamps its own pipeline version, so the page is \
         re-queued here; got {}",
        sc.pipeline_version
    );
    assert!(
        !outl_md::unlogged::content_lines_missing_from(&disk, &sc.blocks).is_empty(),
        "the content guard must still refuse this page — a real hash from an \
         old peer is not evidence the log holds the file"
    );

    // And the re-queue is not theoretical: reconciling here pulls the line
    // into the log and only then advances the hash.
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    let report = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert_eq!(report.unlogged_lines, 0);

    let healed = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(healed.pipeline_version, sidecar::CURRENT_PIPELINE_VERSION);
    assert!(!healed.last_synced_hash.is_empty());
    assert!(
        outl_md::unlogged::content_lines_missing_from(&disk, &healed.blocks).is_empty(),
        "the line the old peer's hash claimed was logged must actually be \
         logged after this binary's pass"
    );
}

/// The other shape of the same claim: an old peer that predates
/// `SidecarBlock::text` drops the field on write, which is the one input
/// that makes `content_lines_missing_from` stand down.
///
/// That stand-down is **not** permission to write — `sidecar_can_answer`
/// exists so the two empty results stay distinguishable, and this pins
/// that it answers `false` here. It also pins the self-healing half the
/// `outl-actions` docs assert: such a binary necessarily stamps its own
/// lower `pipeline_version` (no old write path preserves what it read), so
/// the page is re-queued and the next pass restores the text.
#[test]
fn a_shipped_binary_that_drops_text_cannot_disarm_the_guard_either() {
    let dir = TempDir::new().unwrap();
    let md_path = make_page_holding_unlogged_content(dir.path());
    let disk = fs::read_to_string(&md_path).unwrap();

    shipped_reconcile(&md_path, false);

    let sc = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert!(
        sc.blocks.iter().all(|b| b.text.is_empty()),
        "the fixture must actually model the text-less write"
    );
    assert!(
        !outl_md::unlogged::sidecar_can_answer(&sc.blocks),
        "a text-less sidecar cannot answer 'does the log know this line', \
         and its empty verdict must not read as 'nothing at risk'"
    );
    assert!(
        sc.pipeline_version < sidecar::CURRENT_PIPELINE_VERSION,
        "otherwise the page would be frozen: unanswerable sidecar, no \
         re-reconcile trigger, and no path back to a text-carrying one"
    );

    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let healed = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert!(
        outl_md::unlogged::sidecar_can_answer(&healed.blocks),
        "one pass here must re-arm the guard"
    );
    assert!(outl_md::unlogged::content_lines_missing_from(&disk, &healed.blocks).is_empty());
}

/// `CURRENT_PIPELINE_VERSION` went 3 → 4 in the same change. Unlike
/// `SIDECAR_VERSION` it is not a gate: every reader compares it with
/// `>=` against its own constant, so a value from the future reads as
/// "at least as new as mine" and costs an older binary nothing.
///
/// Pinned because the two constants sit ten lines apart and are easy to
/// reason about as one thing. They are not: bumping the version number
/// refuses the file fleet-wide, bumping the pipeline number re-runs a
/// pass. Only one of them is safe to move in a patch.
#[test]
fn a_pipeline_version_from_the_future_costs_the_shipped_binary_nothing() {
    let dir = TempDir::new().unwrap();
    let md_path = write_page(dir.path(), ORIGINAL);
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    const {
        assert!(
            sidecar::CURRENT_PIPELINE_VERSION > SHIPPED_PIPELINE_VERSION,
            "this test is only meaningful while the current pipeline is ahead"
        )
    };
    let sc = shipped_read(&sidecar_path_for(&md_path))
        .expect("a pipeline version from the future must not fail the read");
    assert_eq!(sc.pipeline_version, sidecar::CURRENT_PIPELINE_VERSION);
    assert!(
        !shipped_would_reconcile(&md_path),
        "and it must not force the shipped binary to redo the page"
    );
}
