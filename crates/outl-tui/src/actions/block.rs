//! Block-level mutations: Insert mode, create / indent / outdent /
//! delete / reorder blocks, TODO prefix cycle.
//!
//! All ops snapshot through [`App::snapshot_for_undo`] so the history
//! stack can roll back any structural change. Saves go through
//! [`App::save`] in `lifecycle`.

use crate::edit_buffer::EditBuffer;
use crate::outline_ops::{
    delete_at_path, descendants_count_at_path, flat_count, indent_at_path, index_for_path,
    insert_sibling_after, insert_sibling_before, node_at_path, node_at_path_mut, outdent_at_path,
    path_for_index, siblings_mut,
};
use crate::state::{App, Mode, DONE_PREFIX, TODO_PREFIX};
use outl_md::parse::OutlineNode;

impl App {
    /// Enter Insert mode at the currently selected block (cursor at end).
    pub(crate) fn enter_insert(&mut self, at_start: bool) {
        if self.flat_len == 0 {
            // No blocks yet — create one and start editing.
            self.page.blocks.push(OutlineNode::default());
            self.flat_len = 1;
            self.selected = 0;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(node) = node_at_path(&self.page.blocks, &path) else {
            return;
        };
        let mut buf = EditBuffer::from_text(&node.text);
        if at_start {
            buf.move_home();
        }
        self.mode = Mode::Insert {
            block_path: path,
            buffer: buf,
            original_text: node.text.clone(),
        };
    }

    /// Commit Insert: write buffer back into the AST and persist.
    pub(crate) fn commit_insert(&mut self) {
        if let Mode::Insert {
            block_path,
            buffer,
            original_text,
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        {
            let new_text = buffer.as_string();
            if new_text != original_text {
                // Only snapshot when the user actually changed something —
                // pressing Esc without typing should not pollute history.
                self.snapshot_for_undo();
                if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                    node.text = new_text;
                }
                self.save();
            } else if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                // Empty round-trip; restore exactly to be safe.
                node.text = original_text;
            }
        }
    }

    /// Abort Insert: throw away the buffer, leave AST unchanged.
    pub(crate) fn abort_insert(&mut self) {
        if let Mode::Insert {
            block_path,
            original_text,
            ..
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        {
            if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                node.text = original_text;
            }
        }
    }

