/**
 * Tauri command wrappers for `outl-desktop`.
 *
 * Shared commands (every client uses identical: navigation, mutations,
 * paste) are **not** redeclared here — import them directly from
 * `@outl/shared/api/commands`. This file is reserved for commands the
 * desktop client adds on top: workspace picker, settings, and the
 * (Phase 4) code execution wrapper.
 */
import { invoke } from "@tauri-apps/api/core";

import type { PageView, WorkspaceSummary } from "@outl/shared/api/types";
import type { DeepLinkNavigate } from "./events";

/**
 * Take (and clear) an `outl://` deep link that arrived during cold
 * start — i.e. a URL that *launched* the app, before the `AppShell`
 * mounted its `deep-link://navigate` listener (issue #98). Returns
 * `null` on a normal launch. Call once on `AppShell` mount; the warm
 * path (app already running) is handled by the live event listener.
 */
export function takePendingDeepLink(): Promise<DeepLinkNavigate | null> {
  return invoke<DeepLinkNavigate | null>("take_pending_deep_link");
}

// ---------------------------------------------------------------------------
// Workspace lifecycle (desktop-only)
// ---------------------------------------------------------------------------

/**
 * Open the workspace rooted at `path`. The backend creates the
 * `ops/`, `journals/`, `pages/` directories if missing, opens the
 * JsonlStorage, runs the legacy migration + orphan reconcile, and
 * persists the choice in `settings.json`.
 *
 * Emits `workspace-ready` when complete — wire `onWorkspaceReady`
 * before calling this so the UI refreshes when the swap lands.
 */
export function setWorkspace(path: string): Promise<void> {
  return invoke<void>("set_workspace", { path });
}

/**
 * Current workspace path, or `null` when the user hasn't picked
 * one yet (first launch, or `last_workspace` no longer exists on
 * disk).
 */
export function currentWorkspace(): Promise<string | null> {
  return invoke<string | null>("current_workspace");
}

/**
 * Re-export of the shared `workspaceStats()` wrapper — kept here for
 * convenience so feature code can import everything desktop-shaped
 * from one file. The DTO is the shared `WorkspaceSummary` (with
 * `ready: boolean`).
 */
export async function workspaceStats(): Promise<WorkspaceSummary> {
  return invoke<WorkspaceSummary>("workspace_stats");
}

// ---------------------------------------------------------------------------
// Code execution (desktop-only)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Undo / redo (desktop-only)
// ---------------------------------------------------------------------------

/**
 * Revert the last committed block mutation on the page. Rejects with
 * `"nothing to undo"` when the page's history stack is empty — the
 * handler surfaces that as a status message, not a crash.
 */
export function undoPage(pageId: string): Promise<PageView> {
  return invoke<PageView>("undo_page", { pageId });
}

/** Re-apply the mutation the last {@link undoPage} reverted. */
export function redoPage(pageId: string): Promise<PageView> {
  return invoke<PageView>("redo_page", { pageId });
}

// ---------------------------------------------------------------------------
// Settings (desktop-only)
// ---------------------------------------------------------------------------

export interface Settings {
  last_workspace: string | null;
  vim_mode: boolean;
  /**
   * Name of the active palette preset. Matches one of
   * `outl_theme::PRESETS` (`"outl"`, `"dracula"`, `"nord"`, …) so
   * the desktop renders identical hues to the TUI / mobile.
   */
  theme: string;
  font_size: number;
  /**
   * Sync transport: `"iroh"` (direct P2P over QUIC, the default) or
   * `"file"` (iCloud Drive / shared filesystem). Mirrors the Rust
   * `Settings.sync_transport` and the `[sync] transport` config key.
   */
  sync_transport: string;
}

/**
 * Palette returned by `get_theme`. Mirrors `outl_theme::Palette`
 * field-for-field — every value is a `#rrggbb` (or `#rrggbbaa`)
 * string that
 * {@link applyPaletteToRoot | the frontend installer} writes as
 * a CSS custom property.
 */
