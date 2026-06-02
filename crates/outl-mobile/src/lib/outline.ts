import { BlockNode } from "./api";

/**
 * Reconstruct the wire-format text of a block (TODO/DONE prefix
 * reattached). The mobile API splits the TODO state out of `text`
 * so the checkbox can render it separately, but Insert mode needs
 * the user to see and be able to erase the prefix — otherwise
 * dropping a TODO from the editor is impossible.
 *
 * Mirror of `outl_actions::split_todo` in reverse. Keep in sync with
 * `outl_actions::TODO_PREFIX` / `DONE_PREFIX`.
 */
export function rawTextWithTodo(block: BlockNode): string {
  if (!block.todo) return block.text;
  return `${block.todo} ${block.text}`;
}

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

/**
 * Flatten an outline into the **visible** DFS-preorder list: the
 * children of a collapsed block are hidden in the UI, so they're
 * skipped here too. Use this (not `flatten`) for anything that walks
 * the cells the user can actually see — e.g. ArrowUp/ArrowDown
 * navigation, which must land on the next *rendered* block.
 */
export function flattenVisible(blocks: BlockNode[]): BlockNode[] {
  const out: BlockNode[] = [];
  for (const b of blocks) {
    out.push(b);
    if (!b.collapsed) out.push(...flattenVisible(b.children));
  }
  return out;
}

/**
 * Id of the block immediately above (`"up"`) or below (`"down"`)
 * `id` in visible order, or `null` when `id` is at the top/bottom
 * edge (or absent). Built on `flattenVisible`, so collapsed subtrees
 * are stepped over rather than entered.
 */
export function neighborId(
  blocks: BlockNode[],
  id: string,
  dir: "up" | "down",
): string | null {
  const flat = flattenVisible(blocks);
  const idx = flat.findIndex((b) => b.id === id);
  if (idx === -1) return null;
  const target = dir === "up" ? flat[idx - 1] : flat[idx + 1];
  return target?.id ?? null;
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
