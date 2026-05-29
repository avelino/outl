//! Construction, disk I/O, and external-edit polling for [`App`].
//!
//! These methods own the "where does the truth live" answers: the
//! workspace root, the page list, the in-memory AST, the last
//! file-mtime we observed. Everything else routes through them.

use crate::commands::CommandRegistry;
use crate::outline_ops::flat_count;
use crate::state::{App, Focus, Mode, View};
use crate::theme::Theme;
use anyhow::{Context, Result};
use chrono::Local;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::{parse, OutlineNode, ParsedPage};
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;
use std::fs;
use std::path::{Path, PathBuf};

impl App {
    pub(crate) fn new(
        workspace_root: PathBuf,
        workspace: Workspace,
        actor: ActorId,
        theme: Theme,
        shared_workspace: bool,
    ) -> Result<Self> {
        let orphans_log = workspace_root.join(".outl").join("orphans.log");
        let mut s = Self {
            hlc: HlcGenerator::new(actor),
            workspace_root,
            workspace,
            orphans_log,
            view: View::Journal(Local::now().date_naive()),
            page: ParsedPage::default(),
            selected: 0,
            flat_len: 0,
            cursor_col: 0,
            page_list: Vec::new(),
            mode: Mode::Normal,
            show_help: false,
            help_tab: 0,
            help_scroll: 0,
            pending_chord: None,
            status: String::new(),
            overlay: None,
            autocomplete: None,
            last_search: None,
            yank_register: Vec::new(),
            last_yanked_ref: None,
            index: WorkspaceIndex::default(),
            index_rx: None,
            shared_workspace,
            jsonl_rx: None,
            pending_reload: false,
            orphan_md_rx: None,
            show_backlinks: true,
            show_sidebar: false,
            sidebar_focus: None,
            sidebar_cursor: 0,
            recent_paths: Vec::new(),
            toasts: Vec::new(),
            focus: Focus::Outline,
            scroll_y: 0,
            viewport_height: 0,
            last_mtime: None,
            last_saved_at: None,
            undo: Vec::new(),
            redo: Vec::new(),
            theme,
            exec_registry: RuntimeRegistry::with_builtins(),
            command_registry: CommandRegistry::with_builtins(),
        };
        s.refresh_page_list();
        s.ensure_view_file_exists()?;
        s.load_current();
        // Build the workspace index off the critical path so the TUI
        // can paint immediately. Backlinks/icons fill in once the
        // worker thread completes (usually < 100ms for small
        // workspaces, longer for big ones — but the user is already
        // typing).
        s.spawn_index_rebuild();
        s.spawn_jsonl_poller();
        s.spawn_orphan_md_scanner();
        Ok(s)
    }

    /// Synchronous workspace-index build. Kept around as escape hatch
    /// for code paths that genuinely need the index *right now*
    /// (none today — production callers use
    /// [`Self::spawn_index_rebuild`]). Avoid in hot paths; it blocks
    /// the event loop while it walks the whole workspace.
    #[allow(dead_code)]
    pub(crate) fn rebuild_index(&mut self) {
        self.index = WorkspaceIndex::build(&self.workspace_root);
        // Cancel any pending background build — we just produced a
        // fresher result. The thread keeps running but its send goes
        // to a dropped receiver.
        self.index_rx = None;
    }

    /// Kick off a workspace-index rebuild on a worker thread.
    ///
    /// Replaces any in-flight build (the previous thread's result is
    /// dropped on arrival). The next call to
    /// [`Self::poll_index_updates`] swaps in the result when ready.
    pub(crate) fn spawn_index_rebuild(&mut self) {
        let root = self.workspace_root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("outl-index".into())
            .spawn(move || {
                let idx = WorkspaceIndex::build(&root);
                // If the receiver was dropped (newer spawn superseded
                // us), this just returns Err — fine.
                let _ = tx.send(idx);
            })
            .expect("spawning the index worker thread should not fail");
        self.index_rx = Some(rx);
    }

    /// `true` while a workspace-index rebuild is in flight on a worker
    /// thread. The event loop uses this to shorten its
    /// `event::poll` timeout so the freshly-built index shows up in
    /// the UI within a frame, not after the next 750 ms key timeout.
    pub(crate) fn has_pending_index(&self) -> bool {
        self.index_rx.is_some()
    }

