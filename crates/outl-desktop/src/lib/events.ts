/**
 * Tauri event listeners for the desktop client.
 *
 * - `workspace-ready` — emitted by the Rust backend after
 *   [`setWorkspace`](./api.ts) finishes opening a directory (and
 *   after the boot-time background opener completes when
 *   `settings.last_workspace` is already set).
 * - `peer-ops-changed` — fired by the cross-platform FS watcher
 *   (`fs_watcher.rs`) when a peer writes its `ops-*.jsonl` or an
 *   external editor touches a `.md` under the workspace. Debounced
 *   to ~100ms by the backend, so the listener can call
 *   `reload_workspace` straight through without extra throttling.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Register a handler for the `workspace-ready` event. Returns an
 * unlisten function — call it on component cleanup to avoid leaks.
 */
export function onWorkspaceReady(handler: () => void): Promise<UnlistenFn> {
  return listen("workspace-ready", () => handler());
}

/**
 * Register a handler for the `peer-ops-changed` event. Returns an
 * unlisten function — call it on component cleanup to avoid leaks.
 */
export function onPeerOpsChanged(handler: () => void): Promise<UnlistenFn> {
  return listen("peer-ops-changed", () => handler());
}

/**
 * Payload emitted with `ref-projection-failed` when `open_ref` resolved
 * the target (the page now lives in the op log) but writing the
 * resulting `pages/<slug>.md` + sidecar failed. The op log is still
 * the source of truth; the next save / orphan scanner retries the
 * projection. The frontend surfaces a toast so the user knows the
 * link they just inserted may not be visible to peers yet.
 */
export interface RefProjectionFailedPayload {
  target: string;
  error: string;
}

/**
 * Register a handler for the `ref-projection-failed` event.
 */
export function onRefProjectionFailed(
  handler: (payload: RefProjectionFailedPayload) => void,
): Promise<UnlistenFn> {
  return listen<RefProjectionFailedPayload>("ref-projection-failed", (e) =>
    handler(e.payload),
  );
}
