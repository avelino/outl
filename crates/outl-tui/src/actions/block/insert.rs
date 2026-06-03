//! Enter / commit / abort Insert mode.
//!
//! Two entry paths: outline edits land on `app.page` via
//! `EditTarget::CurrentPage`, backlink edits load the source page
//! fresh from disk via `EditTarget::SourcePage`. Commit fans back out
//! into either `save()` (current page) or `save_page_with(.., false)`
//! (source page). Abort throws away the in-flight buffer.

use crate::edit_buffer::EditBuffer;
use crate::outline_ops::{node_at_path, node_at_path_mut};
use crate::state::{App, EditTarget, Focus, Mode};
use outl_md::parse::{parse, OutlineNode};

impl App {
    /// Enter Insert mode at the currently focused block (cursor at end).
    ///
    /// Dispatches by `Focus`: outline blocks edit `App.page`; backlink
    /// blocks load the source page from disk and edit it in place via
    /// `EditTarget::SourcePage`.
    pub(crate) fn enter_insert(&mut self, at_start: bool) {
        if let Focus::Backlink { .. } = self.focus.clone() {
            self.enter_insert_backlink(at_start);
            return;
        }
        self.enter_insert_outline(at_start);
    }

    /// Insert path for the current page. Mutations land on `app.page`
    /// and commit through the usual `save()`.
    fn enter_insert_outline(&mut self, at_start: bool) {
        if self.flat_len == 0 {
            // No blocks yet — create one and start editing.
            self.page.blocks.push(OutlineNode::default());
            self.flat_len = 1;
            self.selected = 0;
        }
        let Some(path) = crate::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            return;
        };
        let Some(node) = node_at_path(&self.page.blocks, &path) else {
            return;
        };
        let mut buf = EditBuffer::from_text(&node.text);
        // Preserve the column the user was on in Normal mode (vim:
        // `i` lands *at* the cursor, not at end of line). `I` still
        // forces home via `at_start`. Clamp because `cursor_col` can
        // sit one past the last char after `$`.
        buf.cursor = if at_start {
            0
        } else {
            self.cursor_col.min(buf.chars.len())
        };
        self.mode = Mode::Insert {
            target: EditTarget::CurrentPage,
            block_path: path,
            buffer: buf,
            original_text: node.text.clone(),
        };
    }

    /// Insert path for a backlink block. Loads the source page fresh
    /// from disk (the indexed copy may lag a beat), resolves the
    /// absolute path inside that AST, and stashes both in the
    /// `EditTarget::SourcePage` so `commit_insert` knows where to
    /// write back.
    fn enter_insert_backlink(&mut self, at_start: bool) {
        let (source_path, abs_path) = {
            let Focus::Backlink { idx, sub_path } = &self.focus else {
                return;
            };
            let backlinks = self.backlinks_for_current();
            let Some(bl) = backlinks.get(*idx) else {
                return;
            };
            let Some(path) = bl.source_path.clone() else {
                return;
            };
            let mut full = bl.source_block_path.clone();
            full.extend_from_slice(sub_path);
            (path, full)
        };

        // Authoritative read: don't trust the cached `source_block`
        // — it might be stale relative to a recent disk write (e.g.
        // `outl serve` reconciled in the background).
        let Ok(text) = std::fs::read_to_string(&source_path) else {
            self.status = format!("cannot read backlink source: {}", source_path.display());
            return;
        };
        let source_page = parse(&text);
        let Some(node) = node_at_path(&source_page.blocks, &abs_path) else {
            self.status = "backlink source block missing — index may be stale".into();
            return;
        };
        let original_text = node.text.clone();
        let mut buf = EditBuffer::from_text(&original_text);
        // Same vim convention as the outline path: `i` keeps the
        // cursor where it was in Normal, `I` jumps home.
        buf.cursor = if at_start {
            0
        } else {
            self.cursor_col.min(buf.chars.len())
        };
        self.mode = Mode::Insert {
            target: EditTarget::SourcePage {
                path: source_path,
                page: source_page,
            },
            block_path: abs_path,
            buffer: buf,
            original_text,
        };
    }

    /// Commit Insert: write buffer back into the AST and persist.
    ///
    /// Routes by `EditTarget`:
    /// - `CurrentPage`: mutate `app.page`, snapshot undo, `save()`.
    /// - `SourcePage`: mutate the loaded source AST and write it via
    ///   `save_page_with(.., false)`. No undo snapshot — cross-page
    ///   history would be confusing (undo here = revert change to
    ///   another file you can't currently see).
    pub(crate) fn commit_insert(&mut self) {
        if let Mode::Insert {
            target,
            block_path,
            buffer,
            original_text,
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        {
            let new_text = buffer.as_string();
            match target {
                EditTarget::CurrentPage => {
                    if new_text != original_text {
                        // Only snapshot when the user actually changed
                        // something — pressing Esc without typing
                        // should not pollute history.
                        self.snapshot_for_undo();
                        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                            node.text = new_text;
                        }
                        self.save();
                    } else if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path)
                    {
                        // Empty round-trip; restore exactly to be safe.
                        node.text = original_text;
                    }
                }
                EditTarget::SourcePage { path, mut page } => {
                    if new_text != original_text {
                        if let Some(node) = node_at_path_mut(&mut page.blocks, &block_path) {
                            node.text = new_text;
                        }
                        // Backlinks are computed on-demand from the
                        // workspace (`backlinks_for_current` → the
                        // op log via `outl_actions::backlinks_for_page`),
                        // so once `reconcile_md` finishes inside
                        // `save_page_with` the next render already sees
                        // the new text — no optimistic in-memory cache
                        // patch needed.
                        self.save_page_with(&path, &page, false);
                    }
                    // No restore needed — `page` was a working copy
                    // and is dropped here.
                }
            }
        }
        // If the peer-ops poller fired while we were inside Insert
        // mode, we held the reload back to avoid clobbering the
        // in-flight buffer. Now that the edit landed (or was a
        // no-op), it's safe to fold peer ops in.
        if std::mem::take(&mut self.pending_reload) {
            self.reload_workspace_from_disk();
        }
    }

    /// Abort Insert: throw away the buffer, leave AST unchanged.
    pub(crate) fn abort_insert(&mut self) {
        let Mode::Insert {
            target,
            block_path,
            original_text,
            ..
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        // SourcePage: nothing to restore on `app.page` (we never
        // mutated it). The working copy in `target.page` is just
        // dropped.
        if let (EditTarget::CurrentPage, Some(node)) =
            (target, node_at_path_mut(&mut self.page.blocks, &block_path))
        {
            node.text = original_text;
        }
    }
}
