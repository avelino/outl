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
  ParseWarning,
  PageMeta,
  WorkspaceSummary,
} from "@outl/shared/api/types";

export type Mode =
  | "normal"
  | "edit"
  | "vim-normal"
  | "vim-insert"
  | "vim-visual";

export interface AppStateShape {
  /** `null` until the user picks a workspace or boot opener finishes. */
  workspace: WorkspaceSummary | null;
  /** Currently displayed page (today's journal, a regular page, …). */
  page: PageMeta | null;
  /** Outline of the current page, projected from the workspace. */
  outline: BlockNode[];
  /** Backlinks targeting the current page. */
  backlinks: Backlink[];
  /**
   * Parser recovery records emitted while reading the current page's
   * `.md`. Drives the `<ParseWarningsBanner />` above the outline so
   * the user knows outl had to preserve lines that don't match the
   * dialect (a leading `# heading`, free paragraph, etc.). Empty on
   * a clean file.
   */
  parseWarnings: ParseWarning[];
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
   * Block id where the current Visual selection was anchored. Set by
   * `EnterVisual`, cleared on exit. The Visual range covers every
   * block from this id to `selectedBlockId` in DFS order — direction
   * doesn't matter, the renderer picks `[lo, hi]` itself.
   */
  visualAnchorId: string | null;
  /**
   * Last Visual range captured the moment the user left Visual mode
   * (Esc, yank, delete). `gv` re-enters Visual with the same range.
   * `null` until the first Visual session of the app instance.
   *
   * Stores both endpoints by id so an outline mutation between the
   * exit and the `gv` re-entry doesn't shift the range (block ids are
   * stable; flat indices aren't).
   */
  lastVisualRange: { lo: string; hi: string } | null;
  /**
   * Yank register — list of block texts copied via `yy` (Y) or `y`
   * in Visual. `p` / `P` paste these (handlers TBD). One register
   * cross-block (vim convention).
   */
  yankRegister: string[];
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
  /**
   * Caret intent the next mounting `<BlockRow />` textarea consumes
   * the moment it lands in the DOM. Set by vim-style entry actions
   * that need the caret somewhere other than where the click would
   * leave it — today only `EnterInsertAtEnd` (`A` in vim) uses
   * `"end"`. Cleared by `<BlockRow />` as soon as it applies the
   * intent so a subsequent regular click doesn't get hijacked.
   *
   * Why a signal and not `queueMicrotask` + `document.querySelector`:
   * the textarea is mounted by Solid's `<Show>` swap, which doesn't
   * guarantee the DOM node exists by the next microtask. A signal
   * lets the row itself apply the intent inside its own
   * `createEffect` — same tick the textarea ref is populated, every
   * time.
   */
  caretIntent: "end" | "start" | null;
  /** Picker overlay open state. `Cmd/Ctrl+P` toggles. */
  pickerOpen: boolean;
  /**
   * Optional pre-fill query consumed by `<Picker />` the moment it
   * opens. Set by `*` / `#` (Normal mode "search inside the selected
   * block") before flipping `pickerOpen`. The picker clears this on
   * close so the next manual `Cmd+P` opens blank.
   */
  pickerSeed: string | null;
  /** Settings modal open state. `Cmd/Ctrl+,` toggles. */
  settingsOpen: boolean;
  /** Help overlay open state. `?` in Normal mode toggles. */
  helpOpen: boolean;
  /**
   * First column of the mini-calendar week grid, mirrored from
   * `config.toml`'s `[calendar] week_start` (`outl_config::WeekStart`)
   * via `getSettings()` on boot. Defaults to `"monday"` (the
   * historical layout) until settings hydrate.
   */
  weekStart: "monday" | "sunday";
  /** Last error surfaced to the user (status line). */
  lastError: string | null;
}

const [state, setState] = createStore<AppStateShape>({
  workspace: null,
  page: null,
  outline: [],
  backlinks: [],
  parseWarnings: [],
  selectedPath: null,
  selectedBlockId: null,
  selectedBacklinkBlockId: null,
  editingBlockId: null,
  mode: "normal",
  visualAnchorId: null,
  lastVisualRange: null,
  yankRegister: [],
  sidebarOpen: false,
  backlinksOpen: true,
  caretIntent: null,
  pickerOpen: false,
  pickerSeed: null,
  settingsOpen: false,
  helpOpen: false,
  weekStart: "monday",
  lastError: null,
});

export { state as appState, setState as setAppState };
