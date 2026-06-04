/**
 * Outline traversal helpers — selection navigation, ID lookup,
 * sibling discovery. Pure functions over `BlockNode[]` so they're
 * cheap to call from action handlers without any reactive setup.
 *
 * `flattenVisible` honours each block's `collapsed` flag (folded
 * children are invisible to vim-style `j/k` movement; the user
 * pops them open with `c`/`Enter` first). `flattenAll` walks every
 * id, used by `findNewIdAfter` to identify a freshly-created
 * block in the refreshed view.
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

/** IDs of every block in the outline, ignoring `collapsed`. */
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

/** Previous visible id; clamps at the top. */
export function previousVisibleId(
  current: string | null,
  blocks: BlockNode[],
): string | null {
  const ids = flattenVisible(blocks);
  if (ids.length === 0) return null;
  if (!current) return ids[0];
  const idx = ids.indexOf(current);
  if (idx === -1) return ids[0];
  return ids[Math.max(idx - 1, 0)];
}

/**
 * After `createBlock` returns a refreshed outline, identify which
 * block is new by diffing against the `before` snapshot of ids.
 * Returns the first novel id in DFS order, or `null` when nothing
 * changed (should never happen on a successful insert).
 */
export function findNewId(
  before: Set<string>,
  outline: BlockNode[],
): string | null {
  const all = flattenAll(outline);
  return all.find((id) => !before.has(id)) ?? null;
}
