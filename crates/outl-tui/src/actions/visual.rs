//! Visual mode: a Normal-mode sibling that adds a range anchor so the
//! user can act on N blocks at once (delete, indent, outdent).

use crate::outline_ops::{
    delete_at_path, flat_count, indent_at_path, outdent_at_path, path_for_index,
};
use crate::state::{App, Mode};

impl App {
    /// Enter Visual mode anchored at the current selection.
    pub(crate) fn enter_visual(&mut self) {
        self.mode = Mode::Visual {
            anchor: self.selected,
        };
    }

    /// `gv` — re-enter Visual mode at the last range. No-op when no
    /// Visual session has ever happened in this app instance (the
    /// status line surfaces "no previous selection").
    pub(crate) fn reselect_last_visual(&mut self) {
        let Some((lo, hi)) = self.last_visual else {
            self.status = "no previous selection".into();
            return;
        };
        let max_idx = self.flat_len.saturating_sub(1);
        let lo = lo.min(max_idx);
        let hi = hi.min(max_idx);
        // Re-anchor at `lo` and move selection to `hi` so the range
        // re-renders identically — `visual_range()` swaps them on
        // demand so direction doesn't matter.
        self.mode = Mode::Visual { anchor: lo };
        self.selected = hi;
    }

    /// Snapshot the current Visual range into `last_visual` so a
    /// future `gv` can restore it. Call this right before leaving
    /// Visual mode — every exit path (Esc, yank, delete, indent,
    /// outdent) goes through here.
    pub(crate) fn remember_visual_range(&mut self) {
        if let Some(range) = self.visual_range() {
            self.last_visual = Some(range);
        }
    }

    /// `Esc` from Visual — capture the range, drop back to Normal.
    /// The renderer already shows Normal-mode chrome once `Mode`
    /// changes; nothing else to do.
    pub(crate) fn exit_visual(&mut self) {
        self.remember_visual_range();
        self.mode = Mode::Normal;
    }

    /// Return the (lo, hi) flat indices of the Visual selection,
    /// inclusive on both sides. `None` if not in Visual mode.
    pub(crate) fn visual_range(&self) -> Option<(usize, usize)> {
        if let Mode::Visual { anchor } = self.mode {
            let lo = anchor.min(self.selected);
            let hi = anchor.max(self.selected);
            Some((lo, hi))
        } else {
            None
        }
    }

    /// Delete every block in the Visual range, exit Visual mode.
    pub(crate) fn delete_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        self.remember_visual_range();
        // Delete from hi down to lo so flat indices don't shift mid-loop.
        for idx in (lo..=hi).rev() {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                delete_at_path(&mut self.page.blocks, &path);
            }
        }
        self.mode = Mode::Normal;
        self.selected = lo.min(flat_count(&self.page.blocks).saturating_sub(1));
        self.save();
    }

    /// Indent every block in the Visual range. Skips blocks that can't
    /// indent (no previous sibling); the range as a whole still moves
    /// as best it can.
    pub(crate) fn indent_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        // Indent in increasing order — earlier blocks moving deeper
        // doesn't change later blocks' flat indices (the count is
        // preserved). Indents that fail (first child of parent) are
        // silently skipped.
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = indent_at_path(&mut self.page.blocks, &path);
            }
        }
        self.save();
    }

    /// Outdent every block in the Visual range.
    pub(crate) fn outdent_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        self.snapshot_for_undo();
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                let _ = outdent_at_path(&mut self.page.blocks, &path);
            }
        }
        self.save();
    }
}
