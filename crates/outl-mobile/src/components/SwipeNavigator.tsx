import { JSX, createSignal, onCleanup } from "solid-js";
import { classifyOrigin, decideCapture, EdgeOrigin } from "../lib/edge-swipe";
import { damp } from "../lib/rubber";

interface SwipeNavigatorProps {
  children: JSX.Element;
  /** Pixels of *raw finger travel* before the swipe commits. */
  threshold?: number;
  /** Called when the user swiped right past `threshold`. */
  onSwipeRight?: () => void;
  /** Called when the user swiped left past `threshold`. */
  onSwipeLeft?: () => void;
  /** Disable horizontal capture (e.g. while editing). */
  disabled?: boolean;
}

/**
 * Horizontal swipe gesture wrapper for full-screen navigation
 * (previous/next day in the journal, back-stack on pages).
 *
 * Design intent: the page should *not* feel "loose" while the finger
 * is dragging. We use rubber-band damping so the visible peek is
 * capped at ~40px no matter how far the finger travels, while the
 * underlying finger-distance is what decides whether a commit
 * happens. The only fluid animation is the snap (out on commit,
 * back to zero on cancel).
 *
 * Why not move 1:1: a full-page outline that slides with the finger
 * feels detached from the content — the user can't tell what's
 * happening underneath. The Apple convention for navigation swipes is
 * "show resistance, then snap" — Safari back, Mail message
 * navigation. We match that.
 */
export function SwipeNavigator(props: SwipeNavigatorProps): JSX.Element {
  const threshold = () => props.threshold ?? 80;
  const [offset, setOffset] = createSignal(0);
  const [animating, setAnimating] = createSignal(false);
  let rawDx = 0;
  let startX = 0;
  let startY = 0;
  let active = false;
  let captured = false;
  let origin: EdgeOrigin = null;

  /** Snap back to zero with the same spring used for the commit out. */
  function snapBack() {
    rawDx = 0;
    setAnimating(true);
    setOffset(0);
    window.setTimeout(() => setAnimating(false), 220);
  }

  function onPointerDown(e: PointerEvent) {
    if (props.disabled) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    if ((e.target as HTMLElement).closest("textarea,input,button,[role='button']")) {
      return; // let interactive children handle the press
    }
    startX = e.clientX;
    startY = e.clientY;
    // Page-level navigation is *edge-only*: anywhere in the middle
    // of the screen the gesture must reach inner widgets (e.g.
    // `SwipeRow`'s swipe-to-delete). See `lib/edge-swipe.ts`.
    origin = classifyOrigin(e.clientX, window.innerWidth);
    if (origin === null) return; // pass through to children
    active = true;
    captured = false;
    rawDx = 0;
    setAnimating(false);
    void e.pointerId;
  }

  function onPointerMove(e: PointerEvent) {
    if (!active) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (!captured) {
      const decision = decideCapture({ origin, dx, dy });
      if (decision.abort) {
        active = false;
        return;
      }
      if (!decision.capture) return;
      captured = true;
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    }
    rawDx = dx;
    // Visual peek is heavily damped — finger distance still decides
    // the commit threshold (see commit()).
    setOffset(damp(dx));
  }

  function commit() {
    const dx = rawDx;
    if (Math.abs(dx) > threshold()) {
      // Brief slide-out before the parent swaps the page so the
      // transition reads as "swipe → reveal new", not "swipe → pop".
      setAnimating(true);
      setOffset(dx > 0 ? 120 : -120);
      window.setTimeout(() => {
        if (dx > 0 && props.onSwipeRight) {
          props.onSwipeRight();
        } else if (dx < 0 && props.onSwipeLeft) {
          props.onSwipeLeft();
        }
        // Reset off-screen on the *opposite* side so the new page
        // enters from the direction the old one left toward.
        setOffset(0);
        setAnimating(false);
        rawDx = 0;
        origin = null;
      }, 160);
      return;
    }
    snapBack();
    origin = null;
  }

  function onPointerUp() {
    if (!active) return;
    active = false;
    if (captured) commit();
    else rawDx = 0;
  }

  function onPointerCancel() {
    if (!active) return;
    active = false;
    snapBack();
  }

  onCleanup(() => {
    active = false;
  });

  return (
    <div
      class="relative w-full overflow-x-hidden"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      style={{
        "touch-action": "pan-y",
      }}
    >
      <div
        class="w-full"
        style={{
          transform: `translateX(${offset()}px)`,
          // Apple's UIKit standard navigation curve.
          transition: animating()
            ? "transform 220ms cubic-bezier(0.32, 0.72, 0, 1)"
            : "none",
          "will-change": offset() !== 0 ? "transform" : "auto",
        }}
      >
        {props.children}
      </div>
    </div>
  );
}
