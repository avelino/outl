//! Switch the currently-opened view (journal or page) and pull its
//! state from disk: parse the `.md`, rehydrate the sidecar's stable
//! ids, refresh the page list, push the path through the recent-LRU.
//!
//! Loading also clears the caches that depend on the open view —
//! `backlinks_cache` (different slug owns a different list) and the
//! focused `Focus` (a stale `Focus::Backlink` would point at the
//! previous page's backlinks list).

use crate::outline_ops::flat_count;
use crate::state::{App, Focus};
use anyhow::{Context, Result};
use outl_md::parse::parse;
use outl_md::reconcile::reconcile_md;
use std::fs;
use std::path::{Path, PathBuf};

use super::file_mtime;

impl App {
    /// Walk `pages/` and capture every `.md` (skipping dotfiles) into
    /// `page_list`. Used by the quick-switcher and the recent-LRU
    /// merger.
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
        // Switching views invalidates the cached backlinks: the new
        // slug owns a different list.
        self.invalidate_backlinks_cache();
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
        // Rebuild the flat-index → NodeId mapping from the sidecar
        // (sidecar blocks are already DFS-preorder, so they line up
        // with the render walk's `cursor`) and hydrate the collapsed
        // mirror from `workspace.tree().is_collapsed(_)`. The op log
        // is the single source of truth across devices — see
        // `outl_core::op::Op::SetCollapsed`. The sidecar contributes
        // only the id mapping; a missing or unreadable sidecar
        // leaves both structures empty until the next reconcile
        // populates `.outl`.
        self.id_by_flat.clear();
        self.collapsed.clear();
        let sidecar_path = outl_md::resolve_sidecar_path(&path);
        if let Ok(sc) = outl_md::sidecar::read(&sidecar_path) {
            self.id_by_flat.reserve(sc.blocks.len());
            for b in &sc.blocks {
                self.id_by_flat.push(b.id);
                if self.workspace.tree().is_collapsed(b.id) {
                    self.collapsed.insert(b.id);
                }
            }
        }
        self.recompute_hidden_by_collapse();
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
}
