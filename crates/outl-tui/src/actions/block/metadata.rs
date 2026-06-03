//! Block-level and page-level metadata writes: properties, TODO
//! prefix cycle, the `pinned::` flag.
//!
//! These commit straight to disk through `save()` (or the source-page
//! variant for backlinks) and bypass Insert mode entirely — they're
//! invoked from slash commands, chord shortcuts, or the command
//! palette.

use crate::outline_ops::{node_at_path_mut, path_for_index};
use crate::state::{App, Focus, ToastKind, View};

impl App {
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

    /// Toggle the `pinned:: true` page-level property. Wired to the
    /// `gp` chord in Normal mode and to the `/pin` slash command;
    /// commits straight to disk (no insert-mode buffer to worry
    /// about) and toasts the new state so the user can confirm
    /// without reading the file.
    ///
    /// Refuses to act on Journal pages — pinning a journal would be
    /// semantically weird (today's note auto-rotates) and would
    /// silently dilute the sidebar's `Pinned` list with
    /// date-shaped junk.
    pub(crate) fn toggle_pinned(&mut self) {
        if matches!(self.view, View::Journal(_)) {
            self.toast(ToastKind::Warning, "can't pin a journal page");
            return;
        }
        self.snapshot_for_undo();
        let was_pinned = self.page.properties.iter().any(|(k, v)| {
            k == "pinned"
                && matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "true" | "yes" | "1" | "on"
                )
        });
        if was_pinned {
            self.page.properties.retain(|(k, _)| k != "pinned");
            self.save();
            self.toast(ToastKind::Info, "unpinned");
        } else {
            // Drop any existing falsy `pinned::` value first so the
            // toggle doesn't leave two `pinned::` lines stacked at
            // the top of the file.
            self.page.properties.retain(|(k, _)| k != "pinned");
            self.page
                .properties
                .push(("pinned".to_string(), "true".to_string()));
            self.save();
            self.toast(ToastKind::Success, "pinned");
        }
    }

    /// Cycle the focused block's TODO state: none → `TODO ` → `DONE ` →
    /// none. Dispatches by `Focus`: outline blocks edit `app.page`
    /// directly; backlink blocks route through
    /// [`Self::toggle_todo_backlink`] which loads the source page off
    /// disk.
    pub(crate) fn toggle_todo(&mut self) {
        match self.focus.clone() {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return;
                };
                self.snapshot_for_undo();
                if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
                    node.text = super::cycle_todo_state(&node.text);
                }
                self.save();
            }
            Focus::Backlink { idx, sub_path } => {
                self.toggle_todo_backlink(idx, &sub_path);
            }
        }
    }
}
