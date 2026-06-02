/**
 * Textarea mutation helpers that survive Solid's reactive bindings.
 *
 * The trap: a `value={draft()}` binding on a textarea reassigns
 * `el.value` whenever `setDraft` is called. On iOS WKWebView (and
 * other browsers in some conditions) assigning `el.value` jumps the
 * caret to the end of the text. That means any code that does
 * "mutate textarea, then position caret, then `setDraft`" loses the
 * caret to Solid's late re-binding.
 *
 * Two primitives below paper over that:
 *
 * - [`spliceText`] mutates the textarea via `setRangeText`, the
 *   WHATWG-spec API that splices text without disturbing the caret.
 *   Preferred over `el.value = …` for any mid-text insertion.
 *
 * - [`parkCaret`] places the caret at `pos` synchronously **and**
 *   re-places it inside a microtask. The microtask hop ensures we
 *   also win the race against Solid's binding effect, which runs
 *   synchronously inside the `setDraft` that callers invoke right
 *   after mutating the textarea.
 *
 * Usage pattern:
 *
 * ```ts
 * spliceText(el, start, end, "[[]]");
 * parkCaret(el, start + 2);
 * setDraft(el.value);
 * parkCaret(el, start + 2); // second pass beats the Solid re-binding
 * ```
 */

/**
 * Splice `insert` into `el` at `[start, end)` using `setRangeText`
 * — a textarea mutation that does NOT jump the caret. Falls back
 * to `el.value = …` if `setRangeText` isn't available (very old
 * browsers; should never hit in a current iOS WKWebView).
 */
export function spliceText(
  el: HTMLTextAreaElement,
  start: number,
  end: number,
  insert: string,
): void {
  if (typeof el.setRangeText === "function") {
    el.setRangeText(insert, start, end, "end");
    return;
  }
  el.value = el.value.slice(0, start) + insert + el.value.slice(end);
}

/**
 * Place the caret at `pos`, then re-place it inside a microtask so
 * we win against any Solid `value={draft()}` re-binding that runs
 * after the caller's `setDraft` call. Also re-asserts focus on
 * iOS, where `setSelectionRange` on a blurred element silently
 * does nothing.
 */
export function parkCaret(el: HTMLTextAreaElement, pos: number): void {
  try {
    el.setSelectionRange(pos, pos);
  } catch {
    // ignore — happens if the textarea is momentarily blurred
  }
  queueMicrotask(() => {
    try {
      el.setSelectionRange(pos, pos);
    } catch {
      // ignore — element may be blurred or unmounted by now
    }
    el.focus({ preventScroll: true });
  });
}

/**
 * True when the caret sits on the first line of `value` — i.e. there
 * is no newline before it. Drives ArrowUp cell navigation: we only
 * jump to the previous block when pressing Up wouldn't just move the
 * caret up one line inside a multi-line block.
 *
 * This is a *logical*-line test (counts `\n`), not a visual one: a
 * soft-wrapped long line reads as a single line here. Outline blocks
 * are short enough that the simplification is invisible in practice;
 * swap for caret-rect measurement if that ever stops holding.
 */
export function caretOnFirstLine(value: string, caret: number): boolean {
  return !value.slice(0, Math.max(0, caret)).includes("\n");
}

/**
 * True when the caret sits on the last line of `value` — no newline
 * after it. Mirror of `caretOnFirstLine` for ArrowDown navigation.
 */
export function caretOnLastLine(value: string, caret: number): boolean {
  return !value.slice(Math.max(0, caret)).includes("\n");
}
