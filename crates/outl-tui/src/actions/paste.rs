//! External clipboard paste — convert markdown into a tree of blocks.
//!
//! Wired to `Event::Paste` from crossterm's bracketed-paste support.
//! Delegates the work to [`outl_actions::paste_markdown`] so the
//! semantics stay identical between the TUI and the mobile client.
//!
//! ## v0 anchor policy
//!
//! For simplicity, the TUI always uses [`outl_actions::PasteAnchor::AfterBlock`]
//! against the currently selected block. The mobile client uses
//! `AtCaret` when the paste happens inside a textarea — we deliberately
//! do not match that here in v0 because the TUI runs an AST-first edit
//! pipeline (the buffer is the source while in Insert; the workspace
//! is the source while in Normal). Reusing `AtCaret` from inside Insert
//! would require swapping the workspace state mid-edit, which the
//! peer-sync code path explicitly avoids (see `poll_jsonl_updates`).
//!
//! What we do instead in Insert mode: commit the in-flight buffer
//! first, then paste, then reload the workspace from disk so the new
//! tree shows up.

use outl_actions::{
    children_of, find_by_slug, looks_like_outline, paste_markdown, paste_plain, PasteAnchor,
    PasteOutcome,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::state::{App, EditTarget, Mode};

/// Read the OS clipboard, best-effort. `None` on a headless / no-display
/// environment (the same degradation as the copy side's `arboard`).
fn read_os_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .ok()
}

impl App {
    /// Apply a bracketed-paste payload to the workspace.
    ///
    /// `text` is the verbatim clipboard contents reported by crossterm.
    /// The function:
    ///
    /// 1. Commits any in-flight Insert buffer to disk first so the
    ///    workspace is in a clean state before we apply ops to it.
    /// 2. Resolves the currently selected block's `NodeId` via the
    ///    workspace index (sidecar-backed, O(1)).
    /// 3. Routes the text through `outl_actions::paste_markdown` with
    ///    [`PasteAnchor::AfterBlock`].
    /// 4. Reloads the materialised page from disk and updates the
    ///    status line with the block count the user just pasted.
    ///
    /// Empty paste, no selected block, or no index entry for the
    /// selected block are all soft failures that surface in the
    /// status line and leave the workspace untouched.
    pub(crate) fn paste_external(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        // Plain-text paste inside Insert mode is the common "drop a
        // URL / snippet into what I'm writing" workflow. Splicing the
        // raw text into the live buffer keeps the keyboard up and
        // the cursor where the user expects. Outline-shaped pastes
        // still go through the full pipeline below so they create
        // siblings as documented.
        if !looks_like_outline(&text) {
            if let Mode::Insert { buffer, .. } = &mut self.mode {
                buffer.insert_str(&text);
                self.status = "pasted text".into();
                return;
            }
        }
        self.graft_paste(text, false);
    }

    /// `p` — paste the OS clipboard **with formatting** after the
    /// selected block: outline syntax is converted and multi-paragraph
    /// text is split into one block per paragraph.
    pub(crate) fn paste_clipboard_formatted(&mut self) {
        match read_os_clipboard() {
            Some(text) => self.graft_paste(text, false),
            None => self.status = "clipboard unavailable".into(),
        }
    }

    /// `P` — paste the OS clipboard **without formatting** after the
    /// selected block: the raw text lands as a single block, no
    /// conversion or splitting.
    pub(crate) fn paste_clipboard_plain(&mut self) {
        match read_os_clipboard() {
            Some(text) => self.graft_paste(text, true),
            None => self.status = "clipboard unavailable".into(),
        }
    }

    /// Commit any in-flight edit, resolve the selected block, and graft
    /// `text` after it — through `paste_markdown` (formatted) when
    /// `plain` is false, or `paste_plain` (raw) when true. Reloads the
    /// workspace and repositions the cursor onto the new tail.
    fn graft_paste(&mut self, text: String, plain: bool) {
        if text.is_empty() {
            return;
        }
        // `commit_insert` writes the in-flight buffer back into the
        // AST and — when the buffer changed against the current page
        // — already calls `save()` (render → write → reconcile)
        // internally. Calling `save()` again afterwards would pay the
        // I/O + reconcile a second time for nothing. Track whether
        // the upcoming `commit_insert` will save the current page so
        // we skip the redundant call below.
        let commit_will_save_current = match &self.mode {
            Mode::Insert {
                target,
                buffer,
                original_text,
                ..
            } => matches!(target, EditTarget::CurrentPage) && buffer.as_string() != *original_text,
            _ => false,
        };
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }
        // Force a save + reconcile *before* resolving the selected
        // block's NodeId so the workspace tree mirrors the in-memory
        // AST. Otherwise a freshly opened journal (or a `.md`
        // imported externally that the orphan scanner hasn't picked
        // up yet) leaves the tree with fewer children than the AST
        // shows, and the path walk dead-ends.
        if !commit_will_save_current {
            self.save();
        }

