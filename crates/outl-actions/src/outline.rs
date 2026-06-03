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
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

/// Render a property value as the user-facing string the markdown
/// pipeline already stores. `Text` is the only variant emitted today
/// by `outl-md::diff` (see `crates/outl-md/src/diff.rs`); the other
/// variants are surfaced for forward-compat with the future query DSL
/// but should never appear in v0 workspaces.
pub(crate) fn prop_value_to_string(v: &PropValue) -> String {
    match v {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
        PropValue::List(items) => items
            .iter()
            .map(prop_value_to_string)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Enumerate every DFS path *inside* an [`OutlineNode`] (treated as
/// a self-contained subtree). The first entry is always `vec![]`
/// (the root itself); subsequent entries descend into children in
/// order.
///
/// Used by the TUI's inline backlinks panel so `j`/`k` can step
/// through a referencing block and its descendants without rebuilding
/// the index. Lives here (rather than in `outl-md`) so any future
/// client that consumes [`Backlink::source_block`][crate::Backlink::source_block]
/// can navigate its subtree with the same helper.
pub fn flatten_subtree_paths(root: &OutlineNode) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    out.push(stack.clone());
    walk_subtree(root, &mut stack, &mut out);
    out
}

fn walk_subtree(node: &OutlineNode, stack: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, child) in node.children.iter().enumerate() {
        stack.push(i);
        out.push(stack.clone());
        walk_subtree(child, stack, out);
        stack.pop();
    }
}

/// Build a single [`OutlineNode`] for `node` straight from the
/// workspace, including its subtree and properties.
///
/// Same shape as one element of [`project_outline`] — used by the
/// backlinks builder so each backlink carries the *source block* with
/// its children and properties, instead of forcing the caller to
/// reach back into the workspace per backlink.
pub fn project_outline_node(workspace: &Workspace, node: NodeId) -> OutlineNode {
    let raw = workspace.block_text(node).unwrap_or_default();
    let (todo, body) = split_todo(&raw);
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .map(|(k, v)| (k.to_string(), prop_value_to_string(v)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    OutlineNode {
        id: node.to_string(),
        text: body.to_string(),
        todo,
        collapsed: workspace.tree().is_collapsed(node),
        properties,
        children: project_outline(workspace, node),
    }
}
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
    /// the outline. Overlaid from the workspace via
    /// [`Op::SetCollapsed`][outl_core::op::Op::SetCollapsed]; the op
    /// log is the source of truth. Clients SHOULD still send
    /// `children` so the renderer can show a "(N hidden)" hint
    /// without a second round trip.
    ///
    /// **Use `read_page_view_with_workspace` to populate this.** The
    /// bare [`read_page_view`] has no workspace in scope and leaves
    /// every entry at `false`.
    ///
    /// Mutated via [`crate::collapsed::set_block_collapsed`] /
    /// [`crate::collapsed::toggle_block_collapsed`], which generate
    /// `Op::SetCollapsed` and apply it through `Workspace::apply` —
    /// never via the sidecar.
    pub collapsed: bool,
    /// `(key, value)` properties attached to this block, in
    /// **alphabetical-by-key order**.
    ///
    /// Both producer paths normalise to this order so a backlink
    /// rendering of a block (workspace-driven) and the outline of
    /// the page that owns it (disk-driven) show properties in the
    /// same sequence. The workspace path has no authoring order to
    /// preserve (properties live in a `HashMap` keyed on
    /// `(NodeId, key)`); the disk path used to keep parse-order but
    /// now sorts on `outline_from_parsed` so the two surfaces never
    /// disagree visually.
    ///
    /// Shape mirrors [`outl_md::parse::OutlineNode::properties`].
    /// Populated from [`outl_core::tree::Tree::properties_of`] when
    /// the workspace is in scope, and from the parsed `.md` otherwise.
    pub properties: Vec<(String, String)>,
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
            let mut properties: Vec<(String, String)> = workspace
                .tree()
                .properties_of(id)
                .map(|(k, v)| (k.to_string(), prop_value_to_string(v)))
                .collect();
            properties.sort_by(|a, b| a.0.cmp(&b.0));
            OutlineNode {
                id: id.to_string(),
                text: body.to_string(),
                todo,
                collapsed: workspace.tree().is_collapsed(id),
                properties,
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
/// as a valid ULID. Transient ids (the ones minted by
/// `outline_from_parsed` when the sidecar is missing or shorter than
/// the AST) parse fine but the workspace has never seen them, so
/// `is_collapsed` returns the default `false` — same result as
/// leaving the field untouched, which keeps the contract "every
/// node with no op log presence renders expanded".
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
    // Sort alphabetically so this disk-driven path matches what
    // `project_outline` (workspace-driven) produces. The two surfaces
    // would otherwise disagree on the order properties show up in,
    // visible when a block renders both inside its own page (parse
    // order) and as a backlink elsewhere (workspace order). See the
    // `OutlineNode.properties` doc-comment.
    let mut properties = block.properties.clone();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    // `collapsed` is overlaid by the caller using the workspace as the
    // source of truth (`Op::SetCollapsed` lives in the op log). The
    // bare `read_page_view` path leaves it `false`; the workspace-
    // aware `read_page_view_with_workspace` patches it.
    OutlineNode {
        id,
        text: body.to_string(),
        todo,
        collapsed: false,
        properties,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo::TodoState;

    fn node(text: &str, children: Vec<OutlineNode>) -> OutlineNode {
        OutlineNode {
            id: format!("test-{text}"),
            text: text.into(),
            todo: None,
            collapsed: false,
            properties: Vec::new(),
            children,
        }
    }

    fn leaf(text: &str) -> OutlineNode {
        node(text, Vec::new())
    }

    #[test]
    fn flatten_subtree_paths_returns_dfs_preorder() {
        // Mirrors the previous `outl_md::outline_ops::flatten_backlink_subtree`
        // coverage so behaviour stays identical after the move.
        let root = node(
            "root",
            vec![node("a", vec![leaf("a1"), leaf("a2")]), leaf("b")],
        );
        assert_eq!(
            flatten_subtree_paths(&root),
            vec![
                Vec::<usize>::new(), // root
                vec![0],             // a
                vec![0, 0],          // a1
                vec![0, 1],          // a2
                vec![1],             // b
            ]
        );
    }

    #[test]
    fn flatten_subtree_paths_leaf_returns_just_root() {
        let only = leaf("only-me");
        assert_eq!(flatten_subtree_paths(&only), vec![Vec::<usize>::new()]);
    }

    #[test]
    fn prop_value_to_string_covers_every_variant() {
        // `Text` is what `outl-md` actually emits today; the other
        // variants are surfaced for forward-compat. The helper still
        // has to behave sensibly on each so a future indexer doesn't
        // crash on a non-Text page property.
        assert_eq!(
            prop_value_to_string(&PropValue::Text("high".into())),
            "high"
        );
        assert_eq!(
            prop_value_to_string(&PropValue::PageRef("Avelino".into())),
            "Avelino"
        );
        assert_eq!(
            prop_value_to_string(&PropValue::Tag("urgent".into())),
            "urgent"
        );
        assert_eq!(
            prop_value_to_string(&PropValue::List(vec![
                PropValue::Tag("a".into()),
                PropValue::Tag("b".into()),
            ])),
            "a b"
        );
    }

    #[test]
    fn outline_node_carries_todo_text_and_properties() {
        // Smoke that the DTO surface a backlink hands the renderer
        // exposes the fields the TUI uses. We don't go through the
        // workspace here — that's covered by `backlinks` tests.
        let n = OutlineNode {
            id: "x".into(),
            text: "ship it".into(),
            todo: Some(TodoState::Done),
            collapsed: false,
            properties: vec![("priority".into(), "high".into())],
            children: vec![leaf("child")],
        };
        assert_eq!(n.text, "ship it");
        assert_eq!(n.todo, Some(TodoState::Done));
        assert_eq!(n.properties[0], ("priority".into(), "high".into()));
        assert_eq!(n.children.len(), 1);
    }
}
