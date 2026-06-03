//! # outl-md
//!
//! Markdown parsing, sidecar handling, and the 3-level matching algorithm.
//!
//! This crate is the boundary between the user-visible `.md` files and the
//! op log managed by [`outl_core`]. See `crates/outl-md/CLAUDE.md` and
//! `docs/markdown-format.md` for the dialect specification and the matching
//! algorithm.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod atomic;
pub mod block_index;
pub mod diff;
pub mod index;
pub mod inline;
pub mod matching;
pub mod outline_ops;
pub mod parse;
pub mod reconcile;
pub mod render;
pub mod sidecar;
pub mod slug;
pub mod view;

pub use atomic::write_atomic;
pub use block_index::{BlockEntry, BlockIndex, BlockReference};
pub use diff::{diff_to_ops, DiffPlan};
pub use index::{PageEntry, WorkspaceIndex};
pub use inline::{byte_index_for_char, ref_at_cursor, tokenize, InlineTok, RefTarget};
pub use matching::{match_blocks, Match, MatchLevel};
pub use parse::{parse, OutlineNode, ParsedPage};
pub use reconcile::{
    reconcile_dir, reconcile_md, reconcile_md_with_page_id, ReconcileError, ReconcileReport,
};
pub use render::render;
pub use sidecar::{
    content_hash, file_hash, resolve_sidecar_path, sidecar_path_for, Sidecar, SidecarBlock,
};
pub use slug::{slugify, UNTITLED_SLUG};
pub use view::{block_to_rows, char_to_line_col, BlockRow, BlockRowKind};
