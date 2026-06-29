//! Cross-client sync engine.
//!
//! Encapsulates the reload-workspace + reproject-page dance that both
//! mobile and TUI need when a peer (another device, another local
//! process) writes new ops into `<root>/ops/`. Clients still own:
//!
//! - **Detection.** TUI uses a worker thread that stats the jsonl
//!   files every ~2 s; mobile registers an `NSMetadataQuery` on the
//!   iCloud ubiquity container. Both call [`SyncEngine::snapshot`]
//!   to know what changed.
//! - **Policy.** TUI must defer the reload while the user is in
//!   Insert mode (the in-flight `ParsedPage` would be clobbered);
//!   mobile commits each mutation atomically via Tauri commands and
//!   can always apply immediately.
//!
//! What lives **here**, shared between every client:
//!
//! - Opening a fresh [`Workspace`] from the on-disk op log
//!   ([`SyncEngine::reload_workspace`]).
//! - Re-projecting a page's `.md` + sidecar from the materialised
//!   workspace ([`SyncEngine::reproject_page`]) so the on-disk view
//!   always reflects the merged op log.
//! - The shorthand that does both in sequence
//!   ([`SyncEngine::refresh_page`]) for the typical "peer fired,
//!   pull the new state in" path.
//!
//! Adding a new client means writing a detector + policy and calling
//! these three functions — never re-implementing them.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::journal::apply_page_md_with_sidecar;

/// Reachability snapshot for one known peer, derived from the
/// transport's own dial outcomes (never a fresh probe endpoint).
///
/// A GUI status indicator reads this from the running [`SyncTransport`]
/// (`peer_health`) instead of binding a second iroh endpoint with the
/// device identity — two endpoints sharing one `node_id` make the relay
/// route the inbound sync to the wrong one (see
/// `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerHealthSnapshot {
    /// Peer's node id (string form), as stored in `peers.json`.
    pub node_id: String,
    /// `true` if the most recent dial (boot connect, catch-up, or an
    /// inbound serve) to this peer succeeded.
    pub reachable: bool,
    /// Round-trip-ish duration of the last successful dial, in
    /// milliseconds. `None` when the peer has never been reached this
    /// session.
    pub last_rtt_ms: Option<u64>,
}

/// Transport abstraction — how ops travel between devices.
///
/// iCloud/filesystem: detects file changes via polling.
/// iroh: receives ops over QUIC streams, writes them to local FS, fires signal.
///
/// Both transports result in `ops-<peer>.jsonl` files landing on disk.
/// `SyncEngine::reload_workspace` picks them up identically regardless of transport.
pub trait SyncTransport: Send + Sync + 'static {
    /// Start the transport.
    ///
    /// Spawns whatever background tasks are needed (polling thread, iroh runtime, …).
    /// Sends on `tx` whenever peer ops have been written to the local `ops/`
    /// directory and the workspace is ready to reload.
    fn start(
        &self,
        workspace_root: std::path::PathBuf,
        actor: outl_core::id::ActorId,
        tx: std::sync::mpsc::Sender<()>,
    );

    /// Called after this device commits local ops to the op log.
    ///
    /// FileSyncTransport: no-op (iCloud/Syncthing carries the file).
    /// IrohSyncTransport: gossip-announces the new HLC to connected peers.
    fn announce_local_ops(&self, workspace_id: &str, hlc: Hlc);

    /// Graceful shutdown. Transport must stop background tasks.
    fn shutdown(&self);

    /// Force an immediate sync pass against every known peer, instead of
    /// waiting for the transport's own periodic catch-up tick.
    ///
    /// Drives the "pull to refresh" / "sync now" affordance in the GUI: the
    /// user wants the freshest state right now, so re-dial every peer (even
    /// healthy ones the catch-up loop would otherwise leave to gossip) and run
    /// the same delta sync. A no-op when the transport is down.
    ///
    /// The default does nothing — only transports that actually dial peers
    /// (iroh) have anything to force. [`FileSyncTransport`] relies on the OS
    /// file watcher / its own polling and has no peer to dial.
    fn sync_now(&self) {}

    /// Reachability snapshot for every known peer, derived from the
    /// transport's own dial outcomes.
    ///
    /// GUI status indicators call this instead of standing up a probe
    /// endpoint. The default returns an empty vector — only transports
    /// that actually dial peers (iroh) have anything to report;
    /// [`FileSyncTransport`] has no peer concept.
    fn peer_health(&self) -> Vec<PeerHealthSnapshot> {
        Vec::new()
    }
}

