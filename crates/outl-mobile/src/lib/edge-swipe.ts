/**
 * Edge-swipe gating for the full-page swipe navigator.
 *
 * The mobile outline needs **two** horizontal gestures to coexist:
 *
 * 1. Row-level swipe (`SwipeRow`) — slide left to reveal delete.
 * 2. Page-level swipe (`SwipeNavigator`) — go to previous/next day.
 *
 * If page-level capture fires anywhere on screen, the row swipe is
 * starved because the parent grabs `setPointerCapture` first. The
 * iOS convention used by Safari, Mail, Spark and Bear is *edge
 * swipe*: page navigation only engages when the gesture starts in
 * the leftmost or rightmost gutter. Everywhere else the gesture is
 * forwarded to inner widgets.
 *
 * This module is the pure decision function. Exported for unit
 * tests so we can lock the geometry.
 */

/** Pixels from each side that count as the "edge gutter". */
export const EDGE_GUTTER = 24;

export type EdgeOrigin = "left" | "right" | null;

/** Where the gesture started relative to the screen edges. */
export function classifyOrigin(startX: number, screenWidth: number): EdgeOrigin {
  if (startX <= EDGE_GUTTER) return "left";
  if (startX >= screenWidth - EDGE_GUTTER) return "right";
  return null;
}

export interface CaptureDecision {
  /** Should the navigator take over the pointer? */
  capture: boolean;
  /** Should the navigator give up on this gesture entirely? */
  abort: boolean;
}

/**
 * Decide what the page-level swipe navigator should do given the
 * current pointer state. The caller drives this on every move
 * event until it gets either `capture: true` (own the gesture) or
 * `abort: true` (give up — pass-through to inner widgets).
 *
 * Rules:
 * - If the gesture didn't start at an edge, abort. The row's
 *   `SwipeRow` will take over since we never called
 *   `setPointerCapture`.
 * - If it started at the **left** edge, only capture on a
 *   *rightward* drag (= navigate back / previous day).
 * - If it started at the **right** edge, only capture on a
 *   *leftward* drag (= navigate forward / next day).
 * - In either case, require the drag to be horizontally dominant
 *   (`|dx| > |dy| * 1.5`) and exceed a small threshold (10px).
 * - Big vertical movement aborts (lets scroll keep working).
 */
export function decideCapture(args: {
  origin: EdgeOrigin;
  dx: number;
  dy: number;
}): CaptureDecision {
  const { origin, dx, dy } = args;
  if (origin === null) return { capture: false, abort: true };

  const horizontalDominant =
    Math.abs(dx) > 10 && Math.abs(dx) > Math.abs(dy) * 1.5;

  if (!horizontalDominant) {
    if (Math.abs(dy) > 12) return { capture: false, abort: true };
    return { capture: false, abort: false };
  }

  if (origin === "left" && dx <= 0) {
    // Started on the left edge but the user is pulling further
    // left — not a navigation gesture.
    return { capture: false, abort: true };
  }
  if (origin === "right" && dx >= 0) {
    return { capture: false, abort: true };
  }
  return { capture: true, abort: false };
}
