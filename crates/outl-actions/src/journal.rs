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

/// Remove a page's `.md` projection and its `.outl` sidecar from disk.
///
/// The inverse of [`apply_page_md_with_sidecar`]: after
/// [`crate::page::delete`] moves the page root to the trash, the
/// on-disk projection would otherwise linger. A peer that hasn't
/// received the delete op yet would keep reading the stale `.md`;
/// removing the projection here on the acting device means the next
/// `outl doctor` / orphan scan agrees with the op log, and the page
/// disappears from listings that walk `pages/` directly.
///
/// Idempotent: a missing file is silently OK (the page may never have
/// been projected on this device — common right after a peer-shipped
/// delete). Any other I/O error is returned so the caller can decide
/// whether to swallow (CLI's `remove_or_warn`) or propagate (Tauri
/// command's error envelope).
pub fn remove_page_projection(root: &Path, meta: &PageMeta) -> std::io::Result<()> {
    let md_path = page_md_path(root, meta);
    match std::fs::remove_file(&md_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    match std::fs::remove_file(&sidecar_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(())
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

/// Render every block under `page_root` to a clean `.md` string,
/// **including** the page-level properties stored on the page node
/// (`title::`, `icon::`, `pinned::`, `type::`, `role::`, anything
/// custom). The page's title (`workspace.block_text(page_root)`) is
/// **not** included in the body — clients can prepend it themselves
/// if they want.
///
/// Internal book-keeping keys (`page-slug` / `page-kind`) are skipped:
/// the page-model layer (`outl_actions::page`) owns those through its
/// own ops; surfacing them in the rendered `.md` would re-write the
/// slug on every reconcile (a no-op via the CRDT, but noise on disk).
///
/// Sort order is alphabetical on the key — `HashMap::iter` is
/// unordered, and we don't want the rendered `.md` to flap between
/// runs. The renderer doesn't care about order; users do.
pub fn render_page_md(workspace: &Workspace, page_root: NodeId) -> String {
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(page_root)
        .filter(|(k, _)| {
            // Skip internal book-keeping owned by `outl_actions::page`.
            *k != crate::page::SLUG_KEY && *k != crate::page::KIND_KEY
        })
        .filter_map(|(k, v)| match v {
            // Only textual properties round-trip through the `.md`
            // dialect. PageRef / Tag / List shapes would need
            // dedicated render syntax — skip them silently for now,
            // and revisit if a real consumer asks for them.
            outl_core::property::PropValue::Text(s) => Some((k.to_string(), s.clone())),
            outl_core::property::PropValue::PageRef(s) | outl_core::property::PropValue::Tag(s) => {
                Some((k.to_string(), s.clone()))
            }
            outl_core::property::PropValue::List(_) => None,
        })
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));

    let page = ParsedPage {
        properties,
        blocks: build_outline(workspace, page_root),
        warnings: Vec::new(),
    };
    render(&page)
}

/// Render the block `node` and its subtree to clean outl markdown as
/// a single top-level bullet (with its descendants nested under it).
///
/// This is the "copy block" projection: the desktop's `Cmd+C` in view
/// mode hands the result to the clipboard, and the matching paste
/// re-ingests it through the same `paste_markdown` pipeline external
/// clipboard text uses — so a copy duplicates the subtree with fresh
/// ids. Reuses the exact projection [`render_page_md`] writes to disk,
/// so a copied block reads identically to how it lives in the `.md`.
pub fn render_block_md(workspace: &Workspace, node: NodeId) -> String {
    let block = OutlineNode {
        text: workspace.block_text(node).unwrap_or_default(),
        properties: Vec::new(),
        children: build_outline(workspace, node),
    };
    let page = ParsedPage {
        properties: Vec::new(),
        blocks: vec![block],
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
    let Ok(disk) = std::fs::read_to_string(&path) else {
        // Absent (or unreadable) → project it (issue #120).
        return apply_page_md_with_sidecar(workspace, root, page_root).map(Some);
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
    // what is on disk.
    if file_hash(&render_page_md(workspace, page_root)) == disk_hash {
        return Ok(None);
    }
    apply_page_md_with_sidecar(workspace, root, page_root).map(Some)
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
    fn render_page_md_outputs_title_prop_then_children() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("second")).unwrap();

        // The title lives in the `title::` property (not the root's text),
        // so it renders as a page property above the children.
        let md = render_page_md(&ws, page);
        assert_eq!(md, "title:: Ideas\n\n- first\n- second\n");
    }

    /// Build a page projected while it held `initial`, then return the
    /// `(workspace, hlc, root, page_id, md_path)` so a test can drive the
    /// stale-projection scenarios. The `TempDir` is returned to keep it alive.
    fn projected_page(initial: &str) -> (TempDir, Workspace, HlcGenerator, NodeId, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some(initial)).unwrap();
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
        let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
        (tmp, ws, hlc, page, md_path)
    }

    /// The reported bug: a peer's op lands in the TREE, but the already-present
    /// `.md` the view reads is never re-projected, so the page renders empty.
    /// `_if_stale` must detect the tree ran ahead and re-project.
    #[test]
    fn if_stale_reprojects_when_tree_ran_ahead_of_the_md() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
        assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));

        // A synced-in block enters the tree; nothing re-projects the `.md`.
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();
        assert!(!std::fs::read_to_string(&md_path)
            .unwrap()
            .contains("synced-in"));

        let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
        assert!(
            wrote.is_some(),
            "a tree ahead of its .md must be re-projected"
        );
        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(
            md.contains("first") && md.contains("synced-in"),
            "re-projection must carry the synced-in block: {md:?}"
        );
    }

    /// An in-sync page must NOT be re-projected — otherwise every nav churns the
    /// sidecar's `last_synced_at` and floods sync (the reason `_if_absent`
    /// existed in the first place).
    #[test]
    fn if_stale_is_a_noop_when_the_md_matches_the_tree() {
        let (tmp, ws, _hlc, page, md_path) = projected_page("first");
        let before = std::fs::read_to_string(&md_path).unwrap();
        let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
        assert!(wrote.is_none(), "an in-sync page must not be re-projected");
        assert_eq!(std::fs::read_to_string(&md_path).unwrap(), before);
    }

    /// Absent `.md` (a peer synced the page into the tree but it was never
    /// projected here) → project it. Subsumes `_if_absent` (issue #120).
    #[test]
    fn if_stale_projects_a_page_whose_md_is_absent() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
        let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
        assert!(!md_path.exists());

        let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
        assert!(wrote.is_some());
        assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));
    }

    /// A `.md` whose hash no longer matches its sidecar carries an unreconciled
    /// external edit — `_if_stale` must leave it for the `.md → tree` reconcile,
    /// never clobber it with a tree re-projection.
    #[test]
    fn if_stale_never_clobbers_an_external_edit() {
        let (tmp, ws, _hlc, page, md_path) = projected_page("first");
        std::fs::write(&md_path, "- hand edited externally\n").unwrap();

        let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
        assert!(
            wrote.is_none(),
            "an externally-edited .md must not be clobbered"
        );
        assert_eq!(
            std::fs::read_to_string(&md_path).unwrap(),
            "- hand edited externally\n"
        );
    }

    /// Copy (`Cmd+C` in view mode) snapshots a block via
    /// `render_block_md`. It must capture the **whole** subtree — every
    /// descendant at every depth — so a paste reproduces the block in
    /// full. This pins the "are we grabbing all the sub-blocks?" review
    /// concern: the renderer walks `build_outline` recursively, so a
    /// four-level-deep subtree round-trips with its indentation intact.
    #[test]
    fn render_block_md_captures_the_full_deep_subtree() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        let src = append_block(&mut ws, &hlc, Some(page), Some("src")).unwrap();
        let c1 = append_block(&mut ws, &hlc, Some(src), Some("c1")).unwrap();
        let c1a = append_block(&mut ws, &hlc, Some(c1), Some("c1a")).unwrap();
        append_block(&mut ws, &hlc, Some(c1a), Some("c1a_i")).unwrap();
        append_block(&mut ws, &hlc, Some(c1), Some("c1b")).unwrap();
        append_block(&mut ws, &hlc, Some(src), Some("c2")).unwrap();

        let md = render_block_md(&ws, src);
        assert_eq!(
            md,
            "- src\n  - c1\n    - c1a\n      - c1a_i\n    - c1b\n  - c2\n"
        );
    }

    #[test]
    fn render_page_md_emits_page_level_properties() {
        // Regression for the silent divergence between the op log
        // and the rendered `.md`. Page-level properties (`type::`,
        // `icon::`, etc.) used to be dropped on render because
        // `render_page_md` always passed `properties: Vec::new()`.
        // Result: a person page created via `@` autocomplete in the
        // TUI carried `Op::SetProp { type: person }` in the log but
        // its `.md` had only the blocks — the `WorkspaceIndex`
        // (which parses `.md`) didn't list it under `pages_by_type`,
        // so the next `@` mention never surfaced it.
        use crate::page::set_property;
        use outl_core::property::PropValue;

        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        set_property(
            &mut ws,
            &hlc,
            page,
            crate::person::TYPE_KEY,
            Some(PropValue::Text(crate::person::PERSON_TYPE.to_string())),
        )
        .unwrap();
        set_property(
            &mut ws,
            &hlc,
            page,
            "icon",
            Some(PropValue::Text("🦀".to_string())),
        )
        .unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("bio")).unwrap();

        let md = render_page_md(&ws, page);
        assert!(
            md.contains("type:: person"),
            "rendered .md must carry the type:: person property; got:\n{md}"
        );
        assert!(
            md.contains("icon:: 🦀"),
            "rendered .md must carry the icon property; got:\n{md}"
        );
        // `page-slug` / `page-kind` stay internal — they're owned by
        // the page-model layer, not by the rendered `.md`.
        assert!(
            !md.contains("page-slug"),
            "internal book-keeping property leaked into rendered .md:\n{md}"
        );
        assert!(
            !md.contains("page-kind"),
            "internal book-keeping property leaked into rendered .md:\n{md}"
        );
        // Body still renders the block.
        assert!(md.contains("- bio"));
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
        // In-app pages store their title in the `title::` property (not the
        // root's Yrs text — see `open_or_create`), so it renders at the top.
        assert_eq!(body, "title:: Ideas\n\n- first idea\n");
    }

    /// Regression for https://github.com/avelino/outl/issues/120 —
    /// a page synced from a peer exists in the CRDT tree but has no
    /// `.md` on this device's disk. `open_page_by_slug` calls
    /// `apply_page_md_with_sidecar_if_absent`; without the projection
    /// `read_page_outline` returns an empty outline and the page opens
    /// blank. This test models that scenario: page in workspace, no
    /// file on disk → helper writes the projection → outline is populated.
    #[test]
    fn apply_if_absent_projects_when_md_is_missing() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        // Simulate a synced page: exists in the CRDT tree, no .md on disk.
        let page = open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("peer block")).unwrap();

        // Pre-condition: no .md on disk yet.
        let meta = page_meta(&ws, page).unwrap();
        let path = page_md_path(tmp.path(), &meta);
        assert!(!path.exists(), "test setup error: .md should not exist yet");

        // Call the guarded helper — should project because the file is absent.
        let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
        assert!(
            result.is_some(),
            "expected Some(path) when .md was absent, got None"
        );
        assert!(path.exists(), ".md must be on disk after projection");

        // The projected content must match the CRDT tree (not be empty).
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            body, "title:: Synced\n\n- peer block\n",
            "projected .md must contain the peer's block, not be blank"
        );

        // `read_page_outline` (the path `open_page_by_slug` takes after
        // the projection) must now return populated content.
        let outline = crate::outline::read_page_outline(tmp.path(), &meta).unwrap();
        assert_eq!(outline.nodes.len(), 1, "outline must have the peer's block");
        assert_eq!(outline.nodes[0].text, "peer block");
    }

    /// Guard against sync churn: calling `apply_page_md_with_sidecar_if_absent`
    /// on a page whose `.md` is already on disk must be a **no-op** — it must
    /// not rewrite the `.outl` sidecar. `build_sidecar` stamps
    /// `last_synced_at: now()`, so an unconditional call would rewrite
    /// the sidecar bytes on every page open, generating noise for every
    /// file-transport peer (iCloud / Syncthing) even when nothing changed.
    #[test]
    fn apply_if_absent_is_noop_when_md_already_exists() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let tmp = TempDir::new().unwrap();

        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("a block")).unwrap();

        // First projection: write the .md and .outl to disk.
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

        let meta = page_meta(&ws, page).unwrap();
        let md_path = page_md_path(tmp.path(), &meta);
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);

        // Capture the sidecar bytes before the guarded call.
        let sidecar_before = std::fs::read(&sidecar_path).unwrap();

        // Give the clock a chance to tick so a second `now()` stamp
        // would differ if the sidecar were rewritten.
        std::thread::sleep(std::time::Duration::from_millis(5));

        // Guarded call — file exists, must be a no-op.
        let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
        assert!(
            result.is_none(),
            "expected None when .md already exists, got Some"
        );

        // Sidecar bytes must be unchanged (no `last_synced_at: now()` rewrite).
        let sidecar_after = std::fs::read(&sidecar_path).unwrap();
        assert_eq!(
            sidecar_before, sidecar_after,
            ".outl sidecar must not be rewritten when .md already existed"
        );
    }
}