/// Filesystem / iCloud transport — the v0 implementation.
///
/// Detection: polls `ops/` every 2 s for peer file changes.
/// Delivery: no-op — iCloud Drive / Syncthing / shared FS carries the bytes.
#[derive(Debug, Clone, Default)]
pub struct FileSyncTransport;

impl SyncTransport for FileSyncTransport {
    fn start(
        &self,
        workspace_root: std::path::PathBuf,
        actor: outl_core::id::ActorId,
        tx: std::sync::mpsc::Sender<()>,
    ) {
        // Build a temporary engine just for snapshot polling.
        let engine = SyncEngine::new(workspace_root, actor);
        std::thread::Builder::new()
            .name("outl-file-sync".into())
            .spawn(move || {
                let mut last = engine.snapshot_peers();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let current = engine.snapshot_peers();
                    if current != last {
                        last = current;
                        if tx.send(()).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("failed to spawn outl-file-sync thread");
    }

    fn announce_local_ops(&self, _workspace_id: &str, _hlc: Hlc) {
        // File transport: the file is already on disk; the peer's poller will
        // notice it on the next 2 s tick. Nothing to announce explicitly.
    }

    fn shutdown(&self) {
        // The polling thread exits when the mpsc Sender is dropped by the caller.
    }
}

/// Snapshot of one `ops-<actor>.jsonl` file. Detectors compare these
/// across polls to decide whether to fire a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsFileSnapshot {
    /// Filename inside `<root>/ops/`.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time as reported by the filesystem.
    pub mtime: SystemTime,
}

/// Owns the workspace root + actor identity for one running client.
///
/// Stateless beyond those two fields — every method opens what it
/// needs and returns it. Multiple instances pointing at the same root
/// are safe (the underlying op log is append-only per actor).
#[derive(Clone)]
pub struct SyncEngine {
    workspace_root: PathBuf,
    actor: ActorId,
    transport: Option<std::sync::Arc<dyn SyncTransport>>,
}

impl std::fmt::Debug for SyncEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncEngine")
            .field("workspace_root", &self.workspace_root)
            .field("actor", &self.actor)
            .field(
                "transport",
                &self.transport.as_ref().map(|_| "<dyn SyncTransport>"),
            )
            .finish()
    }
}

impl SyncEngine {
    /// Bind to a workspace root + actor.
    pub fn new(workspace_root: PathBuf, actor: ActorId) -> Self {
        Self {
            workspace_root,
            actor,
            transport: None,
        }
    }

    /// Bind to a workspace root + actor with an explicit transport.
    ///
    /// `transport.start()` is NOT called here; call `start_transport` once the
    /// caller's notification channel is ready.
    pub fn with_transport(
        workspace_root: PathBuf,
        actor: ActorId,
        transport: Box<dyn SyncTransport>,
    ) -> Self {
        Self {
            workspace_root,
            actor,
            transport: Some(std::sync::Arc::from(transport)),
        }
    }

    /// Start the transport's background tasks.
    ///
    /// Calls `transport.start(workspace_root, actor, tx)` if a transport is set.
    /// No-op when no transport was configured (callers manage detection themselves).
    pub fn start_transport(&self, peer_ready_tx: std::sync::mpsc::Sender<()>) {
        if let Some(t) = &self.transport {
            t.start(self.workspace_root.clone(), self.actor, peer_ready_tx);
        }
    }

