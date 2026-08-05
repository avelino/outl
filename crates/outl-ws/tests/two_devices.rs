//! Two devices, one replicated workspace directory.
//!
//! This is the regression for the silent-op-loss bug: `.outl/config.toml`
//! lives inside the workspace, so Syncthing / Dropbox / NFS / `git` hand
//! the same `actor_id` to both machines. `flock(2)` is advisory and
//! machine-local, so both acquired their write lock happily and appended
//! to one `ops-<actor>.jsonl` — last-write-wins, ops gone, no error.
//!
//! Here the two devices differ only in their [`DeviceStore`] (the
//! stand-in for two `$HOME`s); the workspace directory, its `.outl/`, and
//! its `ops/` are literally shared.

use std::path::Path;

use outl_core::device::DeviceStore;
use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_ws::actor::resolve_device_actor;
use outl_ws::layout::{init_with_device, read_config, Paths};
use tempfile::TempDir;

/// Resolve this device's actor, then append one `Create` under the
/// workspace root through its own storage handle.
fn write_one_block(paths: &Paths, store: &DeviceStore, node: NodeId) -> ActorId {
    let cfg = read_config(paths).expect("config");
    let actor = resolve_device_actor(paths, &cfg, store).expect("device actor");
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(paths.ops.clone(), actor).expect("storage");
    let mut ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
        .expect("workspace");
    ws.apply(LogOp {
        ts: hlc.next(),
        actor,
        op: Op::Create {
            node,
            parent: NodeId::root(),
            position: Fractional::between(None, None),
        },
    })
    .expect("apply");
    actor
}

fn ops_files(ops_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(ops_dir)
        .expect("ops dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("ops-") && n.ends_with(".jsonl"))
        .collect();
    names.sort();
    names
}

#[test]
fn two_devices_sharing_one_workspace_dir_never_share_an_ops_file() {
    // Two $HOMEs. Everything else — including `.outl/config.toml` and
    // `ops/` — is one directory both devices see.
    let home_a = TempDir::new().unwrap();
    let home_b = TempDir::new().unwrap();
    let device_a = DeviceStore::at(home_a.path());
    let device_b = DeviceStore::at(home_b.path());

    let ws_dir = TempDir::new().unwrap();
    let paths = Paths::at(ws_dir.path().to_path_buf());
    // Device A created the workspace, so its config claims the actor.
    init_with_device(&paths, &device_a).unwrap();

    let (block_a, block_b) = (NodeId::new(), NodeId::new());
    let actor_a = write_one_block(&paths, &device_a, block_a);
    let actor_b = write_one_block(&paths, &device_b, block_b);

    assert_ne!(
        actor_a, actor_b,
        "two devices must resolve to different actors"
    );
    assert_eq!(
        ops_files(&paths.ops),
        vec![
            format!("ops-{}.jsonl", actor_a.0.min(actor_b.0)),
            format!("ops-{}.jsonl", actor_a.0.max(actor_b.0)),
        ],
        "each device must own its own append-only file"
    );

    // Reopening merges both files through the CRDT: neither device's
    // write was overwritten by the other.
    let reader = JsonlStorage::open(paths.ops.clone(), ActorId::new()).unwrap();
    let ws = Workspace::open_with_storage(
        ActorId::new(),
        Box::new(reader),
        Some(paths.root.to_path_buf()),
    )
    .unwrap();
    assert_eq!(ws.tree().parent(block_a), Some(NodeId::root()));
    assert_eq!(ws.tree().parent(block_b), Some(NodeId::root()));
}

#[test]
fn a_single_device_keeps_writing_to_its_existing_ops_file() {
    let home = TempDir::new().unwrap();
    let device = DeviceStore::at(home.path());

    let ws_dir = TempDir::new().unwrap();
    let paths = Paths::at(ws_dir.path().to_path_buf());
    init_with_device(&paths, &device).unwrap();
    let legacy = read_config(&paths).unwrap().actor().unwrap();

    let first = write_one_block(&paths, &device, NodeId::new());
    let second = write_one_block(&paths, &device, NodeId::new());

    assert_eq!(first, legacy, "migration must adopt the pre-existing actor");
    assert_eq!(second, legacy, "and stay on it across opens");
    assert_eq!(ops_files(&paths.ops), vec![format!("ops-{legacy}.jsonl")]);
}

/// A workspace created **before** the device store existed carries an
/// `actor_id` nobody claimed. `actor_claimed_by` cannot arbitrate it: the
/// default transport is iroh, which ships ops, `workspace-id` and
/// snapshots and never `config.toml`, so a claim stamped on first open
/// reaches no one. Both devices therefore fork, and the legacy ops file
/// keeps being read by everybody.
#[test]
fn a_pre_upgrade_workspace_forks_both_devices_and_keeps_the_old_log_readable() {
    let home_a = TempDir::new().unwrap();
    let home_b = TempDir::new().unwrap();
    let device_a = DeviceStore::at(home_a.path());
    let device_b = DeviceStore::at(home_b.path());

    let ws_dir = TempDir::new().unwrap();
    let paths = Paths::at(ws_dir.path().to_path_buf());
    init_with_device(&paths, &device_a).unwrap();
    let legacy = read_config(&paths).unwrap().actor().unwrap();

    // Strip the claim: this is what a pre-upgrade config.toml looks like.
    let raw = std::fs::read_to_string(&paths.config).unwrap();
    std::fs::write(
        &paths.config,
        raw.lines()
            .filter(|l| !l.starts_with("actor_claimed_by"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    // History the workspace already had under the legacy actor.
    let legacy_block = NodeId::new();
    {
        let hlc = HlcGenerator::new(legacy);
        let storage = JsonlStorage::open(paths.ops.clone(), legacy).unwrap();
        let mut ws =
            Workspace::open_with_storage(legacy, Box::new(storage), Some(paths.root.clone()))
                .unwrap();
        ws.apply(LogOp {
            ts: hlc.next(),
            actor: legacy,
            op: Op::Create {
                node: legacy_block,
                parent: NodeId::root(),
                position: Fractional::between(None, None),
            },
        })
        .unwrap();
    }

    let (block_a, block_b) = (NodeId::new(), NodeId::new());
    let actor_a = write_one_block(&paths, &device_a, block_a);
    let actor_b = write_one_block(&paths, &device_b, block_b);

    assert_ne!(actor_a, actor_b, "two devices must never share an actor");
    assert!(
        actor_a != legacy && actor_b != legacy,
        "an unclaimed legacy actor belongs to nobody"
    );
    assert_eq!(
        ops_files(&paths.ops).len(),
        3,
        "the legacy log plus one fresh file per device: {:?}",
        ops_files(&paths.ops)
    );

    // Nothing was orphaned: a reader merges all three logs.
    let reader = JsonlStorage::open(paths.ops.clone(), ActorId::new()).unwrap();
    let ws = Workspace::open_with_storage(
        ActorId::new(),
        Box::new(reader),
        Some(paths.root.to_path_buf()),
    )
    .unwrap();
    for block in [legacy_block, block_a, block_b] {
        assert_eq!(ws.tree().parent(block), Some(NodeId::root()));
    }
}
