import { For, JSX, Show } from "solid-js";
import type { Backlink, TodoState } from "@outl/shared/api/types";
import { MarkdownInline } from "@outl/shared/markdown";

interface BacklinksSectionProps {
  backlinks: Backlink[];
  onJump: (link: Backlink) => void;
}

/**
 * Backlinks panel rendered below the outline. Each entry shows the
 * source block's text plus the page it lives on; tapping it jumps to
 * the source page.
 *
 * Renders even when the list is empty so newcomers discover the
 * bidirectional-linking feature exists. Without the empty state,
 * a freshly-created page looks identical to a page that has no
 * graph at all — and the user has no idea pages CAN cite each
 * other until they happen to land on one that already has refs.
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
      <Show
        when={props.backlinks.length > 0}
        fallback={
          <div class="overflow-hidden rounded-2xl bg-(--color-ios-card) px-4 py-5 text-center dark:bg-(--color-iosd-card)">
            <p class="text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              No backlinks yet.
            </p>
            <p class="mt-1 text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
              Pages that link here with{" "}
              <code class="font-mono text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                [[this page]]
              </code>{" "}
              will appear in this section.
            </p>
          </div>
        }
      >
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
                  <div class="mt-1 flex items-start gap-2">
                    <Show when={link.todo !== null}>
                      <BacklinkCheckbox todo={link.todo} />
                    </Show>
                    <p
                      class="flex-1 text-[15px] leading-snug"
                      classList={{
                        "text-(--color-ios-text-tertiary) line-through dark:text-(--color-iosd-text-tertiary)":
                          link.todo === "DONE",
                      }}
                    >
                      <MarkdownInline tokens={link.source_block.tokens} />
                    </p>
                  </div>
                </div>
              </button>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}

/**
 * Read-only checkbox marker for a backlink whose source block carries
 * a TODO/DONE state. Mirrors the visual treatment used by
 * `BulletOrCheckbox` in `BlockRow.tsx`, but is presentational only —
 * to toggle the task the user opens the source page.
 */
function BacklinkCheckbox(props: { todo: TodoState | null }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      class="mt-[3px] flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-[1.5px]"
      classList={{
        "border-(--color-ios-accent) bg-(--color-ios-accent) dark:border-(--color-iosd-accent) dark:bg-(--color-iosd-accent)":
          props.todo === "DONE",
        "border-(--color-ios-text-secondary) bg-transparent dark:border-(--color-iosd-text-secondary)":
          props.todo !== "DONE",
      }}
    >
      <Show when={props.todo === "DONE"}>
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="white"
          stroke-width="3.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M5 12l4 4 10-10" />
        </svg>
      </Show>
    </span>
  );
}
