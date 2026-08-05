//! Which actor this **device** writes under for a given workspace.
//!
//! The op log's whole safety story rests on "one `ops-<actor>.jsonl` per
//! device, never shared". The actor id used to live only in
//! `<root>/.outl/config.toml` — inside the workspace, therefore inside
//! whatever the user syncs. Under Syncthing / Dropbox / NFS / `git`
//! (anything that does not silently drop dot-paths the way iCloud
//! Documents does) both devices read the same `actor_id`, both acquire
//! the [`outl_core::ActorWriteLock`] successfully — `flock(2)` is
//! advisory and never leaves the machine — and both append to the same
//! file. Last-write-wins per file, ops gone, no error raised.
//!
//! The fix is [`outl_core::DeviceStore`]: the actor lives outside the
//! workspace, keyed by ([`WorkspaceId`], workspace directory), in a place
//! no transport replicates. `config.toml` is no longer the source of
//! truth for it. This module owns only the *migration* half — whether the
//! legacy `actor_id` still in `config.toml` may be adopted.
//!
//! ## Adoption is opt-in, forking is the default
//!
//! An existing workspace would ideally keep writing to the
//! `ops-<actor>.jsonl` the device has been using. But the legacy
//! `actor_id` sits in a file that copies verbatim, so "may I adopt it?"
//! has no safe *inferred* answer — and getting it wrong is silent op
//! loss, while getting it wrong the other way costs one extra file that
//! every reader merges anyway.
//!
//! So a device adopts `actor_id` in exactly one case: `[workspace]
//! actor_claimed_by` names *this* machine. That marker is stamped when
//! the config is **created** ([`crate::layout::Config::claimed_by`]),
//! never on first open, because the marker has to be inside the bytes a
//! copy carries away:
//!
//! - Stamping on first open only propagates under a transport that keeps
//!   replicating `config.toml`. **iroh is the default transport** and
//!   ships ops, `workspace-id` and snapshots — never the config. Two
//!   machines holding a claim-less copy would each stamp their own local
//!   file, both believe they own the actor, and collide *permanently*.
//! - Stamping at creation is provably safe: the ULID was minted on this
//!   machine microseconds earlier, so nobody else can be writing under
//!   it.
//!
//! | `actor_claimed_by` | Result |
//! |---|---|
//! | names this machine | adopt `actor_id` — this device created the workspace |
//! | names another machine | mint a fresh actor |
//! | absent (pre-upgrade workspace) | mint a fresh actor |
//!
//! The last row is the deliberate cost: every device opening a
//! pre-upgrade workspace forks once, leaving the old `ops-<legacy>.jsonl`
//! in place. Nothing is lost — readers merge every `ops-*.jsonl` in the
//! directory — and no two devices can end up on one file.
//!
//! Where the *directory* is duplicated rather than the config (`cp -R`,
//! a second checkout), the device store makes the two instances diverge
//! on its own; see [`outl_core::DeviceStore::actor_for_instance`].
//!
//! Nothing here writes `config.toml`. Resolution is a pure read plus a
//! device-local, atomic, compare-and-swap binding.

use anyhow::{Context, Result};
use outl_core::device::DeviceStore;
use outl_core::id::ActorId;
use outl_core::workspace_id::WorkspaceId;

use crate::layout::{Config, Paths};

