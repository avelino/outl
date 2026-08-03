import { For, JSX, Show, createEffect, createSignal } from "solid-js";

import type { PageView, Reminder } from "@outl/shared/api/types";
import {
  clearReminderSnooze,
  formatNextFire,
  groupReminders,
  listReminders,
  openPageBySlug,
  reminderSettings,
  snoozeReminder,
} from "@outl/shared/api/commands";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";

interface RemindersSheetProps {
  open: boolean;
  onClose: () => void;
  /** Toast channel for backend errors. */
  onMessage: (text: string) => void;
  /** Refreshed page view after navigating to a reminder's block. */
  onView: (view: PageView) => void;
}

/** Snooze presets, in minutes. Same set as the desktop panel and the
 *  OS banner actions, so the choice doesn't change per surface. */
const SNOOZE_PRESETS: Array<{ label: string; minutes: number }> = [
  { label: "1h", minutes: 60 },
  { label: "Tomorrow", minutes: 60 * 24 },
  { label: "Next week", minutes: 60 * 24 * 7 },
];

/**
 * Bottom sheet listing every block with a `remind::`, grouped Today /
 * Tomorrow / This week / Later / Done.
 *
 * The grouping and the "in 3h" column come from `@outl/shared`
 * (`groupReminders`, `formatNextFire`) — the same functions the desktop
 * panel uses — and the instants behind them come from
 * `outl_actions::reminders` in Rust. Nothing about *when* a reminder
 * fires is decided in this file.
 *
 * Snooze writes `Op::SnoozeRemind`, so silencing a nag here also
 * silences it on the user's laptop.
 */
export function RemindersSheet(props: RemindersSheetProps): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [reminders, setReminders] = createSignal<Reminder[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [enabled, setEnabled] = createSignal(true);
  const [busy, setBusy] = createSignal<string | null>(null);

  async function refresh() {
    setLoading(true);
    try {
      const [list, settings] = await Promise.all([
        listReminders(),
        reminderSettings(),
      ]);
      setReminders(list);
      setEnabled(settings.enabled);
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
      setReminders([]);
    } finally {
      setLoading(false);
    }
  }

  // Re-read every time the sheet opens: a peer's snooze, or the clock
  // simply moving, changes what is due.
  createEffect(() => {
    if (!props.open) return;
    void refresh();
  });

  async function withRow(id: string, run: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(id);
    haptic("light");
    try {
      await run();
      await refresh();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function jumpTo(r: Reminder) {
    try {
      // Navigate to the page; the reminder's block is somewhere in it.
      // Scrolling to the exact block would mean reusing `focusBlockId`,
      // which is the *zoom* root on mobile — overloading it here would
      // silently zoom the user into a single bullet.
      const view = await openPageBySlug(r.page_slug);
      props.onView(view);
      props.onClose();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex flex-col"
        style={{
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-ios-card)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl dark:bg-(--color-iosd-card)/95 dark:shadow-[var(--shadow-capsule-dark)]">
          <span
            class="block py-2"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-ios-divider) dark:bg-(--color-iosd-divider)"
            />
          </span>

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              Reminders
            </span>
          </div>

          {/* An empty list means two different things — "nothing
              scheduled" and "this device never delivers". Say which. */}
          <Show when={!enabled()}>
            <div class="border-t border-(--color-ios-divider)/30 px-4 py-2 text-[12px] text-(--color-ios-text-secondary) dark:border-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
              Notifications are off on this device. The rules below are still
              tracked.
            </div>
          </Show>

          <div class="max-h-[60vh] overflow-y-auto">
            <For each={groupReminders(reminders())}>
              {(group) => (
                <>
                  <div class="border-t border-(--color-ios-divider)/30 bg-(--color-ios-divider)/10 px-4 py-1.5 text-[12px] font-semibold uppercase tracking-wide text-(--color-ios-text-secondary) dark:border-(--color-iosd-divider)/30 dark:bg-(--color-iosd-divider)/10 dark:text-(--color-iosd-text-secondary)">
                    {group.label}
                  </div>
                  <For each={group.items}>
                    {(r) => (
                      <div
                        class="border-t border-(--color-ios-divider)/30 px-4 py-3 dark:border-(--color-iosd-divider)/30"
                        classList={{ "opacity-50": r.done }}
                      >
                        <button
                          type="button"
                          class="flex w-full items-baseline gap-2 text-left"
                          onClick={() => void jumpTo(r)}
                        >
                          <span class="min-w-0 flex-1 truncate text-[16px] text-(--color-ios-text) dark:text-(--color-iosd-text)">
                            {r.text || "(empty block)"}
                          </span>
                          <span class="shrink-0 text-[12px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                            {formatNextFire(r.next_fire)}
                          </span>
                        </button>
                        <div class="mt-0.5 flex gap-2 text-[11px] text-(--color-ios-text-secondary)/80 dark:text-(--color-iosd-text-secondary)/80">
                          <span class="truncate">{r.page_title}</span>
                          <span class="font-mono">{r.rule}</span>
                        </div>
                        <Show when={!r.done}>
                          <div class="mt-2 flex flex-wrap gap-1.5">
                            <For each={SNOOZE_PRESETS}>
                              {(p) => (
                                <button
                                  type="button"
                                  disabled={busy() === r.block_id}
                                  class="rounded-full bg-(--color-ios-divider)/40 px-2.5 py-1 text-[12px] text-(--color-ios-text) active:opacity-60 disabled:opacity-40 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                                  onClick={() =>
                                    void withRow(r.block_id, () =>
                                      snoozeReminder(r.block_id, p.minutes),
                                    )
                                  }
                                >
                                  {p.label}
                                </button>
                              )}
                            </For>
                            <Show when={r.snoozed_until}>
                              <button
                                type="button"
                                disabled={busy() === r.block_id}
                                class="rounded-full bg-(--color-ios-divider)/40 px-2.5 py-1 text-[12px] text-(--color-ios-text) active:opacity-60 disabled:opacity-40 dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text)"
                                onClick={() =>
                                  void withRow(r.block_id, () =>
                                    clearReminderSnooze(r.block_id),
                                  )
                                }
                              >
                                Resume
                              </button>
                            </Show>
                          </div>
                        </Show>
                      </div>
                    )}
                  </For>
                </>
              )}
            </For>

            <Show when={!loading() && reminders().length === 0}>
              <div class="border-t border-(--color-ios-divider)/30 px-4 py-6 text-center text-[14px] text-(--color-ios-text-secondary) dark:border-(--color-iosd-divider)/30 dark:text-(--color-iosd-text-secondary)">
                No reminders yet. Long-press a TODO and pick{" "}
                <span class="font-medium">Remind me…</span>
              </div>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
