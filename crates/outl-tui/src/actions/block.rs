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
use crate::state::{App, EditTarget, Focus, Mode, DONE_PREFIX, TODO_PREFIX};
use outl_md::parse::{parse, OutlineNode, ParsedPage};

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
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
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
            let backlinks = self.index.backlinks(&self.current_slug());
            let Some(bl) = backlinks.get(*idx) else {
                return;
            };
            let mut full = bl.source_block_path.clone();
            full.extend_from_slice(sub_path);
            (bl.source_path.clone(), full)
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
    ///   another file you can't currently see). The in-memory index
    ///   is patched optimistically so the next frame already shows
    ///   the new text — no waiting for a workspace rebuild.
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
                        // Optimistic: patch every backlink that points
                        // at this source page so the UI reflects the
                        // edit *this frame*, not after the next index
                        // rebuild finishes (which scans the whole
                        // workspace and is the dominant cost).
                        self.index.refresh_backlinks_from_source(&path, &page);
                        self.save_page_with(&path, &page, false);
                    }
                    // No restore needed — `page` was a working copy
                    // and is dropped here.
                }
            }
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

    /// Run an op on the source page of the focused backlink and persist.
    ///
    /// `f` receives the source AST (mutable) and the absolute path of
    /// the currently focused block inside that AST, also mutable. The
    /// closure mutates the tree and (when the op changes where the
    /// focused block lives — indent/outdent/move) updates the path
    /// in place. After `f` returns:
    ///
    /// 1. `Focus.sub_path` is rewritten from the post-op absolute path
    ///    (relative to `source_block_path`; clamped to `[]` if the op
    ///    moved the block out of the backlink's scope).
    /// 2. The cached `source_block` of every backlink that points at
    ///    this source page is refreshed in-place (optimistic — no
    ///    workspace rebuild needed). UI sees the new tree this frame.
    /// 3. The page is saved with `save_page_with(.., false)` (no
    ///    index rebuild — step 2 already covered it).
    ///
    /// Returns the post-op `ParsedPage` so callers that need to follow
    /// up (e.g. enter Insert on the newly created block) can reuse it
    /// without re-reading from disk.
    fn apply_to_backlink_source<F>(&mut self, mut f: F) -> Option<(std::path::PathBuf, ParsedPage)>
    where
        F: FnMut(&mut ParsedPage, &mut Vec<usize>),
    {
        let (source_path, source_block_path) = match &self.focus {
            Focus::Backlink { idx, .. } => {
                let backlinks = self.index.backlinks(&self.current_slug());
                let bl = backlinks.get(*idx)?;
                (bl.source_path.clone(), bl.source_block_path.clone())
            }
            _ => return None,
        };
        let sub_path = match &self.focus {
            Focus::Backlink { sub_path, .. } => sub_path.clone(),
            _ => return None,
        };

        // Reuse the working copy if we're already mid-edit on this
        // source — avoids re-reading the file *and* preserves the
        // user's in-flight buffer (we commit it into the AST below).
        let mut source_page = if let Mode::Insert {
            target: EditTarget::SourcePage { path, page },
            block_path,
            buffer,
            ..
        } = &self.mode
        {
            if path == &source_path {
                let mut p = page.clone();
                if let Some(node) = node_at_path_mut(&mut p.blocks, block_path) {
                    node.text = buffer.as_string();
                }
                p
            } else {
                parse(&std::fs::read_to_string(&source_path).ok()?)
            }
        } else {
            parse(&std::fs::read_to_string(&source_path).ok()?)
        };

        let mut abs_path = source_block_path.clone();
        abs_path.extend_from_slice(&sub_path);

        f(&mut source_page, &mut abs_path);

        // Rewrite focus.sub_path from the post-op absolute path. If
        // the op pushed the focused block out of the backlink's
        // visible scope (i.e. it's no longer a descendant of
        // source_block_path), fall back to focusing the source block
        // itself.
        let new_sub_path: Vec<usize> = if abs_path.starts_with(&source_block_path) {
            abs_path[source_block_path.len()..].to_vec()
        } else {
            Vec::new()
        };
        if let Focus::Backlink { sub_path, .. } = &mut self.focus {
            *sub_path = new_sub_path;
        }

        self.index
            .refresh_backlinks_from_source(&source_path, &source_page);
        self.save_page_with(&source_path, &source_page, false);

        Some((source_path, source_page))
    }

    /// Create a new block after the current selection at the same indent,
    /// then enter Insert. Used by `o` (Normal) and Enter (Insert).
    pub(crate) fn create_block_below(&mut self) {
        // Cross-page route (focus on a backlink, or in-place Insert on
        // a source page). The op mutates the source AST, refreshes the
        // index optimistically, and re-enters Insert on the new block.
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            // Drop the existing Insert mode *without* triggering the
            // disk save — we already have the buffer captured via the
            // working copy in `apply_to_backlink_source`, and we'll
            // save once below.
            self.mode = Mode::Normal;

            let result = self.apply_to_backlink_source(|page, abs_path| {
                insert_sibling_after(&mut page.blocks, abs_path);
                // Sibling-after lives at parent ++ [last + 1].
                if let Some(last) = abs_path.last_mut() {
                    *last += 1;
                }
            });
            if let Some((source_path, source_page)) = result {
                // Enter Insert on the freshly inserted block.
                let Focus::Backlink { idx, sub_path } = &self.focus else {
                    return;
                };
                let backlinks = self.index.backlinks(&self.current_slug());
                let Some(bl) = backlinks.get(*idx) else {
                    return;
                };
                let mut abs = bl.source_block_path.clone();
                abs.extend_from_slice(sub_path);
                if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                    let buf = EditBuffer::from_text(&node.text);
                    let original_text = node.text.clone();
                    self.cursor_col = 0;
                    self.mode = Mode::Insert {
                        target: EditTarget::SourcePage {
                            path: source_path,
                            page: source_page,
                        },
                        block_path: abs,
                        buffer: buf,
                        original_text,
                    };
                }
            }
            return;
        }

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
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.mode = Mode::Normal;
            let result = self.apply_to_backlink_source(|page, abs_path| {
                insert_sibling_before(&mut page.blocks, abs_path);
                // The new block now occupies the slot the focused
                // one used to — focus stays at `abs_path` unchanged
                // (the closure does nothing to it), which means we
                // end up inserting *and* selecting the new block.
            });
            if let Some((source_path, source_page)) = result {
                let Focus::Backlink { idx, sub_path } = &self.focus else {
                    return;
                };
                let backlinks = self.index.backlinks(&self.current_slug());
                let Some(bl) = backlinks.get(*idx) else {
                    return;
                };
                let mut abs = bl.source_block_path.clone();
                abs.extend_from_slice(sub_path);
                if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                    let buf = EditBuffer::from_text(&node.text);
                    let original_text = node.text.clone();
                    self.cursor_col = 0;
                    self.mode = Mode::Insert {
                        target: EditTarget::SourcePage {
                            path: source_path,
                            page: source_page,
                        },
                        block_path: abs,
                        buffer: buf,
                        original_text,
                    };
                }
            }
            return;
        }
        let path = path_for_index(&self.page.blocks, self.selected).unwrap_or_else(|| vec![0]);
        self.snapshot_for_undo();
        insert_sibling_before(&mut self.page.blocks, &path);
        self.flat_len = flat_count(&self.page.blocks);
        // Selection stays at `self.selected` — the new block now occupies it.
        self.enter_insert(false);
    }

    pub(crate) fn indent_current(&mut self) -> bool {
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            let in_insert = matches!(self.mode, Mode::Insert { .. });
            let buf_state = if let Mode::Insert { buffer, .. } = &self.mode {
                Some(buffer.clone())
            } else {
                None
            };
            self.mode = Mode::Normal;
            let mut moved = false;
            let result = self.apply_to_backlink_source(|page, abs_path| {
                if let Some(new_path) = indent_at_path(&mut page.blocks, abs_path) {
                    *abs_path = new_path;
                    moved = true;
                }
            });
            // Re-enter Insert if we were editing, on the (possibly
            // re-pathed) block.
            if let (Some(buf), Some((source_path, source_page))) = (buf_state, result) {
                if in_insert {
                    let Focus::Backlink { idx, sub_path } = &self.focus else {
                        return moved;
                    };
                    let backlinks = self.index.backlinks(&self.current_slug());
                    let Some(bl) = backlinks.get(*idx) else {
                        return moved;
                    };
                    let mut abs = bl.source_block_path.clone();
                    abs.extend_from_slice(sub_path);
                    if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                        let original_text = node.text.clone();
                        self.mode = Mode::Insert {
                            target: EditTarget::SourcePage {
                                path: source_path,
                                page: source_page,
                            },
                            block_path: abs,
                            buffer: buf,
                            original_text,
                        };
                    }
                }
            }
            return moved;
        }
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
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            let in_insert = matches!(self.mode, Mode::Insert { .. });
            let buf_state = if let Mode::Insert { buffer, .. } = &self.mode {
                Some(buffer.clone())
            } else {
                None
            };
            self.mode = Mode::Normal;
            let mut moved = false;
            let result = self.apply_to_backlink_source(|page, abs_path| {
                if let Some(new_path) = outdent_at_path(&mut page.blocks, abs_path) {
                    *abs_path = new_path;
                    moved = true;
                }
            });
            if let (Some(buf), Some((source_path, source_page))) = (buf_state, result) {
                if in_insert {
                    let Focus::Backlink { idx, sub_path } = &self.focus else {
                        return moved;
                    };
                    let backlinks = self.index.backlinks(&self.current_slug());
                    let Some(bl) = backlinks.get(*idx) else {
                        return moved;
                    };
                    let mut abs = bl.source_block_path.clone();
                    abs.extend_from_slice(sub_path);
                    if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                        let original_text = node.text.clone();
                        self.mode = Mode::Insert {
                            target: EditTarget::SourcePage {
                                path: source_path,
                                page: source_page,
                            },
                            block_path: abs,
                            buffer: buf,
                            original_text,
                        };
                    }
                }
            }
            return moved;
        }
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
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.mode = Mode::Normal;
            self.apply_to_backlink_source(|page, abs_path| {
                delete_at_path(&mut page.blocks, abs_path);
                // After delete, the focused path no longer resolves;
                // collapse to source_block_path (handled outside via
                // the clamp to `[]`).
                abs_path.clear();
            });
            return;
        }
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
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.apply_to_backlink_source(|page, abs_path| {
                let Some(&last) = abs_path.last() else {
                    return;
                };
                if last == 0 {
                    return;
                }
                let parent = &abs_path[..abs_path.len() - 1];
                let siblings = siblings_mut(&mut page.blocks, parent);
                siblings.swap(last - 1, last);
                *abs_path.last_mut().unwrap() = last - 1;
            });
            return;
        }
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
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.apply_to_backlink_source(|page, abs_path| {
                let Some(&last) = abs_path.last() else {
                    return;
                };
                let parent = abs_path[..abs_path.len() - 1].to_vec();
                let sibling_count = siblings_mut(&mut page.blocks, &parent).len();
                if last + 1 >= sibling_count {
                    return;
                }
                let siblings = siblings_mut(&mut page.blocks, &parent);
                siblings.swap(last, last + 1);
                *abs_path.last_mut().unwrap() = last + 1;
            });
            return;
        }
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
        match self.focus.clone() {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return;
                };
                self.snapshot_for_undo();
                if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
                    node.text = cycle_todo_state(&node.text);
                }
                self.save();
            }
            Focus::Backlink { idx, sub_path } => {
                self.toggle_todo_backlink(idx, &sub_path);
            }
        }
    }

    /// Cross-page TODO cycle: load the source page, mutate the
    /// referencing block (resolved via `source_block_path + sub_path`),
    /// and save it through `save_page` so reconcile keeps IDs stable.
    /// No undo snapshot — same rationale as `commit_insert` on a
    /// `SourcePage`: undo in this view shouldn't silently flip TODOs
    /// in a different file.
    fn toggle_todo_backlink(&mut self, idx: usize, sub_path: &[usize]) {
        let (source_path, abs_path) = {
            let backlinks = self.index.backlinks(&self.current_slug());
            let Some(bl) = backlinks.get(idx) else {
                return;
            };
            let mut full = bl.source_block_path.clone();
            full.extend_from_slice(sub_path);
            (bl.source_path.clone(), full)
        };
        let Ok(text) = std::fs::read_to_string(&source_path) else {
            self.status = format!("cannot read backlink source: {}", source_path.display());
            return;
        };
        let mut source_page = parse(&text);
        let Some(node) = node_at_path_mut(&mut source_page.blocks, &abs_path) else {
            self.status = "backlink source block missing — index may be stale".into();
            return;
        };
        node.text = cycle_todo_state(&node.text);
        // Optimistic: patch the in-memory index so the new TODO/DONE
        // prefix shows up *this frame*. Persist the source page without
        // triggering a full workspace rebuild (the dominant cost) —
        // the next natural rebuild reconverges.
        self.index
            .refresh_backlinks_from_source(&source_path, &source_page);
        self.save_page_with(&source_path, &source_page, false);
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
