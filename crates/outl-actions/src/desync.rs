//! Recovery for projections that ran ahead of the op log.
//!
//! Normally the `.md` + `.outl` pair on disk is a *projection* of the
//! op log.
//! But a crash between "projection written" and "ops appended" (e.g.
//! the OS killing the mobile app right after an offline edit) leaves
//! the inverse: the `.md` and its sidecar carry blocks whose ids exist
//! in **no** op log — the projection walked ahead of the source of
//! truth and the ops are gone.
//!
//! The cheap hash gate ([`crate::sync::SyncEngine::scan_for_orphans`])
//! cannot see this: the sidecar was written together with the `.md`,
//! so `last_synced_hash` matches and the page looks "in sync" forever.
//! The blocks stay visible in the local `.md` and invisible to the
//! CRDT, to search, and to every peer.
//!
//! [`scan_for_desynced_projections`] detects the case — it needs the
//! materialised tree, hence a [`Workspace`], which is exactly why it
//! can't live inside the workspace-free hash scan — and
//! [`recover_desynced_projection`] repairs it by re-emitting ops that
//! recreate the missing blocks **preserving the sidecar ids** (so
//! `((blk-XXXXXX))` handles keep resolving), then re-projecting the
//! merged page.
//!
//! ## Semantics: strictly additive
//!
//! Blocks already known to the tree are never touched.
//! In particular a block the log says is deleted (moved to
//! `TRASH_ROOT`) is **not** resurrected just because the stale `.md`
//! still shows it: a remote delete IS an op, so the id is present in
//! the tree (under trash) and the recovery skips it.
//! Total absence from every op log means "the ops were lost", which is
//! the opposite situation — those blocks are recreated.
//! Blocks that live in the tree but not in the stale `.md` (e.g. a
//! peer edit that arrived while the projection was frozen) remain
//! untouched and reappear when the page is re-projected at the end.

use std::fs;
use std::path::{Path, PathBuf};

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::matching::flatten;
use outl_md::sidecar::{content_hash, file_hash, SidecarBlock};
use outl_md::OutlineNode;
use tracing::warn;

use crate::error::ActionError;
use crate::journal::apply_page_md_with_sidecar;
use crate::page::{KIND_KEY, SLUG_KEY};

/// Find every `.md` under `journals/` and `pages/` whose sidecar is
/// hash-in-sync with the file **but** references ids the materialised
/// tree has never seen — the "projection ahead of the op log" state
/// the hash-based orphan scan is structurally blind to.
///
/// Cost per page: one file read + hash, one sidecar JSON parse, and
/// one O(1) tree lookup per sidecar block.
/// The common case (every id present) short-circuits on the first
/// check, so running this on boot next to the orphan scan is cheap.
///
/// Files the orphan scan already flags (missing sidecar, stale hash)
/// are deliberately **excluded** — `reconcile_md` owns those and its
/// matching pass already recreates missing ids via `Op::Create`.
pub fn scan_for_desynced_projections(ws: &Workspace, workspace_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["journals", "pages"] {
        scan_dir(ws, &workspace_root.join(sub), &mut out);
    }
    out.sort();
    out
}

fn scan_dir(ws: &Workspace, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(ws, &path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if is_desynced(ws, &path) {
            out.push(path);
        }
    }
}

/// `true` when the sidecar next to `md_path` is hash-in-sync with the
/// file but carries at least one id (page or block) the tree does not
/// know.
///
/// `Tree::parent(id)` is `None` only for ids no op log ever created —
/// a trashed node still has a parent (`TRASH_ROOT`), so a legitimate
/// remote delete does **not** trip this check.
fn is_desynced(ws: &Workspace, md_path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(md_path) else {
        return false;
    };
    let sidecar_path = outl_md::resolve_sidecar_path(md_path);
    let Ok(sc) = outl_md::sidecar::read(&sidecar_path) else {
        // Missing / unreadable sidecar → the orphan scan owns it.
        return false;
    };
    if sc.last_synced_hash != file_hash(&text) {
        // Stale hash → `needs_reconcile` already flags it.
        return false;
    }
    if ws.tree().parent(sc.page_id).is_none() {
        return true;
    }
    sc.blocks.iter().any(|b| ws.tree().parent(b.id).is_none())
}

