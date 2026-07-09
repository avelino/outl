//! Render `ParsedPage` → `.md` → disk → reconcile.
//!
//! Three entry points, all converging on `outl_md::write_atomic` +
//! `outl_md::reconcile_md`:
//!
//! - [`App::save`] — persist the in-memory `app.page` to the
//!   currently-opened path. The hot path: every Insert commit,
//!   structural op, paste lands here eventually.
//! - [`App::save_page_with`] — persist a *different* page than the
//!   one the user is looking at. Used by cross-page backlink edits.
//! - [`App::save_page`] — convenience wrapper over `save_page_with`
//!   that always rebuilds the index. Kept for callers that don't
//!   want to think about the optimistic-patch flag.
//!
//! After each save we invalidate `backlinks_cache` (the page just
//! changed, the cached `Vec<Backlink>` for the current slug is
//! stale) and resync `last_mtime` so the polling loop doesn't
//! mistake our own write for an external edit.

use crate::outline_ops::flat_count;
use crate::state::{App, ToastKind};
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;
use std::path::Path;

use super::file_mtime;

impl App {
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

    /// Same as [`Self::save_page`] but lets the caller opt out of
    /// refreshing the workspace index for this page.
    ///
    /// **The `true` branch is incremental.** It calls
    /// [`outl_md::index::WorkspaceIndex::patch_page`] on `path` only,
    /// which is `O(blocks in this page)` — *not* a full workspace
    /// rescan. Pick `true` whenever the write changes anything the
    /// index tracks (block refs `((blk-XXXXXX))`, page title, icon,
    /// pinned flag); pick `false` only when you've already patched
    /// the index by hand and the call would just redo the same work.
    ///
    /// Whether or not the index is patched, `App.backlinks_cache` is
    /// invalidated unconditionally because backlinks live outside
    /// the index now.
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
        // Reconciling the source page may have touched blocks that
        // mention the current view's slug.
        self.invalidate_backlinks_cache();
        if path == self.current_path() {
            self.last_mtime = file_mtime(path);
        }
        // Post-commit hook (cross-page save): same as `save()`. Without it a
        // backlink edit committed locally but never woke peers, so the change
        // only propagated on the catch-up re-sync instead of in real time.
        // The payload slug is informational — the receiver pulls by vector
        // clock — so deriving it from the saved path keeps it honest.
        if let Some(transport) = &self.sync_transport {
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let hlc = self.hlc.next();
            transport.announce_local_ops(&slug, hlc);
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
            self.toast(ToastKind::Error, format!("save failed: {e}"));
            return;
        }
        match reconcile_md(
            &mut self.workspace,
            &self.hlc,
            &path,
            Some(&self.orphans_log),
        ) {
            Ok(_) => self.status.clear(),
            Err(e) => self.toast(ToastKind::Error, format!("reconcile failed: {e}")),
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
        // The page just changed; any cached backlink list is stale.
        self.invalidate_backlinks_cache();
        // Update mtime AFTER the write so the polling loop doesn't
        // mistake our own save for an external edit.
        self.last_mtime = file_mtime(&path);
        self.last_saved_at = Some(std::time::Instant::now());
        // Re-run query / auto-run blocks — workspace state changed,
        // so results may be different now.
        self.run_auto_run_blocks();
        // Post-commit hook: tell the transport new local ops landed.
        // No-op for FileSyncTransport (the file is already on disk and
        // a peer's poller will notice it); IrohSyncTransport gossips
        // the HLC so connected peers pull the new ops over QUIC.
        if let Some(transport) = &self.sync_transport {
            let workspace_id = self.current_slug();
            // The committed ops carry HLCs already minted by
            // `reconcile_md`; `next()` returns a fresh HLC that sorts
            // after all of them, which is what peers need to know the
            // high-water mark to pull up to.
            let hlc = self.hlc.next();
            transport.announce_local_ops(&workspace_id, hlc);
        }
    }
}
