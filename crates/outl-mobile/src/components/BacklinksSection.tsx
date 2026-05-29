import { For, JSX } from "solid-js";
import { Backlink } from "../lib/api";
import { MarkdownInline } from "../lib/markdown";

interface BacklinksSectionProps {
  backlinks: Backlink[];
  onJump: (link: Backlink) => void;
}

/**
 * Backlinks panel rendered below the outline. Each entry shows the
 * source block's text plus the page it lives on; tapping it jumps to
 * the source page.
 */
export function BacklinksSection(props: BacklinksSectionProps): JSX.Element {
  return (
    <section class="mx-3 mt-6">
      <header class="mb-2 flex items-center gap-2 px-2 text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
          <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
        </svg>
        <p class="text-[12px] font-medium uppercase tracking-wider">
          Linked from {props.backlinks.length}
        </p>
      </header>
      <div class="overflow-hidden rounded-2xl bg-(--color-ios-card) dark:bg-(--color-iosd-card)">
        <For each={props.backlinks}>
          {(link, idx) => (
            <button
              type="button"
              onClick={() => props.onJump(link)}
              class="block w-full text-left active:opacity-60"
              classList={{
                "border-t border-(--color-ios-divider)/40 dark:border-(--color-iosd-divider)/40":
                  idx() > 0,
              }}
            >
              <div class="px-4 py-3">
                <p class="text-[13px] font-medium text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                  {link.source_page?.title ?? "untitled"}
                </p>
                <p class="mt-1 text-[15px] leading-snug">
                  <MarkdownInline text={link.block_text} />
                </p>
              </div>
            </button>
          )}
        </For>
      </div>
    </section>
  );
}