/// Re-emit the ops a lost commit should have produced for `md_path`,
/// recreating every sidecar block the tree has never seen — same id,
/// same text, same block properties, positioned to preserve the
/// `.md`'s sibling order relative to the blocks that do exist.
///
/// Strictly additive (see the module docs): blocks already in the tree
/// — including trashed ones — are never created, moved, or edited.
/// When at least one op was applied the page is re-projected
/// (`.md` + sidecar) so the on-disk view shows the merged state.
///
/// Returns the number of ops applied.
/// Returns `Ok(0)` without touching anything when the file no longer
/// qualifies (sidecar missing, hash out of sync — both owned by the
/// normal reconcile path) or when the sidecar and the parsed `.md`
/// disagree structurally (corrupt pair; `outl doctor` territory).
pub fn recover_desynced_projection(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    workspace_root: &Path,
    md_path: &Path,
) -> Result<usize, ActionError> {
    let md_text = fs::read_to_string(md_path)?;
    let sidecar_path = outl_md::resolve_sidecar_path(md_path);
    let Ok(sc) = outl_md::sidecar::read(&sidecar_path) else {
        return Ok(0);
    };
    if sc.last_synced_hash != file_hash(&md_text) {
        return Ok(0);
    }

    let ast = outl_md::parse(&md_text);
    // The sidecar and the `.md` were written together (the file hash
    // matched `last_synced_hash`), so a DFS-preorder walk of the AST
    // lines up 1:1 with `sc.blocks`. Verify before trusting the
    // pairing — a corrupt sidecar must not graft ids onto the wrong
    // blocks.
    if !lockstep_matches(&ast.blocks, &sc.blocks) {
        warn!(
            "desync recovery skipped {}: sidecar does not line up with the parsed .md",
            md_path.display()
        );
        return Ok(0);
    }

    let page_id = sc.page_id;
    // The page node itself may be missing too (whole page authored
    // offline, ops lost). Reuse the reconcile pipeline's materialiser
    // instead of re-deriving slug/kind here.
    let mut applied = outl_md::reconcile::ensure_page_root_in_tree(ws, hlc, page_id, md_path)?;

    // Page-level properties that never reached the log. Only absent
    // keys are filled: an existing value came from a real op, and the
    // log wins over a stale projection.
    for (key, value) in &ast.properties {
        if key == SLUG_KEY || key == KIND_KEY {
            continue;
        }
        if ws.tree().property(page_id, key).is_some() {
            continue;
        }
        apply_log_op(
            ws,
            hlc,
            Op::SetProp {
                node: page_id,
                key: key.clone(),
                value: Some(PropValue::Text(value.clone())),
                old_value: None,
            },
        )?;
        applied += 1;
    }

    recover_level(ws, hlc, &ast.blocks, &sc.blocks, page_id, 0, &mut applied)?;

    if applied > 0 {
        // Re-project so the on-disk `.md` + sidecar show the merged
        // state: recovered blocks AND everything the log already had
        // that the frozen projection was missing.
        apply_page_md_with_sidecar(ws, workspace_root, page_id)?;
    }
    Ok(applied)
}

/// `true` when a DFS-preorder flatten of `blocks` pairs 1:1 (count and
/// per-block content hash) with the sidecar's block list.
fn lockstep_matches(blocks: &[OutlineNode], entries: &[SidecarBlock]) -> bool {
    let flat = flatten(blocks);
    flat.len() == entries.len()
        && flat
            .iter()
            .zip(entries)
            .all(|(f, e)| content_hash(f.text) == e.content_hash)
}

