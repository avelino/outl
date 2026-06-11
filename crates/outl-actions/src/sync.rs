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

use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::journal::apply_page_md_with_sidecar;

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
#[derive(Clone, Debug)]
pub struct SyncEngine {
    workspace_root: PathBuf,
    actor: ActorId,
}

impl SyncEngine {
    /// Bind to a workspace root + actor.
    pub fn new(workspace_root: PathBuf, actor: ActorId) -> Self {
        Self {
            workspace_root,
            actor,
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
/// The propagation check is a one-shot migration trigger: the next
/// `reconcile_md` emits the missing `Op::SetProp`s on the page root,
/// flips `pipeline_v2_complete = true` in the rewritten sidecar, and
/// the page is skipped on every subsequent scan. Without it, pages
/// authored via fixtures, imports, or external editors keep their
/// `type:: person` only in the rendered `.md` — and the desktop's
/// `@` autocomplete (which reads from the CRDT tree) silently
/// disagrees with the TUI's (which reads `WorkspaceIndex`'s parse of
/// the same `.md`).
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
