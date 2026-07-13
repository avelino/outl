//! Split-brain repair: merge page/journal roots that share a slug.
//!
//! Lives beside [`crate::page`] rather than inside it because the merge
//! is a self-contained repair pass (group → pick survivor → re-parent →
//! trash), and `page.rs` already owns the full page-model surface. The
//! one public entry point ([`merge_duplicate_slug_roots`]) is re-exported
//! through `page` and the crate root so callers still reach it at
//! `outl_actions::merge_duplicate_slug_roots`.

use std::collections::HashMap;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::page::{page_id_from_slug, set_property, KIND_KEY, SLUG_KEY};
use crate::tree::{children_of, walk_subtree};

/// Detect and repair page/journal roots that share a slug (the split-brain
/// bug where two creators minted different ids for the same slug). For each
/// slug with >1 root: pick the CANONICAL root (id == [`page_id_from_slug`]
/// if present among them, else the root with the most descendants, tie-break
/// smallest [`NodeId`]), MOVE every child of the other roots to the end of the
/// canonical root's children (preserving each duplicate's child order — NO
/// data loss), copy the slug/kind props onto the canonical if missing, then
/// move each emptied duplicate root to [`NodeId::trash`]. Returns the number of
/// duplicate roots merged (0 when the workspace is already clean).
///
/// Every mutation is an `Op` through [`Workspace::apply`], so running this on
/// ANY client converges on every device via the CRDT. Idempotent.
pub fn merge_duplicate_slug_roots(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<usize, ActionError> {
    // Group root-level pages by their slug text. Skip roots with no
    // `page-slug` — they're not pages, so they don't participate.
    let mut groups: HashMap<String, Vec<NodeId>> = HashMap::new();
    for (id, _) in children_of(workspace, NodeId::root()) {
        if let Some(PropValue::Text(slug)) = workspace.tree().property(id, SLUG_KEY) {
            groups.entry(slug.clone()).or_default().push(id);
        }
    }

    // Deterministic iteration order so the op log is reproducible across
    // runs (HashMap iteration order is not stable).
    let mut slugs: Vec<String> = groups
        .iter()
        .filter(|(_, roots)| roots.len() > 1)
        .map(|(slug, _)| slug.clone())
        .collect();
    slugs.sort();

    let mut merged = 0usize;
    for slug in slugs {
        let roots = &groups[&slug];
        let canonical = pick_canonical_root(workspace, &slug, roots);
        for &dup in roots {
            if dup == canonical {
                continue;
            }
            // Re-parent every child of the duplicate under the canonical
            // root, in position order, appending as last children so the
            // duplicate's internal order is preserved. Read the full child
            // list up front — `move_under` mutates the tree as it goes.
            let children: Vec<NodeId> = children_of(workspace, dup)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            for child in children {
                crate::block::move_under(workspace, hlc, child, canonical)?;
            }
            // The canonical root may be missing the slug/kind props (e.g.
            // it was minted by a peer that never set them). Copy from the
            // duplicate so the survivor is a well-formed page.
            copy_prop_if_missing(workspace, hlc, dup, canonical, SLUG_KEY)?;
            copy_prop_if_missing(workspace, hlc, dup, canonical, KIND_KEY)?;
            // Trash the now-empty duplicate root (a bare move — its
            // children already left).
            crate::block::delete(workspace, hlc, dup)?;
            merged += 1;
        }
    }
    Ok(merged)
}

/// Pick the surviving root for a slug shared by `roots` (len > 1):
/// the one whose id equals [`page_id_from_slug`] if present, otherwise
/// the root with the most descendants, tie-broken by smallest [`NodeId`].
fn pick_canonical_root(workspace: &Workspace, slug: &str, roots: &[NodeId]) -> NodeId {
    let deterministic = page_id_from_slug(slug);
    if let Some(&hit) = roots.iter().find(|&&id| id == deterministic) {
        return hit;
    }
    roots
        .iter()
        .copied()
        .max_by(|&a, &b| {
            descendant_count(workspace, a)
                .cmp(&descendant_count(workspace, b))
                // More descendants wins; on a tie the SMALLER id wins, so
                // invert the id comparison (max_by keeps the larger key).
                .then_with(|| b.cmp(&a))
        })
        // `roots` is non-empty (grouped from at least two ids), but fall
        // back to the deterministic id rather than panic.
        .unwrap_or(deterministic)
}

/// Number of descendants under `node` (its whole subtree, excluding
/// itself). Used to pick the richest duplicate as the merge survivor.
fn descendant_count(workspace: &Workspace, node: NodeId) -> usize {
    let mut count = 0usize;
    walk_subtree(workspace, node, |_| {
        count += 1;
        true
    });
    count
}

/// Copy property `key` from `from` onto `to` when `to` doesn't already
/// have it. No-op when `to` has the property or `from` lacks it.
fn copy_prop_if_missing(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    from: NodeId,
    to: NodeId,
    key: &str,
) -> Result<(), ActionError> {
    if workspace.tree().property(to, key).is_some() {
        return Ok(());
    }
    if let Some(value) = workspace.tree().property(from, key).cloned() {
        set_property(workspace, hlc, to, key, Some(value))?;
    }
    Ok(())
}
