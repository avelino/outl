import { BlockNode } from "./api";

/** Locate a block by id anywhere in a (possibly nested) outline. */
export function findBlock(blocks: BlockNode[], id: string): BlockNode | null {
  for (const b of blocks) {
    if (b.id === id) return b;
    const child = findBlock(b.children, id);
    if (child) return child;
  }
  return null;
}

/** Flatten an outline into a DFS-preorder list. */
export function flatten(blocks: BlockNode[]): BlockNode[] {
  const out: BlockNode[] = [];
  for (const b of blocks) {
    out.push(b);
    out.push(...flatten(b.children));
  }
  return out;
}

/** Total number of descendants under a block (recursive count of its
 *  whole subtree, *excluding* the block itself). Used to warn the
 *  user before a destructive delete. */
export function countDescendants(block: BlockNode): number {
  let n = 0;
  for (const child of block.children) {
    n += 1 + countDescendants(child);
  }
  return n;
}

/** Block that the backend inserted right after `afterId` in a fresh
 * outline, or `null` if `afterId` is the very last node. */
export function findInsertedAfter(
  blocks: BlockNode[],
  afterId: string,
): BlockNode | null {
  const flat = flatten(blocks);
  const idx = flat.findIndex((b) => b.id === afterId);
  if (idx === -1) return null;
  return flat[idx + 1] ?? null;
}
