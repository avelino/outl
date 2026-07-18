import { For, createSignal, onMount, type JSX } from "solid-js";

import { pluginRun, pluginToolbar } from "@outl/shared/api/commands";
import type { PluginToolbarButton as ToolbarButtonEntry } from "@outl/shared/api/types";
import { playPluginViews } from "../lib/plugin-views";
import { appState, setAppState } from "../lib/store";
import { SyncIndicator } from "./SyncIndicator";

/**
 * Small chrome toggle button (sidebar / shortcuts help).
 *
 * These surface keyboard chords that are otherwise invisible
 * (`Cmd/Ctrl+Shift+E` for the sidebar, `?` / `Cmd/Ctrl+/` for the
 * help overlay). They carry no business logic — clicking flips the
 * same store signal the `outl-shortcuts` dispatcher flips, so the
 * button and the keyboard stay in sync automatically.
 */
/** Puzzle-piece icon (lucide), the conventional "plugins" glyph. Inherits
 *  the button's `currentColor` so it follows the active/inactive state. */
function PuzzleIcon() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M15.39 4.39a1 1 0 0 0 1.68-.474 2.5 2.5 0 1 1 3.014 3.015 1 1 0 0 0-.474 1.68l1.683 1.682a2.414 2.414 0 0 1 0 3.414L19.61 16.39a1 1 0 0 1-1.68-.474 2.5 2.5 0 1 0-3.014 3.015 1 1 0 0 1 .474 1.68l-1.683 1.682a2.414 2.414 0 0 1-3.414 0L8.61 20.61a1 1 0 0 0-1.68.474 2.5 2.5 0 1 1-3.014-3.015 1 1 0 0 0 .474-1.68l-1.683-1.682a2.414 2.414 0 0 1 0-3.414L4.39 9.61a1 1 0 0 1 1.68.474 2.5 2.5 0 1 0 3.014-3.015 1 1 0 0 1-.474-1.68l1.683-1.682a2.414 2.414 0 0 1 3.414 0z" />
    </svg>
  );
}

function ChromeToggle(props: {
  glyph: JSX.Element;
  active: boolean;
  label: string;
  title: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      aria-pressed={props.active}
      title={props.title}
      onClick={props.onToggle}
      class={`flex h-7 w-7 items-center justify-center rounded-md text-[14px] leading-none transition-colors ${
        props.active
          ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
          : "text-(--color-outl-fg-dim) hover:bg-(--color-outl-bg-elev) hover:text-(--color-outl-fg)"
      }`}
    >
      <span aria-hidden="true">{props.glyph}</span>
    </button>
  );
}

/**
 * Plugin-contributed toolbar button.
 *
 * A `toolbar-button` capability plugin contributes a glyph + command; we
 * render it next to the native chrome toggles and run the command on
 * click (same path the plugin palette / keybinding take: surface the
 * plugin's output on the status line, re-render the page if it mutated,
 * and play any `ui-render` overlays). Momentary action, never a toggle —
 * `active` is always false.
 */
function PluginToolbarButton(props: { entry: ToolbarButtonEntry }) {
  const [busy, setBusy] = createSignal(false);

  async function run() {
    if (busy()) return;
    setBusy(true);
    try {
      const reply = await pluginRun(
        props.entry.plugin_id,
        props.entry.command_id,
        appState.page?.id ?? null,
      );
      for (const note of reply.notifications) setAppState("lastError", note);
      for (const err of reply.errors)
        setAppState("lastError", `plugin: ${err}`);
      if (reply.view) {
        setAppState({
          page: reply.view.page,
          outline: reply.view.outline,
        });
      }
      playPluginViews(reply.views);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const label = () =>
    props.entry.title ?? `${props.entry.plugin_id} · ${props.entry.command_id}`;

  return (
    <button
      type="button"
      aria-label={label()}
      title={label()}
      disabled={busy()}
      onClick={() => void run()}
      class="flex h-7 w-7 items-center justify-center rounded-md text-[14px] leading-none text-(--color-outl-fg-dim) transition-colors hover:bg-(--color-outl-bg-elev) hover:text-(--color-outl-fg) disabled:opacity-50"
    >
      <span aria-hidden="true">{props.entry.icon}</span>
    </button>
  );
}

/**
 * Bottom-left chrome cluster — sidebar + shortcuts-help toggles.
 *
 * Pinned to the lower-left corner of the window (VS Code's activity-bar
 * convention) so the affordances are always in the same place, independent
 * of which page or pane is open. The sidebar toggle stays reachable here
 * even after the left pane is hidden because the cluster floats over the
 * main pane, not inside the sidebar.
 *
 * The cluster sits on an elevated, bordered surface so it reads with clear
 * contrast against the page content behind it; the active toggle inverts to
 * the accent color for an unmistakable on/off state.
 */
export function ChromeToggleBar() {
  const [toolbar, setToolbar] = createSignal<ToolbarButtonEntry[]>([]);

  // Plugins load lazily on the host's first request once the workspace is
  // open, so a fetch at mount may return empty on a cold boot. Best-effort:
  // a failure leaves the cluster with only the native toggles.
  onMount(() => {
    void (async () => {
      try {
        setToolbar(await pluginToolbar());
      } catch {
        setToolbar([]);
      }
    })();
  });

  return (
    <div class="fixed bottom-3 left-3 z-20 flex items-center gap-1 rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) p-1 shadow-lg">
      <ChromeToggle
        glyph="◫"
        active={appState.sidebarOpen}
        label={appState.sidebarOpen ? "Hide sidebar" : "Show sidebar"}
        title="Toggle sidebar (⌘⇧E)"
        onToggle={() => setAppState("sidebarOpen", !appState.sidebarOpen)}
      />
      <ChromeToggle
        glyph="?"
        active={appState.helpOpen}
        label={
          appState.helpOpen
            ? "Hide keyboard shortcuts"
            : "Show keyboard shortcuts"
        }
        title="Keyboard shortcuts (?)"
        onToggle={() => setAppState("helpOpen", !appState.helpOpen)}
      />
      <ChromeToggle
        glyph={<PuzzleIcon />}
        active={appState.marketplaceOpen}
        label={appState.marketplaceOpen ? "Hide marketplace" : "Browse plugins"}
        title="Plugins"
        onToggle={() =>
          setAppState("marketplaceOpen", !appState.marketplaceOpen)
        }
      />
      {/* Plugin-contributed toolbar buttons sit in the same cluster as the
          native toggles (⧉/◫/?), one per `toolbar-button` contribution. */}
      <For each={toolbar()}>
        {(entry) => <PluginToolbarButton entry={entry} />}
      </For>
      {/* Thin divider, then the always-visible sync status dot. Clicking
          it opens Settings → Sync (peer list + pairing). */}
      <span class="mx-0.5 h-4 w-px bg-(--color-outl-border)" aria-hidden="true" />
      <SyncIndicator />
    </div>
  );
}
