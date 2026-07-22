import { For, Show, createEffect, createMemo, on } from "solid-js";

import {
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  instantiateTemplateAt,
  openExternalUrl,
  openPageBySlug,
  openRef,
  outdentBlock,
  pageBacklinks,
  pasteMarkdown,
  pastePlain,
  pluginRun,
  pluginSyncHooks,
  resolveEmbeds,
  runAutoRunBlocks,
  runCodeBlock,
  setBlockCollapsed,
  toggleTodo,
} from "@outl/shared/api/commands";

import type { PageView } from "@outl/shared/api/types";

import { ParseWarningsBanner } from "@outl/shared/warnings";
import { journalSlugToDate } from "@outl/shared/journal";
import { focusSubtree, visualRangeSet } from "@outl/shared/outline";
import { NATIVE_TEMPLATE_PLUGIN_ID } from "../lib/slash-commands";
import { playPluginViews } from "../lib/plugin-views";
import { appState, setAppState, setOutline } from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { InlineBacklinks } from "./InlineBacklinks";

/**
 * Boot placeholder for the outline body.
 *
 * Rendered only while `appState.page` is still null (the background
 * opener hasn't handed us today's journal yet). It occupies the same
 * column the real rows fill, so the content area reads as "already
 * laid out, filling in" instead of a bare `Loading…` that the outline
 * then displaces. Purely decorative — `aria-hidden` keeps it off the
 * a11y tree.
 */
