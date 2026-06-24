//! # outl-actions
//!
//! UI-agnostic workspace operations.
//!
//! Every outl client (`outl-tui`, `outl-mobile`, the future Tauri
//! desktop, future plugins) needs the *same* high-level operations on
//! the workspace: edit a block's text, toggle its TODO state, indent
//! or outdent it, append a new block, render today's journal as
//! `.md`. This crate is where those operations live so we never
//! duplicate them per client.
//!
//! ## Layering
//!
//! ```text
//! outl-core (CRDT, op log, storage trait)
//!     ↑
//! outl-md   (.md parse/render, sidecar, matching)
//!     ↑
//! outl-actions  ← you are here
//!     ↑
//! outl-cli / outl-tui / outl-mobile / future clients
//! ```
//!
//! ## Contract
//!
//! - Functions take a `&mut Workspace` and a `&HlcGenerator` so callers
//!   stay in control of mutation timing and HLC ordering.
//! - All ops go through `Workspace::apply`, never around it — invariant
//!   #1 from `crates/outl-core/CLAUDE.md` ("op log is source of truth").
//! - TODO/DONE state is encoded as a prefix in the block's text
//!   (`"TODO foo"` / `"DONE foo"`), matching the wire format the TUI
//!   already uses. See [`mod@todo`].
//! - The `.md` projection (see [`journal::apply_page_md`] /
//!   [`journal::apply_all_pages_md`]) is a *projection* of the
//!   materialised tree — never read it back to reconstruct state.
//!   Always read the op log.
//!
//! ## What this crate does NOT own
//!
//! - Anything UI-state related (selection cursors, modal modes,
//!   keymaps, toasts) — those stay in `outl-tui` / `outl-mobile`.
//! - In-flight outline AST manipulation (the user editing a `.md`
//!   buffer that hasn't been parsed yet) — that's `outl-tui`'s
//!   `outline_ops.rs`, which operates on `Vec<OutlineNode>` not on a
//!   workspace.
//! - Storage backends (sqlite, iCloud, ChronDB) — those implement
//!   `outl_core::Storage` and live in the binary that needs them.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backlinks;
pub mod block;
pub mod collapsed;
pub mod error;
pub mod exec;
pub mod history;
pub mod journal;
pub mod outline;
pub mod page;
pub mod paste;
pub mod person;
pub mod quote;
pub mod sync;
pub mod todo;
pub mod tree;

pub use backlinks::{backlinks_for_page, backlinks_for_target, extract_refs, Backlink};
pub use block::{
    append_block, append_forest, append_tree, create_after, create_under, delete, edit_text,
    indent, move_down, move_up, outdent, toggle_quote, toggle_todo, BlockTreeOutcome,
    BlockTreeSpec,
};
pub use collapsed::{set_block_collapsed, toggle_block_collapsed};
pub use error::ActionError;
pub use exec::{run_code_block, ExecOutputDto, RunCodeBlockOutcome};
pub use history::{restore_page_md, HistoryStacks, DEFAULT_HISTORY_CAP};
pub use journal::{
    apply_all_pages_md, apply_page_md, apply_page_md_with_sidecar, journals_dir, mutate_page_md,
    page_md_path, pages_dir, render_page_md, write_md_atomic,
};
pub use outl_md::parse::{ParseWarning, ParseWarningKind};
pub use outline::{
    flat_index_for_block, flatten_subtree_paths, project_outline, project_outline_node,
    read_page_outline, read_page_outline_with_workspace, read_page_view,
    read_page_view_with_workspace, OutlineNode, PageOutline,
};
pub use page::{
    date_from_slug, find_by_slug, is_valid_slug, journal_slug, journal_title,
    list_all as list_pages, migrate_legacy_into_today, next_journal_date, open_journal,
    open_or_create as open_or_create_page, open_or_create_by_name, open_or_create_by_ref,
    open_today, page_meta, previous_journal_date, read_text_prop, set_property, today, PageKind,
    PageMeta,
};
pub use paste::{
    looks_like_outline, normalize_external_syntax, paste_markdown, PasteAnchor, PasteOutcome,
};
pub use person::{search_persons, PERSON_TYPE, TYPE_KEY};
pub use sync::{FileSyncTransport, OpsFileSnapshot, PeerHealthSnapshot, SyncEngine, SyncTransport};
pub use todo::{cycle_todo, split_todo, TodoState, DONE_PREFIX, TODO_PREFIX};
pub use tree::{
    children_of, enclosing_page_id, position_after, position_for_new_last_child, walk_subtree,
};
