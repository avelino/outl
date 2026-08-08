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
use outl_md::matching::guard::OrphanGuard;
use std::fs;
use std::path::Path;

/// Run the `reconcile` subcommand.
///
/// `allow_bulk_delete` is the one place in the binary that reaches
/// [`OrphanGuard::Disabled`]. The guard refuses a pass that would trash
/// most of a page, and the refusal is only defensible while there is a
/// way to say the deletion was meant — a guard with no escape hatch is a
/// wall (root `CLAUDE.md` invariant 9).
pub fn run(path: &Path, ahead_of_log: bool, allow_bulk_delete: bool) -> Result<()> {
    let guard = if allow_bulk_delete {
        OrphanGuard::Disabled
    } else {
        OrphanGuard::Enforced
    };
    if ahead_of_log {
        return run_ahead_of_log(path, guard);
    }
    if allow_bulk_delete {
        return run_allow_bulk_delete(path);
    }
    list_orphans(path)
}

/// Reconcile the pages an ordinary pass refuses because the deletion is
/// too large, with the volume guard turned off.
///
/// **This mode has to exist separately from `--ahead-of-log`, and the
/// reason is that the two select opposite sets.**
/// `--ahead-of-log` visits pages where `content_lines_missing_from > 0`
/// — pages whose `.md` holds *more* than the log. A page the guard
/// refused is the mirror: its `.md` holds *less*, every line on disk is
/// accounted for, and `missing` is zero, so it never appeared in that
/// list. Wiring the flag only into `--ahead-of-log` therefore made it
/// unreachable for every page it existed to unblock, which is the
/// "guard with no escape hatch is a wall" failure root `CLAUDE.md`
/// invariant 9 names — arrived at through the door marked "fixed".
///
/// The selection here is deliberately the plain one: any page whose
/// `.md` no longer matches its sidecar hash, i.e. exactly what an
/// ordinary reconcile would look at. The guard is what changes, not the
/// scope.
fn run_allow_bulk_delete(path: &Path) -> Result<()> {
    let mut ctx = ws::open(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let root = ctx.root.clone();
    let orphan_log = outl_actions::sync::orphans_log_path(&root);

    let stale = collect_stale(&ctx.workspace, &root);
    if stale.is_empty() {
        println!("no page has an unreconciled external edit");
        return Ok(());
    }

    println!(
        "{} page(s) have an unreconciled `.md`; reconciling with the \
         orphan-volume guard OFF (deletions apply at any size):",
        stale.len()
    );
    println!();

    let mut ops_total = 0usize;
    let mut failed = 0usize;
    for (md_path, slug) in &stale {
        match outl_md::reconcile_md_with_guard(
            &mut ctx.workspace,
            &ctx.hlc,
            md_path,
            Some(orphan_log.as_path()),
            &OrphanGuard::Disabled,
        ) {
            Ok(report) => {
                ops_total += report.ops_applied;
                if report.orphans > 0 {
                    println!(
                        "  {:>4} op(s)  {slug} — {} block(s) moved to the trash",
                        report.ops_applied, report.orphans
                    );
                } else {
                    println!("  {:>4} op(s)  {slug}", report.ops_applied);
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("  FAILED    {slug} — {e}");
            }
        }
    }

    println!();
    println!(
        "reconciled {} page(s), {ops_total} op(s) applied, {failed} failed",
        stale.len() - failed
    );
    println!("Deleted blocks are in the trash, not gone — `outl doctor` lists them.");
    if failed > 0 {
        anyhow::bail!("{failed} page(s) failed to reconcile");
    }
    Ok(())
}

/// Pages whose `.md` no longer matches the hash its sidecar recorded —
/// an external edit the `.md → tree` direction has not taken in yet.
fn collect_stale(
    ws: &outl_core::workspace::Workspace,
    root: &Path,
) -> Vec<(std::path::PathBuf, String)> {
    let mut out = scan_pages(ws, root, |meta, md_path, disk, sidecar| {
        (sidecar.last_synced_hash != outl_md::sidecar::file_hash(disk))
            .then(|| (md_path.to_path_buf(), meta.slug.clone()))
    });
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
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
fn run_ahead_of_log(path: &Path, guard: OrphanGuard) -> Result<()> {
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
        match outl_md::reconcile_md_with_guard(
            &mut ctx.workspace,
            &ctx.hlc,
            &page.md_path,
            Some(orphan_log.as_path()),
            &guard,
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
        // Exit non-zero. A partial migration that reports success is how a
        // script marks the recovery done and moves on, leaving those pages
        // outside the log with nobody watching.
        anyhow::bail!("{failed} page(s) still hold content outside the op log");
    }
    println!("Run `outl doctor` to confirm nothing is left outside the log.");
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
    let mut out = scan_pages(ws, root, |meta, md_path, disk, sidecar| {
        // Same reference the write-side guard uses: the sidecar's blocks,
        // not a render. A render answers "do disk and tree disagree",
        // which every remote edit also answers yes to, and reconciling
        // those would write the pre-edit text back as ops — reverting the
        // peer, permanently, since the log is append-only.
        // A sidecar that cannot answer does not get a vote — asked
        // here rather than inside the verdict, so the render-based
        // caller in `doctor` is not silenced by the same rule.
        if !outl_actions::sidecar_can_answer(&sidecar.blocks) {
            return None;
        }
        let missing = outl_actions::content_lines_missing_from(disk, &sidecar.blocks).len();
        (missing > 0).then(|| AheadPage {
            md_path: md_path.to_path_buf(),
            slug: meta.slug.clone(),
            missing,
        })
    });
    // Stable order so two runs report the same list.
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Walk every page, handing `pick` the ones whose `.md` **and** sidecar
/// both read cleanly.
///
/// The skip rule is the reason this is one function rather than two
/// copies: **a `.md` we cannot read is not a page in any interesting
/// state — it is a read failure**, and guessing "empty" there is exactly
/// how content gets deleted (RFC 0210). `doctor` reports those
/// separately. A sidecar that will not parse is skipped for the same
/// reason: without it there is no record of what the log held, so no
/// question here can be answered about the page.
fn scan_pages<T>(
    ws: &outl_core::workspace::Workspace,
    root: &Path,
    pick: impl Fn(&outl_actions::PageMeta, &Path, &str, &outl_md::sidecar::Sidecar) -> Option<T>,
) -> Vec<T> {
    let mut out = Vec::new();
    for meta in outl_actions::list_pages(ws) {
        let md_path = outl_actions::page_md_path(root, &meta);
        let Ok(disk) = fs::read_to_string(&md_path) else {
            continue;
        };
        let Ok(sidecar) = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path))
        else {
            continue;
        };
        if let Some(item) = pick(&meta, &md_path, &disk, &sidecar) {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use outl_md::matching::guard::OrphanGuard;

    /// End-to-end proof that the escape hatch reaches the state the
    /// guard creates.
    ///
    /// The first version of this wiring passed a clap test and was still
    /// broken: `--allow-bulk-delete` only reached `--ahead-of-log`, which
    /// selects pages where `content_lines_missing_from > 0`. A page the
    /// guard refused is the opposite — its `.md` holds *less* than the
    /// log, so `missing` is zero and it never appeared. The flag was
    /// unreachable for every page it existed for.
    ///
    /// So this test does not assert on argument parsing. It builds a page
    /// the guard genuinely refuses, confirms the refusal, and then
    /// confirms the flag's guard value applies it.
    #[test]
    fn the_escape_hatch_applies_a_deletion_the_guard_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let md_path = dir.path().join("big.md");

        let full: String = (0..60).map(|i| format!("- block {i}\n")).collect();
        std::fs::write(&md_path, &full).expect("write");

        let actor = outl_core::id::ActorId::new();
        let mut ws = outl_core::workspace::Workspace::open_in_memory(actor).expect("ws");
        let hlc = outl_core::hlc::HlcGenerator::new(actor);
        outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("seed");

        // The user selects everything and deletes: 59 of 60 blocks go.
        std::fs::write(&md_path, "- block 0\n").expect("truncate");

        let refused = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None);
        assert!(
            matches!(refused, Err(outl_md::ReconcileError::BulkDelete(_))),
            "the guard must refuse 59 of 60 blocks, got {refused:?}"
        );

        // Same page, same bytes, guard off — which is what
        // `outl reconcile --allow-bulk-delete` passes.
        let applied =
            outl_md::reconcile_md_with_guard(&mut ws, &hlc, &md_path, None, &OrphanGuard::Disabled)
                .expect("the explicit opt-in must apply the deletion");
        assert!(
            applied.orphans >= 59,
            "the opt-in must apply the whole deletion, not part of it — got {} orphans",
            applied.orphans
        );
    }
}
