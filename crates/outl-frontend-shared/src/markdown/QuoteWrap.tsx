import type { JSX } from "solid-js";

import { splitQuote } from "./quote";

/**
 * Wraps a `bullet + body` pair with the blockquote chrome when the
 * block is quoted, or with a plain flex container otherwise.
 *
 * Why this lives in `@outl/shared`: mobile and desktop both render
 * `BlockRow` rows where the chrome must sit **outside** the bullet
 * (so the left border lands as `│ ☐ body`, matching the TUI). The
 * *structure* is identical between the two clients; only the theme
 * tokens differ (iOS vs desktop palette). Pulling the conditional
 * wrapper here eliminates the if/else duplication while keeping
 * Tailwind classes as literals in the caller — required for the JIT
 * to discover them at build time.
 *
 * Contract:
 *
 * - `baseClass` is applied **at all times** (flex layout, gap).
 *   When `quoted` is false, this is the *only* class — non-quoted
 *   rows render byte-identical to a plain block.
 * - `chromeClass` is applied **only when quoted**. Each client
 *   passes its own theme tokens (border colour, tint, rounding)
 *   here.
 *
 * @see `outl_actions::quote::split_quote` — the Rust mirror of the
 *      `> ` marker detection used to drive `quoted`.
 */
export interface QuoteWrapProps {
  /**
   * Whether the wrapped row should paint the blockquote chrome.
   * Callers usually compute this with `splitQuote(block.text).quoted`,
   * but they can also pass a derived signal (e.g. when the block has
   * already been split for token rendering and the caller wants to
   * avoid a second `splitQuote` call).
   */
  quoted: boolean;
  /**
   * Tailwind classes applied to the wrapper at all times. Must keep
   * the row's flex layout (`flex min-w-0 flex-1 items-start ...`)
   * so the bullet + body keep stacking correctly when the block is
   * not quoted.
   */
  baseClass: string;
  /**
   * Tailwind classes applied **only** when `quoted` is true. This is
   * where each client passes its theme tokens (`border-l-2`,
   * `border-(--color-…)/x`, `bg-(--color-…)/[0.0y]`, `rounded-r-md`,
   * `pl-2`). The classes must be string literals in the caller so
   * Tailwind's JIT can pick them up at build time.
   */
  chromeClass: string;
  children: JSX.Element;
}

export function QuoteWrap(props: QuoteWrapProps) {
  return (
    <div
      class={
        props.quoted
          ? `${props.baseClass} ${props.chromeClass}`
          : props.baseClass
      }
    >
      {props.children}
    </div>
  );
}

/**
 * Convenience: compute `quoted` straight from a block's raw text.
 * Saves callers from importing `splitQuote` twice in the same file.
 */
export function isBlockQuoted(text: string): boolean {
  return splitQuote(text).quoted;
}
