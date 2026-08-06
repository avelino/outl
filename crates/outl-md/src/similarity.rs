//! Level-2 scoring and assignment for [`crate::matching`].
//!
//! Level 2 recovers a block's id — and with it the `((blk-…))` handle
//! every other page points at — when one external save reworded a block
//! *and* inserted or removed another, so the flat counts disagree and
//! the positional fallback (level 1.5) is out of play.
//!
//! **Assignment is by global confidence, never by document order.**
//! Every (new block, old entry) pair inside the position window is
//! scored first, then the pairs are resolved from the highest score
//! down.
//!
//! Consuming candidates in the order the *new* blocks appear was a
//! silent-corruption bug: a freshly typed block that merely resembled an
//! old one took that old block's id — and its `ref_handle` — purely by
//! sitting at a lower DFS index, while the block the user had actually
//! edited fell to level 3 with a fresh ULID. Every old id was consumed,
//! so `orphans` came back empty and nothing reached `orphans.log`. The
//! runner-up margin could not catch it either: it only compared
//! candidates belonging to the same new block, never two new blocks
//! contending for the same old entry.
//!
//! The margin is therefore **two-sided**. A winning pair must beat the
//! best other claim on its new block *and* the best other claim on its
//! old entry by [`SIMILARITY_RUNNER_UP_MARGIN`]. A pair that merely ties
//! with a rival declines instead of winning by arriving first, and both
//! sides fall to level 3 — recoverable, because level 3 is logged.

use crate::sidecar::SidecarBlock;
use outl_core::id::NodeId;
use std::collections::HashSet;

/// Minimum normalized Levenshtein similarity for a level-2 match.
///
/// `0.8` is the documented threshold (see the crate's `CLAUDE.md`):
/// "the user reworded this block" stays above it; "the user replaced
/// this block with something else" falls below and is treated as a
/// deletion plus an insertion, which is the safe reading.
pub(crate) const SIMILARITY_THRESHOLD: f64 = 0.8;

/// How far apart (in DFS index) a new and an old block may sit and
/// still be considered for a level-2 match.
///
/// **Applied unconditionally.** It used to be skipped whenever the
/// parents agreed — but `parents_agree(None, None)` is `true`, so every
/// pair of *root* blocks agreed and the window never fired. A journal
/// page is a flat list of root blocks, which made it inert on exactly
/// the page shape this workspace has most of: block 0 could take the id
/// (and the `((blk-…))` handle) of block 40.
pub(crate) const SIMILARITY_POSITION_WINDOW: usize = 2;

/// Shortest text (in `char`s) level 2 will consider.
///
/// The ratio pre-filter is scale-free, so two blocks of equal length
/// always clear it, and at 6 chars a single differing character already
/// scores above the threshold. Measured false positives below this
/// floor: `item 1`/`item 2` (0.833), `[[2026-01-01]]`/`[[2026-07-01]]`
/// (0.929), `[[buser/tech]]`/`[[buser/team]]` (0.857),
/// `R$ 1.000,00`/`R$ 9.000,00` (0.909).
///
/// Dates, versions, amounts and namespaced refs are structured, short,
/// and everywhere in a real graph. Short blocks fall to level 3, which
/// is recoverable (recorded in `orphans.log`); a wrong level-2 match is
/// not — it hands one block's ref handle to another and every
/// `((blk-…))` pointing at it silently renders someone else's text.
///
/// Calibrated between the two: every measured lookalike above is 16
/// chars or shorter, and the shortest genuine reword in the test suite
/// (`review the storage RFC` → `…RFCs`) is 22. The floor is **not** the
/// main defence — the unconditional ±2 position window is. It just
/// removes the pairs where similarity carries no signal at all.
pub(crate) const SIMILARITY_MIN_CHARS: usize = 20;

/// How much better the winning level-2 pair must be than its best rival
/// on *either* endpoint.
///
/// Without a margin the choice between two near-identical candidates
/// (`item 1` vs `item 2` against `item 3`) comes down to index
/// proximity, which is a coin flip that costs a ref handle. When the top
/// two are this close, level 2 declines and both fall to level 3.
pub(crate) const SIMILARITY_RUNNER_UP_MARGIN: f64 = 0.05;

/// Longest text (in `char`s) level 2 will run the O(n·m) Levenshtein DP
/// over.
///
/// Blocks this large are pasted documents, not sentences someone
/// reworded; the quadratic cost is not worth paying on every save. They
/// fall through to level 3 exactly as they did before level 2 existed.
pub(crate) const SIMILARITY_MAX_CHARS: usize = 4096;

/// One scored (new block, old sidecar entry) pair.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// DFS index of the new block within the flattened parsed AST.
    pub(crate) new_index: usize,
    /// Index of the old entry within the sidecar's block list.
    pub(crate) old_index: usize,
    /// Normalized Levenshtein similarity, always above
    /// [`SIMILARITY_THRESHOLD`].
    pub(crate) score: f64,
}

