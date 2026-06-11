//! Reconcile a single `.md` file against a `Workspace`.
//!
//! Used by `outl serve` (file watcher), `outl init` (initial seed), and
//! `outl-tui` (after editor commits). Reads the `.md`, optionally loads
//! the sidecar, runs 3-level matching, emits the minimal op sequence,
//! applies it, and writes back the refreshed sidecar.
//!
//! Orphan ids are logged before being moved to `TRASH_ROOT`, so a
//! deletion is never silent.

use crate::matching::match_blocks;
use crate::parse::{parse, OutlineNode};
use crate::sidecar::{self, file_hash, sidecar_path_for, Sidecar, SidecarBlock, SIDECAR_VERSION};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::{Workspace, WorkspaceError};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Outcome of one reconcile pass.
#[derive(Debug, Clone)]
pub struct ReconcileReport {
    /// Path of the `.md` file processed.
    pub md_path: PathBuf,
    /// Number of ops produced and applied.
    pub ops_applied: usize,
    /// Number of orphan ids logged.
    pub orphans: usize,
    /// Whether the sidecar was created fresh.
    pub created_sidecar: bool,
}

/// Errors a reconcile pass may surface.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    /// Filesystem error reading or writing files.
    #[error("io error on {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Invalid sidecar payload.
    #[error("sidecar error: {0}")]
    Sidecar(#[from] sidecar::SidecarError),
    /// Workspace failed to apply an op.
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
}

