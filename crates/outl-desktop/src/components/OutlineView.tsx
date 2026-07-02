import { For, Show, createEffect, createMemo } from "solid-js";

import {
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  openExternalUrl,
  openRef,
  outdentBlock,
  pasteMarkdown,
  pastePlain,
  setBlockCollapsed,
  toggleTodo,
} from "@outl/shared/api/commands";

import type { PageView } from "@outl/shared/api/types";

import { ParseWarningsBanner } from "@outl/shared/warnings";
import { pluginRun, pluginSyncHooks, runCodeBlock } from "../lib/api";
import { playPluginViews } from "../lib/plugin-views";
import { visualRangeSet } from "../lib/outline-walk";
import { appState, setAppState } from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { GhostFirstBlock } from "./GhostFirstBlock";
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
  const setEditingId = (id: string | null) => setAppState("editingBlockId", id);

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
      parseWarnings: view.warnings ?? [],
    });
  }

  /**
   * Memoised Visual-range membership set. Built once per
   * outline / anchor / cursor / mode change, then read O(1) by every
   * `<BlockRow />` via the `visualRangeSet` prop. The previous shape
   * called `isInVisualRange(id, anchor, cursor, outline)` per row,
   * which rebuilt `flattenVisible(blocks)` from scratch — N rows × N
   * DFS = O(N²) per Visual extension keystroke. On a 500-block page
   * the extension felt laggy by the third `j`.
   *
   * `null` outside vim-visual mode (most renders) so `<BlockRow />`
   * can short-circuit without touching the Set at all.
   */
  const visualSet = createMemo<Set<string> | null>(() => {
    if (appState.mode !== "vim-visual") return null;
    return visualRangeSet(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
  });

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

  function handleLinkClick(href: string) {
    // `[label](url)` → open in the system browser via the shared
    // opener wrapper (scheme-guarded to http(s)/mailto). Errors land on
    // the status line instead of throwing into the click handler.
    void openExternalUrl(href).catch((e) => {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    });
  }

  const cb: BlockCallbacks = {
    onStartEdit: (id) => {
      // Sync the selection cursor with whatever the user clicked so
      // `j/k` pick up from there instead of teleporting back to the
      // last vim-driven cursor.
      setAppState("selectedBlockId", id);
      setEditingId(id);
    },
    onRunPluginCommand: async (pluginId, commandId) => {
      // Same dispatch the `⧉` PluginPalette runs: surface the command's
      // notifications / errors on the status line, re-render from the
      // returned view, and play any `ui-render` overlays.
      const reply = await handleError(
        pluginRun(pluginId, commandId, appState.page?.id ?? null),
      );
      if (!reply) return;
      for (const note of reply.notifications) setAppState("lastError", note);
      for (const err of reply.errors) {
        setAppState("lastError", `plugin: ${err}`);
      }
      if (reply.view) applyView(reply.view);
      playPluginViews(reply.views);
    },
    onCommit: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(editBlock(pageId, id, text));
      if (view) applyView(view);
      setEditingId(null);
      // Single post-mutation point for plugin `onOp` hooks. `sync_hooks`
      // dispatches EVERY op since the host's last sweep (not just this
      // edit), so one call after a commit catches up structural ops
      // (indent / move / delete) committed since the previous commit too
      // — mirrors the TUI's once-per-tick sweep. Best-effort; the lock is
      // already released here so the plugin thread can take it.
      const hooked = await handleError(pluginSyncHooks(pageId));
      if (hooked?.view) applyView(hooked.view);
      // Confetti path: a `ui-render` plugin (op-hook + ui-render) emits
      // HTML from its `onOp` hook on e.g. a DONE toggle. Play it as a
      // sandboxed iframe overlay even when nothing was re-rendered.
      if (hooked) playPluginViews(hooked.views);
    },
    onEnter: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      // Commit the in-flight edit first, then create the sibling.
      // The backend returns the freshly-inserted id so we put it
      // straight into edit mode — `<BlockRow />`'s `createEffect`
      // focuses its textarea on the next tick. We used to find the
      // new block by diffing against a snapshot of the old outline,
      // which mis-fired when the host block had children (the diff
      // could pick up an existing descendant).
      await handleError(editBlock(pageId, id, text));
      const reply = await handleError(
        createBlock(pageId, { afterId: id, parentId: null, text: "" }),
      );
      if (!reply) return;
      applyView(reply.view);
      setEditingId(reply.new_id);
    },
    onCreateBefore: async (id, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      // Commit the in-flight edit first, then create the sibling
      // before it. `beforeId` lets the backend pick the fractional
      // index; we focus the freshly-minted block via its returned id
      // (same pattern as `onEnter`).
      await handleError(editBlock(pageId, id, text));
      const reply = await handleError(
        createBlock(pageId, { beforeId: id, text: "" }),
      );
      if (!reply) return;
      applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setEditingId(reply.new_id);
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
    onPastePlain: async (id, caret, text) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const view = await handleError(pastePlain(pageId, id, caret, text));
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
    onLinkClick: handleLinkClick,
  };

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
          <ParseWarningsBanner warnings={appState.parseWarnings} />
          <Show
            when={appState.outline.length > 0}
            fallback={
              // Empty page: a ghost first block the user can type into
              // right away. Nothing reaches the op log until non-empty
              // text is committed, so merely opening a fresh journal
              // day never appends an op (see GhostFirstBlock's doc).
              <Show
                when={appState.page}
                fallback={
                  <div class="px-3 py-2 text-sm opacity-60">Loading…</div>
                }
                keyed
              >
                {(page) => (
                  <GhostFirstBlock
                    pageId={page.id}
                    applyView={applyView}
                    onError={(e) =>
                      setAppState(
                        "lastError",
                        e instanceof Error ? e.message : String(e),
                      )
                    }
                  />
                )}
              </Show>
            }
          >
            <For each={appState.outline}>
              {(block) => (
                <BlockRow
                  block={block}
                  depth={0}
                  editingId={editingId()}
                  visualSet={visualSet()}
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
      {/* The error surface is the top-right `<ErrorToast />` (mounted in
       *  AppShell). A base banner here sat under the fixed ChromeToggleBar
       *  and got covered — issue moved it to the notification corner. */}
    </main>
  );
}
