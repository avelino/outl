/**
 * Platform capability detection.
 *
 * The mobile and desktop shells run the **same** Solid frontend. A few
 * behaviours must diverge, though: physical-keyboard shortcuts
 * (Shift+Enter to create a block, Backspace to delete an empty one,
 * Tab to indent) only make sense where there *is* a physical keyboard.
 *
 * We branch on **input capability**, not OS name. What matters for a
 * keyboard shortcut is "does this device have a hardware keyboard and
 * a precise pointer", which maps cleanly onto "no touch points". This
 * also keeps `bun run dev` (plain browser, no Tauri) working — it
 * reports `maxTouchPoints === 0` on a laptop and behaves like the
 * desktop shell. If we ever need strict OS distinction (e.g. ⌘ vs
 * Ctrl labels) swap this for `@tauri-apps/plugin-os`'s `platform()`.
 */
let cachedDesktop: boolean | null = null;

/**
 * True on a desktop-class device (hardware keyboard, no touchscreen).
 * Cached after the first call — the answer never changes within a
 * session.
 */
export function isDesktop(): boolean {
  if (cachedDesktop !== null) return cachedDesktop;
  if (typeof navigator === "undefined") {
    cachedDesktop = true;
    return cachedDesktop;
  }
  cachedDesktop = (navigator.maxTouchPoints ?? 0) === 0;
  return cachedDesktop;
}