fn io_err(path: &Path, source: io::Error) -> ReconcileError {
    ReconcileError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Reconcile a single `.md` file with the workspace.
///
/// `orphan_log_path` receives one line per orphan id surfaced during
/// matching. Pass `None` to suppress logging (mostly useful for tests).
pub fn reconcile_md(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    let md_text = fs::read_to_string(md_path).map_err(|e| io_err(md_path, e))?;
    let new_ast = parse(&md_text);
    let md_hash = file_hash(&md_text);

    let sidecar_path = sidecar_path_for(md_path);
    // Read the sidecar **once** — both the diff (which needs `page_id`
    // and `old_blocks`) and the short-circuit below (which needs
    // `last_synced_hash` + `pipeline_version`) consume the same JSON.
    // A previous version read it twice with no lock between the
    // reads, leaving a window where another process could rewrite
    // the file mid-call and the two reads would disagree.
    let existing_sidecar: Option<Sidecar> = match sidecar::read(&sidecar_path) {
        Ok(sc) => Some(sc),
        Err(sidecar::SidecarError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        Err(_) => None, // Corrupt sidecar — rebuild from scratch.
    };
    let (page_id, old_blocks, created_sidecar) = match &existing_sidecar {
        Some(sc) => (sc.page_id, sc.blocks.clone(), false),
        None => (NodeId::new(), Vec::new(), true),
    };

    // Short-circuit: file unchanged since last sync AND the sidecar
    // was produced by the current reconcile pipeline (or newer). The
    // `pipeline_version` clause is what triggers the one-shot
    // migration for sidecars predating `diff_to_ops_with_page_props`
    // and `ensure_page_root_in_tree` — without it, legacy pages whose
    // hash hasn't changed (the common case: fixtures, imports,
    // anything authored before the page-prop pipeline) skip the
    // migration silently and the desktop / mobile keep seeing empty
    // page properties while the `.md` shows them.
    if let Some(existing) = &existing_sidecar {
        if existing.last_synced_hash == md_hash
            && existing.pipeline_version >= crate::sidecar::CURRENT_PIPELINE_VERSION
        {
            return Ok(ReconcileReport {
                md_path: md_path.to_path_buf(),
                ops_applied: 0,
                orphans: 0,
                created_sidecar: false,
            });
        }
    }

    let (matches, orphans) = match_blocks(&new_ast.blocks, &old_blocks);

    if !orphans.is_empty() {
        if let Some(log_path) = orphan_log_path {
            log_orphans(log_path, md_path, &orphans, &old_blocks)?;
        }
    }

    let mut ops_applied = 0usize;

    // **Materialise the page root** as a child of `NodeId::root` with
    // `page-slug` + `page-kind` set. Without this, a `.md` authored
    // externally (vim, peer via iCloud, Roam import) emits `Create`
    // ops for the blocks (whose `parent` is the `page_id`) but leaves
    // the page node itself as an unrooted ghost. The CRDT happily
    // stores blocks under it, but `children_of(root)` doesn't list
    // it as a page — so `list_all_pages`, `search_persons`, and the
    // sidebar all miss it silently. The `WorkspaceIndex`-driven
    // surfaces (TUI autocomplete, picker preview) still see the page
    // because they parse `.md` from disk; that hid the bug.
    //
    // Each call is idempotent: we emit `Op::Move` / `Op::SetProp` only
    // when the workspace tree disagrees with what the filesystem says
    // the page should look like. Pages created via the UI
    // (`open_or_create_by_name`) already carry the right state, so
    // this is a no-op for them.
    ops_applied += ensure_page_root_in_tree(ws, hlc, page_id, md_path)?;

    let plan = crate::diff::diff_to_ops_with_page_props(
        &new_ast.blocks,
        &matches,
        &orphans,
        page_id,
        &md_hash,
        &old_blocks,
        &new_ast.properties,
    );

    for op in plan.ops {
        let ts = hlc.next();
        let log_op = LogOp {
            ts,
            actor: ts.actor,
            op,
        };
        ws.apply(log_op)?;
        ops_applied += 1;
    }

    // Synchronise block text with the workspace.
    //
    // `diff_to_ops` only knows about tree structure (Create / Move /
    // SetProp). It never emits `Op::Edit` because computing the Yrs
    // delta needs the live workspace, which isn't in its scope. The
    // result is a tree of nodes that exist but have empty text — fine
    // when the only consumer is the local sidecar (which carries the
    // content hash) but **catastrophic across devices**: a peer
    // replaying the op log materialises empty blocks, regenerates
    // `.md` from that empty state, and iCloud syncs the empty `.md`
    // back to us. Every text edit silently turns into a deletion.
    //
    // Fix: walk the new AST in lockstep with the freshly built
    // sidecar block list (same DFS preorder) and emit one
    // `Op::Edit` per block whose text doesn't match what the
    // workspace already has. Idempotent: `build_text_replace_update`
    // returns an empty update when text is unchanged.
    ops_applied += sync_block_text(ws, hlc, &new_ast.blocks, &plan.new_sidecar.blocks)?;

    let new_sidecar = Sidecar {
        version: SIDECAR_VERSION,
        page_id,
        last_synced_hash: md_hash,
        last_synced_at: plan.new_sidecar.last_synced_at,
        blocks: plan.new_sidecar.blocks,
        pipeline_version: plan.new_sidecar.pipeline_version,
    };
    sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(ReconcileReport {
        md_path: md_path.to_path_buf(),
        ops_applied,
        orphans: orphans.len(),
        created_sidecar,
    })
}

/// Guarantee the page node `page_id` is rooted in the workspace tree
/// as a child of `NodeId::root` with the `page-slug` / `page-kind`
/// properties set, deriving the slug from the filename and the kind
/// from the parent directory (`pages/` vs `journals/`).
///
/// Returns the number of ops applied (0–3). Idempotent: each op is
/// emitted only when the workspace state disagrees with what the
/// filesystem says the page should look like.
///
/// Why this lives in `outl-md` and inlines the key constants:
/// `page-slug` / `page-kind` are owned by `outl-actions::page` but
/// `outl-md` cannot depend on it (layering). The pair of strings
/// stays inlined — if either side ever renames, both call-sites need
/// to update together (the diff.rs's `PAGE_SLUG_KEY` skip-list and
/// here). Keep these in sync with `outl_actions::page::{SLUG_KEY,
/// KIND_KEY}`.
fn ensure_page_root_in_tree(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    page_id: NodeId,
    md_path: &Path,
) -> Result<usize, WorkspaceError> {
    const PAGE_SLUG_KEY: &str = "page-slug";
    const PAGE_KIND_KEY: &str = "page-kind";

    let slug = md_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let kind_value = if md_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("journals")
    {
        "journal"
    } else {
        "page"
    };

    let mut applied = 0usize;

    // **Materialise the page node in the tree.**
    //
    // `Op::Move` on a node that has never been `Op::Create`d is a
    // **no-op** inside `tree::do_op` (see `outl-core/src/tree/op.rs`,
    // the `None` arm of the `match self.nodes.get(node)`). Pages
    // authored externally (`vim` writing `pages/samara.md` directly)
    // never receive a Create through any pipeline — `reconcile_md`
    // only emits Create for the blocks inside the page, never for the
    // page node itself. So emitting only `Op::Move` here would
    // silently fail, the page would never appear under
    // `children_of(root)`, and `search_persons` / `list_all_pages`
    // would skip it forever. That was the bug behind "samara has
    // `type:: person` in the .md, the op log has the SetProp, the
    // sidecar says `pipeline_v2_complete: true`, but the desktop
    // autocomplete still doesn't see it".
    //
    // Three cases:
    //   - node absent from `self.nodes` (parent == None) → Create at root.
    //   - node present but parented somewhere other than root → Move to root.
    //   - node already at root → no-op.
    let current_parent = ws.tree().parent(page_id);
    if current_parent.is_none() {
        // Fresh page: emit `Op::Create` so the node lands in
        // `self.nodes` with the correct parent. Subsequent block
        // `Op::Create` ops (whose parent is `page_id`) and
        // `Op::SetProp` ops were already idempotent against
        // non-existent nodes for `SetProp`, but `Move` was the
        // failure mode that masked this bug for months.
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::Create {
                node: page_id,
                parent: NodeId::root(),
                position: outl_core::fractional::Fractional::between(None, None),
            },
        })?;
        applied += 1;
    } else if let Some(old_parent) = current_parent.filter(|p| *p != NodeId::root()) {
        // Node exists somewhere else in the tree (rare: shouldn't
        // happen for orphan reconcile, but covers the case where a
        // page node migrates from being a block descendant — defensive).
        let old_position = ws
            .tree()
            .position(page_id)
            .cloned()
            .unwrap_or_else(outl_core::fractional::Fractional::first);
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::Move {
                node: page_id,
                new_parent: NodeId::root(),
                position: outl_core::fractional::Fractional::between(None, None),
                old_parent,
                old_position,
            },
        })?;
        applied += 1;
    }
    // `page-slug` property: must equal the filename stem.
    let want_slug = outl_core::property::PropValue::Text(slug.clone());
    if ws.tree().property(page_id, PAGE_SLUG_KEY) != Some(&want_slug) {
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::SetProp {
                node: page_id,
                key: PAGE_SLUG_KEY.to_string(),
                value: Some(want_slug),
                old_value: None,
            },
        })?;
        applied += 1;
    }
    // `page-kind` property: `page` or `journal` based on the directory.
    let want_kind = outl_core::property::PropValue::Text(kind_value.to_string());
    if ws.tree().property(page_id, PAGE_KIND_KEY) != Some(&want_kind) {
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::SetProp {
                node: page_id,
                key: PAGE_KIND_KEY.to_string(),
                value: Some(want_kind),
                old_value: None,
            },
        })?;
        applied += 1;
    }
    Ok(applied)
}

