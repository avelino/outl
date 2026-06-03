//! Reconcile a single `.md` file against a `Workspace`.
//!
//! Used by `outl serve` (file watcher), `outl init` (initial seed), and
//! `outl-tui` (after editor commits). Reads the `.md`, optionally loads
//! the sidecar, runs 3-level matching, emits the minimal op sequence,
//! applies it, and writes back the refreshed sidecar.
//!
//! Orphan ids are logged before being moved to `TRASH_ROOT`, so a
//! deletion is never silent.

use crate::diff::diff_to_ops;
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
///
/// When the file has no sidecar yet, the page node id is minted fresh
/// (`NodeId::new`). Callers that ingest a file *as a page* — so the
/// node id must agree with `page_id_from_slug`, letting a peer that
/// creates the same slug converge on one node — should use
/// [`reconcile_md_with_page_id`] instead.
pub fn reconcile_md(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    reconcile_md_inner(ws, hlc, md_path, None, None, orphan_log_path)
}

/// Like [`reconcile_md`], but pins the page node id used when no
/// sidecar exists yet.
///
/// `outl_actions::ingest_md_file` passes `page_id_from_slug(slug)` so
/// the blocks attach to the page node `open_or_create` just created
/// under root. Without this the two paths mint different ids and the
/// blocks hang off a phantom node that never appears in `page list`
/// (the Logseq-import "blocks but no pages" bug).
pub fn reconcile_md_with_page_id(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    page_id: NodeId,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    reconcile_md_inner(ws, hlc, md_path, Some(page_id), None, orphan_log_path)
}

/// Like [`reconcile_md_with_page_id`], but reuses markdown the caller
/// already read into memory instead of reading the file a second time.
///
/// Bulk-ingest callers (`outl_actions::ingest_md_file`) parse the file
/// once to pull `title::` out of the page properties; passing that same
/// text here avoids a redundant `read_to_string` + parse per file, which
/// is the dominant cost when importing thousands of pages. `md_path` is
/// still needed for the sidecar location and error/hash context.
pub fn reconcile_md_with_page_id_text(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    page_id: NodeId,
    md_text: &str,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    reconcile_md_inner(
        ws,
        hlc,
        md_path,
        Some(page_id),
        Some(md_text),
        orphan_log_path,
    )
}

fn reconcile_md_inner(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    explicit_page_id: Option<NodeId>,
    prefetched_text: Option<&str>,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    let md_text = match prefetched_text {
        Some(t) => t.to_owned(),
        None => fs::read_to_string(md_path).map_err(|e| io_err(md_path, e))?,
    };
    let new_ast = parse(&md_text);
    let md_hash = file_hash(&md_text);

    let sidecar_path = sidecar_path_for(md_path);
    let (page_id, old_blocks, created_sidecar) = match sidecar::read(&sidecar_path) {
        Ok(sc) => (sc.page_id, sc.blocks, false),
        Err(sidecar::SidecarError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            (explicit_page_id.unwrap_or_default(), Vec::new(), true)
        }
        Err(_) => {
            // Corrupt sidecar — rebuild from scratch.
            (explicit_page_id.unwrap_or_default(), Vec::new(), true)
        }
    };

    // Short-circuit: file unchanged since last sync.
    if !created_sidecar {
        if let Ok(existing) = sidecar::read(&sidecar_path) {
            if existing.last_synced_hash == md_hash {
                return Ok(ReconcileReport {
                    md_path: md_path.to_path_buf(),
                    ops_applied: 0,
                    orphans: 0,
                    created_sidecar: false,
                });
            }
        }
    }

    let (matches, orphans) = match_blocks(&new_ast.blocks, &old_blocks);

    if !orphans.is_empty() {
        if let Some(log_path) = orphan_log_path {
            log_orphans(log_path, md_path, &orphans, &old_blocks)?;
        }
    }

    let plan = diff_to_ops(
        &new_ast.blocks,
        &matches,
        &orphans,
        page_id,
        &md_hash,
        &old_blocks,
    );
    let mut ops_applied = 0usize;

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
    };
    sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(ReconcileReport {
        md_path: md_path.to_path_buf(),
        ops_applied,
        orphans: orphans.len(),
        created_sidecar,
    })
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
