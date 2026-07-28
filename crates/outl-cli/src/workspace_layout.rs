//! Filesystem layout of an outl workspace.
//!
//! The implementation lives in [`outl_ws::layout`] so the TUI, MCP
//! server, and external embedders share it; this module re-exports it
//! under the CLI's historical path.

pub use outl_ws::layout::*;
