import { createSignal } from "solid-js";

/**
 * Drag-to-dismiss state for bottom sheets.
 *
 * Wire the returned `pointer*` handlers to the sheet's grab-handle
 * element (not the whole header — otherwise the user can't scroll
 * the list from near the top of the sheet, because the header
 * steals the pointer). The handle element also needs
 * `style="touch-action: none"` so the browser doesn't fight the
 * gesture with scroll interpretation.
 *
 * `translateY` exposes the live offset in pixels — apply it via
 * `transform: translateY(${translateY()}px)` on the sheet
 * container. `dragging()` is true while a finger is down, which we
 * use to skip the spring-back transition during the drag itself.
 *
 * Threshold defaults to 80px (≈ user committed); below that we
 * snap back to zero. The gesture only reacts to downward motion —
 * upward drag is meaningless for a sheet pinned to the bottom.
 */
export interface SheetDragHandlers {
  /** Live offset (pixels) — apply via `transform: translateY(…)`. */
  translateY: () => number;
  /** `true` while the user has a finger down. Disable the
   *  spring-back transition while this is true. */
  dragging: () => boolean;
  onPointerDown: (e: PointerEvent) => void;
  onPointerMove: (e: PointerEvent) => void;
  onPointerUp: () => void;
  onPointerCancel: () => void;
}

export function createSheetDrag(
  onDismiss: () => void,
  threshold = 80,
): SheetDragHandlers {
  const [translateY, setTranslateY] = createSignal(0);
  const [dragging, setDragging] = createSignal(false);
  let startY = 0;

  function onPointerDown(e: PointerEvent) {
    startY = e.clientY;
    setDragging(true);
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging()) return;
    setTranslateY(Math.max(0, e.clientY - startY));
  }

  function endDrag() {
    if (!dragging()) return;
    setDragging(false);
    if (translateY() > threshold) {
      setTranslateY(0);
      onDismiss();
    } else {
      setTranslateY(0);
    }
  }

  return {
    translateY,
    dragging,
    onPointerDown,
    onPointerMove,
    onPointerUp: endDrag,
    onPointerCancel: endDrag,
  };
}
