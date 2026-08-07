//! `outl reconcile` — the `.md → tree` direction.
//!
//! Two modes, both explicit:
//!
//! - **No flags:** read-only listing of `.outl/orphans.log`.
//! - **`--ahead-of-log`:** reconcile the pages whose `.md` holds content
//!   that exists in no op, bypassing the sidecar hash gate.
//!
//! ## Why the second mode has to exist
//!
//! A page can end up hash-faithful (its sidecar agrees with the bytes on
//! disk) while carrying content the op log never saw — see
//! [RFC 0210](../../../../docs/rfcs/0210-md-content-outside-op-log.md) and
//! issue #210. Fixing the parser that produced that state is necessary
//! but not sufficient: `needs_reconcile` compares the sidecar hash
//! against the file, the sidecar already carries the hash of the file
//! *with* the content, so the page reads as in-sync and the ordinary
//! reconcile never looks at it. Measured on a 2,560-page workspace:
//! `serve --once` applied **0 ops** to 233 such pages.
//!
//! So the content needs a path that ignores the hash and reconciles
//! anyway. That path must stay opt-in, because it emits ops for content
//! the log has never seen — a deliberate write, not a repair.
//!
//! ## Ordering that matters
//!
//! Run this only on a build whose parser preserves the content. A
//! reconcile against a parser that still discards prose after a block
//! property emits the **truncated** text as `Op::Edit`, making the loss
//! permanent in the op log — the one place it currently is not.

use crate::workspace_layout::Paths;
use crate::ws;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Run the `reconcile` subcommand.
pub fn run(path: &Path, ahead_of_log: bool) -> Result<()> {
    if ahead_of_log {
        return run_ahead_of_log(path);
    }
    list_orphans(path)
}

/// The original read-only listing.
fn list_orphans(path: &Path) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());
    if !paths.orphans.exists() {
        println!("no orphans recorded");
        return Ok(());
    }
    let text = fs::read_to_string(&paths.orphans)
        .with_context(|| format!("reading {}", paths.orphans.display()))?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        println!("no orphans recorded");
        return Ok(());
    }
    println!("{} orphan(s) pending manual resolution:", lines.len());
    for line in &lines {
        println!("  {line}");
    }
    println!();
    println!("Interactive resolution in the TUI is not yet available.");
    Ok(())
}

/// A page whose `.md` carries content the op log does not have.
///
/// No `page_root` here on purpose: `reconcile_md` re-derives the page
/// identity from the `.md` path and its sidecar, so carrying an id we
/// resolved a moment earlier would just be a second opinion about which
/// page this is.
struct AheadPage {
    md_path: std::path::PathBuf,
    slug: String,
    /// Content lines present on disk and absent from the render.
    missing: usize,
}

/// Find every page whose `.md` runs ahead of the log, then reconcile it.
///
/// The detection is the same `outl_actions::content_lines_missing_from`
/// the doctor and the re-projection guard use — one owner for the verdict,
/// so this command can never disagree with what `doctor` reported.
fn run_ahead_of_log(path: &Path) -> Result<()> {
    let mut ctx = ws::open(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let root = ctx.root.clone();
    let orphan_log = outl_actions::sync::orphans_log_path(&root);

    let ahead = collect_ahead(&ctx.workspace, &root);
    if ahead.is_empty() {
        println!("no page holds content outside the op log");
        return Ok(());
    }

    let total_lines: usize = ahead.iter().map(|p| p.missing).sum();
    println!(
        "{} page(s) hold {total_lines} line(s) of content that exist in no op.",
        ahead.len()
    );
    println!("Reconciling each one (this writes ops for that content):");
    println!();

    let mut ops_total = 0usize;
    let mut failed = 0usize;
    for page in &ahead {
        // `reconcile_md` short-circuits on the recorded hash, which is
        // exactly the state these pages are in — that is why the ordinary
        // reconcile skips them.
        if let Err(e) = invalidate_synced_hash(&page.md_path) {
            failed += 1;
            eprintln!(
                "  FAILED    {} — could not clear sidecar hash: {e}",
                page.slug
            );
            continue;
        }
        match outl_md::reconcile_md(
            &mut ctx.workspace,
            &ctx.hlc,
            &page.md_path,
            Some(orphan_log.as_path()),
        ) {
            Ok(report) => {
                ops_total += report.ops_applied;
                println!(
                    "  {:>4} op(s)  {} ({} line(s) were outside the log)",
                    report.ops_applied, page.slug, page.missing
                );
            }
            Err(e) => {
                failed += 1;
                // Never swallow: a page that could not be reconciled still
                // holds unlogged content, and the user has to know which.
                eprintln!("  FAILED    {} — {e}", page.slug);
            }
        }
    }

    println!();
    println!(
        "reconciled {} page(s), {ops_total} op(s) applied, {failed} failed",
        ahead.len() - failed
    );
    if failed > 0 {
        println!("Re-run to retry the failures; their `.md` is untouched.");
    } else {
        println!("Run `outl doctor` to confirm nothing is left outside the log.");
    }
    Ok(())
}

/// Clear a sidecar's `last_synced_hash` so `reconcile_md` stops
/// short-circuiting on it.
///
/// Safe to write: that field is a "when did I last sync this" marker, not
/// content. It rewrites only that one field, through the same
/// `outl_md::sidecar::{read,write}` pair every other writer uses, so the
/// block entries — the ids and ref handles that actually matter — are
/// preserved byte-for-byte. A crash between this call and the reconcile
/// leaves the page looking dirty, so the next boot reconciles it: the
/// same outcome, later.
///
/// A missing sidecar needs no action — without one there is no hash to
/// short-circuit on.
fn invalidate_synced_hash(md_path: &Path) -> Result<()> {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = match outl_md::sidecar::read(&sidecar_path) {
        Ok(sc) => sc,
        Err(_) => return Ok(()),
    };
    sc.last_synced_hash = String::new();
    outl_md::sidecar::write(&sidecar_path, &sc)
        .with_context(|| format!("rewriting {}", sidecar_path.display()))?;
    Ok(())
}

/// Walk every page, rendering it from the tree and comparing against the
/// `.md` on disk. A page is "ahead" when disk holds content lines the
/// render does not account for.
fn collect_ahead(ws: &outl_core::workspace::Workspace, root: &Path) -> Vec<AheadPage> {
    let mut out = Vec::new();
    for meta in outl_actions::list_pages(ws) {
        let md_path = outl_actions::page_md_path(root, &meta);
        // A `.md` we cannot read is not a page running ahead — it is a
        // read failure, and guessing "empty" here is how content gets
        // deleted (RFC 0210). Skip it; `doctor` reports it separately.
        let Ok(disk) = fs::read_to_string(&md_path) else {
            continue;
        };
        // Same reference the write-side guard uses: the sidecar's blocks,
        // not a render. A render answers "do disk and tree disagree",
        // which every remote edit also answers yes to, and reconciling
        // those would write the pre-edit text back as ops — reverting the
        // peer, permanently, since the log is append-only.
        let Some(sidecar) =
            outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)).ok()
        else {
            continue;
        };
        let missing = outl_actions::content_lines_missing_from(&disk, &sidecar.blocks).len();
        if missing > 0 {
            out.push(AheadPage {
                md_path,
                slug: meta.slug.clone(),
                missing,
            });
        }
    }
    // Stable order so two runs report the same list.
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}
