/**
 * Plugin `ui-render` view queue.
 *
 * A plugin with the `ui-render` capability can emit an HTML/JS document
 * via `ctx.ui.render(html)`; the core surfaces those strings on
 * `PluginRun.views` (already gated by the capability upstream — only
 * plugins that declare `ui-render` ever produce one). The desktop plays
 * each one in an **ephemeral, fully sandboxed `<iframe>` overlay** that
 * covers the window, runs the plugin's script, then auto-removes itself.
 *
 * Security: this is **untrusted plugin code**. The iframe is mounted with
 * `sandbox="allow-scripts"` and deliberately **without** `allow-same-origin`,
 * so the script can run but cannot reach the app's DOM, cookies,
 * `localStorage`, or issue same-origin / credentialed `fetch`. That
 * isolation is the whole point of the feature — never relax it. The HTML
 * goes in via `srcdoc`, never `innerHTML` on the app document.
 *
 * The queue is a plain Solid signal: callers `playPluginViews(views)`
 * from wherever views arrive (the plugin palette `pluginRun`, the
 * post-commit `pluginSyncHooks` sweep), and the mounted
 * `<PluginEffectLayer />` renders one sandboxed iframe per queued entry.
 * Multiple views stack (each gets its own iframe + timer), so two plugins
 * firing on the same op both play.
 */
import { createSignal } from "solid-js";

/** A single queued view: the HTML string + a stable id for keying. */
export interface PluginView {
  id: number;
  html: string;
}

let nextId = 0;

const [views, setViews] = createSignal<PluginView[]>([]);

/** Reactive list of currently-playing views (read by the layer). */
export { views as pluginViews };

/**
 * Enqueue one or more HTML documents to play as sandboxed overlays.
 * Empty / non-array input is a no-op (the common case: most sweeps emit
 * nothing). Safe to call from any view-arrival site.
 */
export function playPluginViews(html: readonly string[] | undefined): void {
  if (!html || html.length === 0) return;
  const queued = html
    .filter((h) => typeof h === "string" && h.length > 0)
    .map((h) => ({ id: nextId++, html: h }));
  if (queued.length === 0) return;
  setViews((prev) => [...prev, ...queued]);
}

/** Remove a finished view from the queue (called by its own timer). */
export function dismissPluginView(id: number): void {
  setViews((prev) => prev.filter((v) => v.id !== id));
}
