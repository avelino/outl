//! Yank register and paste. Vim semantics: `yy` copies one block,
//! Visual `y` copies the range, `p` / `P` paste after / before.

use crate::outline_ops::{flat_count, index_for_path, node_at_path, path_for_index, siblings_mut};
use crate::state::{App, Mode, View};

/// Best-effort copy of `text` to the OS clipboard.
///
/// Returns `true` on success. Failures (no display server, missing
/// clipboard daemon, sandboxed terminal, headless CI) are swallowed
/// — the caller still has `last_yanked_ref` + status as the fallback
/// surface. We never panic over clipboard plumbing.
fn copy_to_os_clipboard(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_string()))
        .is_ok()
}

/// Build the status-line message after a yank attempt.
///
/// `kind` is the human label (`"ref"`, `"embed"`); `token` is the
/// thing that landed on the clipboard. `copied` flips the wording so
/// the user knows whether to expect a paste to work.
fn clipboard_message(kind: &str, token: &str, copied: bool) -> String {
    if copied {
        format!("copied {kind} {token} to clipboard")
    } else {
        format!("yanked {kind} {token} (clipboard unavailable)")
    }
}

impl App {
    /// `yr` — capture the block ref handle of the currently selected
    /// block.
    ///
    /// Looks up the block in the workspace index by `(source_slug,
    /// source_block_path)` and stashes its `((blk-XXXXXX))` form on
    /// `last_yanked_ref` + the status line. `arboard` also writes it
    /// to the OS clipboard so a regular paste works in other apps;
    /// the status line falls back to `(clipboard unavailable)` on
    /// headless / no-display environments.
    ///
    /// Lookup is O(1) — `WorkspaceIndex::block_at_location` uses the
    /// `(slug, dfs_path) → NodeId` secondary index, so the chord stays
    /// snappy regardless of workspace size.
    pub(crate) fn yank_current_ref(&mut self) {
        match self.current_block_ref_handle() {
            Some(h) => {
                let token = format!("(({h}))");
                self.last_yanked_ref = Some(token.clone());
                self.status = clipboard_message("ref", &token, copy_to_os_clipboard(&token));
            }
            None => {
                self.status = "no ref handle yet — save and retry".into();
            }
        }
    }

    /// Yank the **embed** form of the current block: `!((blk-XXXXXX))`.
    ///
    /// Same lookup as [`yank_current_ref`] but stores the embed
    /// formatting so a downstream paste expands the source block
    /// inline instead of linking to it.
    pub(crate) fn yank_current_embed(&mut self) {
        match self.current_block_ref_handle() {
            Some(h) => {
                let token = format!("!(({h}))");
                self.last_yanked_ref = Some(token.clone());
                self.status = clipboard_message("embed", &token, copy_to_os_clipboard(&token));
            }
            None => {
                self.status = "no ref handle yet — save and retry".into();
            }
        }
    }

    /// Resolve the selected block's stable ref handle by looking up
    /// `(source_slug, source_block_path)` in the workspace index.
    ///
    /// O(1) thanks to `WorkspaceIndex::block_at_location`. Returns
    /// `None` when:
    /// - the cursor isn't on a real block (empty page edge case), or
    /// - the block was just created in-memory and the sidecar hasn't
    ///   landed yet (no `BlockEntry` to find).
    pub(crate) fn current_block_ref_handle(&self) -> Option<String> {
        let path = path_for_index(&self.page.blocks, self.selected)?;
        let slug = match &self.view {
            View::Page(p) => p
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            View::Journal(d) => d.format("%Y-%m-%d").to_string(),
        };
        self.index
            .block_at_location(&slug, &path)
            .map(|b| b.ref_handle.clone())
    }

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
        self.remember_visual_range();
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
