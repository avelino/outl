//! `outl recover` — the `op log → tree` direction.
//!
//! ## Why this exists next to `outl reconcile --ahead-of-log`
//!
//! Both recover content that issue #210 took out of circulation, and they
//! read **different sources**, so neither replaces the other:
//!
//! - `reconcile --ahead-of-log` reads the `.md`. It recovers content that
//!   is still on disk but in no op. It cannot help a page whose `.md` was
//!   already overwritten.
//! - `recover` reads the **op log**. The producer bug did not lose the
//!   text into thin air: the reconcile that followed it emitted an
//!   `Op::Edit` carrying the truncated text, and the edit *before* that
//!   one — carrying the full text — is still in the append-only log.
//!
//! So a page whose `.md` was overwritten before the guard existed is
//! unreachable by the first and recoverable by the second.
//!
//! ## Read-only by default
//!
//! A plain run lists what it found and writes nothing. `--apply` is the
//! only writing mode, and what it writes is a **new** `Op::Edit` per
//! block — the log is never rewritten (root `CLAUDE.md` invariant 1).
//!
//! The write is additive by construction: a block only qualifies when its
//! current text is a *prefix* of the revision being restored, so nothing
//! the block shows today is dropped. `outl_actions::recover` owns that
//! rule and re-checks it at write time.

use crate::ws;
use anyhow::Result;
use outl_actions::TruncatedBlock;
use std::path::Path;

/// How many lost non-blank lines a block must carry to be reported.
///
/// One, deliberately. A single-line loss is often ordinary editing, so
/// this default is noisy on purpose: a false positive costs the user a
/// line of output, a false negative costs them the content. `--min-lines`
/// raises the bar when the listing is too long to read.
pub const DEFAULT_MIN_LINES: u16 = 1;

/// Run the `recover` subcommand.
pub fn run(path: &Path, apply: bool, min_lines: u16) -> Result<()> {
    let mut ctx = ws::open(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let found = outl_actions::scan_truncated_blocks(&ctx.workspace, min_lines as usize)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if found.is_empty() {
        println!("no block holds text that an edit truncated");
        return Ok(());
    }

    let total: usize = found.iter().map(|f| f.lost_lines.len()).sum();
    println!(
        "{} block(s) hold {total} line(s) an `Op::Edit` dropped and the op log still has.",
        found.len()
    );
    println!();
    for block in &found {
        print_finding(block);
    }
    println!();

    if !apply {
        println!("Nothing was written. Re-run with `--apply` to restore the text above.");
        println!("Each restore is a new op; the truncating edit stays in the log.");
        if found.iter().any(|f| f.lost_lines.len() == 1) {
            // Named rather than filtered out: a one-line loss is the most
            // likely false positive (a user deleting the tail of what they
            // just typed), and the honest move is to show it and say so
            // rather than to guess on the user's behalf.
            println!(
                "Some findings lost a single line — that is often ordinary editing. \
                 Use `--min-lines` to skip them."
            );
        }
        println!(
            "Run `outl reconcile --ahead-of-log` first if `outl doctor` also reports \
             `.md` content outside the op log: it recovers what is still on disk, \
             which is the wider net."
        );
        return Ok(());
    }

    restore_all(&mut ctx, &found)
}

/// One block, one line of summary plus the text that would come back.
fn print_finding(block: &TruncatedBlock) {
    let page = block.page_slug.as_deref().unwrap_or("(no page)");
    println!(
        "  {page}  block {}  — {} line(s) recoverable",
        block.node,
        block.lost_lines.len()
    );
    println!(
        "      keeps: {}",
        block.current.lines().next().unwrap_or("")
    );
    for line in block.lost_lines.iter().take(SAMPLE_LINES) {
        println!("      +     {line}");
    }
    if block.lost_lines.len() > SAMPLE_LINES {
        println!(
            "      +     … {} more line(s)",
            block.lost_lines.len() - SAMPLE_LINES
        );
    }
}

/// How many recoverable lines to print per block before eliding. Enough
/// to recognise the content, short enough that a 30-line loss doesn't
/// push the next finding off the screen.
const SAMPLE_LINES: usize = 3;

/// Restore every finding, then re-project the pages that changed.
fn restore_all(ctx: &mut ws::WsCtx, found: &[TruncatedBlock]) -> Result<()> {
    let mut restored = 0usize;
    let mut failed = 0usize;
    let mut pages = Vec::new();

    for block in found {
        match outl_actions::restore_truncated_block(&mut ctx.workspace, &ctx.hlc, block) {
            Ok(()) => {
                restored += 1;
                // Re-project by page root, not by block: one page can hold
                // several restored blocks and re-rendering it once per
                // block would rewrite the same file N times.
                if let Some(page) =
                    outl_actions::tree::enclosing_page_id(&ctx.workspace, block.node)
                {
                    if !pages.contains(&page) {
                        pages.push(page);
                    }
                }
            }
            Err(e) => {
                failed += 1;
                // Never swallow: a block that failed still holds truncated
                // text, and the user has to know which one.
                eprintln!("  FAILED    block {} — {e}", block.node);
            }
        }
    }

    println!("restored {restored} block(s), {failed} failed");
    let withheld = reproject(ctx, &pages);
    if withheld > 0 {
        println!();
        println!(
            "{withheld} page(s) kept their old `.md`: it still holds lines the sidecar \
             does not list, so the re-projection guard (RFC 0210) refused to overwrite it."
        );
        println!(
            "That guard reads the sidecar, and the sidecar is what is stale here — the \
             text IS in the op log now. Run `outl reconcile --ahead-of-log` to refresh \
             those sidecars, then `outl serve --once`."
        );
    }

    if failed > 0 {
        // Exit non-zero. A partial recovery that reports success is how a
        // script marks the job done and moves on, leaving the rest
        // truncated with nobody watching.
        anyhow::bail!("{failed} block(s) could not be restored");
    }
    println!("Run `outl doctor` to confirm the workspace is consistent.");
    Ok(())
}

/// Re-render the `.md` of every page a restore touched. Returns how many
/// pages were left as they were.
///
/// Best-effort and deliberately non-fatal: the restored text is already
/// in the op log, which is the source of truth, so a page that does not
/// project has lost nothing — its `.md` catches up on the next
/// `outl serve` or page open. Failing the command here would report
/// "could not restore" for content that *is* restored.
///
/// The common refusal is not an error either. A page whose `.md` still
/// carries the full text (the `.md` was never overwritten, only the log
/// was truncated) trips `PageMarkdownAheadOfLog`, because that guard
/// compares disk against the **sidecar**, and the sidecar is the stale
/// party at this point. Overwriting anyway is precisely what RFC 0210
/// forbids, so the fix belongs to `reconcile --ahead-of-log`, not here.
fn reproject(ctx: &ws::WsCtx, pages: &[outl_core::NodeId]) -> usize {
    let mut withheld = 0usize;
    for page in pages {
        if let Err(e) =
            outl_actions::apply_page_md_with_sidecar_if_stale(&ctx.workspace, &ctx.root, *page)
        {
            withheld += 1;
            eprintln!("  warning: block restored but `.md` not re-rendered for {page}: {e}");
        }
    }
    withheld
}
