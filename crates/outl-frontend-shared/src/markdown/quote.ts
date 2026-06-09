/**
 * Blockquote prefix helper — TS mirror of `outl_actions::quote`.
 *
 * Same wire-format policy as TODO/DONE: the marker lives **in the
 * block's text** as a `"> "` prefix (one space after `>`, matching
 * CommonMark) instead of a new field on the AST. Every client decides
 * how to render it.
 *
 * The Rust side owns the canonical implementation
 * (`crates/outl-actions/src/quote.rs`). This file exists so a client
 * that only has `OutlineNode.text` (mobile's `<BlockRow />`, desktop's
 * row chrome) doesn't need a Tauri round-trip to know if a row should
 * paint with a left border + muted body.
 */

/** Wire prefix for a quoted block — keep in sync with `QUOTE_PREFIX` in Rust. */
export const QUOTE_PREFIX = "> ";

/**
 * Split a block's raw text into `(quoted, body)`.
 *
 * When the prefix is present, `quoted` is `true` and `body` is
 * everything after `"> "` (no leading space). When absent, `quoted`
 * is `false` and `body` is the untouched input.
 *
 * The Rust mirror returns the same shape via `outl_actions::quote::split_quote`.
 */
export function splitQuote(text: string): { quoted: boolean; body: string } {
  if (text.startsWith(QUOTE_PREFIX)) {
    return { quoted: true, body: text.slice(QUOTE_PREFIX.length) };
  }
  return { quoted: false, body: text };
}

/** Does the block's raw text carry the quote marker? */
export function isQuote(text: string): boolean {
  return text.startsWith(QUOTE_PREFIX);
}

import type { InlineToken } from "../api/types";

/**
 * Strip the leading `"> "` prefix from the **first** `Plain` token of
 * a tokenized block body.
 *
 * Why this exists: the backend tokenizes the *raw* block text, so a
 * quoted block emits a single `Plain { value: "> hello" }` token (the
 * quote wrapper is block-level, not inline — see
 * `outl_md::inline::tokenize`). The mobile and desktop frontends
 * render the quote chrome themselves (left border + muted body) and
 * shouldn't double-paint the marker inside the rendered text. This
 * helper produces the token list to feed `<MarkdownInline />` when
 * the chrome already shows "this block is a quote".
 *
 * A no-op when the first token isn't a `Plain` starting with `"> "`
 * — keeps the helper safe to call unconditionally on every row.
 */
export function stripQuoteFromTokens(tokens: InlineToken[]): InlineToken[] {
  if (tokens.length === 0) return tokens;
  const first = tokens[0];
  if (first.kind === "plain" && first.value.startsWith(QUOTE_PREFIX)) {
    return [
      { ...first, value: first.value.slice(QUOTE_PREFIX.length) },
      ...tokens.slice(1),
    ];
  }
  return tokens;
}
