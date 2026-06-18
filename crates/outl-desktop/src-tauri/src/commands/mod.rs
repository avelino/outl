//! Tauri command surface for `outl-desktop`.
//!
//! Split by responsibility so the file-size guard stays happy and
//! each module has one job:
//!
//! - [`workspace`] — pick / open / reload the workspace, surface
//!   stats, resolve refs.
//! - [`page`] — open and navigate pages and journals.
//! - [`block`] — every block mutation (create, edit, todo, indent,
//!   move, paste, collapsed).
//! - [`history`] — undo / redo of committed block mutations.
//!
//! Every command is re-exported at this level so
//! `tauri::generate_handler!` in `lib.rs` doesn't have to know about
//! the file split.

pub(crate) mod block;
pub(crate) mod exec;
pub(crate) mod history;
pub(crate) mod page;
pub(crate) mod shortcuts;
pub(crate) mod theme;
pub(crate) mod workspace;

pub(crate) use block::*;
pub(crate) use exec::*;
pub(crate) use history::*;
pub(crate) use page::*;
pub(crate) use shortcuts::*;
pub(crate) use theme::*;
pub(crate) use workspace::*;
