import { For, Show, createEffect } from "solid-js";

import {
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  openRef,
  outdentBlock,
  pasteMarkdown,
  setBlockCollapsed,
  toggleTodo,
} from "@outl/shared/api/commands";

import type { BlockNode, PageView } from "@outl/shared/api/types";

import { runCodeBlock } from "../lib/api";
import { appState, setAppState } from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { InlineBacklinks } from "./InlineBacklinks";

/**
 * Center pane — title, breadcrumb, editable outline.
 *
 * Owns the editing state (which block has its textarea up) and
 * funnels every mutation through `outl-actions` via the shared
 * Tauri command wrappers. The optimistic refresh path is uniform:
 * every command returns a fresh `PageView` which we splat into the
 * store in one shot.
 */
export function OutlineView() {
  // `editingBlockId` and `selectedBlockId` live on the store so the
  // `outl-shortcuts` dispatcher can flip them from anywhere (Cmd+T,
  // `o`, `i`, `j/k`, …) without prop-drilling callbacks. Local
  // shorthand here keeps the JSX readable.
  const editingId = () => appState.editingBlockId;
  const setEditingId = (id: string | null) =>
    setAppState("editingBlockId", id);

  /**
   * Auto-select the first visible block whenever the *page itself*
   * changes (different journal, different page). We deliberately
   * **don't** depend on `appState.selectedBlockId` here — including
   * it would create a feedback loop where the j/k handlers'
   * selection updates would re-trigger this effect, which would
   * scan the outline and (under some Solid timing windows where the
   * outline `.children` arrays haven't been re-attached yet) flip
   * the selection back to `outline[0]`. That manifested as a "k
   * skips one line" bug.
   *
   * `appState.page?.id` is the right dependency: it changes once
   * per page navigation; on a navigation we reset to the first
   * block; in between, j/k own the cursor uncontested.
   */
  createEffect(() => {
    const pageId = appState.page?.id;
    if (!pageId) {
      setAppState("selectedBlockId", null);
      setAppState("selectedBacklinkBlockId", null);
      return;
    }
    // Navigating to a different page always lands the cursor on
    // the outline (never on a stale backlink from the previous page).
    setAppState("selectedBacklinkBlockId", null);
    const outline = appState.outline;
    if (outline.length === 0) {
      setAppState("selectedBlockId", null);
      return;
    }
    // Untracked read: don't re-run when selectedBlockId changes.
    // We use a peek via the store's underlying signal instead of a
    // reactive read — for Solid stores, indexing into `appState`
    // outside an effect doesn't track. Inside an effect we'd need
    // `untrack`, but we can just compare against null + the current
    // outline scan once per page change.
    const current = appState.selectedBlockId;
    if (current === null) {
      setAppState("selectedBlockId", outline[0].id);
      return;
    }
    // The page id changed and the previous cursor may not exist in
    // the new outline; verify and snap to the first block when not.
    const exists = (() => {
      const walk = (bs: typeof outline): boolean => {
        for (const b of bs) {
          if (b.id === current) return true;
          if (b.children.length > 0 && walk(b.children)) return true;
        }
        return false;
      };
      return walk(outline);
    })();
    if (!exists) {
      setAppState("selectedBlockId", outline[0].id);
    }
  });

  function applyView(view: PageView) {
    setAppState({
      page: view.page,
      outline: view.outline,
      backlinks: view.backlinks,
    });
  }

  async function handleError<T>(promise: Promise<T>): Promise<T | undefined> {
    try {
      return await promise;
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
      return undefined;
    }
  }

  async function handleRefClick(target: string) {
    const view = await handleError(openRef(target));
    if (view) applyView(view);
  }

  function handleTagClick(tag: string) {
    void handleRefClick(tag);
  }

  const cb: BlockCallbacks = {
    onStartEdit: (id) => {
      // Sync the selection cursor with whatever the user clicked so
      // `j/k` pick up from there instead of teleporting back to the
      // last vim-driven cursor.
      setAppState("selectedBlockId", id);
      setEditingId(id);
    },
    onCommit: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(editBlock(pageId, id, text));
      if (view) applyView(view);
      setEditingId(null);
    },
    onEnter: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      // Snapshot the existing block ids so we can identify the
      // freshly-created sibling in the refreshed view returned by
      // `createBlock`. Without this the new block stays read-only
      // and the user has to click it before typing — bad UX.
      const before = new Set<string>();
      const walkIds = (blocks: BlockNode[]) => {
        for (const b of blocks) {
          before.add(b.id);
          if (b.children.length > 0) walkIds(b.children);
        }
      };
      walkIds(appState.outline);

      await handleError(editBlock(pageId, id, text));
      const view = await handleError(
        createBlock(pageId, { afterId: id, parentId: null, text: "" }),
      );
      if (!view) return;
      applyView(view);

      // Find the new block (the one not in `before`) and put it in
      // edit mode so `<BlockRow />`'s `createEffect` focuses its
      // textarea automatically.
      let newId: string | null = null;
      const findNew = (blocks: BlockNode[]) => {
        for (const b of blocks) {
          if (newId) return;
          if (!before.has(b.id)) {
            newId = b.id;
            return;
          }
          if (b.children.length > 0) findNew(b.children);
        }
      };
      findNew(view.outline);
      setEditingId(newId);
    },
    onIndent: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(indentBlock(pageId, id));
      if (view) applyView(view);
    },
    onOutdent: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(outdentBlock(pageId, id));
      if (view) applyView(view);
    },
    onDeleteEmpty: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(deleteBlock(pageId, id));
      if (view) applyView(view);
      setEditingId(null);
    },
    onToggleTodo: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(toggleTodo(pageId, id));
      if (view) applyView(view);
    },
    onToggleCollapsed: async (id, collapsed) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(setBlockCollapsed(pageId, id, collapsed));
      if (view) applyView(view);
    },
    onPasteMarkdown: async (id, caret, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(pasteMarkdown(pageId, id, caret, text));
      if (view) applyView(view);
    },
    onRunCodeBlock: async (id) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const reply = await handleError(runCodeBlock(pageId, id));
      if (!reply) return;
      applyView(reply.view);
      if (reply.error) {
        setAppState("lastError", `${reply.language}: ${reply.error}`);
      }
    },
    onRefClick: handleRefClick,
    onTagClick: handleTagClick,
  };

  async function addFirstBlock() {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const view = await handleError(
      createBlock(pageId, { afterId: null, parentId: null, text: "" }),
    );
    if (view) applyView(view);
  }

  /**
   * Journal day-of-week ("Thursday") — used in the breadcrumb
   * above the ISO title. Returns empty for non-journals or
   * malformed slugs.
   */
  function journalWeekday(): string {
    const page = appState.page;
    if (!page || page.kind !== "journal") return "";
    // Slug is `YYYY-MM-DD`. Parse parts so JS doesn't apply UTC
    // (`new Date("2026-06-02")` is midnight UTC, which renders the
    // previous day in negative-offset timezones).
    const m = page.slug.match(/^(\d{4})-(\d{2})-(\d{2})$/);
    if (!m) return "";
    const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
    return d.toLocaleDateString(undefined, { weekday: "long" });
  }

  return (
    // `min-w-0 min-h-0` is what makes the inner `overflow-y-auto`
    // actually constrain to the viewport. Two defaults bite here:
    //
    //   * Grid items default to `min-width: auto`, which means
    //     "as wide as my content's natural width". A row with
    //     `BUSER-DJANGO-KX9` or `((blk-XXXXXX))` then pushes the
    //     whole main column wider than the window and the body's
    //     `overflow: hidden` clips it. `min-w-0` lifts that floor
    //     so the grid cell can shrink to the viewport and the
    //     inline tokens wrap inside their column.
    //   * Flex children default to `min-height: auto`, which makes
    //     the `flex-1 overflow-y-auto` block expand to fit instead
    //     of scrolling. `min-h-0` is the matching unlock.
    //
    // Classic Tailwind/flexbox/grid pitfall on both axes; the two
    // unlocks pair.
    <main class="flex h-full min-h-0 min-w-0 flex-col">
      <header class="border-b border-(--color-outl-border)/30 px-12 pt-12 pb-8">
        <div class="mx-auto max-w-3xl">
          {/*
           * Breadcrumb — mirrors the TUI's
           * `📅 Journal · Thursday, 2026-06-04` header. For pages,
           * it carries the slug instead.
           */}
          <Show when={appState.page?.kind === "journal"}>
            <div class="mb-2 flex items-baseline gap-1.5 text-[12.5px] text-(--color-outl-fg-dim)">
              <span>{appState.page?.icon || "📅"}</span>
              <span>Journal · {journalWeekday()}</span>
            </div>
          </Show>
          <Show when={appState.page && appState.page.kind !== "journal"}>
            <div class="mb-2 flex items-baseline gap-1.5 text-[12.5px] text-(--color-outl-fg-dim)">
              <span>{appState.page?.icon || "📄"}</span>
              <span class="font-mono">{appState.page?.slug}</span>
            </div>
          </Show>

          <h1 class="font-mono text-[28px] font-semibold leading-[1.15] tracking-tight">
            <Show
              when={appState.page}
              fallback={<span class="opacity-40">No page open</span>}
            >
              <Show
                when={appState.page?.kind === "journal"}
                fallback={appState.page?.title}
              >
                {appState.page?.slug}
              </Show>
            </Show>
          </h1>
        </div>
      </header>

      <div class="min-w-0 flex-1 overflow-y-auto px-12 py-6">
        <div class="mx-auto w-full max-w-3xl">
        <Show
          when={appState.outline.length > 0}
          fallback={
            <button
              type="button"
              onClick={addFirstBlock}
              class="rounded px-3 py-2 text-sm opacity-60 hover:bg-white/5 hover:opacity-100"
            >
              {appState.page ? "Click to add the first block" : "Loading…"}
            </button>
          }
        >
          <For each={appState.outline}>
            {(block) => (
              <BlockRow
                block={block}
                depth={0}
                editingId={editingId()}
                cb={cb}
              />
            )}
          </For>
        </Show>

        {/*
         * Backlinks render inline below the outline (TUI parity),
         * separated by a soft full-width rule. Hidden when the
         * section is toggled off (Cmd+Shift+B) or when the current
         * page has no incoming refs.
         */}
        <InlineBacklinks />
        </div>
      </div>

      <Show when={appState.lastError}>
        <div class="border-t border-(--color-outl-status-message-fg)/20 bg-(--color-outl-status-message-fg)/5 px-12 py-2 text-xs text-(--color-outl-status-message-fg)">
          <div class="mx-auto flex max-w-3xl items-center justify-between">
            <span>{appState.lastError}</span>
            <button
              type="button"
              onClick={() => setAppState("lastError", null)}
              class="ml-2 opacity-70 hover:opacity-100"
              aria-label="Dismiss error"
            >
              ✕
            </button>
          </div>
        </div>
      </Show>
    </main>
  );
}
