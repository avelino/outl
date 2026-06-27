import { For, onCleanup } from "solid-js";

import {
  dismissPluginView,
  pluginViews,
  type PluginView,
} from "../lib/plugin-views";

/**
 * Renders every queued plugin `ui-render` view as a fullscreen,
 * click-through, sandboxed `<iframe>` overlay.
 *
 * Mounted once in `<AppShell />`. It reads the `pluginViews` queue and
 * draws one iframe per entry; each iframe auto-removes after
 * {@link VIEW_TTL_MS}. Several views can be on screen at once (two plugins
 * firing on the same op), each on its own timer.
 *
 * **Security — do not weaken.** Each iframe is
 * `sandbox="allow-scripts"` with **no** `allow-same-origin`. The plugin's
 * script runs in a null origin: it cannot touch the app's DOM, cookies,
 * `localStorage`, or make credentialed / same-origin requests. The HTML
 * is passed via `srcdoc` (never `innerHTML` on the host document). This is
 * untrusted third-party code; the isolation is the feature.
 *
 * Visually the layer is a full-viewport, transparent, `pointer-events:
 * none` surface so a confetti animation paints over the whole UI without
 * stealing clicks. The plugin draws inside its own iframe document.
 */

/** How long a view stays on screen before auto-removal (ms). */
const VIEW_TTL_MS = 6000;

function ViewFrame(props: { view: PluginView }) {
  // Self-removing timer: each frame owns its lifetime so stacked views
  // don't share a single timeout. Cleared on unmount so a workspace swap
  // / HMR can't fire a dismiss for an id that's already gone.
  const timer = window.setTimeout(
    () => dismissPluginView(props.view.id),
    VIEW_TTL_MS,
  );
  onCleanup(() => window.clearTimeout(timer));

  return (
    <iframe
      // SECURITY: allow-scripts WITHOUT allow-same-origin — the plugin
      // JS runs in a null origin, isolated from the app. Never add
      // allow-same-origin here.
      sandbox="allow-scripts"
      srcdoc={props.view.html}
      title="plugin-effect"
      style={{
        position: "fixed",
        inset: "0",
        width: "100vw",
        height: "100vh",
        border: "0",
        background: "transparent",
        "pointer-events": "none",
        "z-index": "9999",
      }}
    />
  );
}

export function PluginEffectLayer() {
  return (
    <For each={pluginViews()}>{(view) => <ViewFrame view={view} />}</For>
  );
}
