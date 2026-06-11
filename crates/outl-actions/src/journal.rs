//! Page-level `.md` projections.
//!
//! The `.md` file is a **projection** of the materialised tree —
//! never the source of truth. Clients regenerate the projection after
//! every workspace mutation so the user can read it from Finder /
//! Files app / `cat`.
//!
//! Layout inside the workspace root:
//!
//! ```text
//! <root>/Documents/outl/
//! ├── journals/
//! │   └── YYYY-MM-DD.md            ← journal pages (page-kind = "journal")
//! └── pages/
//!     └── <slug>.md                ← regular pages (page-kind = "page")
//! ```

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::render::render;
use outl_md::sidecar::{
    content_hash, derive_ref_handle, file_hash, sidecar_path_for, Sidecar, SidecarBlock,
};

use crate::error::ActionError;
use crate::page::{list_all as list_pages, page_meta, PageKind, PageMeta};
use crate::tree::children_of;

/// Path for the `journals/` directory inside the workspace.
///
/// `root` is the **workspace root** — the directory whose immediate
/// children are `journals/`, `pages/`, `ops/`. Callers are responsible
/// for picking the right root: on iOS the mobile app derives this from
/// the iCloud Ubiquity Container (`<container>/Documents/outl`); on
/// desktop the TUI receives it as `--path`. We do **not** re-join
/// `Documents/outl` here — doing so silently nested the layout twice
/// when the TUI passed the already-final workspace path.
pub fn journals_dir(root: &Path) -> PathBuf {
    root.join("journals")
}

/// Path for the `pages/` directory inside the workspace.
///
/// See [`journals_dir`] for the contract on `root`.
pub fn pages_dir(root: &Path) -> PathBuf {
    root.join("pages")
}

/// Build the on-disk path for a given page's `.md` projection.
pub fn page_md_path(root: &Path, meta: &PageMeta) -> PathBuf {
    let folder = match meta.kind {
        PageKind::Journal => journals_dir(root),
        PageKind::Page => pages_dir(root),
    };
    folder.join(format!("{}.md", meta.slug))
}

fn build_outline(workspace: &Workspace, parent: NodeId) -> Vec<OutlineNode> {
    children_of(workspace, parent)
        .into_iter()
        .map(|(id, _)| OutlineNode {
            text: workspace.block_text(id).unwrap_or_default(),
            properties: Vec::new(),
            children: build_outline(workspace, id),
        })
        .collect()
}

/// Render every block under `page_root` to a clean `.md` string. The
/// page's title (`workspace.block_text(page_root)`) is **not** included
/// in the body — clients can prepend it themselves if they want.
pub fn render_page_md(workspace: &Workspace, page_root: NodeId) -> String {
    let page = ParsedPage {
        properties: Vec::new(),
        blocks: build_outline(workspace, page_root),
        warnings: Vec::new(),
    };
    render(&page)
}

/// Best-effort atomic write of `contents` to `path`, creating parents
/// as needed.
pub fn write_md_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Render `page_root`'s sub-tree and write it to its canonical path
/// under `root`.
pub fn apply_page_md(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let md = render_page_md(workspace, page_root);
    let path = page_md_path(root, &meta);
    write_md_atomic(&path, &md)?;
    Ok(path)
}

/// Render the page, write the `.md`, and (re)write its `.outl` sidecar
/// to match the workspace tree exactly.
///
/// This is the call clients use when they want peers to read the
/// projection consistently. Writing `.md` without updating the sidecar
/// is dangerous: a peer running the 3-level matching algorithm would
/// see "different content, old sidecar" and emit phantom `Create` /
/// `Delete` ops in cascade. By regenerating the sidecar from the same
/// workspace tree we just rendered, the peer's matcher sees identical
/// hashes and the reconcile is a no-op.
pub fn apply_page_md_with_sidecar(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let md = render_page_md(workspace, page_root);
    let path = page_md_path(root, &meta);
    write_md_atomic(&path, &md)?;

    let sidecar = build_sidecar(workspace, page_root, &md);
    let sidecar_path = sidecar_path_for(&path);
    outl_md::sidecar::write(&sidecar_path, &sidecar)?;

    Ok(path)
}

