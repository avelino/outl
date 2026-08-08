//! "Does the op log know this line?" — the one owner of that verdict.
//!
//! Three callers need the same answer and must never disagree:
//!
//! - `reconcile_md` asks it **before advancing `last_synced_hash`**, so
//!   the sidecar never claims the log holds content this pass could not
//!   emit an op for (invariant 8);
//! - `outl_actions::apply_page_md_with_sidecar_if_stale` asks it before
//!   re-projecting the tree over a `.md`, so a page holding unlogged
//!   content is refused instead of overwritten;
//! - `outl doctor` asks it in its read-only listing, so the listing can
//!   never promise a repair the writing pass then refuses.
//!
//! It lives in `outl-md` rather than `outl-actions` because it reads
//! nothing but a `.md` string and `SidecarBlock`s — both owned here —
//! and because the producer (`reconcile_md`) is in this crate and
//! cannot depend upwards. `outl-actions` re-exports it so every
//! existing `outl_actions::content_lines_missing_from` path still
//! resolves.

use std::collections::HashMap;

use crate::sidecar::SidecarBlock;

/// Whether `blocks` can answer "does the op log know this line" at all.
///
/// `false` for a sidecar written before `SidecarBlock::text` existed
/// (0.11): every entry carries `text: ""`, so it describes which blocks
/// the page had and nothing about what they said.
///
/// This is the **same condition** [`content_lines_missing_from`] stands
/// down on, exported so callers can distinguish its two empty results,
/// which mean opposite things:
///
/// - *"I checked, nothing is at risk"* → safe to write;
/// - *"I could not check"* → **not** permission to write.
///
/// Reading the second as the first is how a page holding unlogged
/// content gets overwritten by a caller that did ask the question. The
/// two live together so they cannot drift: change the stand-down rule
/// below and this predicate has to move with it, in the same diff.
///
/// An **empty** block list is the opposite case and answers `true` — a
/// page with no blocks has nothing to lose, and treating it as
/// unanswerable would freeze every freshly created page.
pub fn sidecar_can_answer(blocks: &[SidecarBlock]) -> bool {
    blocks.is_empty() || blocks.iter().any(|b| !b.text.is_empty())
}