export interface Palette {
  name: string;
  bg: string;
  bg_elev: string;
  fg: string;
  fg_dim: string;
  fg_dimmer: string;
  border: string;
  hint: string;
  accent: string;
  accent_soft: string;
  accent_alt: string;
  warn: string;
  ref_link_fg: string;
  tag_link_fg: string;
  md_link_fg: string;
  bold_fg: string;
  italic_fg: string;
  strike_fg: string;
  code_fg: string;
  todo_open_fg: string;
  todo_done_fg: string;
  todo_done_body_fg: string;
  property_key_fg: string;
  property_value_fg: string;
  heading_fg: string;
  dim_fg: string;
  selected_bullet_bg: string;
  selected_bullet_fg: string;
  cursor_block_bg: string;
  cursor_block_fg: string;
  cursor_caret_fg: string;
  status_normal_bg: string;
  status_normal_fg: string;
  status_insert_bg: string;
  status_insert_fg: string;
  status_visual_bg: string;
  status_visual_fg: string;
  status_message_fg: string;
  list_selected_bg: string;
  list_selected_fg: string;
  help_title_fg: string;
}

export function listThemes(): Promise<string[]> {
  return invoke<string[]>("list_themes");
}

export function getTheme(name: string | null): Promise<Palette> {
  return invoke<Palette>("get_theme", { name });
}

// ---------------------------------------------------------------------------
// Shortcuts (mirrors outl_shortcuts::{Action, Chord, Binding, Mode})
// ---------------------------------------------------------------------------

/** Modifier bitflags — match `outl_shortcuts::chord::Modifiers`. */
export const MOD_CTRL = 0b0001;
export const MOD_ALT = 0b0010;
export const MOD_SHIFT = 0b0100;
export const MOD_META = 0b1000;

export type ShortcutMode =
  | "global"
  | "normal"
  | "insert"
  | "visual"
  | "overlay";

/** Chord key — tagged union mirror of `outl_shortcuts::chord::Key`. */
export type Key =
  | { kind: "Char"; value: string }
  | { kind: "Enter" }
  | { kind: "Esc" }
  | { kind: "Tab" }
  | { kind: "Backspace" }
  | { kind: "Delete" }
  | { kind: "Up" }
  | { kind: "Down" }
  | { kind: "Left" }
  | { kind: "Right" }
  | { kind: "Home" }
  | { kind: "End" }
  | { kind: "PageUp" }
  | { kind: "PageDown" }
  | { kind: "Space" }
  | { kind: "Function"; value: number };

export interface Chord {
  /** Bitflag combination of `MOD_*` constants. */
  mods: number;
  key: Key;
}

