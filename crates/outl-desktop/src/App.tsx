import { Show, createSignal, onCleanup, onMount } from "solid-js";

import {
  currentWorkspace,
  getSettings,
  getTheme,
  workspaceStats,
} from "./lib/api";
import { onWorkspaceReady } from "./lib/events";
import { applyPaletteToRoot } from "./lib/palette";
import { setAppState } from "./lib/store";
import { AppShell } from "./components/AppShell";
import { WorkspacePicker } from "./components/WorkspacePicker";

/**
 * Root component.
 *
 * Decides between `<WorkspacePicker />` and `<AppShell />` based on
 * whether the backend already has a workspace open. The split is
 * intentional: the user can re-pick a workspace at runtime (from a
 * future "Switch workspace…" menu entry) without remounting the
 * whole app.
 */
function App() {
  const [ready, setReady] = createSignal(false);
  const [checked, setChecked] = createSignal(false);

  async function refresh() {
    try {
      const ws = await currentWorkspace();
      if (ws) {
        const stats = await workspaceStats();
        setAppState("workspace", stats);
        setReady(stats.ready);
      } else {
        setReady(false);
      }
    } finally {
      setChecked(true);
    }
  }

  /**
   * Pull the active palette name from `settings.json` and install
   * it as CSS custom properties on `<html>` + `<body>`. Falls back
   * silently to `outl` when settings aren't readable yet — the
   * default `_root_` CSS already paints the brand background, so a
   * delayed palette load just upgrades the colors.
   */
  async function hydrateTheme() {
    try {
      const s = await getSettings();
      const palette = await getTheme(s.theme || null);
      applyPaletteToRoot(palette);
    } catch {
      // First boot before the workspace lock is created — try the
      // default explicitly so we still ship the brand colors.
      try {
        const palette = await getTheme(null);
        applyPaletteToRoot(palette);
      } catch {
        // No backend at all (shouldn't happen) — keep the static
        // boot frame.
      }
    }
  }

  onMount(async () => {
    // Hydrate the theme BEFORE the rest of the boot routine so the
    // first painted frame already uses the user's chosen palette
    // — running it in parallel left a perceptible flash where
    // `refresh()` rendered with the static defaults from styles.css.
    await hydrateTheme();
    await refresh();

    // Background opener (boot-time) emits `workspace-ready` when it
    // finishes opening `settings.last_workspace`. The picker flow
    // also emits the same event from `set_workspace`.
    const unlisten = await onWorkspaceReady(async () => {
      await refresh();
    });
    onCleanup(() => unlisten());

    // While the background opener is still in flight, poll every
    // 500 ms so the picker → shell transition isn't blocked on a
    // missed event. Cap at 10 attempts.
    if (!ready()) {
      let tries = 0;
      const id = setInterval(async () => {
        tries += 1;
        await refresh();
        if (ready() || tries >= 10) clearInterval(id);
      }, 500);
      onCleanup(() => clearInterval(id));
    }
  });

  return (
    <div class="h-full">
      <Show
        when={checked()}
        fallback={<div class="p-8 opacity-50">Loading…</div>}
      >
        <Show when={ready()} fallback={<WorkspacePicker onPicked={refresh} />}>
          <AppShell />
        </Show>
      </Show>
    </div>
  );
}

export default App;