/// An accepted level-2 assignment: `(new DFS index, old entry index,
/// similarity)`.
pub(crate) type Assignment = (usize, usize, f64);

/// Normalized Levenshtein similarity in `0.0..=1.0`, or `None` when the
/// pair is not worth (or not safe to) score.
///
/// Two cheap rejections come first, both exact — neither can discard a
/// pair that would have scored above the threshold:
///
/// - An empty side (a sidecar entry with no recorded text) has nothing
///   to compare.
/// - Levenshtein distance is at least the length difference, so
///   `similarity <= min_len / max_len`. When that ceiling is already at
///   or below the threshold, the O(n·m) DP cannot change the outcome.
pub(crate) fn similarity(new_text: &str, old_text: &str) -> Option<f64> {
    let new_len = new_text.chars().count();
    let old_len = old_text.chars().count();
    if new_len == 0 || old_len == 0 {
        return None;
    }
    // Absolute floor, not just a ratio: see SIMILARITY_MIN_CHARS. The
    // ratio filter is scale-free, so `[[2026-01-01]]` and
    // `[[2026-07-01]]` sail through it at 0.929.
    if new_len < SIMILARITY_MIN_CHARS || old_len < SIMILARITY_MIN_CHARS {
        return None;
    }
    if new_len > SIMILARITY_MAX_CHARS || old_len > SIMILARITY_MAX_CHARS {
        return None;
    }
    let (min, max) = if new_len < old_len {
        (new_len, old_len)
    } else {
        (old_len, new_len)
    };
    if (min as f64) / (max as f64) <= SIMILARITY_THRESHOLD {
        return None;
    }
    Some(strsim::normalized_levenshtein(new_text, old_text))
}

/// Score every still-open (new block, old entry) pair sitting within
/// [`SIMILARITY_POSITION_WINDOW`] DFS positions of each other.
///
/// The window bounds the work at `2 * SIMILARITY_POSITION_WINDOW + 1`
/// old entries per unmatched new block, so the candidate set stays
/// linear in the document. It never degrades into an n×m sweep, and the
/// Levenshtein DP itself only runs on pairs that survived the length
/// filters in [`similarity`].
pub(crate) fn collect_candidates(
    new_texts: &[&str],
    old_blocks: &[SidecarBlock],
    found: &[Option<NodeId>],
    used: &HashSet<NodeId>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (i, new_text) in new_texts.iter().enumerate() {
        if found[i].is_some() {
            continue;
        }
        let lo = i.saturating_sub(SIMILARITY_POSITION_WINDOW);
        let hi = (i + SIMILARITY_POSITION_WINDOW + 1).min(old_blocks.len());
        for (j, old) in old_blocks.iter().enumerate().take(hi).skip(lo) {
            if used.contains(&old.id) {
                continue;
            }
            let Some(score) = similarity(new_text, &old.text) else {
                continue;
            };
            if score <= SIMILARITY_THRESHOLD {
                continue;
            }
            out.push(Candidate {
                new_index: i,
                old_index: j,
                score,
            });
        }
    }
    out
}

/// Resolve scored candidates into assignments, highest confidence first.
///
/// Marks each winner in `found` / `used` and returns the accepted pairs
/// in ascending new-block order, so the caller's warnings come out in
/// document order.
pub(crate) fn resolve_candidates(
    mut candidates: Vec<Candidate>,
    old_blocks: &[SidecarBlock],
    new_count: usize,
    found: &mut [Option<NodeId>],
    used: &mut HashSet<NodeId>,
) -> Vec<Assignment> {
    // A total order two devices reproduce identically: score, then
    // positional distance, then both indices. `total_cmp` avoids the
    // partial-order escape hatch, and nothing here reads a HashMap's
    // iteration order.
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| {
                a.new_index
                    .abs_diff(a.old_index)
                    .cmp(&b.new_index.abs_diff(b.old_index))
            })
            .then_with(|| a.new_index.cmp(&b.new_index))
            .then_with(|| a.old_index.cmp(&b.old_index))
    });

    // Bucket by endpoint so finding a pair's rivals costs O(window)
    // instead of rescanning every candidate.
    let mut by_new: Vec<Vec<usize>> = vec![Vec::new(); new_count];
    let mut by_old: Vec<Vec<usize>> = vec![Vec::new(); old_blocks.len()];
    for (k, c) in candidates.iter().enumerate() {
        by_new[c.new_index].push(k);
        by_old[c.old_index].push(k);
    }

    let mut accepted: Vec<Assignment> = Vec::new();
    for (k, c) in candidates.iter().enumerate() {
        if found[c.new_index].is_some() || used.contains(&old_blocks[c.old_index].id) {
            continue;
        }
        let rival = best_rival(
            &candidates,
            &by_new[c.new_index],
            &by_old[c.old_index],
            k,
            found,
            used,
            old_blocks,
        );
        // Two live claims this close means we are guessing which block
        // the user reworded. Guessing costs a ref handle, so decline —
        // on both sides, because the loser re-reads the same margin
        // against this still-unclaimed pair on its own turn.
        if rival.is_some_and(|r| c.score - r < SIMILARITY_RUNNER_UP_MARGIN) {
            continue;
        }
        found[c.new_index] = Some(old_blocks[c.old_index].id);
        used.insert(old_blocks[c.old_index].id);
        accepted.push((c.new_index, c.old_index, c.score));
    }

    accepted.sort_by_key(|(i, _, _)| *i);
    accepted
}

