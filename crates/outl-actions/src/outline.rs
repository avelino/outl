//! UI-friendly projection of a page, built either from the workspace
//! tree (materialised op log) or from the `.md` file on disk.
//!
//! Both paths produce the same [`OutlineNode`] shape so the mobile
//! frontend doesn't care which source was used. In v0 the mobile and
//! TUI clients build the outline from `.md` + sidecar; the
//! [`project_outline`] variant stays around for tools that need to
//! materialise straight from the op log (e.g. doctor, debug dumps).

use std::path::Path;
use std::str::FromStr;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::OutlineNode as ParsedOutlineNode;
use outl_md::sidecar::SidecarBlock;
use serde::Serialize;

use crate::error::ActionError;
use crate::journal::page_md_path;
use crate::page::PageMeta;
use crate::todo::{split_todo, TodoState};
use crate::tree::children_of;

/// A node in the outline as seen by the UI.
///
/// `text` is the block body **without** the TODO/DONE prefix (if any).
/// The prefix lives in [`Self::todo`].
#[derive(Debug, Clone, Serialize)]
pub struct OutlineNode {
    /// Stable block identifier, stringified.
    pub id: String,
    /// Block body without the TODO/DONE prefix.
    pub text: String,
    /// `None` for a plain bullet, `Some(Todo)` / `Some(Done)` otherwise.
    #[serde(serialize_with = "serialize_todo_state")]
    pub todo: Option<TodoState>,
    /// Whether the block is rendered collapsed (children hidden) in
    /// the outline. UI-state echoed from the sidecar; clients SHOULD
    /// still send `children` so the renderer can show a "(N hidden)"
    /// hint without a second round trip.
    ///
    /// Mutated via `outl_actions::block::toggle_block_collapsed` /
    /// `set_block_collapsed`, which write directly to the sidecar
    /// (no `Op` — UI state never enters the op log).
    pub collapsed: bool,
    /// Children, in their fractional-index order.
    pub children: Vec<OutlineNode>,
}

fn serialize_todo_state<S>(state: &Option<TodoState>, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match state {
        None => ser.serialize_none(),
        Some(s) => ser.serialize_str(s.as_str()),
    }
}

/// Walk the workspace tree starting from `parent` and return the
/// outline below it. `NodeId::root()` is the usual starting point.
pub fn project_outline(workspace: &Workspace, parent: NodeId) -> Vec<OutlineNode> {
    children_of(workspace, parent)
        .into_iter()
        .map(|(id, _)| {
            let raw = workspace.block_text(id).unwrap_or_default();
            let (todo, body) = split_todo(&raw);
            OutlineNode {
                id: id.to_string(),
                text: body.to_string(),
                todo,
                collapsed: workspace.tree().is_collapsed(id),
                children: project_outline(workspace, id),
            }
        })
        .collect()
}

/// Read the page's `.md`, parse it, attach `NodeId`s from the sidecar,
/// and return the outline.
///
/// This is the **canonical UI path** in v0. The `.md` is the source
/// the user sees in Files.app / iCloud / vim; rendering anything else
/// would let the on-disk view drift from what the app shows.
///
/// Sidecar resolution accepts both the modern `<name>.outl` location
/// and the legacy `.<name>.outl` location and migrates the latter on
/// first read (see [`outl_md::resolve_sidecar_path`]). A missing
/// sidecar is not fatal — the outline returns block ids derived from
/// position so the UI can still render, but those ids are not stable
/// across processes and callers should run a reconcile before mutating.
pub fn read_page_view(root: &Path, meta: &PageMeta) -> Result<Vec<OutlineNode>, ActionError> {
    let md_path = page_md_path(root, meta);
    let md_text = std::fs::read_to_string(&md_path).unwrap_or_default();
    let parsed = outl_md::parse::parse(&md_text);
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    let sidecar = outl_md::sidecar::read(&sidecar_path).ok();

    let mut nodes = Vec::with_capacity(parsed.blocks.len());
    let mut iter = sidecar
        .as_ref()
        .map(|sc| SidecarBlockCursor::Some(sc.blocks.iter()))
        .unwrap_or(SidecarBlockCursor::None);
    for block in &parsed.blocks {
        nodes.push(outline_from_parsed(block, &mut iter));
    }
    Ok(nodes)
}

/// Same as [`read_page_view`] but overlays the workspace's
/// `Op::SetCollapsed` state so each [`OutlineNode`] reports the
/// authoritative `collapsed` flag. UI clients (TUI, mobile) **must**
/// use this variant — the bare `read_page_view` leaves `collapsed`
/// at `false` because it has no op log in scope.
pub fn read_page_view_with_workspace(
    root: &Path,
    meta: &PageMeta,
    workspace: &Workspace,
) -> Result<Vec<OutlineNode>, ActionError> {
    let mut nodes = read_page_view(root, meta)?;
    overlay_collapsed(&mut nodes, workspace);
    Ok(nodes)
}

/// Walk `nodes` in place, setting `collapsed` from
/// `workspace.tree().is_collapsed(id)` for every node whose id parses
/// as a valid `NodeId`. Transient ids (the ones minted by
/// `outline_from_parsed` when the sidecar is missing / short) skip
/// the overlay — they're not in the op log yet.
fn overlay_collapsed(nodes: &mut [OutlineNode], workspace: &Workspace) {
    for node in nodes {
        if let Ok(ulid) = ulid::Ulid::from_str(&node.id) {
            let id = NodeId(ulid);
            node.collapsed = workspace.tree().is_collapsed(id);
        }
        overlay_collapsed(&mut node.children, workspace);
    }
}

enum SidecarBlockCursor<'a> {
    Some(std::slice::Iter<'a, SidecarBlock>),
    None,
}

impl<'a> SidecarBlockCursor<'a> {
    fn next(&mut self) -> Option<&'a SidecarBlock> {
        match self {
            SidecarBlockCursor::Some(it) => it.next(),
            SidecarBlockCursor::None => None,
        }
    }
}

fn outline_from_parsed(
    block: &ParsedOutlineNode,
    iter: &mut SidecarBlockCursor<'_>,
) -> OutlineNode {
    let entry = iter.next();
    // When the sidecar is absent or shorter than the parsed AST, mint a
    // fresh transient NodeId per block. Returning an empty string would
    // give every fallback block the same id, which breaks keyed
    // rendering on the frontend (Solid for-each, React lists). The id is
    // unstable across renders by design — clients are expected to call
    // back into the workspace once `reconcile_md` has populated the
    // sidecar.
    let id = entry
        .map(|b| b.id.to_string())
        .unwrap_or_else(|| outl_core::id::NodeId::new().to_string());
    let (todo, body) = split_todo(&block.text);
    let children = block
        .children
        .iter()
        .map(|child| outline_from_parsed(child, iter))
        .collect();
    // `collapsed` is overlaid by the caller using the workspace as the
    // source of truth (`Op::SetCollapsed` lives in the op log). The
    // bare `read_page_view` path leaves it `false`; the workspace-
    // aware `read_page_view_with_workspace` patches it.
    OutlineNode {
        id,
        text: body.to_string(),
        todo,
        collapsed: false,
        children,
    }
}
