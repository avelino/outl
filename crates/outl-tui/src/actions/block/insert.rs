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

/// Where to drop the Insert-mode cursor when entering Insert from
/// Normal. Mirrors vim's three entry points: `I` (start of line),
/// `i` (at cursor), `a` (one char after cursor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertCursor {
    /// `I` — jump to the start of the block.
    Start,
    /// `i` / `Enter` — land at `cursor_col`.
    AtCursor,
    /// `a` — land one char past `cursor_col` (append after the char
    /// under the cursor). Clamps at end of buffer.
    AfterCursor,
}

impl App {
    /// Enter Insert mode at the currently focused block.
    ///
    /// Dispatches by `Focus`: outline blocks edit `App.page`; backlink
    /// blocks load the source page from disk and edit it in place via
    /// `EditTarget::SourcePage`. The `pos` argument controls where the
    /// Insert cursor lands (`I` / `i` / `a` semantics).
    pub(crate) fn enter_insert(&mut self, pos: InsertCursor) {
        if let Focus::Backlink { .. } = self.focus.clone() {
            self.enter_insert_backlink(pos);
            return;
        }
        self.enter_insert_outline(pos);
    }

    /// Insert path for the current page. Mutations land on `app.page`
    /// and commit through the usual `save()`.
    fn enter_insert_outline(&mut self, pos: InsertCursor) {
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
        buf.cursor = insert_cursor_for(pos, self.cursor_col, buf.chars.len());
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
    fn enter_insert_backlink(&mut self, pos: InsertCursor) {
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
        buf.cursor = insert_cursor_for(pos, self.cursor_col, buf.chars.len());
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
    /// - `SourcePage`: mutate the loaded source AST and write it
    ///   via `save_page_with(.., true)` so the source page's index
    ///   entry (block refs, page title, icon) stays current. No
    ///   undo snapshot — cross-page history would be confusing
    ///   (undo here = revert change to another file you can't
    ///   currently see).
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
                        let is_call = outl_actions::parse_call_invocation(&new_text).is_some();
                        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                            node.text = new_text;
                        }
                        self.save();
                        // Finishing an edit on a `call:<name>` block
                        // re-runs it so the `> **result:**` reflects the
                        // freshly-typed params. The re-run reads the op
                        // log, so persist the coalesced edit first —
                        // otherwise it would run against a stale tree.
                        if is_call {
                            self.flush_pending_save();
                            self.rerun_call_block_at(&block_path);
                        }
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
                        // `rebuild_index = true` runs the cheap
                        // `WorkspaceIndex::patch_page` on the source
                        // page (not a full workspace rescan) so
                        // block-ref resolution (`((blk-XXXXXX))`) on
                        // the just-edited blocks doesn't lag behind
                        // the on-disk text. Backlinks themselves are
                        // computed on-demand from the workspace via
                        // `backlinks_for_current`, so no extra cache
                        // touch is needed for those.
                        self.save_page_with(&path, &page, true);
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

/// Map the Normal-mode `cursor_col` into the Insert-mode buffer
/// cursor, applying vim semantics. Clamps `AfterCursor` at `buf_len`
/// so `a` at end-of-line is a no-op (cursor stays where `i` would
/// land).
fn insert_cursor_for(pos: InsertCursor, cursor_col: usize, buf_len: usize) -> usize {
    match pos {
        InsertCursor::Start => 0,
        InsertCursor::AtCursor => cursor_col.min(buf_len),
        InsertCursor::AfterCursor => cursor_col.saturating_add(1).min(buf_len),
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_cursor_for, InsertCursor};

    #[test]
    fn start_is_always_zero() {
        assert_eq!(insert_cursor_for(InsertCursor::Start, 5, 10), 0);
        assert_eq!(insert_cursor_for(InsertCursor::Start, 0, 0), 0);
    }

    #[test]
    fn at_cursor_clamps_to_buf_len() {
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 3, 10), 3);
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 20, 10), 10);
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 0, 0), 0);
    }

    #[test]
    fn after_cursor_advances_one_then_clamps() {
        // mid-buffer: lands one char to the right
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 3, 10), 4);
        // last char (cursor at N-1 of N): lands at end
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 9, 10), 10);
        // already at end: stays at end
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 10, 10), 10);
        // empty buffer: stays at 0
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 0, 0), 0);
    }
}
