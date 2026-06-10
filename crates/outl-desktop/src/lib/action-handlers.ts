/**
 * Concrete `Action -> handler` map for the desktop client.
 *
 * Built once in `<AppShell />` and handed to `installShortcuts`.
 * Each entry maps an `outl_shortcuts::Action` variant to the Tauri
 * call (or store mutation) that materialises it on the desktop.
 *
 * The map mirrors the TUI's vim bindings 1:1 — `j/k` move the
 * selection, `i/o/O` enter Insert, `Tab/Shift-Tab` indent /
 * outdent, `dd` deletes, `c` folds, `Cmd+T` cycles TODO — so a
 * user moving between clients keeps their muscle memory.
 *
 * The selection cursor lives in `appState.selectedBlockId`;
 * `<BlockRow />` highlights it. Auto-selection of the first block
 * runs from `<OutlineView />` so the dispatcher never has to worry
 * about a `null` cursor.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  createBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  moveBlockDown,
  moveBlockUp,
  nextDay,
  openJournalFor,
  openRef,
  openTodayJournal,
  outdentBlock,
  previousDay,
  setBlockCollapsed,
  todaySlug,
  toggleTodo as toggleTodoCmd,
} from "@outl/shared/api/commands";
import type { BlockNode, PageView } from "@outl/shared/api/types";

import { runCodeBlock } from "./api";
import { insertLink, wrapSelection } from "./markdown-wrap";
import {
  flattenVisible,
  nextVisibleId,
  previousVisibleId,
} from "./outline-walk";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

export interface DesktopHandlerDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

/**
 * Build the desktop's handler map. Closes over the `applyView` /
 * `setError` callbacks from the shell so each action can refresh
 * the rendered page or surface a problem in the status bar.
 */
