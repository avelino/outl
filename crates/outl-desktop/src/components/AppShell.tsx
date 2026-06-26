import { Show, onCleanup, onMount } from "solid-js";

import {
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  reloadWorkspace,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";
import { takePendingDeepLink, workspaceStats } from "../lib/api";
import { onDeepLinkNavigate, onPeerOpsChanged } from "../lib/events";
import type { DeepLinkNavigate } from "../lib/events";
import { installShortcuts } from "../lib/shortcuts";
import { buildHandlers } from "../lib/action-handlers";
import { Sidebar } from "./Sidebar";
import { OutlineView } from "./OutlineView";
import { Picker } from "./Picker";
import { SettingsModal } from "./SettingsModal";
import { HelpOverlay } from "./HelpOverlay";
import { ChromeToggleBar } from "./ChromeToggleBar";

/**
 * 2-pane shell rendered once a workspace is loaded.
 *
 * ```
 * | Sidebar | OutlineView (+ inline Backlinks at bottom) |
 * ```
 *
 * The TUI renders backlinks **inline below the outline**, not as a
 * side panel — same shape here so a user moving between clients
 * sees the same structure. `Cmd/Ctrl+Shift+B` toggles the inline
 * section (`<InlineBacklinks />` reads `appState.backlinksOpen`).
 * `Cmd/Ctrl+Shift+E` toggles the sidebar.
 */
export function AppShell() {
  function applyView(view: PageView) {
    setAppState({
      page: view.page,
      outline: view.outline,
      backlinks: view.backlinks,
    });
  }

  function setError(msg: string) {
    setAppState("lastError", msg);
  }

  async function refreshStats() {
    try {
      const stats = await workspaceStats();
      setAppState("workspace", stats);
    } catch {
      // Backend not ready yet — picker stays visible; nothing to do.
    }
  }

  async function loadToday() {
    try {
      const view = await openTodayJournal();
      applyView(view);
      await refreshStats();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Refresh the active page after a peer-driven workspace reload.
   *
   * Walks the page kind so we use the right command (`open_ref`
   * would create a fresh page if a peer deleted ours mid-flight).
   * If the active page is gone, fall back to today's journal.
   */
  async function refreshActivePage() {
    const page = appState.page;
    if (!page) {
      await loadToday();
      return;
    }
    try {
      const view =
        page.kind === "journal"
          ? await openJournalFor(page.slug)
          : await openPageBySlug(page.slug);
      applyView(view);
    } catch {
      await loadToday();
    }
  }

  async function onPeerChange() {
    try {
      await reloadWorkspace();
      await refreshActivePage();
      await refreshStats();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Navigate in response to an `outl://` deep link (issue #98). The
   * backend already parsed + validated the URL through the shared
   * `outl_actions::parse_deep_link` and focused the window; here we just
   * map the resulting shape onto the same `open*` command the picker /
   * sidebar use, then render it. A failed open lands on the status line.
   */
  async function handleDeepLink(payload: DeepLinkNavigate) {
    try {
      const view =
        payload.kind === "today"
          ? await openTodayJournal()
          : payload.kind === "daily"
            ? await openJournalFor(payload.date)
            : await openPageBySlug(payload.slug);
      applyView(view);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  onMount(async () => {
    // A cold-start deep link wins over today's journal: if an `outl://`
    // URL launched the app, navigate there instead of loading the
    // journal (which would otherwise race and overwrite the target).
    const pending = await takePendingDeepLink();
    if (pending) {
      void handleDeepLink(pending);
    } else {
      void loadToday();
    }

    const handlers = buildHandlers({ applyView, setError });
    const unbindShortcuts = await installShortcuts(handlers);
    onCleanup(unbindShortcuts);

    const unlisten = await onPeerOpsChanged(() => {
      void onPeerChange();
    });
    onCleanup(() => unlisten());

    // `outl://` deep links opened while the app is running (issue #98).
    const unlistenDeepLink = await onDeepLinkNavigate((payload) => {
      void handleDeepLink(payload);
    });
    onCleanup(() => unlistenDeepLink());
  });

  function gridTemplate(): string {
    // `minmax(0, …)` on every track is what lets the columns
    // shrink below their content's natural width. With a bare
    // `1fr`, a row in the outline carrying a long unbreakable
    // token (`BUSER-DJANGO-KX9`, `((blk-XXXXXX))`) would push the
    // column past the viewport and the `overflow: hidden` on body
    // would clip it — outl rendered with cut-off text in narrow
    // windows. Same pitfall flex has on `min-width: auto`; the
    // grid analogue is here.
    const left = appState.sidebarOpen ? "minmax(0, 220px)" : "0";
    return `${left} minmax(0, 1fr)`;
  }

  return (
    <>
      <div
        // `minmax(0, 1fr)` instead of `1fr` so the second grid
        // column can shrink below its content's natural width.
        // Without this, a row with a long unbreakable token pushes
        // the column past the viewport and outl renders with a
        // horizontal cut-off in narrow windows. CSS grid's
        // `min-width: auto` default is the same pitfall flex hits
        // (`min-w-0` on flex children); the grid analogue is
        // baking `minmax(0, …)` into the template.
        class="grid h-full overflow-hidden"
        style={{ "grid-template-columns": gridTemplate() }}
      >
        <Show when={appState.sidebarOpen} fallback={<div />}>
          <Sidebar onToday={loadToday} onPickPage={applyView} />
        </Show>

        <OutlineView />
      </div>

      <ChromeToggleBar />
      <Picker onPicked={applyView} />
      <SettingsModal />
      <HelpOverlay />
    </>
  );
}