    /// Create a new block after the current selection at the same indent,
    /// then enter Insert. Used by `o` (Normal) and Enter (Insert).
    pub(crate) fn create_block_below(&mut self) {
        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else {
            path_for_index(&self.page.blocks, self.selected).unwrap_or_else(|| vec![0])
        };
        self.snapshot_for_undo();
        insert_sibling_after(&mut self.page.blocks, &path);
        // New selection = the newly inserted block (right after the previous one in DFS).
        self.flat_len = flat_count(&self.page.blocks);
        let new_idx = self.selected + 1 + descendants_count_at_path(&self.page.blocks, &path);
        self.selected = new_idx.min(self.flat_len.saturating_sub(1));
        // If we were in Insert, commit the old buffer first, then re-enter on new block.
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }
        self.enter_insert(false);
    }

    /// Create a new block above the current selection, then enter Insert.
    pub(crate) fn create_block_above(&mut self) {
        let path = path_for_index(&self.page.blocks, self.selected).unwrap_or_else(|| vec![0]);
        self.snapshot_for_undo();
        insert_sibling_before(&mut self.page.blocks, &path);
        self.flat_len = flat_count(&self.page.blocks);
        // Selection stays at `self.selected` — the new block now occupies it.
        self.enter_insert(false);
    }

    pub(crate) fn indent_current(&mut self) -> bool {
        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else if let Some(p) = path_for_index(&self.page.blocks, self.selected) {
            p
        } else {
            return false;
        };
        // Snapshot before we know if the op succeeds — cheap to take,
        // and we only ever discard it on a literal no-op.
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        if let Some(new_path) = indent_at_path(&mut self.page.blocks, &path) {
            self.flat_len = flat_count(&self.page.blocks);
            if let Mode::Insert { block_path, .. } = &mut self.mode {
                *block_path = new_path.clone();
            }
            self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
            true
        } else {
            // Roll back the optimistic snapshot.
            if self.undo.len() > snapshot_idx {
                self.undo.truncate(snapshot_idx);
            }
            false
        }
    }

    pub(crate) fn outdent_current(&mut self) -> bool {
        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else if let Some(p) = path_for_index(&self.page.blocks, self.selected) {
            p
        } else {
            return false;
        };
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        if let Some(new_path) = outdent_at_path(&mut self.page.blocks, &path) {
            self.flat_len = flat_count(&self.page.blocks);
            if let Mode::Insert { block_path, .. } = &mut self.mode {
                *block_path = new_path.clone();
            }
            self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
            true
        } else {
            if self.undo.len() > snapshot_idx {
                self.undo.truncate(snapshot_idx);
            }
            false
        }
    }

    pub(crate) fn delete_current(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.snapshot_for_undo();
        delete_at_path(&mut self.page.blocks, &path);
        self.save(); // also handles refilling an empty page with a single bullet
    }

    /// Swap the current block with its previous sibling (drags its
    /// subtree). No-op if already at the top of its parent.
    pub(crate) fn move_block_up(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(&last_idx) = path.last() else {
            return;
        };
        if last_idx == 0 {
            return;
        }
        self.snapshot_for_undo();
        let parent_path = &path[..path.len() - 1];
        let siblings = siblings_mut(&mut self.page.blocks, parent_path);
        siblings.swap(last_idx - 1, last_idx);
        let mut new_path = parent_path.to_vec();
        new_path.push(last_idx - 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.save();
    }

    /// Swap with the next sibling.
    pub(crate) fn move_block_down(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(&last_idx) = path.last() else {
            return;
        };
        let parent_path = path[..path.len() - 1].to_vec();
        let sibling_count = siblings_mut(&mut self.page.blocks, &parent_path).len();
        if last_idx + 1 >= sibling_count {
            return;
        }
        self.snapshot_for_undo();
        let siblings = siblings_mut(&mut self.page.blocks, &parent_path);
        siblings.swap(last_idx, last_idx + 1);
        let mut new_path = parent_path;
        new_path.push(last_idx + 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.save();
    }

    /// Set (or replace) a property on the currently selected block.
    /// If `value` is empty the property is **removed** — gives users
    /// a single command for both edit and delete.
    ///
    /// Bound to `/prop <key> <value>` and `:prop <key> <value>`. Idempotent.
    pub(crate) fn set_property_on_current_block(&mut self, key: &str, value: &str) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            self.status = "no block selected".into();
            return;
        };
        self.snapshot_for_undo();
        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
            if value.is_empty() {
                node.properties.retain(|(k, _)| k != key);
                self.status = format!("removed property `{key}`");
            } else if let Some(p) = node.properties.iter_mut().find(|(k, _)| k == key) {
                p.1 = value.to_string();
                self.status = format!("set {key} = {value}");
            } else {
                node.properties.push((key.to_string(), value.to_string()));
                self.status = format!("added {key} = {value}");
            }
        }
        self.save();
    }

    /// Set (or replace) a *page-level* property — the ones at the
    /// top of the `.md` (`title::`, `icon::`, ...). Empty value
    /// removes. Bound to `/prop-page <key> <value>`.
    pub(crate) fn set_property_on_page(&mut self, key: &str, value: &str) {
        self.snapshot_for_undo();
        if value.is_empty() {
            self.page.properties.retain(|(k, _)| k != key);
            self.status = format!("removed page property `{key}`");
        } else if let Some(p) = self.page.properties.iter_mut().find(|(k, _)| k == key) {
            p.1 = value.to_string();
            self.status = format!("set page {key} = {value}");
        } else {
            self.page
                .properties
                .push((key.to_string(), value.to_string()));
            self.status = format!("added page {key} = {value}");
        }
        self.save();
    }

    pub(crate) fn toggle_todo(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.snapshot_for_undo();
        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
            node.text = cycle_todo_state(&node.text);
        }
        self.save();
    }
}

/// Cycle a block's TODO prefix: none → `TODO ` → `DONE ` → none.
pub(crate) fn cycle_todo_state(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("TODO ") {
        return format!("DONE {rest}");
    }
    if let Some(rest) = text.strip_prefix("DONE ") {
        return rest.to_string();
    }
    format!("TODO {text}")
}

/// Cycle the TODO prefix directly on an [`EditBuffer`], preserving the
/// cursor's *visual* position relative to the user's text.
///
/// - none → `TODO `: prefix added, cursor shifts right by 5.
/// - `TODO ` → `DONE `: replace in place, cursor unchanged.
/// - `DONE ` → none: prefix removed, cursor shifts left by 5
///   (clamped to 0).
pub(crate) fn cycle_todo_inline(buffer: &mut EditBuffer) {
    let prefix_chars = TODO_PREFIX.chars().count(); // 5; same for both
    let current: String = buffer.chars.iter().take(prefix_chars).collect();
    if current == TODO_PREFIX {
        // Replace `TODO ` with `DONE ` in place — same length, cursor intact.
        for (i, ch) in DONE_PREFIX.chars().enumerate() {
            buffer.chars[i] = ch;
        }
        return;
    }
    if current == DONE_PREFIX {
        // Remove the 5-char prefix.
        for _ in 0..prefix_chars {
            buffer.chars.remove(0);
        }
        buffer.cursor = buffer.cursor.saturating_sub(prefix_chars);
        return;
    }
    // No prefix yet — prepend `TODO ` and shift cursor right.
    for (i, ch) in TODO_PREFIX.chars().enumerate() {
        buffer.chars.insert(i, ch);
    }
    buffer.cursor += prefix_chars;
}
