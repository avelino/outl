/**
 * Global Solid store for the desktop client.
 *
 * Holds the currently open page view, the workspace state, and the
 * panel collapse flags. The store is intentionally desktop-specific
 * (3-pane layout) — mobile has its own shape with swipe/gesture
 * state. Generic state shapes are not shared between the two clients
 * because the chrome diverges; only pure helpers and DTOs go through
 * `@outl/shared`.
 */
import { createStore } from "solid-js/store";

import type {
  Backlink,
  BlockNode,
  PageMeta,
  WorkspaceSummary,
} from "@outl/shared/api/types";

export type Mode = "normal" | "edit" | "vim-normal" | "vim-insert";

export interface AppStateShape {
  /** `null` until the user picks a workspace or boot opener finishes. */
  workspace: WorkspaceSummary | null;
  /** Currently displayed page (today's journal, a regular page, …). */
  page: PageMeta | null;
  /** Outline of the current page, projected from the workspace. */
  outline: BlockNode[];
  /** Backlinks targeting the current page. */
  backlinks: Backlink[];
  /** DFS path of the selected block, or `null` for no selection. */
  selectedPath: number[] | null;
  /**
   * Currently selected block id (vim Normal-mode cursor). `null`
   * before the user navigates or after the page changes; the shell
   * auto-selects the first block when the outline loads so `j/k`
   * work from the first keystroke.
   */
  selectedBlockId: string | null;
  /**
   * When the vim cursor crosses into the backlinks section (below
   * the outline), this carries the highlighted backlink's
   * `block_id` — the id of the **source** block on the OTHER page,
   * not anything in `appState.outline`.
   *
   * Mutually exclusive with [`selectedBlockId`]: at most one is
   * non-null at a time so `j`/`k` traverse a single linear cursor.
   * `Enter` on a non-null `selectedBacklinkBlockId` opens the source
   * page (via `openRef(source_page.slug)`) and snaps the cursor to
   * `selectedBlockId = backlink.block_id` so the user lands on the
   * referencing block.
   *
   * Mirrors the TUI's `Focus::Backlinks` state — the desktop now
   * supports the same `j/k`+`Enter` flow over backlinks.
   */
  selectedBacklinkBlockId: string | null;
  /**
   * Block currently in edit mode (textarea mounted). `null` outside
   * Insert mode. Lifted from `<OutlineView />`'s local signal so
   * `outl-shortcuts` action handlers (`EnterInsert`, `NewBlockBelow`,
   * `CommitAndContinue`, …) can flip it without prop-drilling a
   * callback through `buildHandlers`.
   */
  editingBlockId: string | null;
  /** Editor mode. `edit` while a block's textarea is mounted. */
  mode: Mode;
  /**
   * Sidebar (left pane) visibility. Toggled with `Cmd/Ctrl+Shift+E`
   * (mirrors VS Code's "Show Explorer" — see `outl-shortcuts`).
   *
   * Defaults to `false`: editor-hero on first launch (Bear / Ulysses
   * convention), matches the TUI's `show_sidebar: false` default.
   * The user opts in with the chord.
   */
  sidebarOpen: boolean;
  /**
   * Backlinks (right pane) visibility. Toggled with
   * `Cmd/Ctrl+Shift+B` (mirrors the TUI's `Ctrl+B`; we picked the
   * shifted variant on the desktop to keep `Cmd+B` reserved for the
   * universal markdown "bold" chord).
   *
   * Defaults to `true`: references stay visible below the outline so
   * a page's incoming links are discoverable without a chord. The
   * inline section only renders when the page actually has backlinks
   * (`<InlineBacklinks />` guards on `appState.backlinks.length`), so
   * an open default costs nothing on pages with no references.
   */
  backlinksOpen: boolean;
  /** Picker overlay open state. `Cmd/Ctrl+P` toggles. */
  pickerOpen: boolean;
  /** Settings modal open state. `Cmd/Ctrl+,` toggles. */
  settingsOpen: boolean;
  /** Help overlay open state. `?` in Normal mode toggles. */
  helpOpen: boolean;
  /** Last error surfaced to the user (status line). */
  lastError: string | null;
}

const [state, setState] = createStore<AppStateShape>({
  workspace: null,
  page: null,
  outline: [],
  backlinks: [],
  selectedPath: null,
  selectedBlockId: null,
  selectedBacklinkBlockId: null,
  editingBlockId: null,
  mode: "normal",
  sidebarOpen: false,
  backlinksOpen: true,
  pickerOpen: false,
  settingsOpen: false,
  helpOpen: false,
  lastError: null,
});

export { state as appState, setState as setAppState };