    /// Announce new local ops to connected peers.
    ///
    /// Calls `transport.announce_local_ops` if a transport is set.
    /// No-op when no transport was configured.
    pub fn announce_local_ops(&self, workspace_id: &str, hlc: Hlc) {
        if let Some(t) = &self.transport {
            t.announce_local_ops(workspace_id, hlc);
        }
    }

    /// Workspace root this engine talks to.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Actor id this engine writes as.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Open a fresh `Workspace` from disk. The caller swaps it in
    /// place of whatever stale workspace they were holding.
    ///
    /// Reads every `ops-*.jsonl` in `<root>/ops/`, merges them by
    /// HLC, and replays the resulting ordered sequence into the
    /// materialised tree.
    pub fn reload_workspace(&self) -> Result<Workspace, ActionError> {
        let ops_dir = self.workspace_root.join("ops");
        let storage = JsonlStorage::open(ops_dir, self.actor)
            .map_err(|e| ActionError::Io(std::io::Error::other(format!("jsonl open: {e}"))))?;
        let workspace = Workspace::open_with_storage(
            self.actor,
            Box::new(storage),
            Some(self.workspace_root.clone()),
        )?;
        Ok(workspace)
    }

    /// Re-project a single page's `.md` + sidecar from `workspace`.
    ///
    /// Safe to call after [`Self::reload_workspace`] so the on-disk
    /// `.md` reflects the merged state (own ops + peer ops). Other
    /// pages get re-projected lazily when the user navigates to them.
    pub fn reproject_page(
        &self,
        workspace: &Workspace,
        page_id: NodeId,
    ) -> Result<(), ActionError> {
        apply_page_md_with_sidecar(workspace, &self.workspace_root, page_id)?;
        Ok(())
    }

    /// Reload the workspace **and** re-project the focused page in
    /// one go. Returns the new workspace.
    ///
    /// This is the typical entry point for the "peer fired, pull
    /// new state" path. Clients call this from their detector once
    /// they have decided it's safe (e.g. user is not mid-edit).
    pub fn refresh_page(&self, page_id: NodeId) -> Result<Workspace, ActionError> {
        let ws = self.reload_workspace()?;
        self.reproject_page(&ws, page_id)?;
        Ok(ws)
    }

