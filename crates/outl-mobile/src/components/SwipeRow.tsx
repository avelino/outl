import { JSX, createSignal, onCleanup } from "solid-js";

interface SwipeRowProps {
  children: JSX.Element;
  /** Triggered when the user swipes far enough to the left. */
  onSwipeLeft?: () => void;
  /** Label shown on the right action panel (e.g. "Delete"). */
  leftActionLabel?: string;
  /** Threshold in px to consider the swipe committed. */
  threshold?: number;
}

/**
 * Lightweight swipe-to-action wrapper. Tracks pointer drag, slides
 * the row horizontally, and fires `onSwipeLeft` if the drag passes
 * `threshold`. No external animation library needed.
 */
export function SwipeRow(props: SwipeRowProps): JSX.Element {
  const threshold = () => props.threshold ?? 96;
  const [offset, setOffset] = createSignal(0);
  const [animating, setAnimating] = createSignal(false);
  let startX = 0;
  let startY = 0;
  let active = false;
  let captured = false;

  function onPointerDown(e: PointerEvent) {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    startX = e.clientX;
    startY = e.clientY;
    active = true;
    captured = false;
    setAnimating(false);
  }
  function onPointerMove(e: PointerEvent) {
    if (!active) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (!captured) {
      // Decide gesture: only horizontal moves of >8px capture, so
      // tap and vertical scroll still work.
      if (Math.abs(dx) > 8 && Math.abs(dx) > Math.abs(dy) * 1.5) {
        captured = true;
        (e.target as Element).setPointerCapture?.(e.pointerId);
      } else if (Math.abs(dy) > 10) {
        active = false;
        return;
      } else {
        return;
      }
    }
    // Only allow swipe-left for delete (negative dx).
    const v = Math.min(0, dx);
    setOffset(v);
  }
  function commit() {
    if (offset() <= -threshold() && props.onSwipeLeft) {
      props.onSwipeLeft();
    }
    setAnimating(true);
    setOffset(0);
  }
  function onPointerUp() {
    if (!active) return;
    active = false;
    if (captured) commit();
  }
  function onPointerCancel() {
    if (!active) return;
    active = false;
    setAnimating(true);
    setOffset(0);
  }

  onCleanup(() => {
    active = false;
  });

  return (
    <div class="relative overflow-hidden">
      <div
        class="absolute inset-y-0 right-0 flex items-center justify-end bg-(--color-ios-destructive) px-5 text-[15px] font-semibold text-white"
        style={{ width: `${threshold()}px` }}
      >
        {props.leftActionLabel ?? "Delete"}
      </div>
      <div
        class="relative bg-(--color-ios-card) dark:bg-(--color-iosd-card)"
        style={{
          transform: `translateX(${offset()}px)`,
          transition: animating() ? "transform 180ms cubic-bezier(0.4, 0, 0.2, 1)" : "none",
          "touch-action": "pan-y",
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
      >
        {props.children}
      </div>
    </div>
  );
}