function OutlineSkeleton() {
  // Row widths (percent) picked to look like natural outline text,
  // not a uniform block. Static list → stable across renders.
  const widths = [86, 68, 92, 54, 78, 64];
  return (
    <div class="animate-pulse space-y-3 pt-1" aria-hidden="true">
      <For each={widths}>
        {(w) => (
          <div class="flex items-center gap-2">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-(--color-outl-fg)/15" />
            <span
              class="h-3.5 rounded bg-(--color-outl-fg)/10"
              style={{ width: `${w}%` }}
            />
          </div>
        )}
      </For>
    </div>
  );
}

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
    // the outline (never on a stale backlink from the previous page)
    // and drops any zoom held on the previous page's block.
    setAppState("selectedBacklinkBlockId", null);
    setAppState("focusBlockId", null);
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

  /**
   * Lazily fetch backlinks — but ONLY when the page slug changes
   * (navigation), via `on(slug)`. It must NOT refetch on every mutation.
   *
   * `applyView` after a commit replaces `appState.page` with a fresh
   * object carrying the *same* slug, which a bare `createEffect` reading
   * `appState.page?.slug` re-runs anyway (the store tracks `page`). Each
   * re-run hit `pageBacklinks`, and since a mutation invalidated the
   * backend index, that rebuilt the whole index (reading every `.md`)
   * on every keystroke-commit — the "Esc is slow" bug. Editing the
   * current page almost never changes *its own* backlinks (backlinks are
   * other pages pointing here), so refetching per edit is pure waste.
   * `on(slug)` fires once per navigation; the peer-reload path refetches
   * explicitly (`AppShell::refreshActivePage`) since it keeps the slug.
   */
  createEffect(
    on(
      () => appState.page?.slug,
      (slug) => {
        if (!slug) return;
        pageBacklinks(slug)
          .then((r) =>
            setAppState({
              backlinks: r.backlinks,
              backlinksOrder: r.backlinks_order,
            }),
          )
          .catch(() => {});
      },
    ),
  );

  function applyView(view: PageView) {
    setAppState({
      page: view.page,
      parseWarnings: view.warnings ?? [],
    });
    // Reconcile the outline (see `setOutline`): only the block that
    // actually changed re-renders, not all N rows.
    setOutline(view.outline);
    // Auto-run query blocks after page load / commit, then re-resolve
    // embeds with the updated outline.
    void runAutoRunBlocks(view.page.id)
      .then((reply) => {
        if (reply.ran > 0) {
          const updated = reply.view;
          setAppState({
            page: updated.page,
            parseWarnings: updated.warnings ?? [],
          });
          setOutline(updated.outline);
          void resolvePageEmbeds(updated.outline);
        }
      })
      .catch(() => {});
    // Resolve embeds on the initial page view.
    void resolvePageEmbeds(view.outline);
  }

  /** Collect unique embed handles from the outline and batch-resolve. */
  function resolvePageEmbeds(outline: import("@outl/shared/api/types").BlockNode[]) {
    const handleSet = new Set<string>();
    const walk = (nodes: import("@outl/shared/api/types").BlockNode[]) => {
      for (const n of nodes) {
        for (const tok of n.tokens) {
          if (tok.kind === "embed" && tok.value) handleSet.add(tok.value);
        }
        if (n.children.length > 0) walk(n.children);
      }
    };
    walk(outline);
    if (handleSet.size === 0) return;
    const handles = [...handleSet];
    void resolveEmbeds(handles)
      .then((map) => {
        setAppState("embeds", map);
      })
      .catch(() => {});
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

  /**
   * Zoom view (Roam/Workflowy focus). `null` when not zoomed; otherwise
   * the focused block's subtree + ancestor breadcrumb, sliced from the
   * outline the client already holds — pure view state, no round-trip.
   * `focusSubtree` returns `null` when the focused id is no longer in
   * the outline (a peer deleted it, or it moved off-page); we clear the
   * zoom so the full page renders again instead of a blank pane.
   */
  const focus = createMemo<import("@outl/shared/outline").FocusView | null>(
    () => {
      const id = appState.focusBlockId;
      if (!id) return null;
      return focusSubtree(appState.outline, id);
    },
  );

  // Self-heal a stale zoom target *outside* the memo: when the focused
  // id left the outline (peer delete / off-page move) `focus()` is
  // `null` while `focusBlockId` still holds the dead id, so clear it and
  // the full page renders. Kept in an effect, not the memo, so the memo
  // stays a pure derivation (a `setAppState` inside a memo is a
  // reactivity hazard as the component grows).
  createEffect(() => {
    if (appState.focusBlockId && !focus()) {
      setAppState("focusBlockId", null);
    }
  });

  /** Blocks to render in the outline body. When zoomed (Roam-style) the
   *  focused block becomes the header title, so the body shows its
   *  **children**; otherwise the whole page. */
  const rootBlocks = () => {
    const fv = focus();
    return fv ? fv.root.children : appState.outline;
  };

  async function handleError<T>(promise: Promise<T>): Promise<T | undefined> {
    try {
      return await promise;
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
      return undefined;
    }
  }

  /**
   * Persist the in-flight textarea draft into the workspace before a
   * paste splices at `caret`. The caret is measured against the draft
   * (`textarea.value`), but the backend splices against
   * `host_text_for_caret` — if the user typed since the last commit,
   * the two diverge and the paste lands at the wrong offset. Mobile
   * does the same in `Journal.handlePasteMarkdown`. No-op when the
   * block isn't the one being edited (a paste from a click without an
   * open editor). Edit mode is left untouched (`editBlock` doesn't flip
   * `editingBlockId`), so the user keeps typing after a plain paste.
   */
  async function flushDraftBeforePaste(
    pageId: string,
    id: string,
    hostText: string,
  ) {
    if (editingId() !== id) return;
    const committed = await handleError(editBlock(pageId, id, hostText));
    if (committed) applyView(committed);
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
      // Native `/template <name>`: instantiate the structural template
      // under the block the slash was typed in (or the current
      // selection). Intercepted here because it reuses the slash popup
      // but is a core feature, not a plugin — `commandId` is the
      // template name (see `templateSlashCommands`).
      if (pluginId === NATIVE_TEMPLATE_PLUGIN_ID) {
        const target = appState.selectedBlockId;
        if (!target) {
          setAppState("lastError", "select a block to insert a template");
          return;
        }
        const view = await handleError(
          instantiateTemplateAt(commandId, target),
        );
        if (view) applyView(view);
        return;
      }
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
    onPasteMarkdown: async (id, caret, text, hostText) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      await flushDraftBeforePaste(pageId, id, hostText);
      const view = await handleError(pasteMarkdown(pageId, id, caret, text));
      if (view) applyView(view);
    },
    onPastePlain: async (id, caret, text, hostText) => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      await flushDraftBeforePaste(pageId, id, hostText);
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
    onOpenPage: async (slug) => {
      const view = await handleError(openPageBySlug(slug));
      if (view) applyView(view);
    },
    onFocusBlock: (id) => {
      // Zoom into the clicked block. Also sync the selection cursor so
      // `j/k` pick up inside the focused subtree.
      setAppState("selectedBlockId", id);
      setAppState("focusBlockId", id);
    },
  };

  async function addFirstBlock() {
    const pageId = appState.page?.id;
    if (!pageId) return;
    // When zoomed into a leaf, the empty body is *inside* the focused
    // block, so the first block must be created as its child — not at
    // the page root.
    const parentId = appState.focusBlockId;
    const reply = await handleError(
      createBlock(pageId, { afterId: null, parentId, text: "" }),
    );
    if (reply) applyView(reply.view);
  }

  /**
   * Journal day-of-week ("Thursday") — used in the breadcrumb
   * above the ISO title. Returns empty for non-journals or
   * malformed slugs.
   */
  function journalWeekday(): string {
    const page = appState.page;
    if (!page || page.kind !== "journal") return "";
    // `journalSlugToDate` parses parts so JS doesn't apply UTC
    // (`new Date("2026-06-02")` is midnight UTC, which renders the
    // previous day in negative-offset timezones).
    const d = journalSlugToDate(page.slug);
    return d ? d.toLocaleDateString(undefined, { weekday: "long" }) : "";
  }

  /** Page icon, with the same 📅/📄 default the eyebrow uses. */
  function pageIcon(): string {
    return (
      appState.page?.icon ||
      (appState.page?.kind === "journal" ? "📅" : "📄")
    );
  }

  /** Human label for the page crumb in the zoom path — the journal's
   *  ISO slug or a regular page's title (falling back to its slug). */
  function pageCrumbLabel(): string {
    const page = appState.page;
    if (!page) return "";
    return page.kind === "journal" ? page.slug : page.title || page.slug;
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
           * When zoomed (Roam/Workflowy focus) the header becomes the
           * focused block's own page-like header: a clickable path back
           * to the journal/page + ancestors as the eyebrow, and the
           * block's text as the title. Otherwise the normal page header.
           */}
          <Show
            when={focus()}
            fallback={
              <>
                {/*
                 * Breadcrumb — mirrors the TUI's
                 * `📅 Journal · Thursday, 2026-06-04` header. For pages,
                 * it carries the slug instead.
                 *
                 * The eyebrow slot always reserves its row height
                 * (`min-h-5`), even before the page loads (both inner
                 * `<Show>`s off). Without the reserved height the row
                 * collapses to 0 while `appState.page` is null at boot,
                 * then pops in once the journal arrives and shoves the
                 * `<h1>` title down — a visible layout shift. A stable
                 * slot keeps the title pinned from the first frame.
                 */}
                <div class="mb-2 flex min-h-5 items-baseline gap-1.5 text-[12.5px] text-(--color-outl-fg-dim)">
                  <Show when={appState.page?.kind === "journal"}>
                    <span>{pageIcon()}</span>
                    <span>Journal · {journalWeekday()}</span>
                  </Show>
                  <Show when={appState.page && appState.page.kind !== "journal"}>
                    <span>{pageIcon()}</span>
                    <span class="font-mono">{appState.page?.slug}</span>
                  </Show>
                </div>

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
              </>
            }
          >
            {(fv) => (
              <>
                {/*
                 * Zoom path (eyebrow). The leading crumb is the whole
                 * page — click it to exit the zoom and return to the
                 * journal/page; each ancestor crumb re-focuses that
                 * block. The focused block itself is the title below,
                 * so it isn't a crumb.
                 */}
                <nav
                  aria-label="Zoom path"
                  class="mb-2 flex flex-wrap items-center gap-1 text-[12.5px] text-(--color-outl-fg-dim)"
                >
                  <button
                    type="button"
                    onClick={() => setAppState("focusBlockId", null)}
                    class="flex items-center gap-1.5 opacity-70 hover:opacity-100"
                    title="Back to page"
                  >
                    <span>{pageIcon()}</span>
                    <span
                      class={
                        appState.page?.kind === "journal" ? "font-mono" : ""
                      }
                    >
                      {pageCrumbLabel()}
                    </span>
                  </button>
                  <For each={fv().breadcrumb}>
                    {(crumb) => (
                      <>
                        <span aria-hidden="true" class="opacity-40">
                          ›
                        </span>
                        <button
                          type="button"
                          onClick={() => setAppState("focusBlockId", crumb.id)}
                          class="max-w-[16rem] truncate opacity-70 hover:opacity-100"
                          title={crumb.text}
                        >
                          {crumb.text || "(empty)"}
                        </button>
                      </>
                    )}
                  </For>
                </nav>

                {/* Title = the focused block itself. */}
                <h1 class="font-mono text-[28px] font-semibold leading-[1.15] tracking-tight">
                  <Show
                    when={fv().root.text}
                    fallback={<span class="opacity-40">(empty block)</span>}
                  >
                    {fv().root.text}
                  </Show>
                </h1>
              </>
            )}
          </Show>
        </div>
      </header>

      <div class="min-w-0 flex-1 overflow-y-auto px-12 py-6">
        <div class="mx-auto w-full max-w-3xl">
          <ParseWarningsBanner warnings={appState.parseWarnings} />
          <Show
            when={rootBlocks().length > 0}
            fallback={
              // Page still loading (null) → skeleton that reserves the
              // outline's shape so nothing jumps when the rows arrive.
              // Page loaded but genuinely empty → the add-first-block
              // affordance (also the zoomed-into-a-leaf case).
              <Show when={appState.page} fallback={<OutlineSkeleton />}>
                <button
                  type="button"
                  onClick={addFirstBlock}
                  class="rounded px-3 py-2 text-sm opacity-60 hover:bg-(--color-outl-fg)/5 hover:opacity-100"
                >
                  Click to add the first block
                </button>
              </Show>
            }
          >
            <For each={rootBlocks()}>
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
