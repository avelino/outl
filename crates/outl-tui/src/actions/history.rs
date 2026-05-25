//! Undo / redo via [`HistorySnapshot`] stacks.
//!
//! Two bounded stacks. Each structural mutation pushes a snapshot
//! onto `undo`; `undo` pops to `redo`; `redo` pops back to `undo`.
//! A new edit clears `redo` — vim semantics (a new edit branches
//! history).

use crate::outline_ops::flat_count;
use crate::state::{App, HistorySnapshot, MAX_HISTORY};

impl App {
    /// Push the current page state onto the undo stack. Clears the redo
    /// stack — that's vim semantics: a new edit branches history.
    pub(crate) fn snapshot_for_undo(&mut self) {
        let snap = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: self.current_path(),
        };
        self.undo.push(snap);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Pop the most recent snapshot off the undo stack and restore it.
    pub(crate) fn undo(&mut self) {
        let Some(prev) = self.undo.pop() else {
            self.status = "nothing to undo".into();
            return;
        };
        // Capture current state on redo before swapping back.
        let current = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: self.current_path(),
        };
        self.redo.push(current);
        if self.redo.len() > MAX_HISTORY {
            self.redo.remove(0);
        }
        self.restore_snapshot(prev);
        self.status = "undid".into();
    }

    /// Pop from redo and apply.
    pub(crate) fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            self.status = "nothing to redo".into();
            return;
        };
        let current = HistorySnapshot {
            page: self.page.clone(),
            selected: self.selected,
            cursor_col: self.cursor_col,
            view_path: self.current_path(),
        };
        self.undo.push(current);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.restore_snapshot(next);
        self.status = "redid".into();
    }

    /// Replace the in-memory page with a previous snapshot and persist
    /// the change. If the snapshot belongs to a different view (the
    /// user navigated away after the edit), the snapshot is dropped and
    /// the user is notified.
    fn restore_snapshot(&mut self, snap: HistorySnapshot) {
        if snap.view_path != self.current_path() {
            self.status = "skipped: snapshot belongs to a different page".into();
            return;
        }
        self.page = snap.page;
        self.flat_len = flat_count(&self.page.blocks);
        self.selected = snap.selected.min(self.flat_len.saturating_sub(1));
        self.cursor_col = snap.cursor_col.min(self.current_block_char_count());
        // Persist (re-renders .md and reconciles).
        self.save();
    }
}
