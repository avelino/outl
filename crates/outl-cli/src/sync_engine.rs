//! Thin orchestration over `outl_actions::ingest`.
//!
//! The CLI file watcher (and `serve`'s initial scan) ingest `.md`
//! files **as pages** — creating the page node under root with its
//! `page-slug` / `page-kind`, not just its blocks. The bare
//! `outl_md::reconcile` primitive only creates blocks, which leaves
//! freshly-imported or peer-dropped `.md` invisible to `page list`.
//! See `outl_actions::ingest` for the full rationale.

use crate::workspace_layout::Paths;
use anyhow::Result;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
pub use outl_md::reconcile::ReconcileReport;
use std::path::Path;
use tracing::warn;

/// Ingest a single `.md` file as a page, logging orphans to
/// `<.outl>/orphans.log`.
pub fn reconcile_md(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    paths: &Paths,
    md_path: &Path,
) -> Result<ReconcileReport> {
    Ok(outl_actions::ingest_md_file(
        ws,
        hlc,
        md_path,
        Some(&paths.orphans),
    )?)
}

/// Ingest every `.md` in a directory as pages. Per-file failures are
/// logged and skipped so one malformed file doesn't abort the scan.
pub fn reconcile_dir(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    paths: &Paths,
    dir: &Path,
) -> Result<Vec<ReconcileReport>> {
    let mut reports = Vec::new();
    for (path, res) in outl_actions::ingest_dir(ws, hlc, dir, Some(&paths.orphans)) {
        match res {
            Ok(r) => reports.push(r),
            Err(e) => warn!("ingest failed for {}: {e}", path.display()),
        }
    }
    Ok(reports)
}
