import { For, Show, createResource, onCleanup, onMount } from "solid-js";

import { listShortcutBindings, type Binding, type ShortcutMode } from "../lib/api";
import { formatSequence } from "../lib/chord-format";
import { appState, setAppState } from "../lib/store";

/**
 * Help overlay — lists every binding from the shared
 * `outl-shortcuts` catalog grouped by mode. Mirrors the TUI's
 * help popup so a user moving between clients sees the same
 * chord reference.
 *
 * Open with `?` in Normal mode (or via the command palette in a
 * future iteration). Close with `Esc` (routed through
 * `Action::ExitInsert` in the action handlers).
 *
 * Bindings are fetched on first open and cached via the resource —
 * they don't change at runtime today, so this never re-fires.
 */
const MODE_LABELS: Array<{ id: ShortcutMode; label: string; hint?: string }> = [
  { id: "global", label: "Global", hint: "Fires in every mode" },
  { id: "normal", label: "Normal", hint: "Outside textareas (vim-style)" },
  { id: "insert", label: "Insert", hint: "Inside a block's textarea" },
  { id: "visual", label: "Visual", hint: "Range selection" },
  { id: "overlay", label: "Overlay", hint: "Picker / Settings / Help open" },
];

export function HelpOverlay() {
  const [bindings] = createResource(async () => {
    try {
      return await listShortcutBindings();
    } catch {
      return [] as Binding[];
    }
  });

  function close() {
    setAppState("helpOpen", false);
  }

  function groupedBy(mode: ShortcutMode): Binding[] {
    return (bindings() ?? []).filter((b) => b.mode === mode);
  }

  onMount(() => {
    // Esc-to-close is also handled by the shortcut dispatcher
    // (ExitInsert action), but we add a direct listener too in
    // case focus drifted somewhere the dispatcher skips.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && appState.helpOpen) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    onCleanup(() => window.removeEventListener("keydown", onKey, { capture: true } as EventListenerOptions));
  });

  return (
    <Show when={appState.helpOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        <div class="mt-16 max-h-[80vh] w-[720px] max-w-[92vw] overflow-hidden rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) shadow-2xl">
          <header class="flex items-baseline justify-between border-b border-(--color-outl-border) px-5 py-3">
            <h2 class="text-lg font-semibold">Keyboard shortcuts</h2>
            <span class="text-xs opacity-50">Esc to close</span>
          </header>

          <div class="max-h-[68vh] overflow-y-auto px-5 py-3">
            <For each={MODE_LABELS}>
              {(mode) => {
                const entries = groupedBy(mode.id);
                return (
                  <Show when={entries.length > 0}>
                    <section class="mb-5">
                      <div class="mb-2 flex items-baseline gap-2">
                        <h3 class="text-sm font-semibold uppercase tracking-wide text-(--color-outl-help-title-fg)">
                          {mode.label}
                        </h3>
                        <Show when={mode.hint}>
                          <span class="text-xs opacity-50">— {mode.hint}</span>
                        </Show>
                      </div>
                      <table class="w-full table-fixed border-collapse text-sm">
                        <colgroup>
                          <col class="w-[180px]" />
                          <col />
                        </colgroup>
                        <tbody>
                          <For each={entries}>
                            {(b) => (
                              <tr class="border-b border-(--color-outl-border)/40 last:border-b-0">
                                <td class="py-1.5 pr-3 align-top font-mono text-xs text-(--color-outl-accent)">
                                  {formatSequence(b.chord)}
                                </td>
                                <td class="py-1.5 align-top opacity-90">
                                  {b.description}
                                </td>
                              </tr>
                            )}
                          </For>
                        </tbody>
                      </table>
                    </section>
                  </Show>
                );
              }}
            </For>
          </div>

          <footer class="border-t border-(--color-outl-border) px-5 py-2 text-xs opacity-50">
            From <span class="font-mono">outl-shortcuts</span> · shared with the TUI · `?` (Normal) or ⌘/ toggles
          </footer>
        </div>
      </div>
    </Show>
  );
}
