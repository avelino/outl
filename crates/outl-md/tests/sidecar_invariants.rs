//! Sidecar invariant smoke test: version, content_hash alignment, ref_handle, orphan log.

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{self, content_hash, sidecar_path_for};
use outl_md::{reconcile_md, reconcile_md_with_page_id};
use std::fs;

fn setup() -> (tempfile::TempDir, Workspace, HlcGenerator) {
    let dir = tempfile::TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    (dir, ws, hlc)
}

#[test]
fn sidecar_version_is_2_and_json_is_valid() {
    let (dir, mut ws, hlc) = setup();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- alpha\n- beta\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let sp = sidecar_path_for(&md_path);
    let text = fs::read_to_string(&sp).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).expect("sidecar must be valid JSON");
    assert_eq!(v["version"], 2);
}

#[test]
fn content_hash_in_sidecar_matches_expected() {
    let (dir, mut ws, hlc) = setup();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- alpha\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let sp = sidecar_path_for(&md_path);
    let sc = sidecar::read(&sp).unwrap();
    let expected = content_hash("alpha");
    assert_eq!(sc.blocks[0].content_hash, expected);
}

#[test]
fn reconcile_md_with_page_id_pins_the_page() {
    let (dir, mut ws, hlc) = setup();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- hello\n").unwrap();
    let explicit_id = NodeId::new();
    reconcile_md_with_page_id(&mut ws, &hlc, &md_path, explicit_id, None).unwrap();

    let sp = sidecar_path_for(&md_path);
    let sc = sidecar::read(&sp).unwrap();
    assert_eq!(
        sc.page_id, explicit_id,
        "page_id must match the one passed in"
    );
}

#[test]
fn reconcile_md_none_produces_random_page_id() {
    let (dir, mut ws, hlc) = setup();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- hello\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let sp = sidecar_path_for(&md_path);
    let sc = sidecar::read(&sp).unwrap();
    // NodeId::default() calls NodeId::new() (fresh ULID), so the path
    // explicit_page_id.unwrap_or_default() with None is identical to the
    // old NodeId::new() call — a random, non-zero ULID each time.
    // Confirm it doesn't collide with sentinel values (root/trash).
    assert_ne!(
        sc.page_id,
        NodeId::root(),
        "page_id must not be root sentinel"
    );
    assert_ne!(
        sc.page_id,
        NodeId::trash(),
        "page_id must not be trash sentinel"
    );
}

#[test]
fn short_circuit_returns_zero_ops_on_same_hash() {
    let (dir, mut ws, hlc) = setup();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- a\n- b\n").unwrap();
    let r1 = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert!(r1.ops_applied > 0);
    let r2 = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert_eq!(r2.ops_applied, 0, "short-circuit must fire on same hash");
}
