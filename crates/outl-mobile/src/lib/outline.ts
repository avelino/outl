import type { BlockNode } from "@outl/shared/api/types";

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

