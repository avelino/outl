/**
 * Mobile-only Tauri command wrappers.
 *
 * Shared commands (navigation, mutations, paste, peers) live in
 * `@outl/shared/api/commands` — import those directly. This file is
 * reserved for commands the **mobile** client adds on top: today, that's
 * the workspace-folder choice, which is deliberately client-specific
 * (desktop picks a folder via `tauri-plugin-dialog`; mobile keeps notes in
 * its local folder, synced by iroh — see `src-tauri/src/workspace_picker.rs`).
 *
 * Per `@outl/shared/CLAUDE.md`, workspace picking is exactly the kind of
 * client-coupled command that stays in the client's own `lib/api.ts`.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Persist `path` as the workspace folder and ask the app to reopen against
 * it. The reopen is boot-read (the backend emits
 * `workspace-reopen-required`; the next launch picks up the new path), so
 * callers should treat a successful `setWorkspace` as "the choice is saved"
 * rather than "the workspace is live this instant".
 *
 * No caller wires this today — the arbitrary-folder native picker is
 * deferred (`workspace_picker.rs`). It's the entry point that picker will
 * use once it can hand back a security-scoped path.
 */
export function setWorkspace(path: string): Promise<void> {
  return invoke<void>("set_workspace", { path });
}