/// Walk the parsed AST and the freshly built sidecar block list in
/// lockstep (both in DFS preorder) and emit one `Op::Edit` per block
/// whose text doesn't already match what's in the workspace.
///
/// Returns the number of `Op::Edit` ops applied. Idempotent: skips
/// blocks whose text already matches (the Yrs delta would be empty).
fn sync_block_text(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    ast_blocks: &[OutlineNode],
    sidecar_blocks: &[SidecarBlock],
) -> Result<usize, WorkspaceError> {
    let mut idx = 0usize;
    let mut applied = 0usize;
    walk_text_sync(ws, hlc, ast_blocks, sidecar_blocks, &mut idx, &mut applied)?;
    Ok(applied)
}

fn walk_text_sync(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    ast_blocks: &[OutlineNode],
    sidecar_blocks: &[SidecarBlock],
    idx: &mut usize,
    applied: &mut usize,
) -> Result<(), WorkspaceError> {
    for block in ast_blocks {
        if let Some(entry) = sidecar_blocks.get(*idx) {
            let node = entry.id;
            let current = ws.block_text(node).unwrap_or_default();
            if current != block.text {
                let update = ws.build_text_replace_update(node, &block.text);
                if !update.is_empty() {
                    let ts = hlc.next();
                    ws.apply(LogOp {
                        ts,
                        actor: ts.actor,
                        op: Op::Edit {
                            node,
                            text_op: update,
                        },
                    })?;
                    *applied += 1;
                }
            }
        }
        *idx += 1;
        walk_text_sync(ws, hlc, &block.children, sidecar_blocks, idx, applied)?;
    }
    Ok(())
}

