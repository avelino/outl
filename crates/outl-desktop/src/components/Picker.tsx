import {
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";

import { openRef, searchPages } from "@outl/shared/api/commands";
import type { PageMeta, PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";

/**
 * `Cmd/Ctrl+P` quick switcher.
 *
 * A centered modal with a single text input and a fuzzy-ranked
 * results list. The backend already implements the ranking
 * (`search_pages` from `outl-mobile`'s original surface, now shared
 * via `@outl/shared/api/commands`) — three tiers: exact / prefix /
 * substring against `title` and `slug`.
 *
 * Enter → opens the highlighted result through `open_ref` (same
 * decision tree the inline `[[ref]]` click handler uses, so typing
 * a non-existent name creates the page in one round-trip).
 */
export function Picker(props: { onPicked: (view: PageView) => void }) {
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<PageMeta[]>([]);
  const [highlight, setHighlight] = createSignal(0);

  let inputRef: HTMLInputElement | undefined;

  function close() {
    setAppState("pickerOpen", false);
    setAppState("pickerSeed", null);
    setQuery("");
    setHighlight(0);
  }

  async function refresh(q: string) {
    try {
      const list = await searchPages(q);
      setResults(list);
      setHighlight(0);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  // Refresh results whenever the query changes — including the
  // first mount with empty query (top 25 pages).
  createEffect(() => {
    void refresh(query());
  });

  async function pick(target: string) {
    try {
      const view = await openRef(target);
      props.onPicked(view);
      close();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(h + 1, Math.max(results().length - 1, 0)));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const r = results();
      if (r.length > 0) void pick(r[highlight()].slug);
      else if (query().trim()) void pick(query().trim()); // create new
      return;
    }
  }

  /*
   * Focus the search input every time the picker is opened — not
   * only on the component's initial mount. The previous version
   * ran `inputRef?.focus()` inside `onMount`, which fires when the
   * Picker component is added to the tree (boot time, when
   * `pickerOpen === false` and `inputRef` is undefined because
   * `<Show>` hasn't rendered the input yet). Result: user pressed
   * Cmd+P, the input appeared but received no focus — they had to
   * click before typing. UX 101.
   *
   * `createEffect` re-runs whenever `pickerOpen` flips; the
   * microtask gives Solid a tick to commit the `<Show>` mount so
   * `inputRef` is populated by the time we call focus().
   */
  createEffect(() => {
    if (appState.pickerOpen) {
      // Pre-fill from `pickerSeed` if a handler stashed one (e.g. `*`
      // / `#` in vim Normal mode). Cleared on close so the next manual
      // `Cmd+P` opens blank.
      const seed = appState.pickerSeed;
      if (seed && query() === "") {
        setQuery(seed);
      }
      queueMicrotask(() => {
        inputRef?.focus();
        // Select all so the user can either accept the seed or type
        // a fresh query without an extra Backspace.
        inputRef?.select();
      });
    }
  });

  onMount(() => {
    // Click outside closes — handled by overlay div's onClick.
    const escListener = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", escListener, { capture: false });
    onCleanup(() => window.removeEventListener("keydown", escListener));
  });

  return (
    <Show when={appState.pickerOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        <div class="mt-24 w-[520px] max-w-[90vw] overflow-hidden rounded-lg border border-(--color-outl-fg)/15 bg-(--color-outl-bg-elev)/95 shadow-2xl">
          <input
            ref={inputRef}
            type="text"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={handleKey}
            placeholder="Find or create a page…"
            class="w-full border-b border-(--color-outl-fg)/10 bg-transparent px-4 py-3 outline-none"
          />
          <div class="max-h-[60vh] overflow-y-auto">
            <For each={results()}>
              {(p, idx) => (
                <button
                  type="button"
                  onClick={() => void pick(p.slug)}
                  class={`block w-full px-4 py-2 text-left text-sm ${
                    idx() === highlight()
                      ? "bg-(--color-outl-fg)/10"
                      : "hover:bg-(--color-outl-fg)/5"
                  }`}
                >
                  <div>
                    {p.icon
                      ? `${p.icon} `
                      : p.kind === "journal"
                        ? "📅 "
                        : "📄 "}
                    {p.title}
                  </div>
                  <div class="font-mono text-[10px] opacity-50">{p.slug}</div>
                </button>
              )}
            </For>
            <Show when={results().length === 0 && query().trim().length > 0}>
              <button
                type="button"
                onClick={() => void pick(query().trim())}
                class="block w-full px-4 py-2 text-left text-sm hover:bg-(--color-outl-fg)/5"
              >
                <span class="opacity-60">Create new page </span>
                <span class="font-medium">"{query().trim()}"</span>
              </button>
            </Show>
          </div>
          <div class="border-t border-(--color-outl-fg)/10 px-4 py-1.5 text-[10px] opacity-50">
            ↑/↓ navigate · ⏎ open · Esc close
          </div>
        </div>
      </div>
    </Show>
  );
}
