import { For, JSX } from "solid-js";

/**
 * Render a block's text with the minimum set of inline markdown
 * conventions outl supports today:
 *
 * - `**bold**`
 * - `*italic*`
 * - `` `code` ``
 * - `[label](url)` links — non-clickable for now (we'd need to wire
 *   the Tauri opener plugin)
 * - `[[page name]]` wiki refs — rendered as accented pill
 * - `#tag` — rendered as small chip
 *
 * Reuses the same lexicon `outl-md` uses so future migration to the
 * shared tokenizer is mechanical.
 */
interface MarkdownInlineProps {
  text: string;
  /** Invoked when the user taps a `[[ref]]`. The argument is the
   * target name (slug or page title) inside the brackets. Skip the
   * default tap handling (e.g. when this lives inside a textarea). */
  onRefClick?: (target: string) => void;
  /** Invoked when the user taps a `#tag`. */
  onTagClick?: (tag: string) => void;
}

export function MarkdownInline(props: MarkdownInlineProps): JSX.Element {
  const tokens = tokenize(props.text);
  return (
    <For each={tokens}>
      {(tok) => {
        switch (tok.kind) {
          case "plain":
            return <span>{tok.value}</span>;
          case "bold":
            return <span class="font-semibold">{tok.value}</span>;
          case "italic":
            return <span class="italic">{tok.value}</span>;
          case "code":
            return (
              <code class="rounded bg-(--color-ios-divider)/30 px-1 py-0.5 font-mono text-[15px] dark:bg-(--color-iosd-divider)/30">
                {tok.value}
              </code>
            );
          case "link":
            return (
              <span class="text-(--color-ios-accent) underline dark:text-(--color-iosd-accent)">
                {tok.value}
              </span>
            );
          case "ref":
            return (
              <span
                role="button"
                onClick={(e) => {
                  if (!props.onRefClick) return;
                  e.stopPropagation();
                  props.onRefClick(tok.value);
                }}
                class="rounded-md bg-(--color-ios-accent)/12 px-1.5 py-0.5 text-[15px] font-medium text-(--color-ios-accent) active:opacity-60 dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)"
              >
                {tok.value}
              </span>
            );
          case "tag":
            return (
              <span
                role="button"
                onClick={(e) => {
                  if (!props.onTagClick) return;
                  e.stopPropagation();
                  props.onTagClick(tok.value);
                }}
                class="text-(--color-ios-accent) active:opacity-60 dark:text-(--color-iosd-accent)"
              >
                {tok.value}
              </span>
            );
          case "blockref":
            return (
              <span class="rounded bg-(--color-ios-divider)/30 px-1 font-mono text-[14px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
                {tok.value}
              </span>
            );
          case "embed":
            return (
              <span class="rounded bg-(--color-ios-accent)/12 px-1 font-mono text-[14px] text-(--color-ios-accent) dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)">
                !{tok.value}
              </span>
            );
        }
      }}
    </For>
  );
}

export type Token =
  | { kind: "plain"; value: string }
  | { kind: "bold"; value: string }
  | { kind: "italic"; value: string }
  | { kind: "code"; value: string }
  | { kind: "link"; value: string; href: string }
  | { kind: "ref"; value: string }
  | { kind: "tag"; value: string }
  | { kind: "blockref"; value: string }
  | { kind: "embed"; value: string };

const PATTERNS: Array<{
  regex: RegExp;
  build: (m: RegExpExecArray) => Token;
}> = [
  {
    regex: /\*\*([^*]+)\*\*/g,
    build: (m) => ({ kind: "bold", value: m[1] }),
  },
  {
    regex: /`([^`]+)`/g,
    build: (m) => ({ kind: "code", value: m[1] }),
  },
  {
    regex: /\*([^*\s][^*]*)\*/g,
    build: (m) => ({ kind: "italic", value: m[1] }),
  },
  {
    // Underscore italic. Require word boundary on both sides so we
    // don't capture stray underscores in identifiers.
    regex: /(?<![\w_])_([^_\s][^_]*)_(?![\w_])/g,
    build: (m) => ({ kind: "italic", value: m[1] }),
  },
  {
    // Block embed: `!((blk-XXXXXX))`. Matched before plain block ref
    // so the leading `!` isn't lost.
    regex: /!\(\((blk-[a-z0-9]+)\)\)/g,
    build: (m) => ({ kind: "embed", value: m[1] }),
  },
  {
    // Inline block reference: `((blk-XXXXXX))`.
    regex: /\(\((blk-[a-z0-9]+)\)\)/g,
    build: (m) => ({ kind: "blockref", value: m[1] }),
  },
  {
    regex: /\[\[([^\]]+)\]\]/g,
    build: (m) => ({ kind: "ref", value: m[1] }),
  },
  {
    regex: /(?<![\w])#([\p{L}\p{N}_-]+)/gu,
    build: (m) => ({ kind: "tag", value: `#${m[1]}` }),
  },
  {
    regex: /\[([^\]]+)\]\(([^)]+)\)/g,
    build: (m) => ({ kind: "link", value: m[1], href: m[2] }),
  },
];

/**
 * Walk the string left-to-right, picking the earliest pattern match
 * at each position. Anything outside a match becomes a plain token.
 *
 * Exported for unit tests; not part of the public component surface.
 */
export function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    let bestStart = text.length;
    let bestEnd = text.length;
    let bestToken: Token | null = null;

    for (const { regex, build } of PATTERNS) {
      regex.lastIndex = cursor;
      const m = regex.exec(text);
      if (!m) continue;
      if (m.index < bestStart) {
        bestStart = m.index;
        bestEnd = m.index + m[0].length;
        bestToken = build(m);
      }
    }

    if (!bestToken) {
      // No more matches anywhere: everything from `cursor` to the
      // end is plain text. Emit it once and stop.
      if (cursor < text.length) {
        tokens.push({ kind: "plain", value: text.slice(cursor) });
      }
      return tokens;
    }

    if (bestStart > cursor) {
      tokens.push({ kind: "plain", value: text.slice(cursor, bestStart) });
    }
    tokens.push(bestToken);
    cursor = bestEnd;
  }

  return tokens;
}
