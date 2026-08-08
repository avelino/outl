//! Checks that need the **materialized tree**, i.e. a booted
//! [`Workspace`].
//!
//! Everything here answers a question the on-disk bytes cannot:
//! "what did the op log actually build?".
//!
//! - [`check_trash`] — how much of the graph is deleted. Today the user
//!   has no way at all to see this: `Delete` is `Move(node, TRASH_ROOT)`
//!   (root `CLAUDE.md` invariant 6), so a deleted block is still in the
//!   tree, just parked under a sentinel that nothing renders.
//! - [`check_unmaterialized_ops`] — node ids the op log mentions that
//!   never landed in the tree.
//! - [`check_projections`] — every page in the tree vs its `.md` on
//!   disk, which is also where the `--repair` plan comes from.

use std::collections::{HashMap, HashSet};

use outl_actions::page::list_all as list_pages;
use outl_actions::{page_md_path, render_page_md};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{file_hash, sidecar_path_for};

use super::repair::PageWrite;
use super::{Builder, Plan};

/// Cap on how many individual items each listing names.
const MAX_LISTED: usize = 20;

/// How many content lines on disk the render would **not** reproduce —
/// what a re-projection of this page removes.
///
/// The question is deliberately routed through
/// `outl_md::content_lines_missing_from`, the single owner of "which of
/// these disk lines does the reference not account for". A second
/// line-comparison here would drift from the one the write-side guard
/// uses, and then the doctor's count would describe a different
/// operation than the one that runs. The only thing that changes is the
/// reference: that function asks it of the *sidecar's* blocks ("does the
/// op log know this line"), and this asks it of the blocks the new
/// projection will contain ("will this line survive the write").
///
/// Those are genuinely different questions and both are correct here:
/// content a peer legitimately deleted is not unlogged, but it is still
/// content this write removes, and the user is entitled to the number
/// before it happens.
fn lines_removed_by(disk: &str, rendered: &str) -> usize {
    let ast = outl_md::parse(rendered);
    let mut blocks = Vec::new();
    for (i, flat) in outl_md::matching::flatten(&ast.blocks).iter().enumerate() {
        // Only `text` is read by the comparison; the id and line are
        // structural fields the multiset never looks at.
        blocks.push(outl_md::SidecarBlock::from_text(
            NodeId::new(),
            i + 1,
            flat.indent,
            flat.text,
        ));
    }
    outl_md::content_lines_missing_from(disk, &blocks).len()
}

/// Report what sits in the trash, with a preview.
///
/// Counts the whole subtree, not just trash's direct children: deleting
/// a parent moves only that node, its descendants ride along implicitly.
pub(super) fn check_trash(b: &mut Builder, ws: &Workspace) {
    let trash = NodeId::trash();
    let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (node, parent, _) in ws.tree().iter_nodes() {
        children.entry(parent).or_default().push(node);
    }

    let tops = children.get(&trash).cloned().unwrap_or_default();
    if tops.is_empty() {
        b.ok("trash is empty — nothing has been deleted in this workspace");
        return;
    }

    // Walk the whole deleted subtree so the count matches what the user
    // would lose if the trash were ever purged.
    let mut total = 0usize;
    let mut stack = tops.clone();
    while let Some(node) = stack.pop() {
        total += 1;
        if let Some(kids) = children.get(&node) {
            stack.extend(kids.iter().copied());
        }
    }

    b.info(format!(
        "trash holds {total} block(s) across {} top-level deletion(s) — \
         deletes are `Move(node, TRASH_ROOT)`, so nothing was physically removed",
        tops.len()
    ));

    let mut listed: Vec<(NodeId, String)> = tops
        .iter()
        .map(|id| (*id, preview(ws, *id)))
        .collect::<Vec<_>>();
    listed.sort_by_key(|a| a.0);
    for (id, text) in listed.iter().take(MAX_LISTED) {
        b.info(format!("  trashed {id}: {text}"));
    }
    if listed.len() > MAX_LISTED {
        b.info(format!(
            "  … and {} more top-level deletion(s)",
            listed.len() - MAX_LISTED
        ));
    }
}

