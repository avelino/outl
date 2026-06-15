//! Lifecycle methods for [`App`]: construction, view loading, disk
//! persistence, peer-ops sync, external-edit polling.
//!
//! Each concern lives in its own file and contributes its own
//! `impl App { … }` block. The split lets the size of any single
//! file stay under the comfort threshold even as we grow more
//! orchestration around the editor's truth (workspace + page +
//! mtime).
//!
//! | Submodule        | What's in it                                                    |
//! |------------------|-----------------------------------------------------------------|
//! | `mod.rs` (here)  | `App::new` and the shared `file_mtime` helper                   |
//! | `index_build`    | Workspace index rebuild and the background worker poller        |
//! | `peer_sync`      | `ops/<actor>.jsonl` watcher and orphan-`.md` scanner            |
//! | `external`       | Detect/handle edits a different process made to the open `.md`  |
//! | `loading`        | Switch view, parse current `.md`, refresh page list, LRU recent |
//! | `persistence`    | Render `ParsedPage` → `.md`, reconcile, refresh index/cache     |

use crate::commands::CommandRegistry;
use crate::state::{App, Focus, Mode, View};
use crate::theme::Theme;
use anyhow::Result;
use chrono::Local;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::ParsedPage;
use std::path::{Path, PathBuf};

pub(crate) mod external;
pub(crate) mod index_build;
pub(crate) mod loading;
pub(crate) mod peer_sync;
pub(crate) mod persistence;

impl App {
    /// Build the editor's initial state and arm the background
    /// pollers (index rebuild, peer-ops watcher, orphan-`.md`
    /// scanner). The first paintable frame is one
    /// `load_current()` away.
    pub(crate) fn new(
        workspace_root: PathBuf,
        workspace: Workspace,
        actor: ActorId,
        theme: Theme,
        shared_workspace: bool,
    ) -> Result<Self> {
        let orphans_log = workspace_root.join(".outl").join("orphans.log");
        let mut s = Self {
            hlc: HlcGenerator::new(actor),
            workspace_root,
            workspace,
            orphans_log,
            view: View::Journal(Local::now().date_naive()),
            page: ParsedPage::default(),
            selected: 0,
            flat_len: 0,
            cursor_col: 0,
            page_list: Vec::new(),
            mode: Mode::Normal,
            show_help: false,
            help_tab: 0,
            help_scroll: 0,
            pending_chord: None,
            pending_input_op: None,
            last_visual: None,
            status: String::new(),
            parse_warnings: Vec::new(),
            overlay: None,
            autocomplete: None,
            last_search: None,
            yank_register: Vec::new(),
            last_yanked_ref: None,
            index: WorkspaceIndex::default(),
            index_rx: None,
            backlinks_cache: std::cell::RefCell::new(None),
            shared_workspace,
            jsonl_rx: None,
            pending_reload: false,
            orphan_md_rx: None,
            show_backlinks: true,
            show_sidebar: false,
            sidebar_focus: None,
            sidebar_cursor: 0,
            recent_paths: Vec::new(),
            toasts: Vec::new(),
            focus: Focus::Outline,
            scroll_y: 0,
            viewport_height: 0,
            last_mtime: None,
            last_saved_at: None,
            undo: Vec::new(),
            redo: Vec::new(),
            theme,
            exec_registry: RuntimeRegistry::with_builtins(),
            command_registry: CommandRegistry::with_builtins(),
            collapsed: std::collections::HashSet::new(),
            id_by_flat: Vec::new(),
            hidden_by_collapse: Vec::new(),
        };
        s.refresh_page_list();
        s.ensure_view_file_exists()?;
        s.load_current();
        // Build the workspace index off the critical path so the TUI
        // can paint immediately. Backlinks/icons fill in once the
        // worker thread completes (usually < 100ms for small
        // workspaces, longer for big ones — but the user is already
        // typing).
        s.spawn_index_rebuild();
        s.spawn_jsonl_poller();
        s.spawn_orphan_md_scanner();
        Ok(s)
    }
}

/// Last-modified time of `path`, or `None` if the file isn't there
/// (or we lack permission to stat it). Used by the external-edit
/// polling loop and by `save`/`load_current` to keep `last_mtime`
/// in sync with what we just wrote/read.
///
/// Shared by `loading`, `external`, and `persistence` — kept on the
/// parent module so each submodule can `use super::file_mtime`.
pub(super) fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
