/**
 * OS file drag-and-drop wiring, shared by every GUI client.
 *
 * The Tauri webview reports native drag-drop through
 * `getCurrentWebview().onDragDropEvent`; this module turns those raw
 * events (physical-pixel positions + file paths) into the block id under
 * the cursor and hands them to the caller. *What* to do with the drop —
 * import the file, splice the link into the active line vs. append a
 * block — is the client's job (it holds the editor state); this module
 * only resolves *where* the drop landed and stays free of workspace logic.
 *
 * Both `outl-desktop` and `outl-mobile` consume this identically, so the
 * physical→CSS math and the `data-block-id` hit-test can't drift between
 * them. The pure helpers are unit-tested; `installFileDrop` needs a real
 * webview.
 */
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * Convert a physical-pixel position (what the drag-drop event reports)
 * to CSS pixels. `document.elementFromPoint` expects CSS pixels, so on a
 * HiDPI display (`devicePixelRatio > 1`) the raw physical position lands
 * far off-screen without this divide.
 */
export function physicalToCss(
  position: { x: number; y: number },
  dpr: number,
): { x: number; y: number } {
  const ratio = dpr > 0 ? dpr : 1;
  return { x: position.x / ratio, y: position.y / ratio };
}

/**
 * Walk up from a hit element to the nearest node carrying a
 * `data-block-id` (an outline row or its open textarea), returning that
 * block id — or `null` when the drop didn't land on a block (the header,
 * the empty gutter, the backlinks section).
 */
export function blockIdFromElement(el: Element | null): string | null {
  const host = el?.closest<HTMLElement>("[data-block-id]");
  return host?.dataset.blockId ?? null;
}

/**
 * Join several assets' ready-to-insert markdown links into one string,
 * space-separated, dropping empties. Multiple files dropped at once land
 * on one line.
 */
export function joinAssetMarkdowns(markdowns: string[]): string {
  return markdowns.filter((m) => m.length > 0).join(" ");
}

/**
 * Append an asset markdown link to a block's existing text, inserting a
 * single separating space only when the block already had content (so an
 * empty block doesn't gain a leading space).
 */
export function appendMarkdownToBlock(
  existing: string,
  markdown: string,
): string {
  return existing.length > 0 ? `${existing} ${markdown}` : markdown;
}

/** Resolve the block id under a physical-pixel drop position. */
export function blockIdAtPhysical(position: {
  x: number;
  y: number;
}): string | null {
  if (typeof document === "undefined") return null;
  const { x, y } = physicalToCss(position, window.devicePixelRatio);
  return blockIdFromElement(document.elementFromPoint(x, y));
}

export interface FileDropHandlers {
  /** Cursor entered the window dragging files; `blockId` is under it. */
  onEnter?(blockId: string | null): void;
  /** Cursor moved while dragging; `blockId` is the current hover target. */
  onOver?(blockId: string | null): void;
  /** Cursor left the window (drag cancelled or moved out). */
  onLeave?(): void;
  /** Files dropped; `blockId` is the block under the drop (or `null`). */
  onDrop(paths: string[], blockId: string | null): void | Promise<void>;
}

/**
 * Wire the Tauri webview's OS file drag-drop events to `handlers`.
 * Returns the unlisten fn (call on unmount). Block ids are resolved from
 * the drop position via `document.elementFromPoint`. A client that only
 * cares about the drop passes just `onDrop`; the hover callbacks are
 * optional.
 */
export function installFileDrop(
  handlers: FileDropHandlers,
): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    switch (p.type) {
      case "enter":
        handlers.onEnter?.(blockIdAtPhysical(p.position));
        break;
      case "over":
        handlers.onOver?.(blockIdAtPhysical(p.position));
        break;
      case "leave":
        handlers.onLeave?.();
        break;
      case "drop":
        void handlers.onDrop(p.paths, blockIdAtPhysical(p.position));
        break;
    }
  });
}
