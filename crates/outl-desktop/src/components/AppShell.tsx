import { Show, onCleanup, onMount } from "solid-js";

import {
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  reloadWorkspace,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";
import { workspaceStats } from "../lib/api";
import { onPeerOpsChanged } from "../lib/events";
import { installShortcuts } from "../lib/shortcuts";
import { buildHandlers } from "../lib/action-handlers";
import { Sidebar } from "./Sidebar";
import { OutlineView } from "./OutlineView";
import { Picker } from "./Picker";
import { SettingsModal } from "./SettingsModal";
import { HelpOverlay } from "./HelpOverlay";

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

  onMount(async () => {
    void loadToday();

    const handlers = buildHandlers({ applyView, setError });
    const unbindShortcuts = await installShortcuts(handlers);
    onCleanup(unbindShortcuts);

    const unlisten = await onPeerOpsChanged(() => {
      void onPeerChange();
    });
    onCleanup(() => unlisten());
  });

  function gridTemplate(): string {
    const left = appState.sidebarOpen ? "220px" : "0";
    return `${left} 1fr`;
  }

  return (
    <>
      <div
        class="grid h-full overflow-hidden"
        style={{ "grid-template-columns": gridTemplate() }}
      >
        <Show when={appState.sidebarOpen} fallback={<div />}>
          <Sidebar onToday={loadToday} onPickPage={applyView} />
        </Show>

        <OutlineView />
      </div>

      <Picker onPicked={applyView} />
      <SettingsModal />
      <HelpOverlay />
    </>
  );
}