    /// Non-blocking check: if the background index build has finished,
    /// swap the result into `self.index`. Returns `true` when a swap
    /// happened so the event loop can request a redraw.
    pub(crate) fn poll_index_updates(&mut self) -> bool {
        let Some(rx) = &self.index_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(idx) => {
                self.index = idx;
                self.index_rx = None;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Worker died (panic, OOM, ...). Stop polling; the
                // TUI keeps working with the current (possibly empty)
                // index.
                self.index_rx = None;
                false
            }
        }
    }

    /// Full workspace re-read: page list, current view's `.md` from
    /// disk, and the derived index. Used when an external process
    /// (another editor, `outl serve`) may have changed files under us.
    pub(crate) fn refresh_workspace(&mut self) {
        self.refresh_page_list();
        self.load_current();
        self.spawn_index_rebuild();
    }

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
        self.load_current_no_autorun();
    }

    /// Best-effort lookup of the focused page's `NodeId`, when the
    /// current view is a page (journal or named). Returns `None`
    /// when the current slug isn't in the workspace yet (e.g. a
    /// freshly opened journal date the user just navigated to).
    fn current_page_meta_id(&self) -> Option<outl_core::id::NodeId> {
        let slug = self.current_slug();
        outl_actions::find_by_slug(&self.workspace, &slug)
    }

    pub(crate) fn refresh_page_list(&mut self) {
        let pages_dir = self.workspace_root.join("pages");
        let mut entries: Vec<PathBuf> = walkdir::WalkDir::new(&pages_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().is_some_and(|x| x == "md")
                    && !e.file_name().to_string_lossy().starts_with('.')
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        entries.sort();
        self.page_list = entries;
    }

    /// Create the underlying `.md` file (with a single empty block) if it
    /// doesn't already exist. Ensures the editor always has a target.
    pub(crate) fn ensure_view_file_exists(&mut self) -> Result<()> {
        let path = self.current_path();
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
        }
        // Seed an empty outline (one empty bullet so cursor has a home).
        outl_md::write_atomic(&path, b"- \n")
            .with_context(|| format!("create {}", path.display()))?;
        // Reconcile so the sidecar exists with stable IDs.
        let _ = reconcile_md(
            &mut self.workspace,
            &self.hlc,
            &path,
            Some(&self.orphans_log),
        );
        self.refresh_page_list();
        Ok(())
    }

    /// Reparse the current page from disk + (re)trigger auto-run.
    ///
    /// Most navigation paths call this. The auto-run pass is what
    /// makes `auto-run::` blocks "feel live" — open a journal, the
    /// computed cells run themselves.
    pub(crate) fn load_current(&mut self) {
        self.load_current_no_autorun();
        self.run_auto_run_blocks();
    }

    /// Bare reparse from disk, **without** the auto-run pass.
    ///
    /// Internal: used by `run_auto_run_blocks` itself after it writes
    /// new result subblocks, to refresh the in-memory AST without
    /// firing another round of auto-runs (which would loop forever
    /// when something stamps a fresh hash but the hash doesn't stick
    /// for some reason).
    pub(crate) fn load_current_no_autorun(&mut self) {
        let path = self.current_path();
        let text = fs::read_to_string(&path).unwrap_or_default();
        self.page = parse(&text);
        self.flat_len = flat_count(&self.page.blocks);
        if self.selected >= self.flat_len {
            self.selected = self.flat_len.saturating_sub(1);
        }
        // Any view change snaps focus back to the outline. Carrying a
        // stale `Focus::Backlink { idx, … }` across pages would point
        // at the wrong backlink list (the new page has its own).
        self.focus = Focus::Outline;
        // Snapshot the file's mtime so the polling loop can tell when
        // an *external* edit lands (vs. our own save).
        self.last_mtime = file_mtime(&path);
        // Anchor the header's freshness chip ("⟳ 2s ago") to the load
        // instant: from the user's perspective, what's on screen *is*
        // what's on disk at this moment.
        self.last_saved_at = Some(std::time::Instant::now());
        self.touch_recent(&path);
    }

    /// Move `path` to the front of the recent-paths LRU. Used by
    /// `load_current_no_autorun` so any view switch (journal, page,
    /// switch-overlay open) keeps the sidebar's `Recent` list in
    /// sync with what the user actually touched.
    ///
    /// Capped at 20 entries — anything past that drops off the back,
    /// which is enough for a session's worth of context without
    /// turning into infinite scroll.
    pub(crate) fn touch_recent(&mut self, path: &Path) {
        const RECENT_MAX: usize = 20;
        self.recent_paths.retain(|p| p != path);
        self.recent_paths.insert(0, path.to_path_buf());
        self.recent_paths.truncate(RECENT_MAX);
    }

    /// Detect that the current `.md` was edited by another process
    /// (vim, vscode, `outl serve`) since we last loaded or saved it,
    /// and pull the new content in.
    ///
    /// Behaviour:
    /// - No mtime change → returns `false`, nothing happens.
    /// - Changed and we're in Insert mode → returns `true` and writes
    ///   a warning to the status line. We refuse to clobber the
    ///   user's in-flight edit; they decide how to resolve.
    /// - Changed and we're in Normal/Visual → silently reload + reset
    ///   the selection clamp + rebuild the workspace index. Returns
    ///   `true` so the caller knows a redraw is in order.
    pub(crate) fn check_external_changes(&mut self) -> bool {
        let path = self.current_path();
        let Some(disk) = file_mtime(&path) else {
            return false;
        };
        let Some(last) = self.last_mtime else {
            // First time seeing the file — record and move on.
            self.last_mtime = Some(disk);
            return false;
        };
        if disk <= last {
            return false;
        }

        if matches!(self.mode, Mode::Insert { .. }) {
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
            // Toast the warning instead of the status line: this is a
            // conflict the user needs to acknowledge, but the footer
            // hint is more useful for the chord prompt next to it.
            self.toast(
                crate::state::ToastKind::Warning,
                format!("external edit on {fname} — Ctrl+L to reload"),
            );
            // Don't update last_mtime — we'll keep warning until the
            // user explicitly resolves it.
            return true;
        }

        self.load_current();
        // External edit changes one file — incremental patch is enough
        // to bring the index in sync. (A full rebuild is the wrong
        // tool: it would block on rescanning every other page that
        // didn't change.)
        let cur_path = self.current_path();
        self.index.patch_page(&cur_path, &self.page);
        self.toast(crate::state::ToastKind::Info, "reloaded from disk");
        true
    }

    /// Render an arbitrary `ParsedPage` to `path` and reconcile.
    ///
    /// Used by the cross-page commit route (editing a backlink): the
    /// caller has a working copy of a *different* page than
    /// `current_path()`, and wants to persist it without disturbing
    /// `app.view` / `app.page`. Updates status on failure, rebuilds
    /// the workspace index, and keeps `last_mtime` honest when the
    /// path happens to coincide with the open view.
    #[allow(dead_code)]
    pub(crate) fn save_page(&mut self, path: &Path, page: &ParsedPage) {
        self.save_page_with(path, page, true);
    }

    /// Same as [`Self::save_page`] but skips the index rebuild when
    /// the caller has already patched the in-memory state optimistically.
    ///
    /// On `rebuild_index == true`, the cheaper [`patch_page`] is used
    /// instead of the workspace-wide rescan — single-file work
    /// proportional to the page's block count, not to the workspace
    /// size.
    ///
    /// [`patch_page`]: outl_md::index::WorkspaceIndex::patch_page
    pub(crate) fn save_page_with(&mut self, path: &Path, page: &ParsedPage, rebuild_index: bool) {
        let md = render(page);
        if let Err(e) = outl_md::write_atomic(path, md.as_bytes()) {
            self.status = format!("save (source) failed: {e}");
            return;
        }
        if let Err(e) = reconcile_md(
            &mut self.workspace,
            &self.hlc,
            path,
            Some(&self.orphans_log),
        ) {
            self.status = format!("reconcile (source) failed: {e}");
            return;
        }
        self.status.clear();
        if rebuild_index {
            // Incremental: just re-index this one page. Full workspace
            // rescans are reserved for cold start and `Ctrl+L`.
            self.index.patch_page(path, page);
        }
        if path == self.current_path() {
            self.last_mtime = file_mtime(path);
        }
    }

    /// Render the in-memory `page` back to disk and reconcile.
    ///
    /// All writes go through [`outl_md::write_atomic`] so a crash
    /// between render and rename cannot leave a half-written `.md`.
    pub(crate) fn save(&mut self) {
        let path = self.current_path();
        let md = render(&self.page);
        if let Err(e) = outl_md::write_atomic(&path, md.as_bytes()) {
            self.toast(crate::state::ToastKind::Error, format!("save failed: {e}"));
            return;
        }
        match reconcile_md(
            &mut self.workspace,
            &self.hlc,
            &path,
            Some(&self.orphans_log),
        ) {
            Ok(_) => self.status.clear(),
            Err(e) => self.toast(
                crate::state::ToastKind::Error,
                format!("reconcile failed: {e}"),
            ),
        }
        self.flat_len = flat_count(&self.page.blocks);
        if self.flat_len == 0 {
            // Always keep at least one empty bullet so the cursor has a home.
            self.page.blocks.push(OutlineNode::default());
            self.flat_len = 1;
            let _ = outl_md::write_atomic(&path, render(&self.page).as_bytes());
        }
        self.selected = self.selected.min(self.flat_len.saturating_sub(1));
        // Incremental re-index: only this page changed, so just
        // refresh its entries (backlinks, title, icon). Cheap and
        // synchronous — no waiting for a worker to rescan the whole
        // workspace.
        self.index.patch_page(&path, &self.page);
        // Update mtime AFTER the write so the polling loop doesn't
        // mistake our own save for an external edit.
        self.last_mtime = file_mtime(&path);
        self.last_saved_at = Some(std::time::Instant::now());
    }
}

/// Last-modified time of `path`, or `None` if the file isn't there
/// (or we lack permission to stat it). Used by the external-edit
/// polling loop and by `save`/`load_current` to keep `last_mtime` in
/// sync with what we just wrote/read.
fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
