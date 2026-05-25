//! Yank register and paste. Vim semantics: `yy` copies one block,
//! Visual `y` copies the range, `p` / `P` paste after / before.

use crate::outline_ops::{flat_count, index_for_path, node_at_path, path_for_index, siblings_mut};
use crate::state::{App, Mode};

impl App {
    /// Copy the current block (with its subtree) into the yank
    /// register. Doesn't mutate the page.
    pub(crate) fn yank_current(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        if let Some(node) = node_at_path(&self.page.blocks, &path) {
            self.yank_register = vec![node.clone()];
            self.status = "yanked 1 block".into();
        }
    }

    /// Copy every block in the Visual range. The range stays in
    /// flat-index space, so we walk it twice — once to grab nodes,
    /// once to drop the Visual mode.
    pub(crate) fn yank_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        let mut grabbed = Vec::new();
        for idx in lo..=hi {
            if let Some(path) = path_for_index(&self.page.blocks, idx) {
                if let Some(node) = node_at_path(&self.page.blocks, &path) {
                    grabbed.push(node.clone());
                }
            }
        }
        let n = grabbed.len();
        self.yank_register = grabbed;
        self.mode = Mode::Normal;
        self.status = format!("yanked {n} block{}", if n == 1 { "" } else { "s" });
    }

    /// Paste the yank register **after** the current selection at the
    /// same indent. Returns silently if the register is empty.
    pub(crate) fn paste_after(&mut self) {
        if self.yank_register.is_empty() {
            self.status = "yank register empty".into();
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.snapshot_for_undo();
        // Insert each yanked node as a sibling immediately after the
        // current one, in order. Doing it in reverse keeps indices
        // stable as we walk the destination position.
        let Some((last_idx, parent_path)) = path.split_last() else {
            return;
        };
        let siblings = siblings_mut(&mut self.page.blocks, parent_path);
        for (i, node) in self.yank_register.iter().enumerate() {
            siblings.insert(last_idx + 1 + i, node.clone());
        }
        let pasted = self.yank_register.len();
        self.flat_len = flat_count(&self.page.blocks);
        // Move selection to the first pasted block for visibility.
        let mut new_path = parent_path.to_vec();
        new_path.push(last_idx + 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.cursor_col = 0;
        self.save();
        self.status = format!(
            "pasted {pasted} block{}",
            if pasted == 1 { "" } else { "s" }
        );
    }

    /// Paste the yank register **before** the current selection.
    pub(crate) fn paste_before(&mut self) {
        if self.yank_register.is_empty() {
            self.status = "yank register empty".into();
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.snapshot_for_undo();
        let Some((last_idx, parent_path)) = path.split_last() else {
            return;
        };
        let siblings = siblings_mut(&mut self.page.blocks, parent_path);
        for (i, node) in self.yank_register.iter().enumerate() {
            siblings.insert(last_idx + i, node.clone());
        }
        let pasted = self.yank_register.len();
        self.flat_len = flat_count(&self.page.blocks);
        // Selection stays on the first pasted (which now occupies
        // the original position).
        self.cursor_col = 0;
        self.save();
        self.status = format!(
            "pasted {pasted} block{}",
            if pasted == 1 { "" } else { "s" }
        );
    }
}
