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

/** Single-character bracket pairs handled by {@link autoPairBracket}
 *  and {@link autoDeletePair}. Mirrors the TUI's Insert-mode set
 *  (`outl-tui/src/input/insert.rs`). */
const BRACKET_PAIRS: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
};

const BRACKET_CLOSERS = new Set(Object.values(BRACKET_PAIRS));

/**
 * Auto-pair a single bracket keystroke, mirroring the TUI's Insert
 * mode: typing `(`, `[` or `{` inserts the matching closer and
 * leaves the caret between the two; typing `)`, `]` or `}` when that
 * exact character already sits to the right of the caret steps over
 * it instead of doubling (the closer was almost certainly
 * auto-inserted, so the keystroke's intent is "move past it").
 *
 * `typed` is the character the user is about to insert — callers
 * intercept it in `beforeinput` (soft keyboards don't emit reliable
 * `keydown` characters) and apply the returned value themselves.
 * Returns `null` when `typed` isn't a bracket character; let the
 * default insertion through.
 *
 * Pair insertion is deliberately unconditional, like the TUI's
 * `insert_pair`: typing the second `[` next to an auto-inserted `]`
 * yields `[[|]]` directly, which is exactly the shape the `[[` ref
 * flow wants — and {@link autoClosePair} stays a no-op on it, so the
 * two layers never double a closer.
 */
export function autoPairBracket(
  value: string,
  selection: number,
  typed: string,
): PairCompletion | null {
  const close = BRACKET_PAIRS[typed];
  if (close) {
    return {
      value: value.slice(0, selection) + typed + close + value.slice(selection),
      caret: selection + 1,
    };
  }
  if (BRACKET_CLOSERS.has(typed) && value[selection] === typed) {
    return { value, caret: selection + 1 };
  }
  return null;
}

/**
 * Inverse of {@link autoClosePair} / {@link autoPairBracket}: when
 * the caret sits in the empty middle of a pair and the user presses
 * Backspace, delete the whole pair in one shot so they don't have to
 * mash backspace to undo an aborted ref or bracket. Returns `null`
 * when the caret isn't between an empty pair — let the default
 * backspace through.
 *
 * Doubled ref pairs (`[[]]`, `(())`) are checked first and collapse
 * all four characters; the single-character pairs (`()`, `[]`, `{}`)
 * collapse two. The moment the user has typed something in the
 * middle (`[[ave]]`, `(x)`), backspace goes back to deleting one
 * char at a time so they can fix typos.
 */
export function autoDeletePair(
  value: string,
  selection: number,
): PairCompletion | null {
  if (selection >= 2) {
    const left = value.slice(selection - 2, selection);
    const right = value.slice(selection, selection + 2);
    if ((left === "[[" && right === "]]") || (left === "((" && right === "))")) {
      return {
        value: value.slice(0, selection - 2) + value.slice(selection + 2),
        caret: selection - 2,
      };
    }
  }
  if (selection >= 1) {
    const close = BRACKET_PAIRS[value[selection - 1] ?? ""];
    if (close !== undefined && value[selection] === close) {
      return {
        value: value.slice(0, selection - 1) + value.slice(selection + 1),
        caret: selection - 1,
      };
    }
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
