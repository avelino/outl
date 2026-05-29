//! Outline AST helpers — historically lived here as the TUI's
//! "AST-first" exception to the workspace-grounded mutations in
//! [`outl_actions`]. They've moved to [`outl_md::outline_ops`] now
//! that the mobile client needs the same helpers; this file just
//! re-exports them so the rest of the TUI keeps working unchanged.

pub use outl_md::outline_ops::*;