/** Action discriminant — string `kind` mirrors Rust `Action` variants. */
export type Action =
  | { kind: "OpenPicker" }
  | { kind: "OpenCommandPalette" }
  | { kind: "ToggleHelp" }
  | { kind: "ToggleSidebar" }
  | { kind: "ToggleBacklinks" }
  | { kind: "OpenSettings" }
  | { kind: "Quit" }
  | { kind: "OpenToday" }
  | { kind: "PrevDay" }
  | { kind: "NextDay" }
  | { kind: "SelectionDown" }
  | { kind: "SelectionUp" }
  | { kind: "OpenRefUnderCursor" }
  | { kind: "EnterInsert" }
  | { kind: "EnterInsertAtStart" }
  | { kind: "EnterInsertAfter" }
  | { kind: "EnterInsertAtEnd" }
  | { kind: "DeleteCharUnderCursor" }
  | { kind: "DeleteCharBeforeCursor" }
  | { kind: "DeleteToEndOfBlock" }
  | { kind: "ChangeToEndOfBlock" }
  | { kind: "SubstituteBlock" }
  | { kind: "SubstituteChar" }
  | { kind: "ReplaceChar" }
  | { kind: "FindCharForward" }
  | { kind: "FindCharBackward" }
  | { kind: "ToggleCharCase" }
  | { kind: "CursorWordEnd" }
  | { kind: "UnfoldAll" }
  | { kind: "FoldAll" }
  | { kind: "CenterViewport" }
  | { kind: "SearchWordForward" }
  | { kind: "SearchWordBackward" }
  | { kind: "ReselectLastVisual" }
  | { kind: "IndentVisualRange" }
  | { kind: "OutdentVisualRange" }
  | { kind: "NewBlockBelow" }
  | { kind: "NewBlockAbove" }
  | { kind: "IndentBlock" }
  | { kind: "OutdentBlock" }
  | { kind: "MoveBlockUp" }
  | { kind: "MoveBlockDown" }
  | { kind: "DeleteBlock" }
  | { kind: "ToggleCollapsed" }
  | { kind: "ToggleTodo" }
  | { kind: "CopyBlockRef" }
  | { kind: "ExitInsert" }
  | { kind: "CommitAndContinue" }
  | { kind: "DeleteEmptyBlock" }
  | { kind: "EnterVisual" }
  | { kind: "YankCurrentBlock" }
  | { kind: "YankRange" }
  | { kind: "DeleteRange" }
  | { kind: "RunCodeBlock" }
  | { kind: "Undo" }
  | { kind: "Redo" }
  | { kind: "WrapBold" }
  | { kind: "WrapItalic" }
  | { kind: "WrapCode" }
  | { kind: "WrapStrike" }
  | { kind: "InsertLink" };

export interface Binding {
  /** `ChordSequence` — array of one (single chord) or two (vim-style `g j`). */
  chord: Chord[];
  mode: ShortcutMode;
  action: Action;
  description: string;
}

/**
 * Fetch the full binding catalog from the backend. Cached after the
 * first call (bindings never change at runtime today); a future
 * config-reload path can invalidate.
 */
export function listShortcutBindings(): Promise<Binding[]> {
  return invoke<Binding[]>("list_shortcut_bindings");
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(next: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { next });
}

// `runCodeBlock` + the `ExecOutputDto` / `RunCodeBlockReply` DTOs
// moved to `@outl/shared/api/commands` once mobile picked up the same
// command (v0.6.x — long-press → "Run code"). Re-exported here so
// every desktop caller keeps importing from one place.
export type { ExecOutputDto, RunCodeBlockReply } from "@outl/shared/api/types";
export { runCodeBlock } from "@outl/shared/api/commands";

// ---------------------------------------------------------------------------
// Plugins (desktop-only)
// ---------------------------------------------------------------------------
//
// The plugin host (`outl_plugins::PluginHost`) embeds a Boa `Context` that is
// `!Send`, so it runs on a dedicated thread behind `PluginService` (see
// `src-tauri/src/plugin_service.rs`). These commands talk to that thread.

/** A command a loaded plugin contributes — surfaced in the plugin palette. */
export interface PluginCommand {
  plugin_id: string;
  command_id: string;
  title: string;
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
  view?: PageView;
  /**
   * HTML documents the plugin emitted via `ctx.ui.render` (gated by the
   * `ui-render` capability). Each is played as an ephemeral sandboxed
   * iframe overlay — untrusted plugin output, never injected into the
   * app DOM. See `playPluginViews` in `lib/plugin-views.ts`.
   */
  views: string[];
}

/**
 * Reply from {@link pluginSyncHooks}: a refreshed `PageView` **only**
 * when an op-hook mutated the on-screen page (`view` absent otherwise),
 * plus any `ui-render` views the hooks emitted. The `views` path is the
 * confetti trigger — present even when no page re-render is needed.
 */
