/**
 * Outline traversal helpers — selection navigation. Pure functions
 * over `BlockNode[]` so they're cheap to call from action handlers
 * without any reactive setup.
 *
 * `flattenVisible` honours each block's `collapsed` flag (folded
 * children are invisible to vim-style `j/k` movement; the user pops
 * them open with `c`/`Enter` first).
 *
 * There used to be a `flattenAll` + `findNewId` pair here that
 * diff'd outline snapshots to recover the id of a freshly-created
 * block. They were removed once `createBlock` started returning
 * `{ view, new_id }` on the wire — the diff path mis-identified the
 * new block whenever the anchor had children (`flat[idx+1]` landed
 * on `children[0]` instead of the new sibling) and surfaced
 * `block <ULID> is not in the tree` toasts.
 */

import type { BlockNode } from "@outl/shared/api/types";

/**
 * IDs of every block visible in the outline, in flat DFS order
 * (parent before children, siblings in array order). Collapsed
 * subtrees are skipped — `j` after a collapsed block lands on its
 * next sibling, not on its hidden child.
 */
export function flattenVisible(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      out.push(b.id);
      if (!b.collapsed && b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * IDs of **every** block on the page, in flat DFS order, including
 * blocks hidden under collapsed parents. Use this when an operation
 * needs to act on the whole outline regardless of the fold state
 * (vim `zR` / `zM`, full-page reindex, "select all", …).
 *
 * Distinct from [`flattenVisible`] precisely because `zR` must
 * expand subtrees that are currently hidden — the visible-only walk
 * would silently no-op on every descendant of a collapsed parent.
 */
export function flattenAll(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      out.push(b.id);
      if (b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * IDs that `zM` (fold-all) should target. Walks the full outline in
 * DFS order **but skips leaves**: folding a leaf is invisible today
 * yet still writes `Op::SetCollapsed(true)` to the log, so when the
 * user later adds children underneath they appear collapsed — real
 * future-surprise. By contrast, `zR` (unfold-all) wants every id
 * (use [`flattenAll`] for that) — descolapsar leaf é no-op futuro.
 *
 * Mirror of `outl-tui`'s `collect_collapse_candidates` so a 200-block
 * page fires the same op count on every client.
 */
export function flattenParents(blocks: BlockNode[]): string[] {
  const out: string[] = [];
  const walk = (bs: BlockNode[]) => {
    for (const b of bs) {
      if (b.children.length > 0) out.push(b.id);
      if (b.children.length > 0) walk(b.children);
    }
  };
  walk(blocks);
  return out;
}

/**
 * Return the next visible id after `current`, or `current` itself
 * when already at the bottom (clamps; no wrap). When `current` is
 * `null` or not in the outline, returns the first visible id.
 */
export function nextVisibleId(
  current: string | null,
  blocks: BlockNode[],
): string | null {
  const ids = flattenVisible(blocks);
  if (ids.length === 0) return null;
  if (!current) return ids[0];
  const idx = ids.indexOf(current);
  if (idx === -1) return ids[0];
  return ids[Math.min(idx + 1, ids.length - 1)];
}

/** Previous visible id; returns `null` at the top (no clamp — see below). */
export function previousVisibleId(
  current: string | null,
  blocks: BlockNode[],
): string | null {
  const ids = flattenVisible(blocks);
  if (ids.length === 0) return null;
  if (!current) return ids[0];
  const idx = ids.indexOf(current);
  if (idx === -1) return ids[0];
  // No previous block when `current` is the first (or only) one — return
  // null, never `current` itself. The old `Math.max(idx - 1, 0)` →
  // `ids[0]` left the cursor on the very block the caller was about to
  // delete: selection then anchored a *trashed* node, and `o` /
  // new-block created the next block under the trash root (a deleted
  // node still satisfies `tree.contains`), corrupting the page. Callers
  // already handle null (delete clears the cursor; the page-change
  // effect re-snaps to the first block).
  return idx > 0 ? ids[idx - 1] : null;
}

/**
 * Resolve a Visual range to the inclusive `[lo, hi]` pair of ids in
 * DFS visible order. Caller passes anchor + cursor; this function
 * orders them so the rest of the codebase doesn't have to care about
 * direction (anchor below cursor or above).
 *
 * Returns `null` when either endpoint isn't visible (e.g. collapsed
 * subtree shifted the outline between Visual entry and the current
 * read). Caller should treat that as "no range".
 */
export function visualRangeIds(
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): { lo: string; hi: string } | null {
  if (!anchor || !cursor) return null;
  const ids = flattenVisible(blocks);
  const a = ids.indexOf(anchor);
  const c = ids.indexOf(cursor);
  if (a === -1 || c === -1) return null;
  const lo = Math.min(a, c);
  const hi = Math.max(a, c);
  return { lo: ids[lo], hi: ids[hi] };
}

/**
 * Build the **set of block ids inside the Visual range** in one DFS
 * walk. The parent component (`<OutlineView />`) memoises the result
 * inside a `createMemo`; every `<BlockRow />` then answers membership
 * in O(1) via `Set.has(id)`.
 *
 * Why a Set lives at the parent and not inside each row: the earlier
 * implementation called `isInVisualRange(id, anchor, cursor, outline)`
 * per row, and each call rebuilt `flattenVisible(blocks)` from scratch
 * (a full DFS). With N visible blocks that's O(N²) per Visual extension
 * keystroke — on a 500-block page that's measurable lag. The memo
 * pushes the DFS up to one walk per outline/anchor/cursor change.
 *
 * Returns `null` when anchor or cursor is unset, or when either id
 * isn't in the visible outline (collapsed subtree shifted between
 * entry and read). `null` is the caller's signal for "no range".
 */
export function visualRangeSet(
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): Set<string> | null {
  if (!anchor || !cursor) return null;
  const ids = flattenVisible(blocks);
  const a = ids.indexOf(anchor);
  const c = ids.indexOf(cursor);
  if (a === -1 || c === -1) return null;
  const lo = Math.min(a, c);
  const hi = Math.max(a, c);
  return new Set(ids.slice(lo, hi + 1));
}

/**
 * Predicate variant of [`visualRangeIds`]. Kept for the `outline-walk`
 * test suite and any caller that needs an ad-hoc membership check
 * without paying for the Set allocation. **In React/Solid render
 * paths, prefer [`visualRangeSet`] hoisted to a parent `createMemo`** —
 * calling this per row is O(N²) by construction.
 */
export function isInVisualRange(
  id: string,
  anchor: string | null,
  cursor: string | null,
  blocks: BlockNode[],
): boolean {
  const set = visualRangeSet(anchor, cursor, blocks);
  return set !== null && set.has(id);
}
