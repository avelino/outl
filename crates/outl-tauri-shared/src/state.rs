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
    /// Direction `backlinks` was sorted in (`[display] backlinks_order`,
    /// issue #142). Carried on the view so a client's direction toggle
    /// shows the right arrow at boot without a separate settings read.
    /// Serialises as `"newest"` / `"oldest"`.
    pub backlinks_order: outl_config::BacklinksOrder,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<outl_md::ParseWarning>,
}

/// Reply shape for the lazy `page_backlinks` command.
///
/// Backlinks are **not** bundled into [`PageView`] anymore:
/// `backlinks_for_page` is an `O(blocks-in-workspace)` scan, and computing
/// it inside `build_page_view` blocked every page open and every block
/// edit synchronously (a ~66k-node workspace made the first journal paint
/// take seconds on desktop, more on mobile). The frontend now fetches
/// backlinks lazily after the outline paints — the same lazy/cached policy
/// the TUI has always used for its backlinks panel. `PageView.backlinks`
/// stays in the wire shape but comes back empty from the open commands.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinksReply {
    pub backlinks: Vec<Backlink>,
    /// Direction the list was sorted in (`[display] backlinks_order`).
    pub backlinks_order: outl_config::BacklinksOrder,
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

/// One structural template surfaced by `list_templates` — the `/template`
/// picker in every GUI client. Mirrors the invocation `name` (what the
/// user picks) and the `slug` of the page that defines the body.
///
/// Deliberately narrower than `outl_actions::TemplateEntry`: the GUIs
/// only need the name (label) + slug (secondary label / dedupe key);
/// `page_id` and `params` are backend detail the pick doesn't carry.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateDto {
    /// Invocation name (the value of the page's `template::` property).
    pub name: String,
    /// Slug of the page that defines the template.
    pub slug: String,
    /// `true` when another page shares this `template:: <name>` — the
    /// picker surfaces it so the user knows a duplicate silently
    /// shadows the rest (resolution picks the first in tree order).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
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
