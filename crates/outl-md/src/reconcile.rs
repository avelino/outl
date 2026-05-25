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
use crate::parse::parse;
use crate::sidecar::{self, file_hash, sidecar_path_for, Sidecar, SidecarBlock, SIDECAR_VERSION};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::LogOp;
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
    let (page_id, old_blocks, created_sidecar) = match sidecar::read(&sidecar_path) {
        Ok(sc) => (sc.page_id, sc.blocks, false),
        Err(sidecar::SidecarError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            (NodeId::new(), Vec::new(), true)
        }
        Err(_) => {
            // Corrupt sidecar — rebuild from scratch.
            (NodeId::new(), Vec::new(), true)
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

    let plan = diff_to_ops(&new_ast.blocks, &matches, &orphans, page_id, &md_hash);
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
        assert_eq!(sc.version, 1);
        assert_eq!(sc.blocks.len(), 2);
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
