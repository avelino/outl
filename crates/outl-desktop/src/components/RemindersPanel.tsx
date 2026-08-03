/**
 * Reminders panel — every block in the workspace carrying a
 * `remind::`, grouped by when it next fires.
 *
 * Open with `Cmd/Ctrl+Shift+R` (Global) or `g n` in Normal mode; close
 * with `Esc`. Read-only apart from three actions per row: snooze (1h /
 * tomorrow 9am / next week), mark done, and jump to the block.
 *
 * **Nothing about the schedule is computed here.** The grouping labels
 * and the "in 3h" column come from `@outl/shared` (`groupReminders`,
 * `formatNextFire`), and the underlying instants come from
 * `outl_actions::reminders` in Rust. A second opinion in TS about when
 * a reminder fires is exactly the drift that reaches the user first.
 *
 * Snooze writes `Op::SnoozeRemind`, so it silences the block on every
 * paired device — not just this laptop.
 */

import { For, Show, createResource, createSignal } from "solid-js";

import {
  clearReminderSnooze,
  formatNextFire,
  groupReminders,
  listReminders,
  openPageBySlug,
  reminderSettings,
  snoozePresets,
  snoozeReminder,
  toggleTodo,
} from "@outl/shared/api/commands";
import type { PageView, Reminder } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";

export interface RemindersPanelDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

export function RemindersPanel(props: RemindersPanelDeps) {
  // `refetch` is the only refresh path: every mutation below awaits its
  // command and then re-reads, so the list can never show a snooze the
  // op log didn't take.
  const [reminders, { refetch }] = createResource(async () => {
    try {
      return await listReminders();
    } catch (e) {
      props.setError(e instanceof Error ? e.message : String(e));
      return [] as Reminder[];
    }
  });
  const [settings] = createResource(reminderSettings);
  // Fetched, not hardcoded: "tomorrow 9am" is a wall time, so the
  // backend owns resolution and we only render its labels.
  const [presets] = createResource(snoozePresets);
  const [busy, setBusy] = createSignal<string | null>(null);

  async function withRow(id: string, run: () => Promise<unknown>) {
    setBusy(id);
    try {
      await run();
      await refetch();
    } catch (e) {
      props.setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function jumpTo(r: Reminder) {
    const view = await openPageBySlug(r.page_slug);
    props.applyView(view);
    setAppState("selectedBlockId", r.block_id);
    setAppState("remindersOpen", false);
  }

  /**
   * Mark a reminder's block DONE, which cancels every pending fire.
   *
   * Resolves the block's **own** page id first: the panel lists the
   * whole workspace, so `appState.page` is usually a different page,
   * and `toggle_todo` uses the page id to render the reply + queue the
   * projection. Passing the wrong one re-projects the wrong page.
   * The view is only applied when the user is actually looking at that
   * page — ticking a row shouldn't teleport them.
   */
  async function markDone(r: Reminder) {
    const target = await openPageBySlug(r.page_slug);
    const after = await toggleTodo(target.page.id, r.block_id);
    if (appState.page?.slug === r.page_slug) props.applyView(after);
  }

  return (
    <Show when={appState.remindersOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={() => setAppState("remindersOpen", false)}
      >
        <div
          class="mt-16 max-h-[80vh] w-[680px] max-w-[92vw] overflow-hidden rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <header class="flex items-baseline justify-between border-b border-(--color-outl-border) px-5 py-3">
            <h2 class="text-lg font-semibold">Reminders</h2>
            <span class="text-xs opacity-50">Esc to close</span>
          </header>

          {/* An empty list means two very different things — "nothing
              scheduled" and "this device never delivers". Saying which
              is the difference between a feature and a bug report. */}
          <Show when={settings()?.enabled === false}>
            <p class="border-b border-(--color-outl-border) px-5 py-2 text-xs opacity-70">
              Notifications are off on this device. The rules below are
              still tracked — turn delivery on in Settings.
            </p>
          </Show>

          <div class="max-h-[64vh] overflow-y-auto px-5 py-3">
            <Show
              when={(reminders() ?? []).length > 0}
              fallback={
                <p class="py-6 text-center text-sm opacity-60">
                  No reminders yet. Add <code class="font-mono">remind:: 3pm</code>{" "}
                  to a block, or press <kbd>g r</kbd> on one.
                </p>
              }
            >
              <For each={groupReminders(reminders() ?? [])}>
                {(group) => (
                  <section class="mb-5">
                    <h3 class="mb-2 text-sm font-semibold uppercase tracking-wide text-(--color-outl-help-title-fg)">
                      {group.label}
                    </h3>
                    <For each={group.items}>
                      {(r) => (
                        <div
                          class="flex items-start gap-3 border-b border-(--color-outl-border)/40 py-2 last:border-b-0"
                          classList={{ "opacity-50": r.done }}
                        >
                          <div class="min-w-0 flex-1">
                            <button
                              type="button"
                              class="block w-full truncate text-left text-sm hover:underline"
                              onClick={() => void jumpTo(r)}
                            >
                              {r.text || "(empty block)"}
                            </button>
                            <div class="mt-0.5 flex gap-2 text-xs opacity-60">
                              <span>{r.page_title}</span>
                              <span class="font-mono">{r.rule}</span>
                              <Show when={r.snoozed_until}>
                                <span>· snoozed</span>
                              </Show>
                            </div>
                          </div>
                          {/* Overdue reads very differently from
                              upcoming; every task app paints it. */}
                          <span
                            class="shrink-0 pt-0.5 text-xs"
                            classList={{
                              "text-red-500 font-medium": r.urgency === "overdue",
                              "opacity-70": r.urgency !== "overdue",
                            }}
                          >
                            {formatNextFire(r.next_fire)}
                          </span>
                          <div class="flex shrink-0 gap-1">
                            <Show when={!r.done}>
                              <For each={presets() ?? []}>
                                {(p) => (
                                  <button
                                    type="button"
                                    class="rounded px-1.5 py-0.5 text-xs opacity-70 hover:bg-(--color-outl-border) hover:opacity-100"
                                    disabled={busy() === r.block_id}
                                    title={`Snooze ${p.label} (every device)`}
                                    onClick={() =>
                                      void withRow(r.block_id, () =>
                                        snoozeReminder(r.block_id, p.id),
                                      )
                                    }
                                  >
                                    {p.label}
                                  </button>
                                )}
                              </For>
                            </Show>
                            <Show when={r.snoozed_until}>
                              <button
                                type="button"
                                class="rounded px-1.5 py-0.5 text-xs opacity-70 hover:bg-(--color-outl-border) hover:opacity-100"
                                disabled={busy() === r.block_id}
                                title="Resume now"
                                onClick={() =>
                                  void withRow(r.block_id, () =>
                                    clearReminderSnooze(r.block_id),
                                  )
                                }
                              >
                                ↺
                              </button>
                            </Show>
                            <Show when={!r.done}>
                              <button
                                type="button"
                                class="rounded px-1.5 py-0.5 text-xs opacity-70 hover:bg-(--color-outl-border) hover:opacity-100"
                                disabled={busy() === r.block_id}
                                title="Mark done (cancels the reminder)"
                                onClick={() =>
                                  void withRow(r.block_id, () => markDone(r))
                                }
                              >
                                ✓
                              </button>
                            </Show>
                          </div>
                        </div>
                      )}
                    </For>
                  </section>
                )}
              </For>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
