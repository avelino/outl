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

import type { PageView } from "@outl/shared/api/types";

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

// ---------------------------------------------------------------------------
// Plugins (mobile-only wiring; mirrors outl-desktop)
// ---------------------------------------------------------------------------
//
// The plugin host (`outl_plugins::PluginHost`) embeds a Boa `Context` that is
// `!Send`, so it runs on a dedicated thread behind `PluginService` (see
// `src-tauri/src/plugin_service.rs`). These commands talk to that thread.

/** A command a loaded plugin contributes — surfaced in the plugin sheet. */
export interface PluginCommand {
  plugin_id: string;
  command_id: string;
  title: string;
}

/**
 * A toolbar button a plugin contributes to the mobile chrome. `icon` is
 * the glyph painted inline in the header; tapping it runs `command_id`
 * (via `pluginRun`). `title` is the accessible label / tooltip.
 */
export interface PluginToolbarButton {
  plugin_id: string;
  command_id: string;
  icon: string;
  title?: string;
}

/**
 * Outcome of running a plugin command. `view` is the refreshed `PageView`
 * of the page that was on screen when the command fired (so the caller
 * re-renders in one trip); absent when no page id was supplied or the page
 * no longer resolves.
 */
export interface PluginRunReply {
  applied: number;
  notifications: string[];
  errors: string[];
  /**
   * `ctx.ui.render(html)` payloads — each is dropped into a sandboxed,
   * ephemeral `<iframe>` overlay (see `<PluginViewOverlay />`). Empty
   * unless the plugin holds the `ui-render` capability.
   */
  views: string[];
  view?: PageView;
}

/**
 * Outcome of the `onOp` hook sweep. `views` carries any `ctx.ui.render`
 * payloads the hooks emitted (the confetti path — DONE → hook → render);
 * `view` is the refreshed `PageView` **only** when a hook mutated the
 * workspace (otherwise absent, so the caller skips a needless render).
 */
export interface PluginSyncReply {
  views: string[];
  view?: PageView;
}

/**
 * A content transformer a loaded plugin contributes for a code-fence
 * language. The frontend loads the list once when the workspace opens and
 * keys on `lang`: a fenced block whose language matches fires
 * `pluginTransform(plugin_id, lang, body)`.
 *
 * `kind` decides how the result renders inline in the block:
 * - `"text"` → the `content` is markdown/plain text, rendered inline.
 * - `"rich"` → the `content` is HTML, run in a sandboxed `<iframe>` inline.
 */
export interface PluginTransformer {
  plugin_id: string;
  lang: string;
  kind: string;
}

/**
 * The descriptor a transformer produced for a fenced block. `kind` mirrors
 * the matching `PluginTransformer.kind`; `content` is the rendered text
 * (for `"text"`) or HTML (for `"rich"`).
 */
export interface PluginTransformResult {
  kind: string;
  content: string;
}

/**
 * List every command contributed by a loaded plugin. Empty until the
 * workspace opens and plugins load (best-effort — never throws on an empty
 * or failed host).
 */
export function pluginList(): Promise<PluginCommand[]> {
  return invoke<PluginCommand[]>("plugin_list");
}

/**
 * List every content transformer granted on the mobile client. Load this
 * once when the workspace opens and cache it; the fence renderer keys on
 * `lang`. Empty until the workspace opens and plugins load (best-effort —
 * never throws on an empty or failed host).
 */
export function pluginTransformers(): Promise<PluginTransformer[]> {
  return invoke<PluginTransformer[]>("plugin_transformers");
}

/**
 * Run a plugin's content transformer for `lang` against `input`, resolving
 * to the descriptor it produced or `null` when the transformer declined
 * (the caller then falls back to rendering the plain code fence). Pure
 * render — no workspace mutation.
 */
export function pluginTransform(
  pluginId: string,
  lang: string,
  input: string,
): Promise<PluginTransformResult | null> {
  return invoke<PluginTransformResult | null>("plugin_transform", {
    pluginId,
    lang,
    input,
  });
}

/**
 * List every toolbar button a loaded plugin contributes for the mobile
 * client. Rendered as inline glyphs in the header; each runs its command
 * via `pluginRun`. Empty until the workspace opens and plugins load
 * (best-effort — never throws on an empty or failed host).
 */
export function pluginToolbar(): Promise<PluginToolbarButton[]> {
  return invoke<PluginToolbarButton[]>("plugin_toolbar");
}

/**
 * Run a plugin command. Pass the currently-open page id so the reply
 * carries its refreshed `PageView` — the plugin thread re-projects every
 * page's `.md` before returning (a plugin can move blocks across pages).
 */
export function pluginRun(
  pluginId: string,
  commandId: string,
  pageId: string | null,
): Promise<PluginRunReply> {
  return invoke<PluginRunReply>("plugin_run", {
    pluginId,
    commandId,
    pageId,
  });
}

/**
 * Fire the plugins' `onOp` hook sweep after a user mutation. Returns the
 * hooks' `ctx.ui.render` payloads (`views` — the confetti path) plus the
 * refreshed `PageView` of `pageId` **only** when a hook actually mutated
 * the workspace (otherwise absent, so the caller skips a needless render).
 * Best-effort — a host with no op-hook plugins is a cheap no-op.
 */
export function pluginSyncHooks(
  pageId: string | null,
): Promise<PluginSyncReply> {
  return invoke<PluginSyncReply>("plugin_sync_hooks", { pageId });
}

/**
 * One marketplace row: a registry entry (plugins.outl.app) plus this
 * workspace's local state. `installed` / `enabled` drive the install vs.
 * manage affordances.
 */
export interface RegistryItem {
  id: string;
  name: string;
  description: string;
  author: string | null;
  category: string | null;
  capabilities: string[];
  permissions: string[];
  latest: string | null;
  installed: boolean;
  enabled: boolean;
}

/** Fetch the marketplace: the official registry crossed with the lockfile. */
export function pluginRegistryList(): Promise<RegistryItem[]> {
  return invoke<RegistryItem[]>("plugin_registry_list");
}

/** Tap-to-install an official plugin by id; resolves to its display name. */
export function pluginInstallOfficial(id: string): Promise<string> {
  return invoke<string>("plugin_install_official", { id });
}

/** Enable / disable an installed plugin. */
export function pluginSetEnabled(
  id: string,
  enabled: boolean,
): Promise<void> {
  return invoke<void>("plugin_set_enabled", { id, enabled });
}

/** Uninstall a plugin; resolves `true` if anything was removed. */
export function pluginUninstall(id: string): Promise<boolean> {
  return invoke<boolean>("plugin_uninstall", { id });
}
