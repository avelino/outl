import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
} from "solid-js";
import type { PageMeta } from "@outl/shared/api/types";
import { listPages } from "@outl/shared/api/commands";

interface PageSwitcherProps {
  open: boolean;
  currentSlug: string | null;
  onClose: () => void;
  onPick: (slug: string, kind: "page" | "journal") => void;
}

/**
 * Bottom-sheet style page switcher. Lists every page in the
 * workspace with fuzzy filtering. Tap to open, `Enter` to open the
 * first match, swipe down on the handle to dismiss.
 */
export function PageSwitcher(props: PageSwitcherProps) {
  const [pages, { refetch }] = createResource(() =>
    props.open ? listPages() : Promise.resolve<PageMeta[]>([]),
  );
  const [query, setQuery] = createSignal("");
  // Sheet drag-to-dismiss state. Tracks vertical translate while the
  // user drags the grab handle (or the header above it); releases
  // below a threshold call `onClose`, anything else snaps back.
  const [dragY, setDragY] = createSignal(0);

  const filtered = createMemo(() => {
    const all = pages() ?? [];
    const q = query().trim().toLowerCase();
    if (!q) return all;
    return all.filter(
      (p) =>
        p.slug.toLowerCase().includes(q) ||
        p.title.toLowerCase().includes(q),
    );
  });

  function pickFirst() {
    const first = filtered()[0];
    if (!first) return;
    props.onPick(first.slug, first.kind);
  }

  let dragStartY = 0;
  let dragActive = false;
  function onHandleDown(e: PointerEvent) {
    dragStartY = e.clientY;
    dragActive = true;
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }
  function onHandleMove(e: PointerEvent) {
    if (!dragActive) return;
    const dy = e.clientY - dragStartY;
    // Only react to downward drag — upward drag is meaningless for
    // a sheet pinned to the bottom.
    setDragY(Math.max(0, dy));
  }
  function onHandleUp() {
    if (!dragActive) return;
    dragActive = false;
    // 80px of finger travel ≈ user committed to dismiss. Below that
    // we snap back to zero so the gesture is forgiving.
    if (dragY() > 80) {
      setDragY(0);
      props.onClose();
    } else {
      setDragY(0);
    }
  }

  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 bg-black/40 backdrop-blur-md outl-fade-in"
        onClick={props.onClose}
      />
      <div
        class="fixed inset-x-0 bottom-0 z-50 flex max-h-[80vh] flex-col overflow-hidden rounded-t-2xl bg-(--color-ios-bg)/85 shadow-2xl outl-sheet-up backdrop-blur-2xl backdrop-saturate-150 dark:bg-(--color-iosd-bg)/85"
        style={{
          "padding-bottom": "env(safe-area-inset-bottom)",
          transform: `translateY(${dragY()}px)`,
          transition: dragActive ? "none" : "transform 200ms cubic-bezier(0.32, 0.72, 0, 1)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-center gap-3 border-b border-(--color-ios-divider)/30 px-4 py-3 dark:border-(--color-iosd-divider)/30">
          {/* Drag-to-dismiss handle. We attach the drag handlers
              and `touch-action: none` ONLY here, not to the whole
              header — otherwise the user can't scroll the list
              from anywhere near the top of the sheet because the
              header steals the pointer. */}
          <span
            class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
            style={{ "touch-action": "none" }}
            onPointerDown={onHandleDown}
            onPointerMove={onHandleMove}
            onPointerUp={onHandleUp}
            onPointerCancel={onHandleUp}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="block h-1 w-10 mx-auto rounded-full bg-(--color-ios-divider) dark:bg-(--color-iosd-divider)"
            />
          </span>
        </header>
        <div class="px-4 py-2">
          <div class="flex items-center gap-2 rounded-xl bg-(--color-ios-card) px-3 py-2 dark:bg-(--color-iosd-card)">
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)"
              aria-hidden="true"
            >
              <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
            </svg>
            <input
              type="text"
              autofocus
              placeholder="Search pages…"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                // Enter opens the first match — same convention as
                // Spotlight / Raycast / Alfred. Esc dismisses.
                if (e.key === "Enter") {
                  e.preventDefault();
                  pickFirst();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  props.onClose();
                }
              }}
              class="w-full bg-transparent text-[16px] outline-none"
            />
            <Show when={query().length > 0}>
              <button
                type="button"
                aria-label="Clear"
                onClick={() => setQuery("")}
                class="text-(--color-ios-text-secondary) active:opacity-60 dark:text-(--color-iosd-text-secondary)"
              >
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm5 13.59L15.59 17 12 13.41 8.41 17 7 15.59 10.59 12 7 8.41 8.41 7 12 10.59 15.59 7 17 8.41 13.41 12z" />
                </svg>
              </button>
            </Show>
          </div>
        </div>
        <div class="ios-scroll flex-1 px-2 pb-4">
          <Show
            when={filtered().length > 0}
            fallback={
              <p class="px-4 py-8 text-center text-[14px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                No pages match "{query()}".
              </p>
            }
          >
            <For each={filtered()}>
              {(p) => (
                <button
                  type="button"
                  onClick={() => props.onPick(p.slug, p.kind)}
                  class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left active:bg-(--color-ios-card)/60 dark:active:bg-(--color-iosd-card)/60"
                  classList={{
                    "bg-(--color-ios-card)/40 dark:bg-(--color-iosd-card)/40":
                      p.slug === props.currentSlug,
                  }}
                >
                  <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-(--color-ios-accent)/12 text-(--color-ios-accent) dark:bg-(--color-iosd-accent)/20 dark:text-(--color-iosd-accent)">
                    <Show
                      when={p.kind === "journal"}
                      fallback={
                        <svg
                          width="16"
                          height="16"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          aria-hidden="true"
                        >
                          <path d="M4 4h12a4 4 0 0 1 4 4v12H8a4 4 0 0 1-4-4V4z" />
                          <path d="M8 8h8M8 12h8M8 16h5" />
                        </svg>
                      }
                    >
                      <svg
                        width="16"
                        height="16"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <rect x="3" y="4" width="18" height="18" rx="3" />
                        <path d="M3 10h18M8 2v4m8-4v4" />
                      </svg>
                    </Show>
                  </span>
                  <span class="flex min-w-0 flex-col">
                    <span class="truncate text-[15px] font-medium">
                      {p.title}
                    </span>
                    <span class="truncate text-[12px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                      {p.slug}
                    </span>
                  </span>
                </button>
              )}
            </For>
          </Show>
        </div>
      </div>
      <RefreshHook trigger={() => props.open} refetch={refetch} />
    </Show>
  );
}

// Trick to refetch every time `open` flips true without using effects
// outside the component body.
function RefreshHook(props: { trigger: () => boolean; refetch: () => void }) {
  createMemo(() => {
    if (props.trigger()) props.refetch();
  });
  return null;
}
