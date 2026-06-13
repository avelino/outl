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

export type ShortcutMode = "global" | "normal" | "insert" | "visual" | "overlay";

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