export interface PluginSyncHooksReply {
  view?: PageView;
  views: string[];
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
 * Fire the plugins' `onOp` hook sweep after a user mutation. The reply's
 * `view` is the refreshed `PageView` of `pageId` **only** when a hook
 * actually mutated the workspace (absent otherwise, so the caller skips a
 * needless render); `views` carries any `ui-render` HTML the hooks emitted
 * (the confetti path — present even when nothing was re-rendered).
 * Best-effort — a host with no op-hook plugins is a cheap no-op.
 */
export function pluginSyncHooks(
  pageId: string | null,
): Promise<PluginSyncHooksReply> {
  return invoke<PluginSyncHooksReply>("plugin_sync_hooks", { pageId });
}

// The plugin marketplace types + wrappers (`RegistryItem`,
// `pluginRegistryList` / `pluginInstallOfficial` / `pluginSetEnabled` /
// `pluginUninstall`, `filterRegistryItems`) live in `@outl/shared` — both
// clients register identical commands, so the wrapper is client-agnostic.

/**
 * A keybinding a loaded plugin contributes for the desktop.
 *
 * `chord` and `mode` serialize **identically** to the `outl-shortcuts`
 * catalog ({@link Binding}) — `chord` is a `Chord[]` (`ChordSequence` is
 * `#[serde(transparent)]` over `Vec<Chord>`), `mode` is a lowercase
 * {@link ShortcutMode}. The dispatcher in `lib/shortcuts.ts` reuses the
 * same `seqEq` comparison it already runs against native bindings, so an
 * `eventToChord(e)` matches a plugin chord byte-for-byte the way it
 * matches a native one. Plugin chords are always `"global"`.
 */
export interface PluginKeybinding {
  chord: Chord[];
  mode: ShortcutMode;
  plugin_id: string;
  command_id: string;
  description: string;
}

/** A toolbar button a loaded plugin contributes for the desktop chrome. */
export interface ToolbarButton {
  plugin_id: string;
  command_id: string;
  /** Glyph / emoji rendered in the chrome button. */
  icon: string;
  /** Optional tooltip / accessible label. */
  title?: string;
}

/**
 * List plugin-contributed desktop keybindings. The dispatcher folds these
 * into the chord pipeline as a Global overlay that only fires when **no**
 * native binding already owns the chord (native wins). Empty until plugins
 * load (best-effort — never throws).
 */
export function pluginKeybindings(): Promise<PluginKeybinding[]> {
  return invoke<PluginKeybinding[]>("plugin_keybindings");
}

/**
 * List plugin-contributed desktop toolbar buttons — one chrome button per
 * entry (glyph = `icon`, tooltip = `title`, click = {@link pluginRun}).
 * Empty until plugins load (best-effort — never throws).
 */
export function pluginToolbar(): Promise<ToolbarButton[]> {
  return invoke<ToolbarButton[]>("plugin_toolbar");
}

/**
 * A content transformer a loaded plugin declared for a code-fence
 * language. The frontend loads the list once per workspace open and, when
 * a fence's language matches a `lang` here, calls {@link pluginTransform}
 * to render it.
 */
export interface PluginTransformer {
  plugin_id: string;
  lang: string;
  /** `"text"` (inline markdown) or `"rich"` (HTML for a sandboxed iframe). */
  kind: "text" | "rich";
}

/**
 * The descriptor a content transformer produced for a fence body.
 * `kind: "text"` → `content` is markdown/text rendered inline;
 * `kind: "rich"` → `content` is HTML run in a sandboxed iframe (untrusted
 * plugin output, never injected into the app DOM — see `CodeFenceView`).
 */
export interface PluginTransformResult {
  kind: "text" | "rich";
  content: string;
}

/**
 * List every content transformer a loaded plugin declared. Load once per
 * workspace open and match each code fence's language against the result.
 * Empty until plugins load (best-effort — never throws).
 */
export function pluginTransformers(): Promise<PluginTransformer[]> {
  return invoke<PluginTransformer[]>("plugin_transformers");
}

/**
 * Run a content transformer for `lang` against a fence `input` (its body).
 * Read-only: never mutates the workspace. Resolves to `null` when the
 * transformer declined or no plugin owns `lang`, otherwise the
 * `{ kind, content }` descriptor. Cache the result by `(blockId, body)` —
 * re-run only when the body changes.
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
