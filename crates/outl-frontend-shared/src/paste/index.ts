/**
 * Heuristic and helpers for the external-clipboard paste flow.
 *
 * The actual markdown → block-tree conversion happens in the Rust
 * backend (`outl_actions::paste_markdown`, exposed as the
 * `paste_markdown_at` Tauri command). The frontend's job is to:
 *
 * 1. Detect that the user pasted *outline-like* content (otherwise we
 *    let the browser do its default thing — splice the text into the
 *    textarea — so plain text doesn't trigger a round trip).
 * 2. Hand the raw clipboard string to the backend along with the
 *    block id and caret position; the backend mutates the workspace
 *    and returns the refreshed page view.
 *
 * The same heuristic is mirrored in
 * `outl_actions::paste::looks_like_outline`. Keep them in sync.
 */

/**
 * True when `text` looks like a markdown bullet list (at least one
 * non-blank line starts with `- ` or is just `-`). The detector errs
 * on the side of "outline" — false positives only cost one Tauri
 * round trip while false negatives lose the user's hierarchy.
 *
 * Mirror of `outl_actions::paste::looks_like_outline`. Keep both in
 * sync — the Rust side is the canonical contract; the JS copy exists
 * to gate the Tauri round-trip before the user sees a flash of
 * "browser default splice" while the backend runs.
 */
export function looksLikeOutline(text: string): boolean {
  if (!text) return false;
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.replace(/^[ \t]+/, "");
    if (trimmed === "-" || trimmed.startsWith("- ")) {
      return true;
    }
  }
  return false;
}

/**
 * Convert a UTF-16 code unit offset (what the DOM textarea reports
 * via `selectionStart`) into a Unicode-codepoint offset, which is
 * what `outl_actions::PasteAnchor::AtCaret { caret }` expects — Rust
 * `str::chars()` iterates codepoints, not UTF-16 code units.
 *
 * For text that lives entirely in the BMP (every codepoint ≤ U+FFFF)
 * the two counts are equal, so this is a no-op for ASCII / most CJK.
 * It only matters for content with characters in supplementary planes
 * — emoji, mathematical symbols, less-common CJK extensions — where
 * each codepoint takes 2 UTF-16 code units. Without this conversion,
 * pasting after such a character lands the splice one position too
 * late per high-plane char before the caret.
 */
export function utf16OffsetToCharOffset(
  text: string,
  utf16Offset: number,
): number {
  if (utf16Offset <= 0) return 0;
  let chars = 0;
  let i = 0;
  while (i < utf16Offset && i < text.length) {
    const cp = text.codePointAt(i);
    if (cp === undefined) break;
    i += cp > 0xffff ? 2 : 1;
    chars += 1;
  }
  return chars;
}
