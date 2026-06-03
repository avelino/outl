//! Background watchers that pull peer-written changes into the
//! workspace: the per-actor `ops-<actor>.jsonl` poller and the
//! orphan-`.md` scanner.
//!
//! Both run on worker threads and communicate with the event loop
//! over `std::sync::mpsc` channels. The main loop drains them every
//! tick and either swaps the workspace (peer ops landed) or runs
//! `reconcile_md` (an `.md` showed up that the op log doesn't know
//! about yet, e.g. a Roam import or a vim save).
//!
//! Both pollers defer when the user is in Insert mode — the
//! in-flight `ParsedPage` would be lost if we swapped the workspace
//! mid-keystroke. See `App.pending_reload` for how `commit_insert`
//! drains the queued reload.

use crate::state::App;

impl App {
    /// Spawn the background poller that watches `<root>/ops/` for new
    /// `.jsonl` entries written by peers (mobile, another TUI).
    ///
    /// The poller delegates change detection to
    /// [`outl_actions::SyncEngine::snapshot`] every 2 s and sends a
    /// `()` over the channel stored in `App.jsonl_rx` whenever the
    /// snapshot differs from the previous one. The event loop drains
    /// the channel via [`Self::poll_jsonl_updates`] and asks
    /// `SyncEngine` to reopen the workspace + reproject the focused
    /// page.
    ///
    /// Workspaces using the SQLite backend don't have `ops/`; the
    /// snapshot stays empty forever and the poller never fires.
    pub(crate) fn spawn_jsonl_poller(&mut self) {
        // Gate on the configured backend, not on whether `ops/` exists
        // on disk. A workspace running SQLite that happens to have an
        // `ops/` directory (manual mkdir, leftover from a partial
        // migration) would otherwise get its workspace silently swapped
        // for an empty `JsonlStorage` one on the next peer-poll fire,
        // wiping the UI even though the SQLite log is intact.
        if !self.shared_workspace {
            return;
        }
        let ops_dir = self.workspace_root.join("ops");
        if !ops_dir.is_dir() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let engine = outl_actions::SyncEngine::new(self.workspace_root.clone(), self.hlc.actor());
        std::thread::Builder::new()
            .name("outl-jsonl-poll".into())
            .spawn(move || {
                use std::time::Duration;
                // Watch only peer jsonl files. Reacting to our own
                // file (which we just touched on save) would close a
                // destructive loop: every commit would fire a reload
                // that re-projects `.md` and races the next commit.
                let mut last = engine.snapshot_peers();
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let current = engine.snapshot_peers();
                    if current != last {
                        last = current;
                        if tx.send(()).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("spawning the jsonl poll worker thread should not fail");
        self.jsonl_rx = Some(rx);
    }

    /// Spawn the background scanner that finds `.md` files whose op
    /// log entry doesn't exist yet (no sidecar) or is out of sync
    /// (sidecar `last_synced_hash` differs from the file's current
    /// hash). The scanner sends the list back over a channel; the
    /// event loop drains it via [`Self::poll_orphan_md_updates`] and
    /// runs `reconcile_md` on the main thread (where the workspace
    /// handle lives).
    ///
    /// Runs an immediate scan on spawn (catches bootstrap / imports
    /// like the Roam dump we just dropped in), then re-scans every
    /// 10 s to pick up external edits (vim, VS Code) or peer-written
    /// `.md` files that arrived without a sidecar.
    pub(crate) fn spawn_orphan_md_scanner(&mut self) {
        let engine = outl_actions::SyncEngine::new(self.workspace_root.clone(), self.hlc.actor());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("outl-orphan-md".into())
            .spawn(move || {
                use std::time::Duration;
                loop {
                    let orphans = engine.scan_for_orphans();
                    if !orphans.is_empty() && tx.send(orphans).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(10));
                }
            })
            .expect("spawning the orphan md scanner thread should not fail");
        self.orphan_md_rx = Some(rx);
    }

    /// Drain the orphan-md channel and reconcile every `.md` the
    /// scanner flagged. Returns `true` when something was reconciled
    /// so the event loop can request a redraw + a fresh index
    /// rebuild (new blocks change backlinks / page counts).
    pub(crate) fn poll_orphan_md_updates(&mut self) -> bool {
        let Some(rx) = &self.orphan_md_rx else {
            return false;
        };
        let mut all_paths: Vec<std::path::PathBuf> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(paths) => all_paths.extend(paths),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.orphan_md_rx = None;
                    break;
                }
            }
        }
        if all_paths.is_empty() {
            return false;
        }
        // Skip when the user is mid-edit: reconcile reads `.md`,
        // mutates the workspace, and writes the sidecar. Doing that
        // mid-Insert could race the user's buffer. We'll see the
        // same orphans on the next 10 s tick.
        if matches!(self.mode, crate::state::Mode::Insert { .. }) {
            return false;
        }
        let orphans_log = self.orphans_log.clone();
        let mut reconciled = 0usize;
        for path in &all_paths {
            if let Ok(_report) = outl_md::reconcile::reconcile_md(
                &mut self.workspace,
                &self.hlc,
                path,
                Some(&orphans_log),
            ) {
                reconciled += 1;
            }
        }
        if reconciled > 0 {
            // New blocks in the workspace mean the workspace index
            // (backlinks, page list) is stale.
            self.spawn_index_rebuild();
        }
        true
    }

    /// `true` while a peer-ops poller is registered. Used by the event
    /// loop to keep `event::poll` from sleeping past the next
    /// expected reload window.
    #[allow(dead_code)]
    pub(crate) fn has_jsonl_watcher(&self) -> bool {
        self.jsonl_rx.is_some()
    }

    /// Non-blocking drain of the peer-ops channel.
    ///
    /// When the poller signalled (one or more files changed):
    ///
    /// - **In Insert mode**, the channel is drained but the reload is
    ///   *deferred*: the user is in the middle of an edit that has
    ///   not been committed to the op log yet. Reopening the
    ///   workspace + reparsing `.md` right now would discard the
    ///   in-flight `ParsedPage`. We flip `pending_reload` so the
    ///   next `commit_insert` can fold the peer ops in.
    /// - **Outside Insert mode**, we reopen the workspace so the
    ///   merged op log shows up immediately and reparse the on-disk
    ///   `.md` so the rendered outline reflects whatever the peer
    ///   wrote (the peer is responsible for keeping `.md` + sidecar
    ///   coherent on its own write).
    ///
    /// Notably this **does not** call `apply_page_md_with_sidecar` —
    /// the TUI does not own the `.md` from the op log alone. Its
    /// ParsedPage is the source of truth between commits and we let
    /// `reconcile_md` reconstruct ops on commit. Rewriting `.md` from
    /// the workspace here would overwrite both peer-side edits the
    /// op log doesn't capture and the user's in-flight buffer.
    pub(crate) fn poll_jsonl_updates(&mut self) -> bool {
        let Some(rx) = &self.jsonl_rx else {
            return false;
        };
        let mut any = false;
        loop {
            match rx.try_recv() {
                Ok(_) => any = true,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.jsonl_rx = None;
                    break;
                }
            }
        }
        if !any {
            return false;
        }
        if matches!(self.mode, crate::state::Mode::Insert { .. }) {
            self.pending_reload = true;
            return false;
        }
        self.reload_workspace_from_disk();
        true
    }

    /// Reopen the workspace from disk, re-project the current page's
    /// `.md` + sidecar to match the merged op log, and reparse it so
    /// the in-memory `ParsedPage` shows peer edits.
    ///
    /// Caller (the poller's drain path) is responsible for not
    /// invoking this while the user is in Insert mode — the
    /// in-flight AST would be lost. See [`Self::poll_jsonl_updates`]
    /// for the deferral logic and `commit_insert` for the drain.
    pub(crate) fn reload_workspace_from_disk(&mut self) {
        let engine = outl_actions::SyncEngine::new(self.workspace_root.clone(), self.hlc.actor());
        // Resolve the focused page id *before* swapping the
        // workspace; the slug→id lookup needs a stable workspace.
        let focused_page = self.current_page_meta_id();
        let fresh = match (engine.reload_workspace(), focused_page) {
            (Ok(ws), Some(page_id)) => {
                let _ = engine.reproject_page(&ws, page_id);
                ws
            }
            (Ok(ws), None) => ws,
            (Err(_), _) => return,
        };
        self.workspace = fresh;
        // Peer ops landed; any cached backlink list is stale.
        self.invalidate_backlinks_cache();
        self.load_current_no_autorun();
    }

    /// Best-effort lookup of the focused page's `NodeId`, when the
    /// current view is a page (journal or named). Returns `None`
    /// when the current slug isn't in the workspace yet (e.g. a
    /// freshly opened journal date the user just navigated to).
    pub(super) fn current_page_meta_id(&self) -> Option<outl_core::id::NodeId> {
        let slug = self.current_slug();
        outl_actions::find_by_slug(&self.workspace, &slug)
    }
}
