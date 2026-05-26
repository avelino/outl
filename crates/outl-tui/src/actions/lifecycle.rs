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
            index: WorkspaceIndex::default(),
            index_rx: None,
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