/// One-line preview of a block's text, safe to print.
fn preview(ws: &Workspace, node: NodeId) -> String {
    let text = ws.block_text(node).unwrap_or_default();
    let single = text.replace(['\n', '\r'], " ");
    let trimmed = single.trim();
    if trimmed.is_empty() {
        return "(empty block)".to_string();
    }
    let mut out: String = trimmed.chars().take(80).collect();
    if trimmed.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// Node ids the op log touches that are absent from the materialized
/// tree.
///
/// A node reaches the tree through `Create` or `Move`; an `Edit` /
/// `SetProp` / `SetCollapsed` on a node that never got either is an op
/// whose effect the user will never see. On a healthy workspace this is
/// zero.
pub(super) fn check_unmaterialized_ops(
    b: &mut Builder,
    ws: &Workspace,
    op_nodes: &HashSet<NodeId>,
) {
    if op_nodes.is_empty() {
        return;
    }
    let mut missing: Vec<NodeId> = op_nodes
        .iter()
        .copied()
        .filter(|id| !ws.tree().contains(*id))
        .collect();
    if missing.is_empty() {
        b.ok(format!(
            "every node the op log touches is in the materialized tree ({} node(s))",
            op_nodes.len()
        ));
        return;
    }
    missing.sort();
    b.warn(format!(
        "{} node id(s) appear in the op log but not in the materialized tree — \
         their ops (Edit / SetProp / SetCollapsed) never took effect",
        missing.len()
    ));
    for id in missing.iter().take(MAX_LISTED) {
        b.warn(format!("  unmaterialized node {id}"));
    }
    if missing.len() > MAX_LISTED {
        b.warn(format!("  … and {} more", missing.len() - MAX_LISTED));
    }
}

/// Compare every page in the tree against its `.md` projection, and
/// record what `--repair` may safely act on.
///
/// Three repairable shapes, and one deliberately-not-repairable one:
///
/// - `.md` absent → the page exists in the op log but was never
///   projected here. Safe to write: nothing on disk to lose.
/// - `.md` present, hash matches its sidecar (a *faithful* projection),
///   but the tree now renders differently → the projection is stale.
///   Safe to rewrite: the on-disk bytes carry no unreconciled edit.
/// - `.md` present, no sidecar, and its bytes equal what the tree
///   renders → only the sidecar is missing. Safe: the `.md` is
///   rewritten byte-identical and the sidecar rebuilt from the tree.
/// - `.md` present, no sidecar, bytes differ from the tree → the file
///   may hold content the log never saw. **Never** repaired here;
///   `outl reconcile` owns the `.md → tree` direction.
///
/// The final call at repair time belongs to
/// `outl_actions::apply_page_md_with_sidecar_if_stale`, which re-runs
/// the faithful/stale test itself. What lives here is detection only —
/// `outl-actions` exposes no dry-run of that decision.
///
/// `log_damaged` comes from the caller's [`super::oplog::OpLogHealth`]: a
/// torn op log replays a truncated tree, which makes every page look like
/// it carries unlogged content. That verdict belongs to the log, not the
/// pages, so the unlogged-content check stands down and the caller's gate
/// reports the recoverable cause instead.
pub(super) fn check_projections(
    b: &mut Builder,
    ws: &Workspace,
    root: &std::path::Path,
    log_damaged: bool,
) -> Plan {
    let mut plan = Plan::default();
    let mut absent = 0usize;
    let mut stale = 0usize;
    let mut sidecar_only = 0usize;
    let mut pending_edit = 0usize;
    let mut ahead = 0usize;
    let mut ahead_lines = 0usize;
    let mut removed_lines = 0usize;

    for meta in list_pages(ws) {
        let Ok(page_root) = meta.id.parse::<ulid::Ulid>().map(NodeId) else {
            continue;
        };
        let path = page_md_path(root, &meta);
        let disk = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                absent += 1;
                b.warn(format!(
                    "{}: page `{}` is in the op log but has no `.md` on disk",
                    path.display(),
                    meta.slug
                ));
                // Nothing on disk, so nothing to remove. Counting it as
                // a zero keeps the common bulk case — a device that just
                // paired and has the whole graph unprojected — from
                // tripping a guard aimed at deletion.
                plan.reproject.push(PageWrite::additive(page_root, path));
                continue;
            }
            Err(e) => {
                b.warn(format!("{}: unreadable: {e}", path.display()));
                continue;
            }
        };

        let disk_hash = file_hash(&disk);
        let sidecar_path = sidecar_path_for(&path);
        // Read the sidecar once: the faithful test needs its hash and the
        // unlogged-content check below needs its blocks.
        let sidecar = outl_md::sidecar::read(&sidecar_path).ok();
        let faithful = sidecar
            .as_ref()
            .map(|sc| sc.last_synced_hash == disk_hash)
            .unwrap_or(false);
        // Kept, not just hashed: the stale branch below has to measure
        // what re-projecting it would remove, and rendering the page a
        // second time to answer that would double the cost of the check
        // on every drifted page.
        let rendered = render_page_md(ws, page_root);
        let rendered_matches = file_hash(&rendered) == disk_hash;

        if !sidecar_path.exists() {
            if rendered_matches {
                sidecar_only += 1;
                // Byte-identical by precondition — the sidecar is what
                // is missing, not the content.
                plan.rebuild_sidecar
                    .push(PageWrite::additive(page_root, path));
            } else {
                b.warn(format!(
                    "{}: no sidecar AND content differs from the op log — \
                     `--repair` will not touch it, run `outl reconcile` so the `.md` \
                     is matched back into the log first",
                    path.display()
                ));
            }
            continue;
        }
        if !faithful {
            // **Except when the hash is empty**, which is not a stale
            // projection but a withheld one: `reconcile_md` writes that
            // sentinel when it read content it could not log
            // (invariant 8). Counting it as a pending external edit is
            // how the page the producer flagged becomes the one page
            // this report never names — the guard erasing its own
            // signal.
            let withheld = sidecar
                .as_ref()
                .map(|sc| sc.last_synced_hash.is_empty())
                .unwrap_or(false);
            if !withheld || log_damaged {
                // An external edit is pending. `outl reconcile` owns it;
                // saying it twice as a warning would drown the real signal.
                pending_edit += 1;
                continue;
            }
        }
        if !rendered_matches {
            // `faithful` proves the sidecar agrees with these bytes, not
            // that the bytes came from the log. Ask the same question
            // `apply_page_md_with_sidecar_if_stale` asks before writing,
            // so this listing never offers a repair that pass refuses.
            //
            // Skipped while the log is damaged: a torn `ops/` replays a
            // truncated tree, so *every* page looks like it holds
            // unlogged content and the real cause — the log — would never
            // be named. The caller's `OpLogHealth` gate suppresses these
            // repairs with the message that says how to recover, and it
            // can only count what it sees in the plan.
            let unlogged = match (log_damaged, &sidecar) {
                // `sidecar_can_answer` is asked explicitly now: the
                // stand-down used to live inside
                // `content_lines_missing_from`, where it also silenced
                // `lines_removed_by` above — which asks the same
                // function about a *render*, whose empty blocks are an
                // answer, not an absence of one.
                (false, Some(sc)) if outl_actions::sidecar_can_answer(&sc.blocks) => {
                    outl_actions::content_lines_missing_from(&disk, &sc.blocks)
                }
                _ => Vec::new(),
            };
            if let Some(sample) = unlogged.first() {
                ahead += 1;
                ahead_lines += unlogged.len();
                b.warn(format!(
                    "{}: `.md` holds {} line(s) that exist in no op (e.g. {sample:?}) — \
                     `--repair` will not touch it, run `outl reconcile --ahead-of-log` so they enter \
                     the op log first",
                    path.display(),
                    unlogged.len(),
                ));
                continue;
            }
            stale += 1;
            // Measured here, before the plan is even offered, because
            // `--repair` printing `708 fixed` after the fact is exactly
            // how 1,426 lines went unnoticed (RFC 0210).
            let removed = lines_removed_by(&disk, &rendered);
            removed_lines += removed;
            b.warn(format!(
                "{}: `.md` is a stale projection — the op log renders different content \
                 (re-projecting removes {removed} content line(s) from disk)",
                path.display()
            ));
            plan.reproject.push(PageWrite {
                page_root,
                path,
                lines_removed: removed,
            });
        }
    }

    if ahead > 0 {
        b.warn(format!(
            "{ahead} page(s) hold {ahead_lines} line(s) of content that reached the `.md` but \
             never the op log — they do not sync to other devices, and `--repair` leaves them \
             alone. `outl reconcile --ahead-of-log` is what brings them into the log"
        ));
    }

    if removed_lines > 0 {
        // The headline the old output never had. `--repair` used to
        // print a page count and nothing about content, so a pass that
        // removed 1,426 lines read as `708 fixed`. Stated in both modes:
        // a read-only run is where the user decides whether to authorise
        // the write at all.
        b.warn(format!(
            "re-projecting the stale page(s) above removes {removed_lines} content line(s) \
             across {} page(s) — read the list before running `--repair`",
            plan.reproject
                .iter()
                .filter(|p| p.lines_removed > 0)
                .count()
        ));
    }

    if pending_edit > 0 {
        b.info(format!(
            "{pending_edit} page(s) carry an unreconciled external edit — run `outl reconcile`"
        ));
    }
    if sidecar_only > 0 {
        b.warn(format!(
            "{sidecar_only} page(s) have a correct `.md` but no sidecar — `--repair` rebuilds them"
        ));
    }
    if absent + stale + sidecar_only == 0 {
        b.ok("every page in the op log has a matching `.md` projection on disk");
    }
    plan
}
