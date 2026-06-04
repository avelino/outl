/**
 * Markdown-wrap helpers for the active block textarea.
 *
 * Called from `action-handlers.ts` when the user fires `WrapBold`,
 * `WrapItalic`, etc. Wraps the current selection with the
 * delimiter pair; with no selection, inserts the pair and parks
 * the caret between the two halves so the user can start typing
 * immediately.
 *
 * `document.activeElement` is the source of truth — we don't keep
 * a Solid signal for "which textarea is active". The dispatcher
 * fires when a textarea has focus (Insert mode), so by the time we
 * land here the right element is already focused.
 *
 * Re-firing the `input` event matters: `<BlockRow />` binds
 * `onInput={(e) => setDraft(e.currentTarget.value)}`, so a direct
 * `ta.value = …` write bypasses the signal. Dispatching an `input`
 * event keeps Solid in sync, and `commit()` on blur / Esc picks up
 * the wrapped value as the new draft.
 */

function activeTextarea(): HTMLTextAreaElement | null {
  const el = document.activeElement;
  return el instanceof HTMLTextAreaElement ? el : null;
}

function fireInputEvent(ta: HTMLTextAreaElement) {
  ta.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Wrap the current selection with `prefix` … `suffix`. With no
 * selection, insert the pair and place the caret between them.
 */
export function wrapSelection(prefix: string, suffix: string = prefix) {
  const ta = activeTextarea();
  if (!ta) return;
  const start = ta.selectionStart ?? 0;
  const end = ta.selectionEnd ?? start;
  const before = ta.value.slice(0, start);
  const selected = ta.value.slice(start, end);
  const after = ta.value.slice(end);

  ta.value = `${before}${prefix}${selected}${suffix}${after}`;
  fireInputEvent(ta);

  if (selected.length === 0) {
    // Park caret between the delimiters: `**|**`.
    const caret = start + prefix.length;
    ta.setSelectionRange(caret, caret);
  } else {
    // Keep the selection on the previously-selected text — the
    // delimiters wrap around it.
    ta.setSelectionRange(start + prefix.length, end + prefix.length);
  }
}

/**
 * Insert `[label](url)`. When the user has a selection, that text
 * becomes the label and `url` is highlighted for them to type into.
 * Otherwise we splice `[text](url)` and select `text` first — the
 * user types the label, presses Tab to jump to `url`, and types
 * the destination.
 */
export function insertLink() {
  const ta = activeTextarea();
  if (!ta) return;
  const start = ta.selectionStart ?? 0;
  const end = ta.selectionEnd ?? start;
  const before = ta.value.slice(0, start);
  const selected = ta.value.slice(start, end);
  const after = ta.value.slice(end);

  if (selected.length > 0) {
    const url = "url";
    ta.value = `${before}[${selected}](${url})${after}`;
    fireInputEvent(ta);
    const urlStart = start + 1 + selected.length + 2; // "[" + label + "]("
    ta.setSelectionRange(urlStart, urlStart + url.length);
  } else {
    const placeholder = "text";
    const url = "url";
    ta.value = `${before}[${placeholder}](${url})${after}`;
    fireInputEvent(ta);
    const labelStart = start + 1; // after "["
    ta.setSelectionRange(labelStart, labelStart + placeholder.length);
  }
}