/// Resolve the actor this device writes under for the workspace at
/// `paths`, migrating a pre-device-store workspace on first open.
///
/// Pure of locking: the returned actor is the device *default*, still
/// subject to [`outl_core::resolve_write_actor`] when a second process
/// on this machine already holds it.
pub fn resolve_device_actor(paths: &Paths, cfg: &Config, store: &DeviceStore) -> Result<ActorId> {
    let workspace = WorkspaceId::read_or_create(&paths.root)
        .with_context(|| format!("reading workspace id at {}", paths.root.display()))?;
    let machine = store
        .machine_id()
        .with_context(|| format!("reading device id at {}", store.dir().display()))?;

    // An unparseable legacy `actor_id` is not fatal: it only means there
    // is nothing to adopt, so this device mints its own.
    let fallback = match (cfg.actor().ok(), cfg.workspace.actor_claimed_by.as_deref()) {
        (Some(legacy), Some(claim)) if claim == machine.as_str() => legacy,
        _ => ActorId::new(),
    };

    store
        .actor_for_instance(&workspace, &paths.root, fallback)
        .with_context(|| format!("binding an actor for {}", paths.root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{init_with_device, read_config, write_config};
    use tempfile::TempDir;

    fn device() -> (TempDir, DeviceStore) {
        let dir = TempDir::new().unwrap();
        let store = DeviceStore::at(dir.path());
        (dir, store)
    }

    /// A workspace root created *by* `store`'s device, so its `actor_id`
    /// carries that device's claim.
    fn workspace(store: &DeviceStore) -> (TempDir, Paths) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        init_with_device(&paths, store).unwrap();
        (dir, paths)
    }

    fn resolve(paths: &Paths, store: &DeviceStore) -> ActorId {
        let cfg = read_config(paths).unwrap();
        resolve_device_actor(paths, &cfg, store).unwrap()
    }

    #[test]
    fn the_device_that_created_the_workspace_keeps_its_ops_file() {
        let (_d, store) = device();
        let (_ws, paths) = workspace(&store);
        let legacy = read_config(&paths).unwrap().actor().unwrap();

        assert_eq!(resolve(&paths, &store), legacy, "must adopt its own actor");
        assert_eq!(resolve(&paths, &store), legacy, "and stay on it");
    }

    #[test]
    fn second_device_sharing_the_same_dot_outl_gets_a_different_actor() {
        let (_a, store_a) = device();
        let (_b, store_b) = device();
        let (_ws, paths) = workspace(&store_a);
        let legacy = read_config(&paths).unwrap().actor().unwrap();

        let actor_a = resolve(&paths, &store_a);
        // Device B reads the very same config.toml (Syncthing replicated
        // `.outl/` verbatim), claim included.
        let actor_b = resolve(&paths, &store_b);

        assert_eq!(actor_a, legacy);
        assert_ne!(actor_a, actor_b, "two devices must never share an actor");
        // The claim is not stolen, and nothing rewrote the config.
        let cfg = read_config(&paths).unwrap();
        assert_eq!(cfg.actor().unwrap(), legacy);
        assert_eq!(
            cfg.workspace.actor_claimed_by.as_deref(),
            Some(store_a.machine_id().unwrap().as_str())
        );
    }

    /// The literal failing case: `.outl/config.toml` copied between
    /// devices (git clone, rsync, restored backup).
    #[test]
    fn a_copied_config_toml_does_not_hand_over_the_actor() {
        let (_a, store_a) = device();
        let (_src, src_paths) = workspace(&store_a);
        let actor_a = resolve(&src_paths, &store_a);

        let (_b, store_b) = device();
        let (_dst, dst_paths) = workspace(&store_b);
        std::fs::copy(&src_paths.config, &dst_paths.config).unwrap();
        assert_ne!(resolve(&dst_paths, &store_b), actor_a);
    }

    /// Defect: `actor_claimed_by` is stamped into `config.toml`, and the
    /// **default** transport (iroh) never ships that file. Two devices
    /// holding a claim-less copy of a pre-upgrade workspace used to both
    /// adopt the legacy actor and each stamp their own local config —
    /// a permanent collision, not a one-open race.
    #[test]
    fn a_pre_upgrade_workspace_forks_on_every_device() {
        let (_a, store_a) = device();
        let (_ws, paths) = workspace(&store_a);

        // Roll the config back to its pre-device-store shape.
        let mut cfg = read_config(&paths).unwrap();
        let legacy = cfg.actor().unwrap();
        cfg.workspace.actor_claimed_by = None;
        let raw = std::fs::read_to_string(&paths.config).unwrap();
        std::fs::write(
            &paths.config,
            raw.lines()
                .filter(|l| !l.starts_with("actor_claimed_by"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert!(read_config(&paths)
            .unwrap()
            .workspace
            .actor_claimed_by
            .is_none());

        let (_b, store_b) = device();
        let (a, b) = (resolve(&paths, &store_a), resolve(&paths, &store_b));
        assert_ne!(a, b, "no two devices may share an actor");
        assert!(
            a != legacy && b != legacy,
            "an unclaimed legacy actor is adopted by nobody"
        );
        // Stable: forking is a one-time decision, not per-open churn.
        assert_eq!(
            (resolve(&paths, &store_a), resolve(&paths, &store_b)),
            (a, b)
        );
    }

    #[test]
    fn device_store_wins_over_the_config_actor() {
        let (_d, store) = device();
        let (_ws, paths) = workspace(&store);
        let actor = resolve(&paths, &store);

        // Someone rewrites config.toml's actor (a restored backup from
        // another device). This device keeps writing to its own file.
        let mut rewritten = read_config(&paths).unwrap();
        rewritten.workspace.actor_id = ActorId::new().to_string();
        write_config(&paths, &rewritten).unwrap();
        assert_eq!(resolve(&paths, &store), actor);
    }

    #[test]
    fn a_wiped_device_store_readopts_its_own_claim() {
        let dir = TempDir::new().unwrap();
        let store = DeviceStore::at(dir.path());
        let (_ws, paths) = workspace(&store);
        let actor = resolve(&paths, &store);

        // Drop only the workspace→actor mapping, keeping the machine id
        // (e.g. the user cleared `~/.config/outl/actors/`).
        std::fs::remove_dir_all(dir.path().join("actors")).unwrap();
        assert_eq!(resolve(&paths, &store), actor);
    }

    #[test]
    fn an_unparseable_legacy_actor_yields_a_fresh_one() {
        let (_d, store) = device();
        let (_ws, paths) = workspace(&store);
        let mut cfg = read_config(&paths).unwrap();
        cfg.workspace.actor_id = "not-a-ulid".into();
        write_config(&paths, &cfg).unwrap();
        // Resolves rather than failing the open.
        resolve(&paths, &store);
    }

    /// Defect: the workspace id lives *inside* the workspace, so two
    /// copies of one directory resolved to one actor — and the P2P
    /// transport keys its gossip topic on that id, so the copies dedup
    /// each other's distinct ops by `ts`.
    #[test]
    fn two_copies_of_one_directory_on_one_device_do_not_share_an_actor() {
        let (_d, store) = device();
        let (_ws, paths) = workspace(&store);
        let original = resolve(&paths, &store);

        let copy_dir = TempDir::new().unwrap();
        let copy = Paths::at(copy_dir.path().to_path_buf());
        std::fs::create_dir_all(&copy.dot_outl).unwrap();
        std::fs::copy(&paths.config, &copy.config).unwrap();
        std::fs::copy(
            WorkspaceId::path_for(&paths.root),
            WorkspaceId::path_for(&copy.root),
        )
        .unwrap();

        assert_ne!(resolve(&copy, &store), original);
        assert_eq!(
            resolve(&paths, &store),
            original,
            "the original is untouched"
        );
    }
}
