/**
 * Inline auto-complete helpers for the block editor.
 *
 * Two related concerns live here:
 *
 * 1. **Pair completion**: when the user types the second `[` to form
 *    `[[`, or the second `(` to form `((`, we proactively insert the
 *    matching `]]` / `))` to the right of the caret and keep the
 *    caret in the middle. Same trick most editors use for quotes and
 *    brackets — saves the user typing the closing pair when their
 *    intent is unambiguous.
 *
 * 2. **Context detection**: while the caret sits *inside* an open
 *    `[[…]]` or `((…))` we can offer a suggestion list (pages for
 *    refs, blocks for block refs). [`detectRefContext`] reads the
 *    text around the caret and returns the kind + the current query
 *    if we're in that state.
 *
 * Both helpers are pure functions over the textarea string + caret
 * index, so they're trivial to unit-test and they make no assumption
 * about the DOM.
 */

/** Result of {@link autoClosePair}. */
export interface PairCompletion {
  /** Full textarea value after the completion. */
  value: string;
  /** Where to place the caret. */
  caret: number;
}

/**
 * Inspect `value` after the user has already typed the new
 * character that produced `selection`. If the two characters
 * immediately to the *left* of the caret are `[[` or `((`, append the
 * matching `]]` / `))` to the right of the caret so the user lands
 * inside an empty ref. Returns `null` when nothing should change.
 *
 * We deliberately don't try to handle the case where the user typed
 * the opening pair *next to* an existing closer (`[[]]`-already-
 * present-on-the-right) — pair completion only fires when the right
 * side starts with something other than the matching closer, so we
 * never end up with `]]]]`.
 */
export function autoClosePair(
  value: string,
  selection: number,
): PairCompletion | null {
  if (selection < 2) return null;
  const left = value.slice(selection - 2, selection);
  const right = value.slice(selection, selection + 2);
  if (left === "[[" && right !== "]]") {
    return {
      value: value.slice(0, selection) + "]]" + value.slice(selection),
      caret: selection,
    };
  }
  if (left === "((" && right !== "))") {
    return {
      value: value.slice(0, selection) + "))" + value.slice(selection),
      caret: selection,
    };
  }
  return null;
}

/**
 * Inverse of {@link autoClosePair}: when the caret sits in the empty
 * middle of `[[]]` or `(())` and the user presses Backspace, delete
 * the whole pair in one shot so they don't have to mash backspace
 * four times to undo an aborted ref. Returns `null` when the caret
 * isn't between an empty pair — let the default backspace through.
 *
 * Only fires for *empty* pairs (`[[]]`, `(())`). The moment the user
 * has typed something in the middle (`[[ave]]`), backspace goes
 * back to deleting one char at a time so they can fix typos.
 */
export function autoDeletePair(
  value: string,
  selection: number,
): PairCompletion | null {
  if (selection < 2) return null;
  const left = value.slice(selection - 2, selection);
  const right = value.slice(selection, selection + 2);
  if (left === "[[" && right === "]]") {
    return {
      value: value.slice(0, selection - 2) + value.slice(selection + 2),
      caret: selection - 2,
    };
  }
  if (left === "((" && right === "))") {
    return {
      value: value.slice(0, selection - 2) + value.slice(selection + 2),
      caret: selection - 2,
    };
  }
  return null;
}

/**
 * Insert `open + close` at `selection`, returning the new value and
 * the caret position *between* them. Used by toolbar buttons that
 * pre-seed a delimiter pair (`[[`, `((`).
 */
export function insertPair(
  value: string,
  selection: number,
  open: string,
  close: string,
): PairCompletion {
  return {
    value: value.slice(0, selection) + open + close + value.slice(selection),
    caret: selection + open.length,
  };
}

/**
 * Insert `snippet` at `selection`. Returns the new value and the
 * caret moved past the inserted text.
 */
export function insertText(
  value: string,
  selection: number,
  snippet: string,
): PairCompletion {
  return {
    value: value.slice(0, selection) + snippet + value.slice(selection),
    caret: selection + snippet.length,
  };
}

/** Result of {@link detectRefContext}. */
export interface RefContext {
  /** What kind of reference the caret is sitting inside. */
  kind: "page" | "block";
  /** The text between the opener and the caret — what to filter by. */
  query: string;
  /** Index of the first delimiter character (the leftmost `[` / `(`). */
  openIndex: number;
  /** Index of the character right after the closer, or the caret if
   *  no closer was found before the caret. Used to replace the whole
   *  ref when a suggestion is accepted. */
  replaceEnd: number;
}

/**
 * Detect whether the caret is inside an open `[[…]]` or `((…))`
 * reference. When it is, return the kind + the text between the
 * opener and the caret (so the caller can use it as a fuzzy query)
 * and the span the caller should replace when the user accepts a
 * suggestion.
 *
 * The rule is: walk left from the caret looking for `[[` or `((`
 * *without* hitting the matching closer first. If we find a `]]` or
 * `))` first, the caret isn't inside that ref. We also stop on
 * newlines so a stray `[[` on a previous line doesn't capture a
 * later caret.
 *
 * **Cross-runtime mirror.** The TUI runs the same walk-back in Rust
 * at `outl_tui::actions::overlay::detect_trigger`, plus `#` and `/`
 * triggers no GUI client surfaces yet. Frontend clients keep a local
 * TS copy (here in `@outl/shared`) so the autocomplete popup never
 * pays a Tauri round-trip per keystroke. If you extend either side
 * (a new opener, a different closing rule), update both in the same
 * commit.
 */
export function detectRefContext(
  value: string,
  selection: number,
): RefContext | null {
  let i = selection - 1;
  while (i >= 1) {
    const pair = value.slice(i - 1, i + 1);
    if (pair === "]]" || pair === "))") return null;
    if (value[i] === "\n") return null;
    if (pair === "[[") {
      const query = value.slice(i + 1, selection);
      const replaceEnd = findCloser(value, selection, "]]");
      return { kind: "page", query, openIndex: i - 1, replaceEnd };
    }
    if (pair === "((") {
      const query = value.slice(i + 1, selection);
      const replaceEnd = findCloser(value, selection, "))");
      return { kind: "block", query, openIndex: i - 1, replaceEnd };
    }
    i -= 1;
  }
  return null;
}

/**
 * Apply a chosen suggestion to a textarea string. Replaces the span
 * between `openIndex` (the `[[` or `((`) and `replaceEnd` (just past
 * the closer or at the caret) with `delim_open + replacement +
 * delim_close`. Returns the new value plus the caret position right
 * after the closer.
 */
export function applySuggestion(
  value: string,
  ctx: RefContext,
  replacement: string,
): PairCompletion {
  const [open, close] = ctx.kind === "page" ? ["[[", "]]"] : ["((", "))"];
  const before = value.slice(0, ctx.openIndex);
  const after = value.slice(ctx.replaceEnd);
  const rebuilt = `${open}${replacement}${close}`;
  return {
    value: before + rebuilt + after,
    caret: before.length + rebuilt.length,
  };
}

/**
 * Scan forward from `from` looking for `closer`. If the closer is
 * found before the end of the line, return the index *after* it.
 * Otherwise return `from` so callers replace only up to the caret.
 */
function findCloser(value: string, from: number, closer: string): number {
  for (let i = from; i < value.length - 1; i += 1) {
    if (value[i] === "\n") return from;
    if (value.slice(i, i + 2) === closer) return i + 2;
  }
  return from;
}
