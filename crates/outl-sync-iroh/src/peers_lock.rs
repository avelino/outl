//! Cross-process advisory flock guarding `peers.json` writes (issue #160).
//!
//! `peers.json` is written by pairing persist, the 5s membership gossip tick,
//! the inbound-connection address refresh, and any number of concurrent
//! processes (GUI + MCP server + `outl sync`). Without serialization two
//! writers' read-modify-write windows interleave and one clobbers the other.
//!
//! [`PeersWriteLock`] mirrors the mechanism already used for op-log appends
//! (`engine_sync::OpsDirAppendLock`) and workspace coordination
//! (`outl_core::lock`): a blocking advisory flock on a sibling dotfile next to
//! the guarded file. The empty lock file is never synced (flock state is
//! kernel-local, recreated on demand, so an iCloud/Syncthing transport dropping
//! the dotted path costs nothing), acquisition blocks until the lock is free,
//! and the lock releases when the fd closes on drop.

use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// A held cross-process flock guarding one `peers.json`. Drop releases it.
pub(crate) struct PeersWriteLock {
    _file: std::fs::File,
}

impl PeersWriteLock {
    /// Acquire the flock for the `peers.json` at `peers_path`, on a sibling
    /// `.peers.lock` dotfile. Blocks until the lock is free, so writers
    /// serialize instead of interleaving. Runs on the caller's thread — the
    /// `save` path is synchronous, and the write it guards is short.
    pub(crate) fn acquire(peers_path: &Path) -> Result<Self> {
        let lock_path = peers_path.with_file_name(".peers.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("open peers lock {}", lock_path.display()))?;
        file.lock()
            .with_context(|| format!("flock peers lock {}", lock_path.display()))?;
        Ok(Self { _file: file })
    }
}

/// Atomically replace `path` with `bytes`: write a sibling `.tmp`, `fsync` it,
/// then `rename` into place (same shape as `outl_core::snapshot::write_to_disk`).
/// A crash leaves the old file intact or a stale `.tmp` — never a half-written
/// target a concurrent reader could choke on. The caller must already hold the
/// [`PeersWriteLock`] (this does not take it).
pub(crate) fn atomic_write_json(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    let mut out = std::fs::File::create(&tmp_path)
        .with_context(|| format!("create {}", tmp_path.display()))?;
    out.write_all(bytes)
        .with_context(|| format!("write {}", tmp_path.display()))?;
    out.sync_all()
        .with_context(|| format!("fsync {}", tmp_path.display()))?;
    drop(out);
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))
}

#[cfg(test)]
mod tests {
    use crate::peers::{workspace_peers_path, PeerEntry, PeersStore};

    /// Build a minimal, unique peer entry keyed on `n` — enough for a distinct
    /// node_id so the dedup-by-node_id in `PeersStore::add` never collapses two
    /// writers' entries.
    fn entry(n: usize) -> PeerEntry {
        PeerEntry {
            node_id: iroh::SecretKey::generate().public().to_string(),
            alias: Some(format!("peer-{n}")),
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// Issue #160: many concurrent load-modify-save cycles against ONE
    /// `peers.json` must not lose an entry and must never leave a torn file.
    ///
    /// Each thread does the full read-modify-write the production writers do
    /// (pairing persist, membership tick, addr refresh): load the current file,
    /// add its own distinct entry, save. With the old plain `std::fs::write`
    /// (no lock, truncate-then-stream) two threads racing here lost updates (one
    /// stale copy clobbered another's add) and could expose a half-written file.
    /// The flock + temp+fsync+rename in [`PeersStore::save`] closes both: every
    /// add survives, and every observer parses the file whole.
    #[test]
    fn concurrent_saves_never_lose_an_entry_or_tear_the_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = workspace_peers_path(tmp.path());
        // Seed the file so a concurrent reader always has a valid file to parse.
        PeersStore::load_or_default(&path)
            .expect("seed load")
            .add(entry(0))
            .expect("seed add");

        const WRITERS: usize = 16;
        std::thread::scope(|scope| {
            for n in 1..=WRITERS {
                let path = path.clone();
                scope.spawn(move || {
                    // Full read-modify-write, exactly like the production writers.
                    let mut store = PeersStore::load_or_default(&path).expect("load");
                    store.add(entry(n)).expect("add + save");
                });
            }
        });

        // No lost updates: the seed + all WRITERS entries are present, and the
        // final file parses cleanly (never torn).
        let store = PeersStore::load_or_default(&path).expect("final load parses");
        assert_eq!(
            store.list().len(),
            WRITERS + 1,
            "every concurrent add must survive — no lost updates"
        );
    }
}