/// Construct a sidecar that lines up with the `.md` we just rendered
/// from the workspace. Walks the page subtree in DFS preorder — the
/// same order [`render_page_md`] emits — so every block's index in
/// the walk maps 1:1 to its line in the `.md`.
fn build_sidecar(workspace: &Workspace, page_root: NodeId, md: &str) -> Sidecar {
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    walk_sidecar(workspace, page_root, 0, &mut line, &mut blocks);
    Sidecar {
        version: 2,
        page_id: page_root,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // This builder runs after a workspace-driven render — the
        // workspace tree already holds the page properties, so by
        // construction they're in the op log. Stamp the current
        // pipeline version to keep the orphan scanner from looping
        // on this page.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_sidecar(
    workspace: &Workspace,
    parent: NodeId,
    indent: u32,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
) {
    for (id, _) in children_of(workspace, parent) {
        let text = workspace.block_text(id).unwrap_or_default();
        out.push(SidecarBlock {
            id,
            line: *line,
            indent,
            content_hash: content_hash(&text),
            ref_handle: derive_ref_handle(id),
        });
        *line += 1;
        walk_sidecar(workspace, id, indent + 1, line, out);
    }
}

/// Apply a pure-AST mutation to a page's `.md`, then rewrite both the
/// `.md` and its sidecar.
///
/// **This is the path mobile mutations should take.** The workspace
/// op log isn't on the hot edit path here — we read the `.md` as the
/// source of truth, mutate the parsed AST, render it back, and rebuild
/// the sidecar by content-hash-matching the new blocks against the
/// previous sidecar so unchanged blocks keep their `NodeId`. Anything
/// the closure inserts gets a fresh ULID. Peers reading the resulting
/// `.md` + `.outl` see consistent ids.
///
/// The closure receives a map `NodeId -> block_path` derived from the
/// sidecar so callers can translate the ids the frontend passes in
/// (e.g. "create after block ABC") into the path-based mutations that
/// [`outl_md::outline_ops`] expects.
pub fn mutate_page_md<F>(root: &Path, meta: &PageMeta, mutation: F) -> Result<PathBuf, ActionError>
where
    F: FnOnce(
        &mut outl_md::parse::ParsedPage,
        &std::collections::HashMap<NodeId, Vec<usize>>,
    ) -> Result<(), ActionError>,
{
    use std::collections::HashMap;

    let md_path = page_md_path(root, meta);
    let md_text = std::fs::read_to_string(&md_path).unwrap_or_default();
    let mut parsed = outl_md::parse::parse(&md_text);

    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    let old_sidecar = outl_md::sidecar::read(&sidecar_path).ok();

    // Build NodeId -> block_path map from the AST + sidecar (DFS
    // preorder lines up between the two).
    let mut id_to_path: HashMap<NodeId, Vec<usize>> = HashMap::new();
    if let Some(sc) = &old_sidecar {
        let mut iter = sc.blocks.iter();
        build_id_path_map(&parsed.blocks, &mut Vec::new(), &mut iter, &mut id_to_path);
    }

    mutation(&mut parsed, &id_to_path)?;

    let new_md = outl_md::render::render(&parsed);
    outl_md::write_atomic(&md_path, new_md.as_bytes())?;

    let page_id_ulid = ulid::Ulid::from_string(&meta.id)
        .map_err(|e| ActionError::NotInTree(format!("invalid page id {}: {e}", meta.id)))?;
    let page_id = NodeId(page_id_ulid);
    let new_sidecar = build_sidecar_from_ast(&parsed, old_sidecar.as_ref(), &new_md, page_id);
    outl_md::sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(md_path)
}

fn build_id_path_map<'a>(
    blocks: &[outl_md::parse::OutlineNode],
    current_path: &mut Vec<usize>,
    sidecar_iter: &mut std::slice::Iter<'a, SidecarBlock>,
    out: &mut std::collections::HashMap<NodeId, Vec<usize>>,
) {
    for (i, block) in blocks.iter().enumerate() {
        current_path.push(i);
        if let Some(sc) = sidecar_iter.next() {
            out.insert(sc.id, current_path.clone());
        }
        build_id_path_map(&block.children, current_path, sidecar_iter, out);
        current_path.pop();
    }
}

