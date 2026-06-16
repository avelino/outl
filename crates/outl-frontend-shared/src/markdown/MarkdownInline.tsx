import { For, JSX, Show } from "solid-js";

import type { InlineToken } from "../api/types";

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
 * - `[label](url)` links — clickable when the host passes
 *   `onLinkClick` (desktop wires it to the Tauri opener so the URL
 *   opens in the system browser); inert text otherwise.
 * - `[[page name]]` wiki refs and `#tag` (visual style controlled
 *   by `variant`)
 * - `((blk-XXXXXX))` block refs and `!((blk-XXXXXX))` embeds
 *
 * ## `variant`
 *
 * - **`"pill"`** (default) — touch-friendly chips with filled
 *   backgrounds. Right for mobile where fingers need a fat target.
 *   References `--color-ios-accent` so mobile's existing palette
 *   keeps working.
 * - **`"inline"`** — TUI-style inline text with underline + color
 *   only, no chip background. Right for desktop where dense
 *   reading is the norm; pills there feel heavy. Uses the
 *   canonical `--color-outl-ref-link-fg` (and `--color-outl-tag-link-fg`)
 *   tokens for separate hues on refs vs tags.
 *
 * The host client supplies the matching CSS custom properties; the
 * renderer is otherwise unaware of the active theme.
 */
type Variant = "pill" | "inline";

interface MarkdownInlineProps {
  /** Tokens produced by `outl_md::tokenize_owned` on the backend. */
  tokens: InlineToken[];
  /** Render style. Defaults to `"pill"` (mobile-style chips).
   *  Desktop passes `"inline"` for TUI-style underlined text. */
  variant?: Variant;
  /** Invoked when the user taps a `[[ref]]`. The argument is the
   * target name (slug or page title) inside the brackets. Skip the
   * default tap handling (e.g. when this lives inside a textarea). */
  onRefClick?: (target: string) => void;
  /** Invoked when the user taps a `#tag`. */
  onTagClick?: (tag: string) => void;
  /** Invoked when the user taps an external `[label](url)` link. The
   * argument is the raw `href`. When omitted the link renders as inert
   * text (mobile / backlink contexts that don't open URLs yet). */
  onLinkClick?: (href: string) => void;
}

export function MarkdownInline(props: MarkdownInlineProps): JSX.Element {
  const variant = (): Variant => props.variant ?? "pill";

  return (
    <For each={props.tokens}>
      {(tok) => {
        switch (tok.kind) {
          case "plain":
            return <span>{tok.value}</span>;
          // Bold / italic / strike recurse: their `inner` is a
          // fully-tokenized list, so nested refs/tags/block-refs
          // re-enter the renderer and pick up their own styling
          // under the wrapping element. Without this recursion the
          // pattern `**[[avelino]]**` rendered as literal text.
          case "bold":
            return (
              <span class="font-semibold">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "italic":
            return (
              <span class="italic">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "strike":
            return (
              <span class="line-through opacity-70">
                <MarkdownInline
                  tokens={tok.inner}
                  variant={props.variant}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                  onLinkClick={props.onLinkClick}
                />
              </span>
            );
          case "code":
            return (
              <code class="rounded bg-(--color-ios-divider)/30 px-1 py-0.5 font-mono text-[14px] dark:bg-(--color-iosd-divider)/30">
                {tok.value}
              </code>
            );
          case "link": {
            const fire = () => props.onLinkClick?.(tok.href);
            const onClick = (e: MouseEvent) => {
              if (!props.onLinkClick) return;
              e.stopPropagation();
              fire();
            };
            const onKeyDown = (e: KeyboardEvent) => {
              if (!props.onLinkClick) return;
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                fire();
              }
            };
            // Only expose the button affordances (role / tabindex /
            // handlers / pointer cursor) when a handler is actually
            // wired. An inert link is a plain `<span>`, not a fake
            // button — otherwise screen readers announce a control that
            // does nothing and keyboard users land on a dead tab stop.
            const a11y: JSX.HTMLAttributes<HTMLSpanElement> = props.onLinkClick
              ? { role: "button", tabindex: 0, onClick, onKeyDown }
              : {};
            return (
              <Show
                when={variant() === "inline"}
                fallback={
                  <span
                    {...a11y}
                    title={tok.href}
                    class="text-(--color-ios-accent) underline active:opacity-60 dark:text-(--color-iosd-accent)"
                    classList={{ "cursor-pointer": !!props.onLinkClick }}
                  >
                    {tok.value}
                  </span>
                }
              >
                <span
                  {...a11y}
                  title={tok.href}
                  class="text-(--color-outl-md-link-fg) underline decoration-(--color-outl-md-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-md-link-fg)"
                  classList={{ "cursor-pointer": !!props.onLinkClick }}
                >
                  {tok.value}
                </span>
              </Show>
            );
          }
          case "ref":
            return (
              <Show
                when={variant() === "inline"}
                fallback={
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
                }
              >
                <span
                  role="button"
                  onClick={(e) => {
                    if (!props.onRefClick) return;
                    e.stopPropagation();
                    props.onRefClick(tok.value);
                  }}
                  class="cursor-pointer text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-ref-link-fg)"
                >
                  {tok.value}
                </span>
              </Show>
            );
          case "tag":
            return (
              <Show
                when={variant() === "inline"}
                fallback={
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
                }
              >
                <span
                  role="button"
                  onClick={(e) => {
                    if (!props.onTagClick) return;
                    e.stopPropagation();
                    props.onTagClick(tok.value);
                  }}
                  class="cursor-pointer text-(--color-outl-tag-link-fg) underline decoration-(--color-outl-tag-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-tag-link-fg)"
                >
                  {/* The Rust token serialiser stores the leading `#` in
                      `value` already (see `outl_md::inline::InlineToken::from_borrowed`
                      and the `round_trips_every_variant_into_serializable_form`
                      contract test that asserts `tag == "#tag"`). Adding
                      another `#` here paints it twice (`##avelino`) —
                      bug seen on desktop pretty render. */}
                  {tok.value}
                </span>
              </Show>
            );
          case "blockref":
            return (
              <span class="rounded bg-(--color-ios-divider)/30 px-1 font-mono text-[13px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
                {tok.value}
              </span>
            );
          case "embed":
            return (
              <span class="rounded bg-(--color-ios-accent)/12 px-1 font-mono text-[13px] text-(--color-ios-accent) dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)">
                !{tok.value}
              </span>
            );
          case "emoji":
            // The backend's catalog gate guarantees `glyph` is set on
            // every Emoji token. Defensive fall-back: if a peer ever
            // ships an unknown shortcode (binary too old to know it),
            // render the literal `:shortcode:` form so the user sees
            // *something* instead of a blank span.
            return (
              <span
                title={`:${tok.shortcode}:`}
                aria-label={tok.shortcode}
                class="mx-0.5"
              >
                {tok.glyph || `:${tok.shortcode}:`}
              </span>
            );
        }
      }}
    </For>
  );
}
