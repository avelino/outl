use chrono::NaiveDate;

use outl_actions::{find_by_slug, instantiate_template};
use outl_core::id::NodeId;

use crate::actions::paste::resolve_node_id_at_path;
use crate::state::{App, EditTarget, Mode};

impl App {
    /// Instantiate a structural template under the currently
    /// selected block, following the same commit → resolve → apply
    /// → reload pattern as `graft_paste`.
    pub(crate) fn instantiate_template_at_cursor(&mut self, name: &str) {
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
        if !commit_will_save_current {
            self.save();
        }

        let slug = self.current_slug();
        let Some(path) = outl_md::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            self.status = "template: no selected block".into();
            return;
        };

        let Some(page_id) = find_by_slug(&self.workspace, &slug) else {
            self.status = "template: current page not in workspace".into();
            return;
        };

        let Some(target_id) = resolve_node_id_at_path(&self.workspace, page_id, &path) else {
            self.status = "template: could not resolve selected block".into();
            return;
        };

        let page_date = NaiveDate::parse_from_str(&slug, "%Y-%m-%d").ok();

        match instantiate_template(
            &mut self.workspace,
            &self.hlc,
            name,
            target_id,
            &slug,
            page_date,
        ) {
            Ok(ids) => {
                let count = ids.len();

                if let Some(root) = self.workspace.root.as_ref() {
                    let _ =
                        outl_actions::apply_page_md_with_sidecar(&self.workspace, root, page_id);
                }

                self.reload_workspace_from_disk();
                self.refresh_page_list();
                self.spawn_index_rebuild();
                self.flat_len = outl_md::outline_ops::flat_count(&self.page.blocks);
                self.pending_chord = None;
                self.status = format!(
                    "instantiated template `{name}` ({count} block{s})",
                    s = if count == 1 { "" } else { "s" }
                );
            }
            Err(e) => {
                self.status = format!("template failed: {e}");
            }
        }
    }

    /// Resolve a callable template, execute its code block with the
    /// given params, and attach stdout as a `> **result:**` subtree
    /// under the selected block. Thin wrapper over the shared
    /// [`App::run_callable_template`] so the `/template <name> k=v`
    /// slash command and the `call:` fence (`gx`) stay identical.
    pub(crate) fn execute_callable_template(&mut self, name: &str, params: &[(String, String)]) {
        let anchor = self
            .id_by_flat
            .get(self.selected)
            .copied()
            .unwrap_or(NodeId::root());
        match self.run_callable_template(name, params, anchor) {
            Ok(dur) => self.status = format!("ran template `{name}` ({}ms)", dur.as_millis()),
            Err(e) => self.status = format!("template: {e}"),
        }
    }
}
