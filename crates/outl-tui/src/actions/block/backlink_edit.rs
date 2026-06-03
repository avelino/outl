//! Cross-page edits triggered while the focus is on a backlink.
//!
//! `apply_to_backlink_source` is the workhorse: it loads the source
//! `.md`, lets the caller mutate its AST + the focused block's path,
//! then writes the result through `save_page_with`. Used by every
//! structural op in `structural.rs` when `Focus::Backlink` is
//! active, and by `toggle_todo_backlink` here.

use crate::outline_ops::node_at_path_mut;
use crate::state::{App, EditTarget, Focus, Mode};
use outl_md::parse::{parse, ParsedPage};

impl App {
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
    /// 2. The page is saved with `save_page_with(.., false)` —
    ///    backlinks are now computed straight from the workspace
    ///    (`backlinks_for_current`), so the post-op tree is visible
    ///    on the next render with no cache patch required.
    ///
    /// Returns the post-op `ParsedPage` so callers that need to follow
    /// up (e.g. enter Insert on the newly created block) can reuse it
    /// without re-reading from disk.
    pub(super) fn apply_to_backlink_source<F>(
        &mut self,
        mut f: F,
    ) -> Option<(std::path::PathBuf, ParsedPage)>
    where
        F: FnMut(&mut ParsedPage, &mut Vec<usize>),
    {
        let (source_path, source_block_path) = match &self.focus {
            Focus::Backlink { idx, .. } => {
                let backlinks = self.backlinks_for_current();
                let bl = backlinks.get(*idx)?;
                (bl.source_path.clone()?, bl.source_block_path.clone())
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

        self.save_page_with(&source_path, &source_page, false);

        Some((source_path, source_page))
    }

    /// Cross-page TODO cycle: load the source page, mutate the
    /// referencing block (resolved via `source_block_path + sub_path`),
    /// and save it through `save_page_with` so reconcile keeps IDs stable.
    /// No undo snapshot — same rationale as `commit_insert` on a
    /// `SourcePage`: undo in this view shouldn't silently flip TODOs
    /// in a different file.
    pub(super) fn toggle_todo_backlink(&mut self, idx: usize, sub_path: &[usize]) {
        let (source_path, abs_path) = {
            let backlinks = self.backlinks_for_current();
            let Some(bl) = backlinks.get(idx) else {
                return;
            };
            let Some(path) = bl.source_path.clone() else {
                return;
            };
            let mut full = bl.source_block_path.clone();
            full.extend_from_slice(sub_path);
            (path, full)
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
        node.text = super::cycle_todo_state(&node.text);
        self.save_page_with(&source_path, &source_page, false);
    }
}
