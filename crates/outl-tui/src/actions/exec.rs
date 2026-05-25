//! Run the code block under the cursor through [`outl_exec`].
//!
//! The result subblock lives in the `.md` (`> **result:**` child of
//! the code block). We commit any in-flight Insert first so the
//! runtime sees the user's freshest source, then reparse so the
//! in-memory AST picks up the newly inserted/updated result child.

use crate::state::{App, Mode};
use outl_md::parse::OutlineNode;

impl App {
    /// Run the code block under the current selection through
    /// [`outl_exec`]. The result lands as a `> **result:**` subblock
    /// (or replaces an existing one), the `.md` is rewritten, and the
    /// op log is reconciled — all in one shot.
    pub(crate) fn run_current_block(&mut self) {
        let path = self.current_path();
        let idx = self.selected;
        let orphans = self.orphans_log.clone();

        // Commit any in-flight Insert first — otherwise the user's
        // latest edits aren't on disk yet and the runtime would see
        // stale source.
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }

        match outl_exec::run_block_at_index(
            &mut self.workspace,
            &self.hlc,
            &path,
            idx,
            &self.exec_registry,
            Some(&orphans),
        ) {
            Ok(report) => {
                match &report.result {
                    Ok(out) => {
                        self.status =
                            format!("ran {} in {}ms", report.language, out.duration.as_millis());
                    }
                    Err(e) => {
                        // Infrastructure failure (rustc missing, timeout,
                        // OOM, sandbox couldn't load the wasm). Multi-line
                        // and detailed — show it in the modal so the
                        // whole message lands on screen instead of being
                        // truncated to one row.
                        let title = format!("{} runtime error", report.language);
                        self.show_error(title, format!("{e}"));
                    }
                }
                // Reparse — the `.md` now has the result subblock (or
                // an error message embedded inside it) and we need it
                // in the in-memory AST.
                self.load_current();
            }
            Err(e) => {
                // Couldn't even start: bad block, unknown language,
                // failed read/write. Same modal treatment.
                self.show_error("run failed", format!("{e}"));
            }
        }
    }

    /// Run every block on the current page that has an `auto-run::`
    /// property and whose source has changed since the last execution.
    ///
    /// Called after each `load_current`. Cache-aware via SHA-256 of
    /// the fence body (vs `source-hash::` stamped on the result
    /// subblock), so navigating away and back is a no-op when nothing
    /// changed.
    pub(crate) fn run_auto_run_blocks(&mut self) {
        // Collect candidate flat indices upfront so we don't fight
        // the borrow checker mid-mutation.
        let mut targets: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        collect_auto_run_targets(&self.page.blocks, &mut cursor, &mut targets);
        if targets.is_empty() {
            return;
        }

        let path = self.current_path();
        let orphans = self.orphans_log.clone();
        let mut ran = 0usize;

        for idx in targets {
            match outl_exec::run_block_at_index_if_source_changed(
                &mut self.workspace,
                &self.hlc,
                &path,
                idx,
                &self.exec_registry,
                Some(&orphans),
            ) {
                Ok(Some(_report)) => ran += 1,
                Ok(None) => {} // cache hit; nothing to say
                Err(e) => {
                    // Don't show modal during auto-run — the user
                    // didn't ask for this, surfacing a popup on
                    // page-open would be jarring. Status line only.
                    self.status = format!("auto-run skipped block {idx}: {e}");
                }
            }
        }

        if ran > 0 {
            // Reparse so the AST picks up new/updated result subblocks.
            self.load_current_no_autorun();
            self.status = format!("auto-ran {ran} block{}", if ran == 1 { "" } else { "s" });
        }
    }
}

/// DFS-preorder walk collecting flat indices of blocks that carry an
/// `auto-run::` property. Mirrors `block_at_flat_index_mut` in
/// `outl-exec` so coordinates line up.
fn collect_auto_run_targets(blocks: &[OutlineNode], cursor: &mut usize, out: &mut Vec<usize>) {
    for b in blocks {
        if b.properties.iter().any(|(k, _)| k == "auto-run") {
            out.push(*cursor);
        }
        *cursor += 1;
        collect_auto_run_targets(&b.children, cursor, out);
    }
}
