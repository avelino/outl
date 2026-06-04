import { For, JSX } from "solid-js";
import type { InlineToken } from "./api";

/**
 * Render a block's pre-tokenized inline markdown.
 *
 * The Rust backend (`outl_md::tokenize_owned`) tokenizes every block
 * before it leaves the workspace, so the renderer receives the
 * tokens directly. No parallel TS tokenizer lives here — adding a
 * new variant in `outl_md::InlineTok` plus its `InlineToken`
 * counterpart and a render case below is the whole flow.
 *
 * Variants currently supported:
 *
 * - `**bold**`, `*italic*` / `_italic_`, `~~strike~~`, `` `code` ``
 * - `[label](url)` links (non-clickable for now, would need the
 *   Tauri opener plugin)
 * - `[[page name]]` wiki refs (rendered as accented pill)
 * - `#tag` (rendered as small chip)
 * - `((blk-XXXXXX))` block refs and `!((blk-XXXXXX))` embeds
 */
interface MarkdownInlineProps {
  /** Tokens produced by `outl_md::tokenize_owned` on the backend. */
  tokens: InlineToken[];
  /** Invoked when the user taps a `[[ref]]`. The argument is the
   * target name (slug or page title) inside the brackets. Skip the
   * default tap handling (e.g. when this lives inside a textarea). */
  onRefClick?: (target: string) => void;
  /** Invoked when the user taps a `#tag`. */
  onTagClick?: (tag: string) => void;
}

export function MarkdownInline(props: MarkdownInlineProps): JSX.Element {
  return (
    <For each={props.tokens}>
      {(tok) => {
        switch (tok.kind) {
          case "plain":
            return <span>{tok.value}</span>;
          case "bold":
            return <span class="font-semibold">{tok.value}</span>;
          case "italic":
            return <span class="italic">{tok.value}</span>;
          case "strike":
            return <span class="line-through opacity-70">{tok.value}</span>;
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
