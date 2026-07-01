//! Mouse handling, active only under `[tui] mouse_capture`.
//!
//! With the mouse captured the app owns selection: the wheel moves the
//! selection, a left click selects the block under the pointer, and a
//! left drag selects a Visual range that is copied to the OS clipboard as
//! markdown on release (the same `yank_visual_range` the keyboard `y`
//! uses). The terminal's native text selection is off while this is on —
//! which is exactly why the whole feature is opt-in (see `outl-config`).
//!
//! Mapping a screen `(column, row)` back to a block relies on two things
//! the renderer persists every frame: [`App::outline_area`] (the bordered
//! rect) and [`App::block_starts`] (the visual-line → flat-index map). The
//! arithmetic mirrors the scroll math in `view::render_main`.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::state::{App, Focus, Mode};

impl App {
    /// Dispatch a terminal mouse event. Only reached when mouse capture
    /// is enabled (the runtime gates `Event::Mouse` on the flag).
    pub(crate) fn handle_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            // Wheel moves the selection one block; the render's
            // auto-scroll then keeps it on screen, so this composes with
            // the existing viewport math instead of fighting it.
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),

            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(idx) = self.block_at_position(ev.column, ev.row) {
                    self.focus = Focus::Outline;
                    self.selected = idx;
                    self.cursor_col = 0;
                    self.mouse_anchor = Some(idx);
                    // A click starts in Normal; it only becomes a Visual
                    // range once the user drags onto another block.
                    self.mode = Mode::Normal;
                }
            }

            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(anchor) = self.mouse_anchor else {
                    return;
                };
                if let Some(idx) = self.block_at_position(ev.column, ev.row) {
                    // Extend (or open) the Visual range from the press
                    // anchor to the block now under the pointer.
                    self.mode = Mode::Visual { anchor };
                    self.selected = idx;
                    self.cursor_col = 0;
                }
            }

            MouseEventKind::Up(MouseButton::Left) => {
                // A drag that entered Visual copies the range as markdown
                // and drops back to Normal (`yank_visual_range` does
                // both). A plain click (no drag) leaves the selection
                // where it landed and copies nothing.
                if self.mouse_anchor.is_some() && matches!(self.mode, Mode::Visual { .. }) {
                    self.yank_visual_range();
                }
                self.mouse_anchor = None;
            }

            _ => {}
        }
    }

    /// Resolve a terminal `(column, row)` to a flat block index, or
    /// `None` when the click is outside the outline body, on its border,
    /// in the backlinks tail below the outline, or on a page-property
    /// line above the first block.
    fn block_at_position(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.outline_area?;
        // The widget draws a 1-cell border on every side; the content
        // lives strictly inside it.
        let inner_left = area.x + 1;
        let inner_right = area.x + area.width.saturating_sub(1);
        let inner_top = area.y + 1;
        if column < inner_left || column >= inner_right || row < inner_top {
            return None;
        }
        // Screen row → visual line: undo the top border, add back the
        // lines scrolled off the top.
        let visual_line = (row - inner_top) as usize + self.scroll_y as usize;
        if visual_line >= self.outline_line_count {
            // Below the outline proper (the inline backlinks section, or
            // empty space past the last block).
            return None;
        }
        self.block_at_visual_line(visual_line)
    }

    /// The block whose rendered rows cover `visual_line`: the entry with
    /// the greatest start line not past it. `block_starts` is ascending
    /// by start line (DFS order, every block emits at least its bullet
    /// row), so binary-search for the last `start <= visual_line`. A drag
    /// fires an event per pixel-row, so this stays O(log n) per event.
    fn block_at_visual_line(&self, visual_line: usize) -> Option<usize> {
        let idx = self
            .block_starts
            .partition_point(|&(start, _)| start <= visual_line);
        idx.checked_sub(1).map(|i| self.block_starts[i].1)
    }
}