/// The content lines in `disk` that **no block the op log knows** can
/// account for.
///
/// `sidecar_blocks` is the reference, not a fresh render of the tree, and
/// the distinction is the whole point. The sidecar's blocks are what the
/// log held when the two last agreed, so comparing against them answers
/// *"does the op log know this line"*. Comparing against a render answers
/// *"do disk and tree disagree"*, which is also yes for every remote
/// edit, every remote delete and every reorder — a guard built on that
/// question refuses to re-project any page a peer has touched, which is
/// issue #166 with the blame moved.
///
/// Compared as a multiset, so a line duplicated on disk but present once
/// in the log still counts as at-risk.
///
/// Lines are normalised before comparison: the bullet marker, the indent
/// and trailing whitespace all come from the renderer's layout, not from
/// the content, so a pure indent or outdent must not read as new text.
/// Blank lines are dropped. Everything else is compared verbatim — this
/// decides whether bytes get deleted, so it errs toward calling a line
/// at-risk.
pub fn content_lines_missing_from(disk: &str, sidecar_blocks: &[SidecarBlock]) -> Vec<String> {
    /// One line of the `.md`, reduced to the text a sidecar block would
    /// hold, or `None` when the line is not block content at all.
    ///
    /// The two sides are deliberately asymmetric: the `.md` carries the
    /// renderer's layout (indent, `- ` marker) and a sidecar's `text` does
    /// not, so only this side strips.
    fn disk_line(line: &str) -> Option<&str> {
        let t = line.trim();
        if t.is_empty() {
            return None;
        }
        // Strip the marker exactly once. Repeating it would turn
        // `- - - x` into `x` and let it match a logged block `x`, which is
        // a false negative: unlogged content walking past the guard.
        // A bare `-` is an empty block, a first-class state (every Enter in
        // the TUI makes one), and normalises to the empty text its sidecar
        // entry carries.
        if let Some(body) = t.strip_prefix("- ") {
            return Some(body.trim_start());
        }
        if t == "-" {
            return Some("");
        }
        // No marker. Either a `key:: value` line the renderer emits for a
        // page or block property — which never lives in a block's `text`,
        // so comparing it would flag every page that has one — or a
        // continuation line, which is content. `parse_property_line` is
        // the single owner of that distinction, and it is the same one the
        // parser applies, so the two cannot disagree.
        //
        // Note this is decided *after* the marker check on purpose: a
        // bullet whose own text looks like a property (`- note:: remember`)
        // is stored by the parser as the block's text, so skipping it here
        // would hide real content.
        if crate::parse::parse_property_line(t).is_some() {
            return None;
        }
        Some(t)
    }
    /// A sidecar block's `text` split into the lines it occupies in the
    /// `.md`. No marker to strip and no property filtering: whatever is in
    /// `text` is, by definition, content the log holds.
    ///
    /// An empty block still occupies one line (the renderer emits a bare
    /// `-`), and `"".lines()` yields nothing, so it is contributed
    /// explicitly. Without this the bare `-` on disk matches no block and
    /// every page holding an empty block reads as unlogged.
    fn logged_lines(text: &str) -> impl Iterator<Item = &str> {
        std::iter::once("")
            .take(usize::from(text.is_empty()))
            .chain(text.lines().map(str::trim))
    }

    // **No stand-down here.** A caller that hands over blocks which
    // cannot answer the question must say so itself, via
    // [`sidecar_can_answer`], because only the caller knows where the
    // blocks came from.
    //
    // This function used to stand down on its own when every block had
    // an empty `text` — right for a pre-0.11 sidecar, and wrong for the
    // other kind of caller. `outl doctor` builds its reference from a
    // **render**, and a render of empty blocks is a definitive answer
    // ("the tree holds no text"), not an inability to answer. It hit the
    // same branch and the count came back `0`, so the doctor printed
    // "removes nothing" and the volume guard stayed quiet for a repair
    // that would empty the page — the exact run that guard exists to
    // stop. Measured: disk with 4 content lines, tree rendering `-\n-\n`,
    // reported 0 lines removed.
    //
    // The condition is unchanged, only its owner: it lives at each call
    // site now, next to the knowledge of what the blocks are.

    // A block's text can span lines (continuation), and each of those
    // lands as its own line in the `.md`, so index the pieces.
    let mut known: HashMap<&str, usize> = HashMap::new();
    for block in sidecar_blocks {
        for piece in logged_lines(&block.text) {
            *known.entry(piece).or_insert(0) += 1;
        }
    }

    let mut missing = Vec::new();
    for raw in disk.lines() {
        let Some(line) = disk_line(raw) else { continue };
        // Try the stripped form first, then the line as written.
        //
        // `disk_line` removes the bullet because the renderer *adds* one
        // — for a block's first line. It does not add one inside a block's
        // text, so a bullet that lives in a code fence, or in a pasted
        // list a block carries as continuation, is part of the text
        // verbatim, marker and all. Asking only the stripped form makes
        // `- endpoint:` on disk fail to match the `- endpoint:` the log
        // holds, and the page reads as carrying unlogged content.
        //
        // That is a false positive, and its cost is the one this whole
        // RFC calls worst: the page is refused for re-projection, its
        // `last_synced_hash` is withheld, and it reconciles on every boot
        // forever — frozen, with nothing wrong with it. Measured on the
        // reporting workspace: 8 pages, 49 lines, every one a bullet
        // inside a fence, and every one verified to survive
        // `parse → render` intact.
        //
        // The second shape does not widen *which* lines the log knows,
        // only how an indented one may match, so a line absent from the
        // log still fails both lookups.
        // Only an **indented** line gets the second chance. The renderer
        // never writes a continuation line at column 0, so a bullet
        // there is a block's own marker and nothing else. Allowing the
        // verbatim form for it would let a root-level `- secret` match a
        // fenced `- secret` the log holds somewhere else on the page —
        // narrow, but this is a data-loss guard and the comment below
        // used to claim it could not happen at all.
        let verbatim = if raw.starts_with([' ', '\t']) {
            Some(raw.trim())
        } else {
            None
        };
        let hit = match known.get_mut(line) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => match verbatim.and_then(|v| known.get_mut(v)) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    true
                }
                _ => false,
            },
        };
        if !hit {
            missing.push(line.to_string());
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::NodeId;

    fn blk(text: &str) -> SidecarBlock {
        SidecarBlock::from_text(NodeId::new(), 1, 0, text)
    }

    /// A bullet inside a code fence lives in the block's text **with its
    /// marker**, because the renderer does not add one there.
    ///
    /// Asking only the marker-stripped form made every such page read as
    /// carrying unlogged content — which withholds its hash and refuses
    /// its re-projection, freezing a page that has nothing wrong with it.
    /// Measured on a real workspace: 8 pages, 49 lines, all of this shape,
    /// every one verified to survive `parse → render` intact.
    #[test]
    fn a_bullet_inside_a_code_fence_is_not_reported_as_unlogged() {
        let disk = "- intro\n  ```yaml\n  - endpoint:\n    method: POST\n  ```\n";
        let logged = [blk("intro\n```yaml\n- endpoint:\n  method: POST\n```")];
        assert!(
            content_lines_missing_from(disk, &logged).is_empty(),
            "a fenced bullet the log holds verbatim must not read as unlogged"
        );
    }

    /// The same shape one level down, which is how it actually appeared:
    /// a fenced list pasted from Roam under a nested block.
    #[test]
    fn a_fenced_list_under_a_nested_block_is_not_reported_as_unlogged() {
        let disk = "- parent\n  - ```md\n    ## Objetivos\n    - Aumentar resistência\n    - Desenvolver força\n    ```\n";
        let logged = [
            blk("parent"),
            blk("```md\n## Objetivos\n- Aumentar resistência\n- Desenvolver força\n```"),
        ];
        assert!(
            content_lines_missing_from(disk, &logged).is_empty(),
            "got: {:?}",
            content_lines_missing_from(disk, &logged)
        );
    }

    /// **The guard must not have gone blind.** Accepting a second shape
    /// per line widens how a known line may match; it must not widen
    /// *which* lines count as known.
    #[test]
    fn content_the_log_does_not_have_is_still_reported() {
        let disk = "- intro\n  ```yaml\n  - endpoint:\n  ```\n- a line the log never saw\n";
        let logged = [blk("intro\n```yaml\n- endpoint:\n```")];
        let missing = content_lines_missing_from(disk, &logged);
        assert_eq!(missing, vec!["a line the log never saw".to_string()]);
    }

    /// A bullet on disk whose text the log does not hold in *either* form
    /// fails both lookups — the widening is per-shape, not per-line.
    #[test]
    fn an_unlogged_bullet_is_reported_whichever_shape_it_takes() {
        let disk = "- known\n  - unknown child\n";
        let logged = [blk("known")];
        let missing = content_lines_missing_from(disk, &logged);
        assert_eq!(missing, vec!["unknown child".to_string()]);
    }

    /// Multiset semantics survive the second lookup: a line the log holds
    /// once and disk holds twice still reports one.
    #[test]
    fn a_line_duplicated_on_disk_still_counts_the_surplus() {
        let disk = "- dup\n- dup\n";
        let logged = [blk("dup")];
        assert_eq!(content_lines_missing_from(disk, &logged).len(), 1);
    }

    /// The stand-down moved to the callers, so this function must now
    /// answer even when every block is empty.
    ///
    /// `outl doctor` builds its reference from a **render**, and a render
    /// of empty blocks says "the tree holds no text" — an answer. While
    /// the rule lived in here it silenced that caller too, so the doctor
    /// reported `0` lines removed for a repair that would empty the page
    /// and the volume guard never fired. Measured: 4 content lines on
    /// disk, tree rendering `-\n-\n`, reported 0.
    #[test]
    fn empty_reference_blocks_still_report_the_disk_content() {
        let disk = "- line one\n- line two\n- line three\n- line four\n";
        let empty_render = [blk(""), blk("")];
        assert_eq!(
            content_lines_missing_from(disk, &empty_render).len(),
            4,
            "a reference of empty blocks is an answer, not an absence of one"
        );
    }

    /// A root-level bullet does not get the verbatim second chance.
    ///
    /// The renderer never writes a continuation at column 0, so a bullet
    /// there is a block's own marker. Letting it match a fenced
    /// `- secret` the log holds elsewhere on the page would hide real
    /// unlogged content — narrow, but this decides whether bytes are
    /// deleted.
    #[test]
    fn a_root_level_bullet_does_not_match_a_fenced_copy_of_itself() {
        let disk = "- secret\n";
        let logged = [blk("```\n- secret\n```")];
        assert_eq!(
            content_lines_missing_from(disk, &logged),
            vec!["secret".to_string()],
            "a root bullet must not borrow a fenced line's identity"
        );
    }
}