export function buildHandlers(deps: DesktopHandlerDeps): ActionHandlers {
  const safeCall = async <T>(p: Promise<T>): Promise<T | undefined> => {
    try {
      return await p;
    } catch (e) {
      deps.setError(e instanceof Error ? e.message : String(e));
      return undefined;
    }
  };

  /** Resolve the block the user means right now — the focused
   *  textarea's id (Insert) takes priority over the selection
   *  cursor (Normal). Returns `null` when neither is set. */
  function targetBlockId(): string | null {
    const el = document.activeElement;
    if (el instanceof HTMLTextAreaElement && el.dataset.blockId) {
      return el.dataset.blockId;
    }
    return appState.selectedBlockId;
  }

  /** Block ids of each visible backlink, in render order. The
   *  selection cursor traverses this list with `j/k` when the
   *  backlinks section is reachable. */
  function backlinkBlockIds(): string[] {
    if (!appState.backlinksOpen) return [];
    return appState.backlinks.map((b) => b.block_id);
  }

  /** Find a block in the outline by id; deep search. */
  function lookupBlock(id: string): BlockNode | null {
    const walk = (bs: BlockNode[]): BlockNode | null => {
      for (const b of bs) {
        if (b.id === id) return b;
        if (b.children.length > 0) {
          const hit = walk(b.children);
          if (hit) return hit;
        }
      }
      return null;
    };
    return walk(appState.outline);
  }

  /** Common pattern: await a Tauri command that returns a
   *  `PageView`, apply it, and leave selection on `nextSelectedId`
   *  (or keep current when `undefined`). */
  async function runOn(
    cmd: Promise<PageView>,
    nextSelectedId?: string | null,
  ) {
    const view = await safeCall(cmd);
    if (!view) return;
    deps.applyView(view);
    if (nextSelectedId !== undefined) {
      setAppState("selectedBlockId", nextSelectedId);
    }
  }

  return {
    // ── chrome ────────────────────────────────────────────────────
    OpenPicker: () => {
      setAppState("pickerOpen", !appState.pickerOpen);
    },
    OpenSettings: () => {
      setAppState("settingsOpen", !appState.settingsOpen);
    },
    ToggleSidebar: () => {
      setAppState("sidebarOpen", !appState.sidebarOpen);
    },
    ToggleBacklinks: () => {
      setAppState("backlinksOpen", !appState.backlinksOpen);
    },
    ToggleHelp: () => {
      setAppState("helpOpen", !appState.helpOpen);
    },
    Quit: async () => {
      // `qq` chord in Normal + `Ctrl+C` Global. Close the active
      // window; Tauri tears the app down once the last window
      // closes.
      try {
        await getCurrentWindow().close();
      } catch (e) {
        deps.setError(e instanceof Error ? e.message : String(e));
      }
    },

    // ── page-level navigation ────────────────────────────────────
    OpenToday: async () => {
      const view = await safeCall(openTodayJournal());
      if (view) deps.applyView(view);
    },
    PrevDay: async () => {
      const anchor =
        appState.page?.kind === "journal"
          ? appState.page.slug
          : await safeCall(todaySlug());
      if (!anchor) return;
      const slug = await safeCall(previousDay(anchor));
      if (!slug) return;
      const view = await safeCall(openJournalFor(slug));
      if (view) deps.applyView(view);
    },
    NextDay: async () => {
      const anchor =
        appState.page?.kind === "journal"
          ? appState.page.slug
          : await safeCall(todaySlug());
      if (!anchor) return;
      const slug = await safeCall(nextDay(anchor));
      if (!slug) return;
      const view = await safeCall(openJournalFor(slug));
      if (view) deps.applyView(view);
    },

    // ── selection (vim j/k/↑/↓) ──────────────────────────────────
    //
    // The cursor spans two regions: the outline blocks (top) and
    // the inline backlinks (bottom). Only one of
    // `selectedBlockId` / `selectedBacklinkBlockId` is non-null at
    // any time so navigation stays a single linear motion.
    //
    // Backlinks are only reachable when the section is visible
    // (`backlinksOpen` + the page has at least one ref); otherwise
    // `j` past the last outline block clamps at the bottom — same
    // behaviour the TUI ships when `show_backlinks == false`.
    SelectionDown: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx >= 0 && idx < backlinkIds.length - 1) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx + 1]);
        }
        // last backlink → stay (no wrap into outline)
        return;
      }
      const cur = appState.selectedBlockId;
      const list = flattenVisible(appState.outline);
      const next = nextVisibleId(cur, appState.outline);
      const atLast = cur !== null && next === cur;
      if (atLast && backlinkIds.length > 0) {
        // Cross into the backlinks section.
        setAppState("selectedBlockId", null);
        setAppState("selectedBacklinkBlockId", backlinkIds[0]);
        return;
      }
      console.info(
        `[shortcuts] SelectionDown cur=${cur} → ${next} (idx ${list.indexOf(cur ?? "")} → ${list.indexOf(next ?? "")} of ${list.length})`,
      );
      if (next) setAppState("selectedBlockId", next);
    },
    SelectionUp: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx > 0) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx - 1]);
          return;
        }
        // First backlink → step back into the outline's last
        // visible block.
        const list = flattenVisible(appState.outline);
        if (list.length > 0) {
          setAppState("selectedBacklinkBlockId", null);
          setAppState("selectedBlockId", list[list.length - 1]);
        }
        return;
      }
      const cur = appState.selectedBlockId;
      const list = flattenVisible(appState.outline);
      const prev = previousVisibleId(cur, appState.outline);
      console.info(
        `[shortcuts] SelectionUp cur=${cur} → ${prev} (idx ${list.indexOf(cur ?? "")} → ${list.indexOf(prev ?? "")} of ${list.length})`,
      );
      if (prev) setAppState("selectedBlockId", prev);
    },

    // ── enter Insert (i / Shift+I / Enter) ───────────────────────
    EnterInsert: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    EnterInsertAtStart: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    OpenRefUnderCursor: async () => {
      // Normal-mode `Enter`. Two branches in priority order:
      //
      // 1. Cursor is on a backlink → open the source page and snap
      //    the cursor to the referencing block on that page
      //    (backlink rows are read-only, so "open" is the only
      //    meaningful gesture there).
      // 2. Otherwise → enter Insert on the selected block.
      //
      // Unlike the TUI, the desktop's Normal mode has no character
      // cursor — only a selected block — so "the ref under the
      // cursor" cannot be resolved. An earlier version approximated
      // it as "the first `[[ref]]` in the block", which made every
      // ref-carrying block impossible to edit via Enter. Following
      // a ref on the desktop is the click on the token
      // (`onRefClick` in OutlineView).
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const link = appState.backlinks.find(
          (b) => b.block_id === curBacklink,
        );
        const target = link?.source_page?.slug;
        if (!target) return;
        const view = await safeCall(openRef(target));
        if (!view) return;
        deps.applyView(view);
        // Reset cursor to the source block on the freshly-opened
        // page so the user lands exactly where the ref lives.
        setAppState("selectedBacklinkBlockId", null);
        setAppState("selectedBlockId", curBacklink);
        return;
      }
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },

    // ── create siblings (o / O) ──────────────────────────────────
    NewBlockBelow: async () => {
      const pageId = appState.page?.id;
      const after = appState.selectedBlockId;
      if (!pageId) return;
      const reply = await safeCall(
        createBlock(pageId, {
          afterId: after,
          parentId: null,
          text: "",
        }),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setAppState("editingBlockId", reply.new_id);
    },
    NewBlockAbove: async () => {
      const pageId = appState.page?.id;
      const anchor = appState.selectedBlockId;
      if (!pageId) return;
      const reply = await safeCall(
        createBlock(pageId, {
          afterId: null,
          parentId: null,
          text: "",
        }),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      const newId = reply.new_id;
      // Walk it up until it sits immediately before the anchor.
      let cursorOutline = reply.view.outline;
      while (anchor) {
        const visible = flattenVisible(cursorOutline);
        const newIdx = visible.indexOf(newId);
        const anchorIdx = visible.indexOf(anchor);
        if (newIdx < 0 || anchorIdx < 0) break;
        if (newIdx + 1 >= anchorIdx) break;
        const stepped = await safeCall(moveBlockDown(pageId, newId));
        if (!stepped) break;
        deps.applyView(stepped);
        cursorOutline = stepped.outline;
      }
      setAppState("selectedBlockId", newId);
      setAppState("editingBlockId", newId);
    },

    // ── block structure ops on the selected block ────────────────
    IndentBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(indentBlock(pageId, id));
    },
    OutdentBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(outdentBlock(pageId, id));
    },
    MoveBlockUp: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(moveBlockUp(pageId, id));
    },
    MoveBlockDown: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      await runOn(moveBlockDown(pageId, id));
    },
    DeleteBlock: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      // Move selection to the previous visible block first so the
      // cursor doesn't land on `null` after the delete.
      const prev = previousVisibleId(id, appState.outline);
      await runOn(deleteBlock(pageId, id), prev);
    },
    ToggleCollapsed: async () => {
      const pageId = appState.page?.id;
      const id = targetBlockId();
      if (!pageId || !id) return;
      const block = lookupBlock(id);
      if (!block || block.children.length === 0) return;
      await runOn(setBlockCollapsed(pageId, id, !block.collapsed));
    },

    // ── block TODO toggle (Cmd+T) ────────────────────────────────
    //
    // Targets the focused textarea (Insert) or the selected block
    // (Normal) — same fallback chain as IndentBlock/etc.
    ToggleTodo: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError("Select or click a block first, then ⌘T toggles its TODO");
        return;
      }
      const view = await safeCall(toggleTodoCmd(pageId, id));
      if (view) deps.applyView(view);
    },

    // ── overlays + insert escape ─────────────────────────────────
    //
    // Esc cascades: close the topmost overlay first (Help → Picker
    // → Settings), and if no overlay is up but a textarea is
    // focused, blur it. The blur fires `<BlockRow />`'s `onBlur`
    // handler (which commits + flips `editingBlockId` to null), so
    // the user is back in Normal mode without a second key.
    ExitInsert: () => {
      if (appState.helpOpen) {
        setAppState("helpOpen", false);
        return;
      }
      if (appState.pickerOpen) {
        setAppState("pickerOpen", false);
        return;
      }
      if (appState.settingsOpen) {
        setAppState("settingsOpen", false);
        return;
      }
      const el = document.activeElement;
      if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
        el.blur();
      }
    },

    // ── inline markdown wrappers (Insert mode) ───────────────────
    //
    // These act on the active `<textarea>` via DOM — no Solid
    // signal lookup needed. The dispatcher only fires them when
    // mode === "insert", so we know a textarea is focused.
    WrapBold: () => wrapSelection("**"),
    // outl's canonical italic delimiter is `_` (the markdown parser
    // accepts `*…*` too, but `_…_` is the form the workspace ships
    // and the form we want Cmd+I to produce).
    WrapItalic: () => wrapSelection("_"),
    WrapCode: () => wrapSelection("`"),
    WrapStrike: () => wrapSelection("~~"),
    InsertLink: () => insertLink(),

    // ── run code block (Cmd+X) ───────────────────────────────────
    //
    // Targets the focused textarea (Insert) or the selected block
    // (Normal). Resolves the block, asks the backend to execute the
    // `\`\`\`lang … \`\`\`` fence via `outl-exec`, and surfaces the
    // resulting stdout/stderr through `applyView`.
    RunCodeBlock: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const id = targetBlockId();
      if (!id) {
        deps.setError(
          "Select or focus a code block first, then ⌘X runs it",
        );
        return;
      }
      const reply = await safeCall(runCodeBlock(pageId, id));
      if (reply) deps.applyView(reply.view);
    },

    // ── commit + new sibling ─────────────────────────────────────
    //
    // Fires on `Cmd+Shift+Enter` inside a textarea (Insert mode).
    // Reads the live draft straight off the focused textarea and
    // commits, then asks the backend for a fresh sibling and parks
    // edit mode on the new id so the user keeps typing without a
    // click. Mirrors the old `<BlockRow />` intercept that used to
    // own this chord before it moved to the catalog.
    CommitAndContinue: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const el = document.activeElement;
      if (!(el instanceof HTMLTextAreaElement) || !el.dataset.blockId) {
        return;
      }
      const id = el.dataset.blockId;
      const text = el.value;
      const editedView = await safeCall(editBlock(pageId, id, text));
      if (editedView) deps.applyView(editedView);
      const reply = await safeCall(
        createBlock(pageId, { afterId: id, parentId: null, text: "" }),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      setAppState("editingBlockId", reply.new_id);
      setAppState("selectedBlockId", reply.new_id);
    },

    // ── catalog-only (no JS handler today) ───────────────────────
    //
    // Intentionally absent so the dispatcher falls through (no
    // preventDefault) and the textarea / OS handles the chord:
    //
    //   EditBlock chord buffer (CopyBlockRef), EnterVisual,
    //   YankRange, DeleteRange, Undo, Redo, OpenCommandPalette,
    //   RunCodeBlock (per-block button).
    //
    // Each shows up as `[shortcuts] … no JS handler` in DevTools so
    // a debugging session can see what's still on the to-do list.
  };
}
