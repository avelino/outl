/**
 * Content-transformer registry + result cache for code fences.
 *
 * A plugin can declare a content transformer for a code-fence language
 * (e.g. `mermaid`, `chart`): when a fenced block's language matches a
 * loaded transformer, the client calls `pluginTransform` and renders
 * the descriptor it returns — `text` inline, `rich` as HTML in a
 * sandboxed iframe (same isolation posture as `ui-render`).
 *
 * Two concerns live here so each client's fence renderer stays a
 * renderer:
 *
 * 1. **Registry** — the `lang → PluginTransformer` map, (re)loaded via
 *    {@link loadTransformers}. Plugins load lazily on the first host
 *    request after the workspace opens, so a single boot fetch can
 *    come back empty; callers re-run on `workspace-ready` and after a
 *    workspace swap. A plain Solid signal so a fence that mounts
 *    before the load reactively picks the transformer up once it
 *    arrives.
 *
 * 2. **Result cache** — keyed by `(blockId + "\0" + body)`. The
 *    transform runs the plugin's JS, so it must not re-run on every
 *    render; it only re-runs when the fence body changes (a new key).
 *    The cache is a module-level `Map`, not size-bounded — fence
 *    bodies are small and the working set is the visible outline.
 *    A pending promise is cached too, so concurrent renders of the
 *    same fence don't fire duplicate host calls.
 *
 * Both GUI clients (`outl-desktop`, `outl-mobile`) consume this module
 * — it used to exist as two near-identical copies under each client's
 * `lib/transformers.ts`, which is exactly the drift `@outl/shared`
 * exists to remove.
 */
import { createSignal } from "solid-js";

import {
  pluginTransform,
  pluginTransformers,
} from "../api/commands";
import type {
  PluginTransformResult,
  PluginTransformer,
} from "../api/types";

// lang → transformer. Empty until `loadTransformers` runs against an
// open workspace with content-transformer plugins installed.
const [registry, setRegistry] = createSignal<Map<string, PluginTransformer>>(
  new Map(),
);

/**
 * Look up the transformer for a code-fence language. Reactive: a fence
 * that renders before the registry loads re-evaluates once it arrives.
 * Returns `null` when no plugin owns `lang` (the normal case — most
 * fences are plain code).
 */
export function transformerFor(lang: string): PluginTransformer | null {
  return registry().get(lang) ?? null;
}

/**
 * (Re)load the content-transformer registry from the plugin host. Call
 * on `workspace-ready` and after a workspace swap — plugins load
 * lazily, so an early call may return an empty list and a later one
 * fills it in. Best-effort: a transient host failure clears the
 * registry (fences fall back to plain code; the next `workspace-ready`
 * re-loads) — never left stale so a swap to a plugin-less workspace
 * doesn't keep old entries.
 */
export async function loadTransformers(): Promise<void> {
  try {
    const list = await pluginTransformers();
    const next = new Map<string, PluginTransformer>();
    for (const t of list) {
      // First declaration wins on a lang collision; deterministic and rare.
      if (!next.has(t.lang)) {
        next.set(t.lang, t);
      }
    }
    setRegistry(next);
  } catch {
    setRegistry(new Map());
  }
  // Registry changed → previously-cached renders may now resolve
  // differently (a plugin was added, removed, or swapped).
  resultCache.clear();
}

// (blockId + "\0" + body) → resolved descriptor (or null when the
// transformer declined), or the in-flight promise for it.
const resultCache = new Map<
  string,
  PluginTransformResult | null | Promise<PluginTransformResult | null>
>();

/** Composite cache key for a fenced block. */
function cacheKey(blockId: string, body: string): string {
  return `${blockId}\0${body}`;
}

/**
 * Run (or replay) a transformer for a fence body, cached by
 * `(blockId, body)`. Re-runs only when the body changes. Returns
 * `null` when the transformer declined or the host failed (the caller
 * then falls back to the plain code view). A `null` decline is cached;
 * a host *failure* drops the key so a later render retries instead of
 * replaying `null` forever.
 */
export async function runTransform(
  blockId: string,
  transformer: PluginTransformer,
  body: string,
): Promise<PluginTransformResult | null> {
  const key = cacheKey(blockId, body);
  const hit = resultCache.get(key);
  if (hit !== undefined) {
    return hit;
  }
  const pending = pluginTransform(transformer.plugin_id, transformer.lang, body)
    .then((res) => {
      resultCache.set(key, res);
      return res;
    })
    .catch(() => {
      // Don't poison the cache on a transient host failure — drop the
      // key so a later render retries.
      resultCache.delete(key);
      return null;
    });
  resultCache.set(key, pending);
  return pending;
}