/// Highest score among the candidates still contending for either
/// endpoint of `candidates[k]`.
///
/// "Still contending" means both of the rival's own endpoints are free:
/// a pair whose new block or old entry was already claimed by a stronger
/// assignment is no longer competition. A pair that was *declined*
/// keeps both endpoints free and therefore keeps competing — that is
/// what makes a decline symmetric instead of handing the id to the
/// runner-up on the very next iteration.
fn best_rival(
    candidates: &[Candidate],
    by_new: &[usize],
    by_old: &[usize],
    k: usize,
    found: &[Option<NodeId>],
    used: &HashSet<NodeId>,
    old_blocks: &[SidecarBlock],
) -> Option<f64> {
    by_new
        .iter()
        .chain(by_old)
        .copied()
        .filter(|&r| r != k)
        .filter(|&r| {
            let rival = candidates[r];
            found[rival.new_index].is_none() && !used.contains(&old_blocks[rival.old_index].id)
        })
        .map(|r| candidates[r].score)
        .max_by(f64::total_cmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_skips_pairs_whose_lengths_alone_rule_out_a_match() {
        // The length-ratio pre-filter is exact: it may only reject
        // pairs that could not have cleared the threshold anyway.
        assert!(similarity("hello there", "").is_none());
        assert!(similarity("", "hello there").is_none());
        assert!(similarity("short", "a very much longer block of text").is_none());
        let score = similarity(
            "buy groceries at the market",
            "buy groceries at the markets",
        )
        .expect("near-identical texts must be scored");
        assert!(score > SIMILARITY_THRESHOLD, "got {score}");
    }

    /// Every pair here was measured scoring above the 0.8 threshold
    /// before the length floor existed. They are the shapes a real graph
    /// is made of — dates, namespaced refs, amounts, versions — and a
    /// level-2 match on any of them hands one block's `((blk-…))` handle
    /// to a different block.
    #[test]
    fn short_structured_lookalikes_never_reach_level_2() {
        for (a, b) in [
            ("item 1", "item 2"),
            ("[[2026-01-01]]", "[[2026-07-01]]"),
            ("[[buser/tech]]", "[[buser/team]]"),
            ("R$ 1.000,00", "R$ 9.000,00"),
            ("v1.2.3", "v1.2.9"),
            ("TODO", "DONE"),
        ] {
            assert!(
                similarity(a, b).is_none(),
                "{a:?} vs {b:?} must not be scored at all — it is below the length floor"
            );
        }
    }

    /// The floor must not disarm the case level 2 exists for: a genuine
    /// reword of a real sentence.
    #[test]
    fn a_genuine_reword_of_a_real_sentence_still_scores() {
        let before = "decide the storage backend before the sprint ends";
        let after = "decide the storage backend before this sprint ends";
        let score = similarity(before, after).expect("a real reword must still be scored");
        assert!(score > SIMILARITY_THRESHOLD, "got {score}");
    }

    /// A length difference big enough to be a replacement, not a reword,
    /// stays rejected by the ratio filter.
    #[test]
    fn a_replacement_is_not_a_reword() {
        assert!(similarity(
            "decide the storage backend before the sprint ends",
            "ship it"
        )
        .is_none());
    }

    #[test]
    fn candidates_outside_the_position_window_are_never_scored() {
        let ids: Vec<NodeId> = (0..6).map(|_| NodeId::new()).collect();
        let old: Vec<SidecarBlock> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                SidecarBlock::from_text(*id, i + 1, 0, "decide the storage backend this sprint")
            })
            .collect();
        let texts = ["decide the storage backend this sprints"];
        let found = vec![None; texts.len()];
        let candidates = collect_candidates(&texts, &old, &found, &HashSet::new());
        assert_eq!(
            candidates.len(),
            SIMILARITY_POSITION_WINDOW + 1,
            "only old entries 0..=2 sit within the window of new block 0"
        );
        assert!(candidates.iter().all(|c| c.old_index <= 2));
    }
}