        let slug = self.current_slug();
        let Some(path) = outl_md::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            self.status = "paste: no selected block".into();
            return;
        };
        // Resolve the selected block's NodeId by walking the workspace
        // tree directly. We deliberately don't go through
        // `WorkspaceIndex::block_at_location` here: the index is
        // sidecar-backed and rebuilt off the critical path, so right
        // after a freshly opened journal (or a previous paste that
        // hasn't reprojected yet) the entry may not exist. The
        // workspace tree is always up to date.
        let Some(page_id) = find_by_slug(&self.workspace, &slug) else {
            self.status = "paste: current page not in workspace".into();
            return;
        };
        let Some(node_id) = resolve_node_id_at_path(&self.workspace, page_id, &path) else {
            self.status = "paste: could not resolve selected block in tree".into();
            return;
        };

        let anchor = PasteAnchor::AfterBlock(node_id);
        let result: Result<PasteOutcome, _> = if plain {
            paste_plain(&mut self.workspace, &self.hlc, anchor, &text)
        } else {
            paste_markdown(&mut self.workspace, &self.hlc, anchor, &text)
        };
        match result {
            Ok(out) => {
                // Full refresh: re-read everything from disk and
                // rebuild the workspace index. The lighter
                // `reload_workspace_from_disk` path leaves the page
                // list and index pointing at pre-paste state, which
                // showed up as ghost cells on the right edge of the
                // outline after the user moved the cursor.
                self.reload_workspace_from_disk();
                self.refresh_page_list();
                self.spawn_index_rebuild();
                // Land the selection on the bottom of what we just
                // pasted so the user sees the new tail without
                // scrolling — nicer than landing wherever the post-
                // reload flat index happens to fall.
                self.flat_len = outl_md::outline_ops::flat_count(&self.page.blocks);
                let landed = self.flat_index_for_node(node_id, out.new_blocks.last().copied());
                if let Some(idx) = landed {
                    self.selected = idx.min(self.flat_len.saturating_sub(1));
                }
                self.cursor_col = 0;
                // Any half-pressed Vim chord (`y`, `g`, `d`, `q`) from
                // before the paste must not survive — otherwise the
                // next keystroke fires the chord against a freshly
                // pasted block the user hadn't reviewed yet.
                self.pending_chord = None;
                self.status = if out.root_count > 0 {
                    format!(
                        "pasted {n} block{s}",
                        n = out.root_count,
                        s = if out.root_count == 1 { "" } else { "s" }
                    )
                } else {
                    "pasted text".into()
                };
            }
            Err(e) => {
                self.status = format!("paste failed: {e}");
            }
        }
    }

    /// Locate a freshly-pasted block in the current AST so the caller
    /// can move the selection cursor onto it. Tries the last pasted
    /// id first, falls back to the anchor block.
    fn flat_index_for_node(&self, anchor: NodeId, last_pasted: Option<NodeId>) -> Option<usize> {
        let target = last_pasted.unwrap_or(anchor);
        self.id_by_flat.iter().position(|id| *id == target)
    }
}

/// Walk the tree from `page_id` following the DFS path produced by
/// `outl_md::outline_ops::path_for_index`. Returns the `NodeId` at
/// `path` when every step lines up, `None` if any segment is out of
/// range (in practice only happens when the AST drifted from the
/// workspace state, e.g. a peer added blocks since the last reload).
pub(crate) fn resolve_node_id_at_path(
    workspace: &Workspace,
    page_id: NodeId,
    path: &[usize],
) -> Option<NodeId> {
    let mut current = page_id;
    for &idx in path {
        let kids = children_of(workspace, current);
        let (child, _) = kids.into_iter().nth(idx)?;
        current = child;
    }
    Some(current)
}