/// Number of nodes in `block`'s subtree, itself included.
fn subtree_len(block: &OutlineNode) -> usize {
    1 + block.children.iter().map(subtree_len).sum::<usize>()
}

/// Walk one sibling list of the AST (whose entries start at `start` in
/// the DFS-preorder sidecar list) and create every block the tree has
/// never seen, preserving `.md` order relative to existing siblings.
fn recover_level(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    blocks: &[OutlineNode],
    entries: &[SidecarBlock],
    parent: NodeId,
    start: usize,
    applied: &mut usize,
) -> Result<(), ActionError> {
    // Sidecar index of each direct child in this sibling list.
    let mut child_idx = Vec::with_capacity(blocks.len());
    let mut idx = start;
    for block in blocks {
        child_idx.push(idx);
        idx += subtree_len(block);
    }

    // `left` tracks the position of the previous `.md` sibling that
    // sits under `parent` (pre-existing or just created), so recovered
    // blocks keep their `.md` order without moving anything.
    //
    // It starts at the parent's current **last** child position (not
    // `None`): blocks the log has that the stale `.md` doesn't (peer
    // edits that landed while the projection was frozen) are invisible
    // to this walk, and `Fractional::between(None, None)` would hand
    // every run the same midpoint key — colliding with them and making
    // `children_of`'s tie-break (HashMap order) decide the rendered
    // order. Appending after the existing children keeps the recovered
    // run deterministic and collision-free while preserving `.md`
    // order *within* the run.
    let mut left: Option<Fractional> = crate::tree::children_of(ws, parent)
        .last()
        .map(|(_, pos)| pos.clone());
    for (i, block) in blocks.iter().enumerate() {
        let id = entries[child_idx[i]].id;
        if ws.tree().parent(id).is_none() {
            // Right bound: the nearest following `.md` sibling already
            // under this parent. A peer move may have reordered
            // siblings, making the bound ≤ left — fall back to
            // appending after `left` in that case.
            let right = child_idx[i + 1..]
                .iter()
                .find_map(|&ei| {
                    let sid = entries[ei].id;
                    if ws.tree().parent(sid) == Some(parent) {
                        ws.tree().position(sid).cloned()
                    } else {
                        None
                    }
                })
                .filter(|r| left.as_ref().is_none_or(|l| l < r));
            let position = Fractional::between(left.as_ref(), right.as_ref());
            apply_log_op(
                ws,
                hlc,
                Op::Create {
                    node: id,
                    parent,
                    position: position.clone(),
                },
            )?;
            *applied += 1;
            if !block.text.is_empty() {
                let update = ws.build_text_replace_update(id, &block.text);
                if !update.is_empty() {
                    apply_log_op(
                        ws,
                        hlc,
                        Op::Edit {
                            node: id,
                            text_op: update,
                        },
                    )?;
                    *applied += 1;
                }
            }
            for (k, v) in &block.properties {
                apply_log_op(
                    ws,
                    hlc,
                    Op::SetProp {
                        node: id,
                        key: k.clone(),
                        value: Some(PropValue::Text(v.clone())),
                        old_value: None,
                    },
                )?;
                *applied += 1;
            }
            left = Some(position);
        } else if ws.tree().parent(id) == Some(parent) {
            left = ws.tree().position(id).cloned();
        }
        // else: the log moved this block elsewhere (another page, or
        // trash — a real remote delete). The log wins: don't touch it
        // and don't use its position as a bound under this parent.

        // Children recurse against the `.md` parent id even when the
        // parent itself was skipped: a missing child still belongs to
        // that id wherever the log placed it.
        recover_level(
            ws,
            hlc,
            &block.children,
            entries,
            id,
            child_idx[i] + 1,
            applied,
        )?;
    }
    Ok(())
}

fn apply_log_op(ws: &mut Workspace, hlc: &HlcGenerator, op: Op) -> Result<(), ActionError> {
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op,
    })?;
    Ok(())
}
