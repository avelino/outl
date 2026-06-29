/**
 * First-run onboarding for the desktop client.
 *
 * Two honest steps, no filler:
 *
 *   1. **Storage** — reuse the existing `<WorkspacePicker />` (folder
 *      pick via `tauri-plugin-dialog` → `set_workspace`). When a folder
 *      is picked the picker fires `onPicked`, which advances us to the
 *      sync step. The picker also refreshes `<App />`'s workspace gate,
 *      so by the time we finish, `<AppShell />` can mount.
 *   2. **Sync (optional)** — a short, account-free explainer plus the
 *      existing `<SyncPanel />` so the user can pair another device
 *      right here, or skip. A single device is a first-class setup.
 *
 * Onboarding is a pure-UI concern, so "has the user seen it" is a
 * frontend flag (`localStorage`), not workspace state — it must not go
 * through the op log (it's per-install, never converges across devices).
 * `<App />` owns that flag; this component only drives the steps and
 * calls `props.onFinish()` when the user is done.
 *
 * The copy ({@link STORAGE_STEP}, {@link SYNC_STEP}) lives in
 * `@outl/shared/onboarding` so mobile and desktop say the same thing.
 */

import { Show, createSignal } from "solid-js";

import { FINISH_CTA, SYNC_STEP } from "@outl/shared/onboarding";

import { SyncPanel } from "./SyncPanel";
import { WorkspacePicker } from "./WorkspacePicker";

type Step = "storage" | "sync";

export function Onboarding(props: {
  /** Re-run `<App />`'s workspace gate after a folder is picked. */
  onWorkspacePicked: () => void;
  /** Mark onboarding complete and hand off to `<AppShell />`. */
  onFinish: () => void;
}) {
  const [step, setStep] = createSignal<Step>("storage");

  function handlePicked() {
    // Refresh the parent gate so the workspace is "ready" by the time we
    // finish, then advance to the optional sync step.
    props.onWorkspacePicked();
    setStep("sync");
  }

  return (
    <div class="flex h-full flex-col">
      <Show when={step() === "storage"}>
        {/* `<WorkspacePicker />` carries its own brand title + the
            "pick a folder" framing, so the storage copy is already on
            screen — we just advance once a folder is chosen. */}
        <WorkspacePicker onPicked={handlePicked} />
      </Show>

      <Show when={step() === "sync"}>
        <div class="mx-auto flex h-full w-full max-w-md flex-col items-stretch justify-center gap-5 p-8">
          <div class="text-center">
            <h1 class="text-2xl font-semibold">{SYNC_STEP.title}</h1>
            <p class="mt-2 text-sm opacity-70">{SYNC_STEP.body}</p>
          </div>

          <ul class="space-y-1.5 text-sm opacity-70">
            {SYNC_STEP.bullets.map((line) => (
              <li class="flex gap-2">
                <span aria-hidden="true" class="opacity-60">
                  ·
                </span>
                <span>{line}</span>
              </li>
            ))}
          </ul>

          {/* Reuse the real pairing UI — no parallel implementation. The
              user can pair here or just continue. */}
          <SyncPanel />

          <button
            type="button"
            onClick={() => props.onFinish()}
            class="mt-2 rounded-md bg-(--color-outl-accent) px-4 py-2 text-sm font-medium text-(--color-outl-bg) hover:opacity-90"
          >
            {FINISH_CTA}
          </button>
          <button
            type="button"
            onClick={() => props.onFinish()}
            class="text-center text-xs opacity-60 hover:opacity-100"
          >
            {SYNC_STEP.skipCta}
          </button>
        </div>
      </Show>
    </div>
  );
}
