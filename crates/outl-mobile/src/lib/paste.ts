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
