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
