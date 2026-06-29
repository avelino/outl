import { For, createSignal } from "solid-js";
import { Portal } from "solid-js/web";

/**
 * `<PluginViewOverlay />` — paints plugin-emitted `ctx.ui.render(html)`
 * payloads as **sandboxed, ephemeral** fullscreen iframes.
 *
 * A plugin granted the `ui-render` capability can emit HTML/JS (e.g. a
 * confetti burst when a block is marked DONE). The host propagates those
 * payloads as `views: string[]`; this overlay drops each into its own
 * `<iframe>` and auto-removes it after {@link VIEW_TTL_MS}.
 *
 * ## Security (load-bearing)
 *
 * The iframe is **`sandbox="allow-scripts"` WITHOUT `allow-same-origin`**.
 * Plugin JS is untrusted author code: omitting `allow-same-origin` forces
 * the iframe into an opaque origin, so it CANNOT reach the app's DOM,
 * cookies, `localStorage`, or Tauri bridge. Adding `allow-same-origin`
 * alongside `allow-scripts` would defeat the sandbox entirely (the
 * combination lets the frame remove its own sandbox attribute). Never add
 * it.
 *
 * The content goes in via `srcdoc` (the HTML string itself), not `src` —
 * no network fetch, no URL the plugin controls.
 *
 * ## Layout
 *
 * Each frame is `position: fixed; inset: 0` covering the viewport with a
 * transparent background and `pointer-events: none`, so a confetti
 * animation rains over the UI without intercepting taps. `z-index: 9999`
 * keeps it above every sheet.
 *
 * Imperative API rather than props: the overlay owns its own list so any
 * caller (the plugin sheet's `run`, the Journal's `commitEdit` hook sweep)
 * can `push()` views from anywhere without threading state through every
 * component. `bindPluginViews(push)` hands the `push` fn up to the caller.
 */

/** How long a rendered view stays on screen before auto-removal. */
const VIEW_TTL_MS = 6000;

interface ActiveView {
  /** Monotonic id so `<For>` keys stay stable as views come and go. */
  key: number;
  html: string;
}

export function PluginViewOverlay(props: {
  /** Receives the `push(html)` fn so callers can show a view from anywhere. */
  bind: (push: (html: string) => void) => void;
}) {
  const [views, setViews] = createSignal<ActiveView[]>([]);
  let nextKey = 0;

  function push(html: string) {
    const key = nextKey++;
    setViews((cur) => [...cur, { key, html }]);
    // Ephemeral: drop the frame after the TTL so a plugin can't pin an
    // invisible (pointer-events:none) iframe over the app indefinitely.
    window.setTimeout(() => {
      setViews((cur) => cur.filter((v) => v.key !== key));
    }, VIEW_TTL_MS);
  }

  props.bind(push);

  return (
    <Portal>
      <For each={views()}>
        {(v) => (
          <iframe
            // OMITTING `allow-same-origin` is the whole point — see the
            // module doc. Do not add it alongside `allow-scripts`.
            sandbox="allow-scripts"
            srcdoc={v.html}
            title="plugin view"
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
        )}
      </For>
    </Portal>
  );
}
