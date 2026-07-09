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

        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }

        // Skip auto-run runtimes on manual `gx` — they execute
        // automatically on page load and after every save. Running
        // them manually provides no additional value.
        let auto_run_langs = self.collect_auto_run_langs();
        if self.block_flat_is_auto_run_lang(idx, &auto_run_langs) {
            self.status = "query blocks auto-run — no manual execution needed".into();
            return;
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
                        let title = format!("{} runtime error", report.language);
                        self.show_error(title, format!("{e}"));
                    }
                }
                self.load_current();
            }
            Err(e) => {
                self.show_error("run failed", format!("{e}"));
            }
        }
    }

    /// Run every block on the current page that either:
    /// - carries an `auto-run::` property (cache-aware), or
    /// - uses a runtime whose `auto_run()` returns `true` (always
    ///   re-runs — results depend on workspace state, not the fence
    ///   body).
    ///
    /// Called after each `load_current` and after each `save()`.
    pub(crate) fn run_auto_run_blocks(&mut self) {
        let auto_run_langs = self.collect_auto_run_langs();
        let mut targets: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        collect_auto_run_targets(
            &self.page.blocks,
            &auto_run_langs,
            &mut cursor,
            &mut targets,
        );
        if targets.is_empty() {
            return;
        }

        let path = self.current_path();
        let orphans = self.orphans_log.clone();
        let mut ran = 0usize;

        for idx in targets {
            // For runtimes with auto_run() == true, bypass the
            // source-hash cache: query results depend on workspace
            // state, not the fence body. For blocks with just the
            // auto-run:: property (no auto_run runtime), keep the
            // cache so navigation is cheap.
            let force = self.block_flat_is_auto_run_lang(idx, &auto_run_langs);
            let result = if force {
                outl_exec::run_block_at_index(
                    &mut self.workspace,
                    &self.hlc,
                    &path,
                    idx,
                    &self.exec_registry,
                    Some(&orphans),
                )
                .map(Some)
            } else {
                outl_exec::run_block_at_index_if_source_changed(
                    &mut self.workspace,
                    &self.hlc,
                    &path,
                    idx,
                    &self.exec_registry,
                    Some(&orphans),
                )
            };
            match result {
                Ok(Some(_report)) => ran += 1,
                Ok(None) => {}
                Err(e) => {
                    self.status = format!("auto-run skipped block {idx}: {e}");
                }
            }
        }

        if ran > 0 {
            self.load_current_no_autorun();
            self.status = format!("auto-ran {ran} block{}", if ran == 1 { "" } else { "s" });
        }
    }

    /// Build the set of fence languages whose runtime declares
    /// `auto_run() == true`.
    fn collect_auto_run_langs(&self) -> Vec<String> {
        self.exec_registry
            .languages()
            .filter(|lang| {
                self.exec_registry
                    .get(lang)
                    .map(|r| r.auto_run())
                    .unwrap_or(false)
            })
            .map(String::from)
            .collect()
    }

    /// Check whether the block at `flat_idx` uses a fence language
    /// whose runtime has `auto_run() == true`.
    fn block_flat_is_auto_run_lang(&self, flat_idx: usize, langs: &[String]) -> bool {
        let mut cursor = 0usize;
        let block = find_block_at_flat(&self.page.blocks, flat_idx, &mut cursor);
        let Some(b) = block else {
            return false;
        };
        let Some(parts) = outl_exec::extract_fence(&b.text) else {
            return false;
        };
        let canonical = outl_md::lang::canonical(&parts.language).unwrap_or(&parts.language);
        langs.iter().any(|l| l == canonical)
    }
}

/// DFS-preorder walk collecting flat indices of blocks that should
/// auto-run: either they carry the `auto-run::` property, or their
/// fence language is in `auto_run_langs`.
fn collect_auto_run_targets(
    blocks: &[OutlineNode],
    auto_run_langs: &[String],
    cursor: &mut usize,
    out: &mut Vec<usize>,
) {
    for b in blocks {
        let has_prop = b.properties.iter().any(|(k, _)| k == "auto-run");
        let is_auto_lang = if let Some(parts) = outl_exec::extract_fence(&b.text) {
            let canonical = outl_md::lang::canonical(&parts.language).unwrap_or(&parts.language);
            auto_run_langs.iter().any(|l| l == canonical)
        } else {
            false
        };
        if has_prop || is_auto_lang {
            out.push(*cursor);
        }
        *cursor += 1;
        collect_auto_run_targets(&b.children, auto_run_langs, cursor, out);
    }
}

/// Find the block at `target_idx` in DFS preorder. Returns `None` if
/// out of range.
fn find_block_at_flat<'a>(
    blocks: &'a [OutlineNode],
    target_idx: usize,
    cursor: &mut usize,
) -> Option<&'a OutlineNode> {
    for b in blocks {
        if *cursor == target_idx {
            return Some(b);
        }
        *cursor += 1;
        if let Some(found) = find_block_at_flat(&b.children, target_idx, cursor) {
            return Some(found);
        }
    }
    None
}
