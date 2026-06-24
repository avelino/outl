/**
 * Mobile-only Tauri command wrappers.
 *
 * Shared commands (navigation, mutations, paste, peers) live in
 * `@outl/shared/api/commands` — import those directly. This file is
 * reserved for commands the **mobile** client adds on top: today, that's
 * the workspace-folder choice, which is deliberately client-specific
 * (desktop picks a folder via `tauri-plugin-dialog`; mobile chooses
 * between the local default and the iCloud container — see
 * `src-tauri/src/workspace_picker.rs`).
 *
 * Per `@outl/shared/CLAUDE.md`, workspace picking is exactly the kind of
 * client-coupled command that stays in the client's own `lib/api.ts`.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Persist `path` as the workspace folder and ask the app to reopen
 * against it. Returns whether the chosen folder lives inside iCloud, so
 * the caller can show the right "syncs via iCloud + iroh" vs
 * "syncs via iroh" affordance.
 *
 * The reopen is boot-read today (the backend emits
 * `workspace-reopen-required`; the next launch picks up the new path),
 * so callers should treat a successful `setWorkspace` as "the choice is
 * saved" rather than "the workspace is live this instant".
 */
export function setWorkspace(path: string): Promise<boolean> {
  return invoke<boolean>("set_workspace", { path });
}

/**
 * Resolve the app's iCloud container workspace root for the
 * "store my notes in iCloud" choice.
 *
 * Returns `null` when the device isn't signed into iCloud (or the
 * entitlement is missing) — the caller then hides the iCloud option and
 * keeps the local default. Opt-in only; nothing here is forced.
 */
export function pickInICloud(): Promise<string | null> {
  return invoke<string | null>("pick_in_icloud");
}
