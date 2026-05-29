//! UI-friendly projection of a page, built either from the workspace
//! tree (materialised op log) or from the `.md` file on disk.
//!
//! Both paths produce the same [`OutlineNode`] shape so the mobile
//! frontend doesn't care which source was used. In v0 the mobile and
//! TUI clients build the outline from `.md` + sidecar; the
//! [`project_outline`] variant stays around for tools that need to
//! materialise straight from the op log (e.g. doctor, debug dumps).

use std::path::Path;

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
    let id = entry.map(|b| b.id.to_string()).unwrap_or_default();
    let (todo, body) = split_todo(&block.text);
    let children = block
        .children
        .iter()
        .map(|child| outline_from_parsed(child, iter))
        .collect();
    OutlineNode {
        id,
        text: body.to_string(),
        todo,
        children,
    }
}
