//! Terminal UI for the outl outliner.
//!
//! Phase 1 is read-only and journal-first: opening the TUI lands on
//! today's journal. Navigation between dates and between pages works;
//! editing in-place lands in phase 4.
//!
//! Reused by the `outl` binary so that `outl` with no subcommand opens
//! the TUI in the current directory. See `crates/outl-tui/CLAUDE.md`.
//!
//! ## Crate layout
//!
//! - [`state`]   — plain data: `App`, modes, overlays, snapshots.
//! - [`actions`] — methods on `App` that mutate state (the bulk).
//! - [`input`]   — key handlers; route a `KeyEvent` to an action.
//! - [`view`]    — ratatui rendering.
//! - [`runtime`] — `pub fn run`, terminal lifecycle, event loop.
//! - [`keymap`]  — keymap documentation (no code).
//! - [`app`]     — thin re-export shim + TUI-side tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod actions;
pub mod app;
pub mod commands;
pub mod edit_buffer;
pub mod editor;
pub mod fuzzy;
pub mod input;
pub mod keymap;
pub mod outline_ops;
pub mod runtime;
pub mod state;
pub mod theme;
pub mod ui;
pub mod view;

pub use app::{run, run_with_theme_override};
pub use edit_buffer::EditBuffer;
pub use theme::{by_name as theme_by_name, default_theme, Theme, PRESETS as THEME_PRESETS};
