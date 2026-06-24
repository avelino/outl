/**
 * First-run onboarding for the mobile client.
 *
 * Three honest steps, no filler:
 *
 *   1. **Storage** — "Where do your notes live?" Keep on this device
 *      (the local default, recommended) vs store in iCloud. The iCloud
 *      option only appears when the device is actually signed into
 *      iCloud (`pickInICloud()` resolves a path); otherwise it is
 *      hidden, never shown as a dead button. Either choice persists via
 *      `setWorkspace`.
 *   2. **Sync (optional)** — a short, account-free explainer plus a
 *      button into the existing `<DevicesSheet />` pairing flow. Fully
 *      skippable: a single device works fine.
 *   3. Hand off to today's journal via `props.onFinish()`.
 *
 * Onboarding is a pure-UI concern, so "has the user seen it" is a
 * frontend flag (`localStorage` in `App.tsx`), not workspace state — it
 * must not go through the op log (per-install, never converges across
 * devices).
 *
 * The copy ({@link STORAGE_STEP}, {@link SYNC_STEP}, {@link FINISH_CTA})
 * lives in `@outl/shared/onboarding` so mobile and desktop say the same
 * thing; the bottom-sheet chrome + haptics stay here.
 *
 * Storage facts the flow respects (see `src-tauri/src/workspace_picker.rs`):
 *   - `setWorkspace` is **boot-read** — the chosen folder takes effect on
 *     the next launch. We surface that as a one-line "restart to apply"
 *     note when the user picks iCloud, instead of pretending the swap is
 *     instant. The local default is already what a fresh install opened,
 *     so "Keep on this device" needs no restart.
 *   - The arbitrary-folder native picker is deferred, so the only two
 *     choices today are the local default and the iCloud container.
 */

import { Show, createSignal, onMount } from "solid-js";

import { FINISH_CTA, STORAGE_STEP, SYNC_STEP } from "@outl/shared/onboarding";

import { pickInICloud, setWorkspace } from "../lib/api";
import { haptic } from "../lib/haptics";
import { DevicesSheet } from "./DevicesSheet";

type Step = "storage" | "sync";