fn log_orphans(
    log_path: &Path,
    md_path: &Path,
    orphans: &[NodeId],
    old_blocks: &[SidecarBlock],
) -> Result<(), ReconcileError> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| io_err(log_path, e))?;
    let now = chrono::Local::now().to_rfc3339();
    for id in orphans {
        let hash_snippet = old_blocks
            .iter()
            .find(|b| b.id == *id)
            .map(|b| b.content_hash.as_str())
            .unwrap_or("?");
        writeln!(
            f,
            "{now}\tmd={}\tid={}\thash={}",
            md_path.display(),
            id,
            hash_snippet,
        )
        .map_err(|e| io_err(log_path, e))?;
    }
    Ok(())
}

/// Scan a directory for `.md` files and reconcile each one.
pub fn reconcile_dir(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    dir: &Path,
    orphan_log_path: Option<&Path>,
) -> Result<Vec<ReconcileReport>, ReconcileError> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut entries: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.path().extension().is_some_and(|x| x == "md")
                && !e.file_name().to_string_lossy().starts_with('.')
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    entries.sort();
    for path in entries {
        out.push(reconcile_md(ws, hlc, &path, orphan_log_path)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;
    use std::fs;
    use tempfile::TempDir;

    fn setup_workspace() -> (TempDir, Workspace, HlcGenerator) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        (dir, ws, hlc)
    }

    #[test]
    fn first_reconcile_creates_sidecar_and_applies_ops() {
        let (dir, mut ws, hlc) = setup_workspace();
        let md_path = dir.path().join("foo.md");
        fs::write(&md_path, "title:: foo\n\n- alpha\n- beta\n").unwrap();

        let report = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
        assert!(report.created_sidecar);
        assert!(report.ops_applied > 0);
        assert_eq!(report.orphans, 0);

        let sidecar_path = sidecar_path_for(&md_path);
        assert!(sidecar_path.exists());
        let sc = sidecar::read(&sidecar_path).unwrap();
        assert_eq!(sc.version, sidecar::SIDECAR_VERSION);
        assert_eq!(sc.blocks.len(), 2);
        // Every block must carry a non-empty `ref_handle` after a fresh
        // reconcile — the v2 invariant.
        assert!(
            sc.blocks.iter().all(|b| !b.ref_handle.is_empty()),
            "v2 sidecar must populate ref_handle on every block: {:?}",
            sc.blocks
        );
    }

    #[test]
    fn idempotent_no_change_means_zero_ops() {
        let (dir, mut ws, hlc) = setup_workspace();
        let md_path = dir.path().join("foo.md");
        fs::write(&md_path, "- a\n- b\n").unwrap();

        let first = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
        assert!(first.ops_applied > 0);

        let second = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
        assert_eq!(second.ops_applied, 0);
    }

    #[test]
    fn orphans_get_logged_when_log_path_set() {
        let (dir, mut ws, hlc) = setup_workspace();
        let md_path = dir.path().join("foo.md");
        let log_path = dir.path().join("orphans.log");
        fs::write(&md_path, "- a\n- b\n").unwrap();
        reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();

        fs::write(&md_path, "- a\n").unwrap();
        let report = reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();
        assert_eq!(report.orphans, 1);

        let log = fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("id="),
            "orphans.log should contain entry:\n{log}"
        );
    }
}
