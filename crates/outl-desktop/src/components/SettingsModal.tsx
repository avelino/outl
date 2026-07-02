import { For, Show, createResource, createSignal, onMount } from "solid-js";

import {
  getSettings,
  getTheme,
  listThemes,
  updateSettings,
  type Settings,
} from "../lib/api";
import { applyPaletteToRoot } from "../lib/palette";
import { appState, setAppState } from "../lib/store";
import { SyncPanel } from "./SyncPanel";

/**
 * Settings modal, opened via `Cmd/Ctrl+,`.
 *
 * Edits the desktop-local preferences (vim mode, theme, font size).
 * `last_workspace` is shown read-only — it's swapped through the
 * file picker, not edited as text. The modal is unmounted while
 * `appState.settingsOpen === false` so the resource load only fires
 * the first time the user opens it.
 */
export function SettingsModal() {
  const [draft, setDraft] = createSignal<Settings | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [themes] = createResource(async () => {
    try {
      return await listThemes();
    } catch {
      return [] as string[];
    }
  });

  function close() {
    setAppState("settingsOpen", false);
  }

  onMount(async () => {
    try {
      const s = await getSettings();
      setDraft(s);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
      close();
    }
  });

  /**
   * Apply a theme to the live document as soon as the user picks
   * it from the dropdown, so they can preview without committing.
   * Save (or Cancel) is what persists / reverts.
   */
  async function previewTheme(name: string) {
    try {
      const palette = await getTheme(name);
      applyPaletteToRoot(palette);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  async function save() {
    const d = draft();
    if (!d) return;
    setBusy(true);
    try {
      const persisted = await updateSettings(d);
      setDraft(persisted);
      close();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Show when={appState.settingsOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        <div class="mt-24 flex max-h-[calc(100vh-7rem)] w-[480px] max-w-[90vw] flex-col overflow-hidden rounded-lg border border-(--color-outl-fg)/15 bg-(--color-outl-bg-elev)/95 shadow-2xl">
          <header class="shrink-0 border-b border-(--color-outl-fg)/10 px-5 py-3">
            <h2 class="text-lg font-semibold">Settings</h2>
          </header>

          <Show
            when={draft()}
            fallback={<div class="px-5 py-6 opacity-60">Loading…</div>}
          >
            <div class="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
              <label class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Vim mode</div>
                  <div class="text-xs opacity-60">
                    Modal keybindings inside the block editor. Off by default.
                  </div>
                </div>
                <input
                  type="checkbox"
                  checked={draft()!.vim_mode}
                  onChange={(e) =>
                    setDraft({ ...draft()!, vim_mode: e.currentTarget.checked })
                  }
                  class="h-4 w-4"
                />
              </label>

              <label class="block">
                <div class="mb-1 text-sm font-medium">Theme</div>
                <select
                  value={draft()!.theme}
                  onChange={(e) => {
                    const next = e.currentTarget.value;
                    setDraft({ ...draft()!, theme: next });
                    void previewTheme(next);
                  }}
                  class="w-full rounded border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/5 px-2 py-1 text-sm outline-none focus:border-(--color-outl-fg)/30"
                >
                  <For each={themes() ?? []}>
                    {(name) => <option value={name}>{name}</option>}
                  </For>
                </select>
                <div class="mt-1 text-xs opacity-50">
                  Live preview — pick to see, Save to persist, Cancel to revert.
                </div>
              </label>

              <label class="block">
                <div class="mb-1 text-sm font-medium">Font size (px)</div>
                <input
                  type="number"
                  min={10}
                  max={32}
                  value={draft()!.font_size}
                  onInput={(e) =>
                    setDraft({
                      ...draft()!,
                      font_size: Number(e.currentTarget.value) || 15,
                    })
                  }
                  class="w-24 rounded border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/5 px-2 py-1 text-sm outline-none focus:border-(--color-outl-fg)/30"
                />
              </label>

              <div>
                <div class="text-sm font-medium">Workspace</div>
                <div class="mt-1 truncate rounded bg-(--color-outl-fg)/5 px-2 py-1 font-mono text-xs opacity-70">
                  {draft()!.last_workspace ?? "(none)"}
                </div>
                <div class="mt-1 text-xs opacity-50">
                  Use File → Switch workspace… to change.
                </div>
              </div>

              <label class="block">
                <div class="mb-1 text-sm font-medium">Sync transport</div>
                <select
                  value={draft()!.sync_transport}
                  onChange={(e) =>
                    setDraft({
                      ...draft()!,
                      sync_transport: e.currentTarget.value,
                    })
                  }
                  class="w-full rounded border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/5 px-2 py-1 text-sm outline-none focus:border-(--color-outl-fg)/30"
                >
                  <option value="iroh">iroh — direct P2P (default)</option>
                  <option value="file">file — iCloud / shared folder</option>
                </select>
                <div class="mt-1 text-xs opacity-50">
                  iroh syncs device-to-device over QUIC; file relies on a synced
                  folder. Takes effect after the app restarts.
                </div>
              </label>

              <SyncPanel />
            </div>
          </Show>

          <footer class="flex shrink-0 justify-end gap-2 border-t border-(--color-outl-fg)/10 px-5 py-3">
            <button
              type="button"
              onClick={close}
              class="rounded px-3 py-1 text-sm opacity-70 hover:opacity-100"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={busy() || !draft()}
              class="rounded bg-(--color-outl-fg)/15 px-3 py-1 text-sm font-medium hover:bg-(--color-outl-fg)/25 disabled:opacity-50"
            >
              {busy() ? "Saving…" : "Save"}
            </button>
          </footer>
        </div>
      </div>
    </Show>
  );
}
