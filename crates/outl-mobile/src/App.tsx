import { Show, createSignal } from "solid-js";

import { Journal } from "./components/Journal";
import { Onboarding } from "./components/Onboarding";

/** `localStorage` key for the first-run flag (pure UI state — never an Op). */
const ONBOARDED_KEY = "outl.onboarded";

/**
 * Whether the user has completed (or skipped) first-run onboarding.
 *
 * This is a per-install UI flag, not workspace state, so it lives in
 * `localStorage` and deliberately does NOT go through the op log — it
 * must not converge across devices (each device onboards once).
 *
 * Mobile has no "is a workspace chosen?" backend gate (a fresh install
 * always resolves *a* root — the local default), so the first-run flag
 * is the only signal that distinguishes a brand-new install from a
 * returning one.
 */
function hasOnboarded(): boolean {
  try {
    return localStorage.getItem(ONBOARDED_KEY) === "1";
  } catch {
    return false;
  }
}

function markOnboarded() {
  try {
    localStorage.setItem(ONBOARDED_KEY, "1");
  } catch {
    // Private mode / disabled storage — onboarding re-shows next launch.
    // Harmless, never blocks the app.
  }
}

function App() {
  const [onboarded, setOnboarded] = createSignal(hasOnboarded());

  function finishOnboarding() {
    markOnboarded();
    setOnboarded(true);
  }

  return (
    <div class="flex h-full flex-col bg-(--color-ios-bg) dark:bg-(--color-iosd-bg)">
      <Show
        when={onboarded()}
        fallback={<Onboarding onFinish={finishOnboarding} />}
      >
        <Journal />
      </Show>
    </div>
  );
}

export default App;