    /// List every `ops-*.jsonl` file in the workspace with size and
    /// mtime. Used by polling detectors (TUI) to decide whether a
    /// peer wrote since the last check.
    ///
    /// Returns an empty vec when `<root>/ops/` is absent (workspace
    /// is using the SQLite backend, or hasn't been initialised yet).
    pub fn snapshot(&self) -> Vec<OpsFileSnapshot> {
        let ops_dir = self.workspace_root.join("ops");
        let Ok(entries) = std::fs::read_dir(&ops_dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            out.push(OpsFileSnapshot {
                name,
                size: meta.len(),
                mtime,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Like [`Self::snapshot`] but **excludes this engine's own
    /// `ops-<actor>.jsonl` file**.
    ///
    /// This is what polling detectors should call: a TUI that
    /// reacted to its *own* `.jsonl` growing would reload the
    /// workspace, re-project the page, and overwrite the `.md` the
    /// user just edited — closing a destructive loop where each save
    /// triggers a reload that races the user's next save. Reacting
    /// only to peer files avoids that entirely.
    pub fn snapshot_peers(&self) -> Vec<OpsFileSnapshot> {
        let own = format!("ops-{}.jsonl", self.actor);
        self.snapshot()
            .into_iter()
            .filter(|f| f.name != own)
            .collect()
    }

    /// Find every `.md` under `journals/` and `pages/` that the op
    /// log doesn't reflect yet.
    ///
    /// Two reasons a `.md` ends up orphaned:
    ///
    /// 1. **Bootstrap.** The file was just dropped in by an importer
    ///    (Roam → outl, copy from a Logseq graph), by a peer that
    ///    only ships the projection, or by an external editor like
    ///    vim. No sidecar exists yet.
    /// 2. **External edit.** The user opened the `.md` outside the
    ///    TUI / mobile (vim, VS Code, Finder Quick Look) and saved.
    ///    The sidecar still references the old contents, so
    ///    `last_synced_hash` no longer matches.
    ///
    /// Both look identical to a peer reading via `read_page_view` —
    /// the outline comes out empty or stale. Running `reconcile_md`
    /// on the file resolves both: it emits Create / Move / Edit ops
    /// for whatever the file actually contains and rewrites the
    /// sidecar.
    ///
    /// This call is **read-only** and cheap (one `file_hash` per
    /// `.md`, one sidecar JSON parse). Safe to run on a background
    /// thread; clients call `outl_md::reconcile::reconcile_md` on
    /// the main thread once they have the workspace handle available.
    pub fn scan_for_orphans(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for sub in ["journals", "pages"] {
            scan_dir(&self.workspace_root.join(sub), &mut out);
        }
        out
    }
}

fn scan_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if needs_reconcile(&path) {
            out.push(path);
        }
    }
}

/// `true` when the `.md` at `md_path` is not reflected in its
/// sidecar: either the sidecar is missing, its `last_synced_hash`
/// doesn't match the file's current hash, or page-level properties
/// (`type::`, `pinned::`, `icon::`, …) haven't been propagated into
/// the op log yet (legacy sidecars predating
/// `diff_to_ops_with_page_props`).
///
/// The version check is a forward-compatible migration trigger.
/// Every bump of [`outl_md::sidecar::CURRENT_PIPELINE_VERSION`]
/// forces every legacy sidecar (with a lower value, including the
/// default `0` from `#[serde(default)]` on payloads written before
/// the field existed) through `reconcile_md` once.
/// The reconcile emits the missing ops on the page root and stamps
/// the new version in the rewritten sidecar.
/// Subsequent scans skip the page until the next pipeline bump.
/// Without this, pages authored via fixtures, imports, or external
/// editors keep their `type:: person` only in the rendered `.md`,
/// and the desktop's `@` autocomplete (which reads from the CRDT
/// tree) silently disagrees with the TUI's (which reads
/// `WorkspaceIndex`'s parse of the same `.md`).
fn needs_reconcile(md_path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(md_path) else {
        return false;
    };
    let current = outl_md::sidecar::file_hash(&text);
    let sidecar_path = outl_md::resolve_sidecar_path(md_path);
    match outl_md::sidecar::read(&sidecar_path) {
        Ok(sc) => {
            sc.last_synced_hash != current
                || sc.pipeline_version < outl_md::sidecar::CURRENT_PIPELINE_VERSION
        }
        Err(_) => true, // sidecar missing or unreadable → orphan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn snapshot_returns_empty_when_no_ops_dir() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
        assert!(engine.snapshot().is_empty());
    }

    #[test]
    fn snapshot_lists_ops_files_and_skips_others() {
        let tmp = TempDir::new().unwrap();
        let ops = tmp.path().join("ops");
        std::fs::create_dir(&ops).unwrap();
        std::fs::write(ops.join("ops-A.jsonl"), b"x").unwrap();
        std::fs::write(ops.join("ops-B.jsonl"), b"yz").unwrap();
        std::fs::write(ops.join("README.md"), b"hello").unwrap();

        let actor = ActorId::new();
        let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
        let snap = engine.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "ops-A.jsonl");
        assert_eq!(snap[0].size, 1);
        assert_eq!(snap[1].name, "ops-B.jsonl");
        assert_eq!(snap[1].size, 2);
    }

    #[test]
    fn reload_workspace_opens_empty_workspace_when_no_ops() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
        let ws = engine.reload_workspace().expect("should open clean");
        // Materialised tree starts empty.
        assert_eq!(
            crate::tree::children_of(&ws, outl_core::id::NodeId::root()).len(),
            0
        );
    }
}
