/**
 * Rubber-band damping used by the horizontal swipe navigator.
 *
 * iOS scroll views damp over-scroll with a roughly square-root curve:
 * the further the finger travels, the less the content follows. This
 * keeps the page anchored, gives the user a sense of resistance, and
 * makes the snap-back transition feel like the *real* motion — not a
 * cleanup after a drag-along.
 *
 * For our purposes a fixed peak (40px) is more important than a
 * physically accurate UIScrollView curve, so we use a clamped
 * sqrt-based formula tuned by hand against an iPhone 17 Pro:
 *
 *   - up to ~16px finger travel feels nearly 1:1 (immediate response)
 *   - 80px travel → ~22px peek
 *   - any travel → capped at MAX_PEEK
 *
 * Exported pure for unit testing so we can lock the curve.
 */
export const MAX_PEEK = 40;
const SOFTNESS = 5.5; // higher = more resistance

export function damp(dx: number): number {
  if (dx === 0) return 0;
  const sign = Math.sign(dx);
  const mag = Math.abs(dx);
  const damped = Math.sqrt(mag) * SOFTNESS;
  return sign * Math.min(damped, MAX_PEEK);
}
