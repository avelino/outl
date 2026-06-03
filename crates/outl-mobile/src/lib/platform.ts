/**
 * Platform capability detection.
 *
 * The mobile and desktop shells run the **same** Solid frontend. A few
 * behaviours must diverge, though: physical-keyboard shortcuts
 * (Shift+Enter to create a block, Backspace to delete an empty one,
 * Tab to indent, Arrow keys to move between blocks) only make sense
 * where there *is* a hardware keyboard.
 *
 * We branch on **input capability**, not OS name. "Has a fine pointer"
 * (`any-pointer: fine`) is the most reliable proxy for "has a hardware
 * keyboard": it is true on every laptop/desktop — *including*
 * touchscreen laptops (Windows, Chromebook, Surface) that also report
 * touch points — and false on phones/tablets whose only pointer is a
 * coarse touch surface. Keying off touch-point count alone
 * misclassified those hybrid devices and stripped their shortcuts. A
 * plain desktop browser (`bun run dev`) reports a fine pointer and so
 * behaves like the desktop shell. If we ever need strict OS distinction
 * (⌘ vs Ctrl labels) swap this for `@tauri-apps/plugin-os`'s
 * `platform()`.
 */

/**
 * True on a desktop-class device (has a fine pointer, hence almost
 * certainly a hardware keyboard).
 *
 * Not cached: recomputing is negligible and lets the answer track a
 * keyboard/pointer being plugged in or unplugged, a switch between touch
 * and pointer input, or a test overriding `matchMedia` / `navigator`.
 */
export function isDesktop(): boolean {
  if (typeof navigator === "undefined") return true;
  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function"
  ) {
    return window.matchMedia("(any-pointer: fine)").matches;
  }
  // Fallback for environments without `matchMedia`: treat "no touch
  // points" as desktop, matching the original heuristic.
  return (navigator.maxTouchPoints ?? 0) === 0;
}
