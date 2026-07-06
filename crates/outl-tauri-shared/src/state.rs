//! Wire types shared by every GUI client.
//!
//! These are the reply shapes the Solid frontends (via `@outl/shared`)
//! deserialize — field names and shapes are part of the wire contract.
//! The `AppState` structs themselves stay in the client crates (their
//! fields differ); only what crosses the Tauri bridge lives here.

use outl_actions::{Backlink, OutlineNode, PageMeta};
use serde::Serialize;

/// Sentinel error returned by workspace-touching commands while the
/// workspace is still being opened (background thread) or while the
/// user hasn't picked one yet. The frontend retries on a short interval.
pub const ERR_LOADING: &str = "workspace_loading";

/// Returned by the `workspace_stats` command.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSummary {
    pub blocks: usize,
    pub ops: usize,
    pub actor: String,
    pub storage_root: String,
    /// `true` when a workspace is loaded; `false` while the picker is
    /// still up or the background opener is in flight.
    pub ready: bool,
}

/// Reply shape for every "open page / open journal" command. Bundles
/// the page meta with the outline so the frontend gets everything in
/// one trip.
///
/// `warnings` is the verbatim `outl_md::ParseWarning` list surfaced by
/// `outl_actions::read_page_outline_with_workspace`; the shared
/// `<ParseWarningsBanner />` consumes it. Empty (or absent) on a clean
/// file — `skip_serializing_if` keeps the JSON quiet.
#[derive(Debug, Clone, Serialize)]
pub struct PageView {
    pub page: PageMeta,
    pub outline: Vec<OutlineNode>,
    pub backlinks: Vec<Backlink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<outl_md::ParseWarning>,
}

/// One hit from `search_blocks` — the `((…))` block-ref autocomplete.
///
/// The frontend inserts `handle` wrapped in `((…))` (never the display
/// `text`: block refs resolve by handle, not by content) and shows
/// `text` + `source_slug` as the suggestion label.
#[derive(Debug, Clone, Serialize)]
pub struct BlockHit {
    /// Ref handle to insert, e.g. `blk-r6s4a1`.
    pub handle: String,
    /// Block text (snippet) for the popup label.
    pub text: String,
    /// Slug of the page hosting the block, for context.
    pub source_slug: String,
}

/// Reply for `create_block`. Pairs the refreshed [`PageView`] with the
/// id of the freshly-inserted block so the frontend can focus / start
/// editing it without re-discovering the id via a DFS diff (the diff
/// path mis-identified the new block when the anchor had children
/// — `flat[idx+1]` would land on `children[0]` instead of the new
/// sibling, and the eventual `edit_block` would target a stale id and
/// surface the `block <ULID> is not in the tree` toast).
#[derive(Debug, Clone, Serialize)]
pub struct CreateBlockReply {
    pub view: PageView,
    pub new_id: String,
}
