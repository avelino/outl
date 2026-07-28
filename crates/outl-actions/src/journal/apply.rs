//! Write `.md` + `.outl` projections to disk — the `apply_*` family,
//! `mutate_page_md`, and the workspace-wide sweep.

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{
    content_hash, derive_ref_handle, file_hash, sidecar_path_for, Sidecar, SidecarBlock,
};

use super::paths::{page_md_path, write_md_atomic};
use super::render::render_page_md;
use crate::error::ActionError;
use crate::page::{list_all as list_pages, page_meta, PageMeta};
use crate::tree::children_of;

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
    write_page_projection(workspace, root, page_root, &meta, &md)
}

/// Like [`apply_page_md_with_sidecar`] but reuses an already-rendered
/// `md` instead of rendering the page again.
///
/// The GUI commit path renders the page once to diff it for undo; passing
/// that string here saves a second whole-page render (which materializes
/// every block's text). On a large journal that render is tens of ms in
/// release, hundreds in debug, and it ran on every keystroke-commit.
pub fn apply_page_md_with_sidecar_rendered(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    md: &str,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    write_page_projection(workspace, root, page_root, &meta, md)
}

/// Write an already-rendered page `md` to its `.md` and rebuild the matching
/// sidecar from the same tree. Split out of [`apply_page_md_with_sidecar`] so a
/// caller that already rendered the page (to detect a stale projection) reuses
/// that string instead of rendering it a second time.
fn write_page_projection(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    meta: &PageMeta,
    md: &str,
) -> Result<PathBuf, ActionError> {
    let path = page_md_path(root, meta);
    write_md_atomic(&path, md)?;
    let sidecar = build_sidecar(workspace, page_root, md);
    outl_md::sidecar::write(&sidecar_path_for(&path), &sidecar)?;
    Ok(path)
}

/// Like [`apply_page_md_with_sidecar`], but **skips the write when the
/// `.md` file already exists on disk**.
///
/// Use this on read paths (e.g. `open_page_by_slug`) where the goal is
/// to lazily materialise a page that a peer synced into the CRDT tree
/// but never projected to disk on this device.
/// Calling the unconditional variant on every page open would rewrite
/// the `.outl` sidecar on every navigation because `build_sidecar`
/// stamps `last_synced_at: now()` — turning the hottest nav path into
/// constant sync churn even when nothing changed.
///
/// Returns `Some(path)` when the file was absent and was written, or
/// `None` when the file already existed and no I/O was performed.
pub fn apply_page_md_with_sidecar_if_absent(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<Option<PathBuf>, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let path = page_md_path(root, &meta);
    if path.exists() {
        return Ok(None);
    }
    apply_page_md_with_sidecar(workspace, root, page_root).map(Some)
}

/// Like [`apply_page_md_with_sidecar`], but writes **only when the on-disk
/// `.md` is missing or stale relative to the tree**.
///
/// This is the re-projection counterpart to
/// [`apply_page_md_with_sidecar_if_absent`]: that one only covers an *absent*
/// `.md` (a page synced into the tree but never projected here — issue #120).
/// It leaves a page **projected empty before its content synced** stale
/// forever: the file then exists, so the `_if_absent` guard skips it, and the
/// view — which reads the `.md` via [`crate::outline::read_page_outline`] —
/// keeps rendering blank even though the tree holds the blocks. That is the
/// "day created on one device shows empty on another" bug.
///
/// Three cases:
/// - `.md` absent → project it (subsumes `_if_absent`, issue #120).
/// - `.md` present and a **faithful projection** (its hash matches the
///   sidecar's `last_synced_hash`, i.e. no unreconciled external edit) but the
///   tree now renders to something different → re-project it. This is the sync
///   case the bug lives in.
/// - `.md` present but **not** matching its sidecar → an external edit is
///   pending; leave it untouched (`.md → tree` reconcile owns that), so this
///   never clobbers a hand-edited file.
///
/// Only writes on a real change, so it does not churn the sidecar's
/// `last_synced_at` on a page already in sync.
///
/// Returns `Some(path)` when it (re)projected, `None` when it left disk alone.
pub fn apply_page_md_with_sidecar_if_stale(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<Option<PathBuf>, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let path = page_md_path(root, &meta);
    let disk = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Genuinely absent → project it (issue #120).
            return apply_page_md_with_sidecar(workspace, root, page_root).map(Some);
        }
        // Present but unreadable (permissions, non-UTF8, …): do NOT treat as
        // absent — re-projecting would clobber a file that may hold an external
        // edit. Surface the error; the caller logs and leaves the file alone.
        Err(e) => return Err(e.into()),
    };
    let disk_hash = file_hash(&disk);
    // Only re-project a file that is a faithful projection of the tree its
    // sidecar was built from. A `.md` whose hash no longer matches its sidecar
    // carries an external edit — that is the orphan reconcile's job
    // (`.md → tree`); re-projecting here would clobber it.
    let sidecar_path = sidecar_path_for(&path);
    let faithful = outl_md::sidecar::read(&sidecar_path)
        .map(|sc| sc.last_synced_hash == disk_hash)
        .unwrap_or(false);
    if !faithful {
        return Ok(None);
    }
    // The tree has moved past the projection iff rendering it now differs from
    // what is on disk. Render once and reuse it for the write below.
    let rendered = render_page_md(workspace, page_root);
    if file_hash(&rendered) == disk_hash {
        return Ok(None);
    }
    write_page_projection(workspace, root, page_root, &meta, &rendered).map(Some)
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