fn build_sidecar_from_ast(
    parsed: &outl_md::parse::ParsedPage,
    old_sidecar: Option<&Sidecar>,
    md: &str,
    page_id: NodeId,
) -> Sidecar {
    use std::collections::HashSet;
    let mut used: HashSet<NodeId> = HashSet::new();
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    walk_ast_for_sidecar(
        &parsed.blocks,
        0,
        old_sidecar,
        &mut used,
        &mut line,
        &mut blocks,
    );
    Sidecar {
        version: outl_md::sidecar::SIDECAR_VERSION,
        page_id,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // Built from a parsed `.md` + workspace tree — both sources
        // already carry the page properties consistently, so this
        // sidecar represents a fully-propagated state.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_ast_for_sidecar(
    blocks: &[outl_md::parse::OutlineNode],
    indent: u32,
    old_sidecar: Option<&Sidecar>,
    used: &mut std::collections::HashSet<NodeId>,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
) {
    for block in blocks {
        let hash = content_hash(&block.text);
        let id = old_sidecar
            .and_then(|sc| {
                sc.blocks
                    .iter()
                    .find(|b| b.content_hash == hash && !used.contains(&b.id))
                    .map(|b| b.id)
            })
            .unwrap_or_else(|| {
                // No content-hash match: this is a freshly inserted
                // block, so allocate a new random id.
                NodeId::new()
            });
        used.insert(id);
        out.push(SidecarBlock {
            id,
            line: *line,
            indent,
            content_hash: hash,
            ref_handle: derive_ref_handle(id),
        });
        *line += 1;
        walk_ast_for_sidecar(&block.children, indent + 1, old_sidecar, used, line, out);
    }
}

/// Render **every** page in the workspace to its `.md` file. Useful
/// after a workspace-wide change (sync pull, migration, …) when we
/// don't know which pages actually moved.
pub fn apply_all_pages_md(workspace: &Workspace, root: &Path) -> Result<Vec<PathBuf>, ActionError> {
    let mut written = Vec::new();
    for meta in list_pages(workspace) {
        let id = parse_node_id(&meta.id)?;
        let md = render_page_md(workspace, id);
        let path = page_md_path(root, &meta);
        write_md_atomic(&path, &md)?;
        written.push(path);
    }
    Ok(written)
}

fn parse_node_id(s: &str) -> Result<NodeId, ActionError> {
    use std::str::FromStr;
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| ActionError::NotInTree(format!("invalid id {s}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::{open_journal, open_or_create, PageKind};
    use chrono::NaiveDate;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use tempfile::TempDir;

    #[test]
    fn render_page_md_outputs_children_only() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("second")).unwrap();

        let md = render_page_md(&ws, page);
        assert_eq!(md, "- first\n- second\n");
    }

    #[test]
    fn page_md_path_routes_journals_and_pages_separately() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        let regular = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        let journal =
            open_journal(&mut ws, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();

        let r_meta = page_meta(&ws, regular).unwrap();
        let j_meta = page_meta(&ws, journal).unwrap();

        assert!(page_md_path(tmp.path(), &r_meta).ends_with("pages/ideas.md"));
        assert!(page_md_path(tmp.path(), &j_meta).ends_with("journals/2026-05-27.md"));
    }

    #[test]
    fn apply_all_pages_writes_each_to_disk() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("first idea")).unwrap();

        let written = apply_all_pages_md(&ws, tmp.path()).unwrap();
        assert_eq!(written.len(), 1);
        let body = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(body, "- first idea\n");
    }
}