export function Onboarding(props: { onFinish: () => void }) {
  const [step, setStep] = createSignal<Step>("storage");
  const [icloudPath, setIcloudPath] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [needsRestart, setNeedsRestart] = createSignal(false);
  const [devicesOpen, setDevicesOpen] = createSignal(false);

  // Probe iCloud availability once: a `null` path means the device isn't
  // signed into iCloud, so we hide that option entirely (no dead button).
  onMount(async () => {
    try {
      setIcloudPath(await pickInICloud());
    } catch {
      // Treat any probe failure as "iCloud unavailable" — local still works.
      setIcloudPath(null);
    }
  });

  /** Keep the workspace on this device (the local default a fresh install
   *  already opened). No `setWorkspace` call needed — just advance. */
  function chooseLocal() {
    haptic("light");
    setError(null);
    setNeedsRestart(false);
    setStep("sync");
  }

  /** Move the workspace into the iCloud container. Boot-read, so we flag a
   *  restart instead of pretending the swap is live. */
  async function chooseICloud() {
    const path = icloudPath();
    if (!path || busy()) return;
    haptic("medium");
    setError(null);
    setBusy(true);
    try {
      await setWorkspace(path);
      setNeedsRestart(true);
      setStep("sync");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function finish() {
    haptic("success");
    props.onFinish();
  }

  return (
    <div
      class="flex h-full w-full flex-col bg-(--color-ios-bg) dark:bg-(--color-iosd-bg)"
      style={{ "padding-top": "max(env(safe-area-inset-top), 16px)" }}
    >
      <div
        class="flex flex-1 flex-col px-6 py-8"
        style={{ "padding-bottom": "max(env(safe-area-inset-bottom), 16px)" }}
      >
        {/* ---- Step 1: storage ---- */}
        <Show when={step() === "storage"}>
          <div class="flex flex-1 flex-col">
            <h1 class="mb-2 text-[28px] font-bold leading-tight">
              {STORAGE_STEP.title}
            </h1>
            <p class="mb-8 text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              {STORAGE_STEP.body}
            </p>

            <div class="flex flex-col gap-3">
              {/* Recommended: local default. */}
              <button
                type="button"
                onClick={chooseLocal}
                disabled={busy()}
                class="flex w-full flex-col items-start gap-1 rounded-2xl bg-(--color-ios-accent) px-5 py-4 text-left active:opacity-80 disabled:opacity-50 dark:bg-(--color-iosd-accent)"
              >
                <span class="text-[17px] font-semibold text-white">
                  Keep on this device
                </span>
                <span class="text-[13px] text-white/80">
                  Recommended — works offline, syncs peer-to-peer
                </span>
              </button>

              {/* Opt-in: iCloud, only when the device is signed in. */}
              <Show when={icloudPath()}>
                <button
                  type="button"
                  onClick={() => void chooseICloud()}
                  disabled={busy()}
                  class="flex w-full flex-col items-start gap-1 rounded-2xl border border-(--color-ios-divider) px-5 py-4 text-left active:opacity-60 disabled:opacity-50 dark:border-(--color-iosd-divider)"
                >
                  <span class="text-[17px] font-semibold text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                    Store in iCloud
                  </span>
                  <span class="text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                    {busy()
                      ? "Saving…"
                      : "Also available across your iCloud devices"}
                  </span>
                </button>
              </Show>
            </div>

            <Show when={error()}>
              <div
                role="alert"
                class="mt-4 rounded-xl bg-red-500/15 px-4 py-3 text-[14px] text-red-500 dark:text-red-300"
              >
                {error()}
              </div>
            </Show>
          </div>
        </Show>

        {/* ---- Step 2: optional sync ---- */}
        <Show when={step() === "sync"}>
          <div class="flex flex-1 flex-col">
            <h1 class="mb-2 text-[28px] font-bold leading-tight">
              {SYNC_STEP.title}
            </h1>
            <p class="mb-5 text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              {SYNC_STEP.body}
            </p>

            <ul class="mb-6 flex flex-col gap-2.5">
              {SYNC_STEP.bullets.map((line) => (
                <li class="flex gap-2.5 text-[14px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                  <span
                    aria-hidden="true"
                    class="text-(--color-ios-accent) dark:text-(--color-iosd-accent)"
                  >
                    ✓
                  </span>
                  <span>{line}</span>
                </li>
              ))}
            </ul>

            <Show when={needsRestart()}>
              <div class="mb-6 rounded-xl bg-(--color-ios-card)/60 px-4 py-3 text-[13px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-card)/60 dark:text-(--color-iosd-text-secondary)">
                Your iCloud folder will be active after you restart outl.
              </div>
            </Show>

            <div class="mt-auto flex flex-col gap-3">
              {/* Open the existing pairing flow — never reimplemented here. */}
              <button
                type="button"
                onClick={() => {
                  haptic("light");
                  setDevicesOpen(true);
                }}
                class="w-full rounded-xl border border-(--color-ios-divider) px-4 py-3 text-[16px] font-medium text-(--color-ios-accent) active:opacity-60 dark:border-(--color-iosd-divider) dark:text-(--color-iosd-accent)"
              >
                {SYNC_STEP.pairCta}
              </button>

              <button
                type="button"
                onClick={finish}
                class="w-full rounded-xl bg-(--color-ios-accent) px-4 py-3 text-[16px] font-semibold text-white active:opacity-80 dark:bg-(--color-iosd-accent)"
              >
                {FINISH_CTA}
              </button>

              <button
                type="button"
                onClick={finish}
                class="py-1 text-center text-[14px] text-(--color-ios-text-tertiary) active:opacity-60 dark:text-(--color-iosd-text-tertiary)"
              >
                {SYNC_STEP.skipCta}
              </button>
            </div>
          </div>
        </Show>
      </div>

      {/* Reuse the real pairing sheet (scan QR / show my QR). We only
          flip its `open` prop — its internals are owned elsewhere. */}
      <DevicesSheet
        open={devicesOpen()}
        onClose={() => setDevicesOpen(false)}
      />
    </div>
  );
}
