# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

## Status

**Feature-complete v0.**
Outline edit, journal nav, picker (Cmd+P), backlinks panel, `outl-exec` code blocks, cross-platform FS watcher + auto-reload, settings modal, and the `desktop.yml` CI workflow are all in.
Signed bundles, Homebrew cask, and graph view ride incrementally on top.

## Layering

```text
outl-core                    (CRDT, op log, storage trait)
outl-md                      (.md parse/render, sidecar)
outl-actions                 (workspace operations + SyncEngine, shared with TUI + mobile)
   ↑
outl-desktop (this crate)
   ├── src-tauri/src/lib.rs  (Tauri commands: parse args → outl-actions → render)
   └── (frontend in ../src)  (Solid components, Tailwind, @outl/shared)
```

## Hard rule

**This crate adds no business logic.**
If a Tauri command does something that involves the workspace shape (edit, move, todo, journal render), it delegates to `outl-actions`.
If you find yourself writing a tree walk or an op-generating helper inside `src-tauri/src/lib.rs`, stop — move it to `outl-actions` instead.
The TUI and mobile clients need it too.

Same rule on the frontend: before writing a helper under `src/lib/`, check `@outl/shared` (see [`crates/outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md)).
The renderer for inline tokens, paste detection, ref autocomplete, DTO types, and shared Tauri command wrappers all live there.

What this crate **does** own:

- Path discovery (file picker via `tauri-plugin-dialog`; persisted in settings JSON; cross-platform default).
- Cross-platform FS watcher (`notify` crate) that signals the frontend when peer `ops-*.jsonl` files grow — replaces the `NSMetadataQuery`/`NSFileCoordinator` dance the mobile crate has to do for iOS.
- Desktop-only Tauri command surface (workspace picker, settings IO).
  The code-execution command (`run_code_block`) is a **thin adapter** — the orchestration (flat-DFS walk, `.md` path resolution, `outl-exec` invocation, DTO build) lives in `outl_actions::exec` so the mobile client shares the exact same flow.
  The desktop adapter only parses NodeIds, locks the workspace, calls the action, and wraps the outcome with a refreshed `PageView`.
  Adding behaviour to `commands/exec.rs` is almost always a smell — promote it to `outl-actions` instead.
- Solid frontend with **3-pane layout** (Sidebar / OutlineView / BacklinksPanel) and **OS-standard keyboard shortcuts** (`Cmd+P`, `Cmd+J`, `Cmd+T`, `Cmd+Enter`, `Cmd+,`) plus optional vim mode.

## Layout

```
crates/outl-desktop/
├── package.json / tsconfig*.json / vite.config.ts / vitest.config.ts / index.html
├── src/                       # frontend (Solid)
│   ├── index.tsx  App.tsx (Onboarding/AppShell gate)  styles.css  setup.test.ts
│   ├── components/            # AppShell, Sidebar, OutlineView (owns BlockCallbacks),
│   │                         #   BlockRow (+CodeFenceView), BacklinksPanel, Picker,
│   │                         #   SettingsModal, ChromeToggleBar, SyncIndicator,
│   │                         #   Onboarding, WorkspacePicker
│   └── lib/                   # api.ts (desktop-only cmds), code-block.ts, events.ts,
│                             #   shortcuts.ts, action-handlers.ts, store.ts
└── src-tauri/
    ├── Cargo.toml  build.rs  tauri.conf.json (app.outl.desktop)  capabilities/  icons/
    └── src/                   # main.rs, lib.rs (run() registers all commands),
                              #   settings.rs, state.rs, helpers.rs, workspace_open.rs,
                              #   plugin_service.rs, fs_watcher.rs,
                              #   commands/ (thin shims over outl_tauri_shared::commands)
```

## First-run onboarding

`components/Onboarding.tsx` is the first-run flow. `App.tsx` decides between it and `<AppShell />`:

- **Returning user** — workspace already opens at boot (`currentWorkspace()` + `workspaceStats().ready`) → straight to `<AppShell />`. `refresh()` also silently sets the onboarded flag for them, so they never see the flow.
- **First run** (or workspace folder removed) → `<Onboarding />`.

The flow is two honest steps, no filler:

1. **Storage** — reuses the existing `<WorkspacePicker />` (folder pick via `tauri-plugin-dialog` → `set_workspace`).
   On pick it fires `onWorkspacePicked` (re-runs `App`'s gate) and advances.
2. **Sync (optional)** — the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) + the existing `<SyncPanel />` so the user can pair right there, or skip.
   A single device is first-class.

The "has the user onboarded" flag is a **per-install UI flag in `localStorage`** (`outl.onboarded`), **not** workspace state — it deliberately does NOT go through the op log.
It is intentionally not in `settings.json` either, since `settings.last_workspace` is the only first-run signal the backend tracks.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to mobile); only the chrome is desktop-local.
Pairing is **not** reimplemented — `Onboarding` renders the real `<SyncPanel />`.

### Sync status dot (always-visible)

`<SyncIndicator />` sits in the bottom-left `<ChromeToggleBar />` cluster so the mesh state is glanceable without opening Settings.
Green = at least one iroh peer reachable, orange = none, dim = first probe still running; clicking opens Settings → Sync.
It derives reachability from `peerStatus()` → `peersOnline()` (`@outl/shared/peers`), the same source the Sync panel and the mobile dot use.
It re-probes on a slow interval and immediately on `peer-ops-changed`.
Do not add a second reachability path; `peersOnline` is the one owner.

### Sidebar page deletion

`Sidebar.tsx`'s `<Row>` takes an optional `onDelete` callback; when provided, a `×` button appears on hover.
`handleDelete(p)` calls `window.confirm(...)`, then `deletePage(slug)` (from `@outl/shared/api/commands`), applies the returned today's-journal view, and refetches the page list.
Journals are excluded — only regular pages show the affordance.
The `g d` chord (Normal mode, "go delete") routes through the `DeletePage` case in `action-handlers.ts`.
It runs the same `window.confirm` + `deletePage(slug)` flow as the `×` button.
The backend `delete_page` Tauri command is the shared `outl_tauri_shared::commands::page::delete_page` body — no desktop-specific logic.

`InlineBacklinks.tsx`'s header direction button (`setBacklinksOrder`) flips newest/oldest; `appState.backlinksOrder` hydrates at boot.

## Blockquote chrome

A `"> "`-prefixed block renders with a left border + ~6% tint, right-rounded, body full-colour; the outline bullet stays outside the quote chrome.
Detection is `splitQuote` + `stripQuoteFromTokens`; toggling routes `toggleQuote` → `toggle_quote` → `outl_actions::block::toggle_quote`.
Full convention: [`docs/clients.md` → Blockquote convention](../../docs/clients.md#blockquote-convention).

## Theme tokens

`src/lib/palette.ts::applyPaletteToRoot` writes two CSS custom-property namespaces on every theme switch:

- **`--color-outl-*`** — the canonical set.
  New desktop code uses only these (`bg-(--color-outl-bg-elev)`, `border-(--color-outl-fg)/15`, etc.).
- **`--color-ios-*` / `--color-iosd-*`** — legacy names still consumed by `MarkdownInline`, mapped from the active palette until it migrates.

`src/styles.css` provides boot-default values for both namespaces so the page isn't flash-unstyled before `applyPaletteToRoot` runs.
`color-scheme` is set from the palette's `bg` luminance so native controls (scrollbars, `<select>`) follow the active preset.

When `MarkdownInline` migrates to `--color-outl-*`, the `--color-ios-*` writes in `applyPaletteToRoot` + the legacy `styles.css` block can both go — see [`outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md#theming-note).

## Running

```bash
# from the repo root
bun install                            # hoists workspace deps

# dev (Tauri opens a native window with the Vite dev server inside)
cd crates/outl-desktop
cargo tauri dev

# production bundle (.dmg / .AppImage / .msi depending on host OS)
cargo tauri build
```

The Vite dev server runs on **port 1421** so it can coexist with `outl-mobile` (port 1420) when both are running side by side.

## Tests

| Layer | Tool | What it covers |
|-------|------|----------------|
| Rust commands | `cargo test -p outl-desktop` | command shims, settings IO, fs_watcher, surgical undo invalidation across a peer reload (`helpers::invalidate_changed_history` — only pages whose projection changed lose their stacks) |
| Frontend logic | `bun --filter outl-desktop test` | scaffold smoke, components + helpers |

Frontend suites today: `src/setup.test.ts` (scaffold smoke — `@outl/shared` alias resolves),
`src/lib/chord-format.test.ts`,
`src/lib/markdown-wrap.test.ts`,
and `src/lib/action-handlers.test.ts` — `OpenRefUnderCursor` regression (`Enter` edits the block; backlink rows open the source; pins #70).
Same file smoke-tests the block clipboard (cut arms `blockClipboard`; paste routes cut → `moveBlockAfter`, copy → `pasteBlockAfter`).

## Shortcuts

The full catalog lives in **`crates/outl-shortcuts`** (single source of truth, also consumed by the TUI).
The desktop fetches it via the `list_shortcut_bindings` Tauri command on boot and wires every `Action` through `lib/action-handlers.ts`.

Two of these chords also have **visible icon affordances** in a fixed bottom-left cluster (`components/ChromeToggleBar.tsx`, mounted by `AppShell`):
the **sidebar toggle** (`◫`, mirrors `Cmd/Ctrl+Shift+E`) and the **shortcuts-help toggle** (`?`, mirrors `?` / `Cmd/Ctrl+/`).
They carry no business logic — clicking flips the same store signal the dispatcher flips, so button and keyboard stay in sync.
The cluster floats over the main pane on an elevated, bordered surface (active toggle inverts to the accent color), so the sidebar button stays reachable even after the left pane is hidden.

### OS-standard chrome and undo / redo

The full per-chord table is in [`docs/shortcuts.md`](../../docs/shortcuts.md) — the single source of truth, shared with the TUI.
Desktop-specific: `Cmd/Ctrl+Shift+X` runs the focused / selected code block (plain `Cmd/Ctrl+X` is OS cut / view-mode block cut).

### Undo / redo (Normal mode — fire when no textarea is focused)

Chords (`Cmd/Ctrl+Z` undo, `Cmd/Ctrl+Shift+Z` redo, `u` / `Ctrl+R` vim spelling) live in [`docs/shortcuts.md`](../../docs/shortcuts.md).
Deliberately **Normal**, not Global: with a textarea focused the chord falls through to the webview (the in-flight draft is the textarea's own undo domain), and a Global binding would `preventDefault` it away.
History is **block-level**: each mutation that goes through `finish_in_page` (edit, create, indent / outdent, move, delete, TODO / quote toggle, paste)
pushes the page's pre-mutation `.md` render onto a bounded per-page stack (`outl_actions::history::HistoryStacks`);
undo restores the snapshot through `outl_md::reconcile_md`, so the restore is itself ops in the log — the op log stays the source of truth, nothing is rewritten.
Fold toggles (`set_block_collapsed`) bypass `finish_in_page` and are not undoable, matching their "view state, not content" semantics.
Invalidation is **surgical**: a workspace **switch** clears every stack,
but a peer-driven **reload** (`peer-ops-changed` → `reload_workspace`) drops only the stacks of pages whose projection actually changed across the reload — restoring one of those would silently revert the peer's edits.
Pages the peer didn't touch keep their full undo depth.

### Inline markdown (Insert mode — fire when a textarea is focused)

`Cmd/Ctrl+B`/`I`/`E`/`Shift+X`/`K` wrap the selection (or insert the delimiter pair around the caret) — bold / italic / inline code / strikethrough / link.
The full chord + output table lives in [`docs/shortcuts.md`](../../docs/shortcuts.md).
Implementation lives in `lib/markdown-wrap.ts`: each handler reads `document.activeElement`, splices the value, dispatches an `input` event so `<BlockRow />`'s Solid signal stays in sync, then repositions the caret / selection.

### Paste (with and without formatting)

User-facing behaviour + routing lives in [`docs/paste.md`](../../docs/paste.md).
Three guards (mobile mirrors them):

- Code-fence host bails `Cmd/Ctrl+V` to the native splice (`detectFence` early-return), keeping it literal.
- `Cmd/Ctrl+Shift+V` reads via `tauri-plugin-clipboard-manager` (`clipboard-manager:allow-read-text`), dodging the macOS webview "Paste" gate.
- Both pass `textarea.value` so `flushDraftBeforePaste` commits the draft first.

`create_block`: stale `after_id` (`NotInTree`) → append at page end (fixes `o`-key crash after peer reload).

### Block-editor chords (inside a block's textarea)

The user-facing chord table lives in [`docs/shortcuts.md`](../../docs/shortcuts.md).
Load-bearing notes a contributor needs:

- **Plain `Enter` → commit + new sibling below** (`onEnter`, TUI parity).
  `Shift+Enter` → literal `\n` soft break (issue #119), handled in `BlockRow`'s `handleKeydown` (not the catalog; see the code comment).
- `Cmd/Ctrl+X` (cut) and `Cmd/Ctrl+Z` (undo) deliberately fall through to the webview — no catalog binding matches inside a textarea.
  Native per-keystroke undo is still broken: the controlled `value={draft()}` binding resets the textarea's undo stack on every keystroke (issue #80).
- Bracket auto-pairing (`[[`/`((` auto-close, `(`/`[`/`{` auto-pair with caret between, closer step-over, empty-pair collapse on `Backspace`) all live in `@outl/shared/autocomplete` (`autoPairBracket` / `autoDeletePair`, TUI parity).
- The four inline autocomplete popups (slash / emoji / block-ref / page-ref) share one keyboard contract via `handlePopupNav` (`lib/popup-nav.ts`, unit-tested): arrows cycle, `Enter`/`Tab` with no modifiers accept, `Esc` closes.
  `Shift+Tab` outdents on every popup now (the page-ref one used to accept it).

### Vim parity (Normal + Visual)

User-facing chord list lives in [`docs/shortcuts.md`](../../docs/shortcuts.md) — don't duplicate it here.
This section captures only the **architectural decisions** a contributor needs to know before touching `lib/action-handlers.ts`.

- **Three categories of vim ops**, by what they need from the cursor model:
  1. **Block-level** (`a`, `A`, `S`, `Y`, `*`, `#`, `z R`, `z M`, `z z`, `V`, `g v`, `>` / `<` in Visual, `y` / `d` in Visual) — work on `selectedBlockId` or a range of block ids.
     **Implemented.**
  2. **Char-cursor in Normal** (`x` `X` `D` `C` `s` `r{ch}` `f{ch}` `F{ch}` `~` `e`) — need a character cursor inside the selected block.
     The desktop has no such cursor (only an id), so these handlers **surface a status-line nudge** pointing the user at `i` + textarea edits.
     Catalog entries stay so the help overlay shows them.
  3. **Pending-input** (`r{ch}`, `f{ch}`, `F{ch}`) — read a second character before applying.
     The dispatcher has no machinery for this today; categorised as char-cursor since they're blocked anyway.

- **Visual mode is real, and reachable without vim (issue #23)**: `Mode::"vim-visual"` in the store with `visualAnchorId` + `lastVisualRange`, painted at 18% accent opacity.
  `Shift+↓` / `Shift+↑` (`SelectRange{Down,Up}`, bound in **both** Normal and Visual) start or grow the range via the DOM "nothing focused" → Normal fallback, so one machinery serves vim and non-vim.
  `extendVisualRange` stays in the outline, never crossing into read-only backlinks.
  Every exit funnels through one `exitVisual()` (captures `lastVisualRange` for `g v`); resting `vim-normal` folds to `normal` in `detectMode` + `StatusBar`.

- **`<BatchToolbar />`** (`components/BatchToolbar.tsx`) floats `N selected` + Indent / Outdent / Move ↑↓ / Delete / Done while `mode === "vim-visual"`, firing the **same** `handlers` the keyboard does so button and chord can't drift.
  Its **Delete** confirms (`window.confirm`) when a selected block has children; keyboard `d` does not (vim convention).

- **Range ops walk bottom-up or top-down via `applyVisualBlockOp`, and tolerate id-already-gone.**
  `DeleteRange` + `MoveVisualRangeDown` iterate `[hi → lo]` (children before parents; a descending move clears the block below first); `IndentVisualRange` + `MoveVisualRangeUp` walk `[lo → hi]`.
  NodeIds are stable (`deleteBlock` is `Move(node, TRASH)`, moves preserve identity), so the highlight follows the re-render; `safeCall` swallows per-id failures.

- **`UnfoldAll` / `FoldAll` walk via `flattenAll` / `flattenParents`, never `flattenVisible`.**
  `zR` must expand subtrees hidden under a collapsed parent; a visible-only walk would no-op on every descendant of a folded node.
  `zM` (fold-all) uses `flattenParents` because `set_block_collapsed` always writes `Op::SetCollapsed` (the CRDT needs every flip to converge), so folding a leaf would make future children appear collapsed.
  `zR` (unfold-all) uses `flattenAll` (unfolding a leaf has no future effect).
  Mirrors `outl-tui`'s `collect_collapse_candidates` so the op count matches across clients.

- **`A` (`EnterInsertAtEnd`) routes through `appState.caretIntent`.**
  The handler sets `caretIntent: "end"` *before* `editingBlockId`; `<BlockRow />`'s `createEffect` reads it on mount, applies `setSelectionRange`, then clears the signal.

- **Visual highlight uses a memoised `Set<id>` at the parent, not a per-row predicate.**
  `<OutlineView />` builds `visualSet = createMemo(() => visualRangeSet(...))` once per change; `<BlockRow />` answers `props.visualSet?.has(id) ?? false` in O(1).
  The old per-row `isInVisualRange` (O(N²)/keystroke) is kept in `@outl/shared/outline` for tests only.

- **Char-cursor nudge is one shared handler:** the 10 char-cursor entries (`x` `X` `D` `C` `s` `r` `~` `e` `f` `F`) all point at `charCursorNudge`, so the message can't drift.

- **`Y` / Visual `y` copy to the OS clipboard** via `copy_markdown` + `navigator.clipboard.writeText` (fills `yankRegister`; paste-in `p`/`P` deferred).

- **`NewBlockAbove` (`O`) uses `beforeId`, not a post-creation move walk.**
  `createBlock({ beforeId: anchor })` → `create_before` (floor-slot swap in core); never reintroduce the old create-at-tail + `moveBlockDown`-loop.
  `Cmd/Ctrl+Shift+Enter` is caret-aware in `BlockRow`'s keydown (col 0 → *before*, past col 0 → *below*); `stopImmediatePropagation` preempts the catalog's create-below binding.

- **Block clipboard: view-mode cut/copy/paste of a whole block** (chords: [`docs/shortcuts.md`](../../docs/shortcuts.md)).
  **Normal**-mode only; `appState.blockClipboard` = `{ kind: "cut", nodeId } | { kind: "copy", markdown }` (backend resolves the page via `enclosing_page_id`).
  Cut is one identity-preserving `Op::Move` (`block::move_after`, cross-page, self-subtree rejected); copy duplicates via `paste_block_after` with fresh ids.

### Zoom / focus into a block (Roam/Workflowy)

Click a neutral `•` bullet, or fire `ZoomIn` (`z i` / `Cmd/Ctrl+Shift+]`), to zoom into the selected block; `ZoomOut` (`z o` / `Cmd/Ctrl+Shift+[`) pops one level.
The header becomes the focused block's own **page-like header** (Roam-style).
The focused block's text is the `<h1>` title.
The eyebrow is a clickable **zoom path**: a leading page crumb (`📅 <slug>` / `📄 <title>`) exits the zoom back to the journal/page, then one crumb per ancestor re-focuses it.
The outline body renders the focused block's **children** (`rootBlocks()` → `fv.root.children`), so the block isn't duplicated as both title and first row.
`navBlocks()` traverses the same children so `j`/`k` stay in the body.
`addFirstBlock()` creates into the focused block (`parentId: focusBlockId`) when it's a zoomed-into leaf.

Load-bearing decisions:

- Zoom is **local view state, never an op** — `appState.focusBlockId` (default `null`), sliced at render time via `focusSubtree` (`@outl/shared/outline`).
  No Tauri round-trip, no `PageView` change; a display preference like `backlinksOrder`, not cross-device state.
- **No zoom stack:** `ZoomOut` reads the current focus's breadcrumb — last crumb is the parent, empty breadcrumb means top-level so it exits to the full page.
- **Bullet gesture split** (no TODO collision): a `•` (non-TODO) bullet zooms via `onFocusBlock`; a `▢`/`▣` checkbox keeps its TODO toggle; the fold **chevron** stays collapse.
- **Stale zoom self-heals:** `focusSubtree` → `null` when the id left the outline (peer delete / off-page move) clears `focusBlockId`; page navigation resets it too.
- **`j`/`k` stay inside the zoom:** `SelectionUp`/`SelectionDown` walk `navBlocks()` (`[fv.root]` when zoomed) so the cursor can't escape the subtree.

### `Enter` outside a textarea (Normal mode)

With a block selected and no textarea focused (the DOM fallback puts the dispatcher in Normal mode even with `vim_mode == false`),
`Enter` resolves to the shared `OpenRefUnderCursor` action — but the desktop handler **always enters Insert on the selected block**.
The one exception: when the selection sits on a **backlink row** (read-only), `Enter` opens the source page and lands the cursor on the referencing block.

Why diverge from the TUI: the TUI has a char cursor so "open the ref under cursor" is well-defined; the desktop only has a selected block, so **following a ref is the click on the token** (`onRefClick`) and `Enter` means edit.

### `:shortcode:` emoji autocomplete

Inside an open `:shortcode` trigger, `BlockRow` shows `EmojiSuggestPopup`, reusing `detectEmojiContext` / `applyEmojiSuggestion` and the `searchEmojis` command (backed by `outl_md::emoji::search`).
Accept inserts the canonical `:shortcode:` (the `.md` stores the literal, never the codepoint).
It beats the ref popup at the same caret (`detectEmojiContext` only fires on word-initial `:[a-z]`).

### `[[page]]` ref autocomplete

Inside an open `[[…]]`, `BlockRow` shows `RefSuggestPopup`, reusing the shared `detectRefContext` / `applySuggestion` helpers and the `search_pages` command the `Cmd+P` picker already calls.
Accept inserts the page title (or ISO slug for journals).

### `((block ref))` autocomplete

The `((` counterpart of `[[page]]` (issue #116).
Inside an open `((…))`, `BlockRow` shows `BlockSuggestPopup`, reusing `detectRefContext` (`kind: "block"`) / `applySuggestion` plus `search_blocks` (`outl_md::WorkspaceIndex::search_block_text`).
Rows show snippet + slug; the pick inserts the **ref handle** (`((blk-XXXXXX))`), never the text; mobile registers the command for parity, popup unwired.
`search_blocks` rebuilds the index from disk, so it's debounced ~150ms.

### Clicking external `[label](url)` links

`<MarkdownInline />` renders external markdown links clickable when given an `onLinkClick(href)` prop.
`OutlineView` wires it to `openExternalUrl` (`@outl/shared/api/commands`),
which scheme-guards to `http(s)`/`mailto` and opens in the system browser via **`tauri-plugin-opener`** (registered in `src-tauri/src/lib.rs`;
the capability grants a scoped `opener:allow-open-url` for `http`/`https`/`mailto` in `capabilities/default.json`).
Failures (malformed URL, disallowed scheme) land on the status line via `appState.lastError`.
The `[[ref]]` / `#tag` click handlers are unchanged (they navigate the workspace, not the browser).
The opener call lives in the shared wrapper (not a custom Tauri command), so mobile can opt in later.
Backlink rows stay inert (the whole row is already a navigate-to-source button; nesting a second click target would conflict).

### `/template` slash entry

The block-initial `/` menu lists native `template: <name>` rows (`templateSlashCommands`, `lib/slash-commands.ts`) that `OutlineView` runs via `instantiateTemplateAt`.
Contract + backend: [`docs/clients.md` → Structural templates](../../docs/clients.md#structural-templates).

In a `call:<name>` fence, `CodeFenceView`'s `CALL:<NAME>` chip links to the template page — `onOpenPage`→`openPageBySlug` (exact, not `openRef`), slug via `listTemplates()`; unknown name = inert chip.

## Plugins

JS plugins (`outl_plugins::PluginHost`) run on the desktop, but the host embeds a Boa `Context` that is **`!Send`**, so it can never live in the `Send + Sync` `AppState`.
The host therefore runs on a **dedicated plugin thread** (`src-tauri/src/plugin_service.rs`); `AppState` holds only a `PluginService` (a `Send + Sync` clone of a `std::sync::mpsc::Sender<PluginRequest>`).

Design:

- `spawn_plugin_service(workspace, storage_root, hlc)` (the desktop shim over `outl_tauri_shared::PluginService::spawn`, called once in `lib.rs::setup` after `open_today`/opener wiring) starts the thread.
  It is handed **clones of the same `Arc<Mutex<Option<Workspace>>>` and `Arc<Mutex<Option<PathBuf>>>` every Tauri command locks**, plus the per-device `HlcGenerator`.
  The `Workspace` is `Send`; the Boa `Context` never crosses a thread boundary.
- The thread owns the `PluginHost`.
  It loads plugins from `<root>/.outl/plugins/` lazily on the **first request after the workspace opens** (`ensure_loaded`), then `mark_synced` so pre-existing ops don't fire `onOp` hooks at boot.
  A workspace **swap** (different `storage_root`) rebuilds the host against the new root.
- Each request (`ListCommands` / `RunCommand` / `SyncHooks`) carries a one-shot `std::sync::mpsc::Sender` reply channel.
  The Tauri command sends the request, then **blocks on `recv()` with the workspace `Mutex` released** (never held across the reply) — the plugin thread is the one that locks the workspace to run the host.
  No `.await` ever holds the lock.
- After a plugin mutation (`run.applied > 0`), the plugin thread re-projects **every** page's `.md` via `outl_actions::apply_all_pages_md` before replying.
  A plugin can move blocks to any page — same rationale as the TUI's `reproject_after_plugin`.

Capabilities honored: `slash-command` + `op-hook` + `ui-render` + `keybinding` + `toolbar-button`.
The host filters `keybinding` / `toolbar-button` by declared capability **before** `keybindings("desktop")` / `toolbar_buttons("desktop")` return anything,
so both must be in `client_capabilities()` or the desktop sees an empty list.

Tauri commands (`commands/plugin.rs`):

| Command | Returns | Behaviour |
|---|---|---|
| `plugin_list` | `Vec<PluginCommandDto>` | Every contributed command (best-effort; empty until plugins load) |
| `plugin_run(plugin_id, command_id, page_id?)` | `PluginRunReply` (`applied`, `notifications`, `errors`, `view?`, `views`) | Runs the command on the plugin thread; `view` is the refreshed `PageView` of the on-screen page, `views` are emitted `ui-render` HTML overlays |
| `plugin_sync_hooks(page_id?)` | `PluginSyncHooksReply` (`view?`, `views`) | Fires the `onOp` sweep; `view` is a refreshed `PageView` **only** when a hook mutated the workspace, `views` are emitted `ui-render` overlays (present even on the no-mutation path) |
| `plugin_keybindings` | `Vec<PluginKeybindingDto>` (`chord`, `mode`, `plugin_id`, `command_id`, `description`) | Plugin-contributed desktop chords (best-effort; empty until plugins load) |
| `plugin_toolbar` | `Vec<ToolbarButtonDto>` (`plugin_id`, `command_id`, `icon`, `title?`) | Plugin-contributed desktop chrome buttons (best-effort; empty until plugins load) |
| `plugin_transformers` | `Vec<TransformerDto>` (`plugin_id`, `lang`, `kind`) | Content transformers a plugin declared for a code-fence language (best-effort; empty until plugins load) |
| `plugin_transform(plugin_id, lang, input)` | `Option<TransformResultDto>` (`kind`, `content`) | Runs the content transformer for `lang` against a fence body. **Read-only** — never mutates the workspace, no re-projection. `None` when the transformer declined or no plugin owns `lang` |

### `keybinding` + `toolbar-button` contributions

`lib/shortcuts.ts` loads `plugin_keybindings()` per `installShortcuts` (re-fetched on workspace swap, **not** module-cached) and folds the chords into the `keydown` dispatcher as a **Global overlay**.
The DTO's `chord` / `mode` serialize identically to the `outl-shortcuts` catalog, so the dispatcher reuses its `Chord` / `seqEq` machinery unchanged.
**Native always wins:** a plugin chord fires only after the native catalog matched nothing (match *and* prefix) and no native binding owns that chord in *any* mode (`nativeOwnsChord`) — a plugin can't shadow `Cmd+B` / `Cmd+P`.
`components/ChromeToggleBar.tsx` loads `plugin_toolbar()` on mount and renders one momentary button per entry in the native cluster (glyph = `icon`, tooltip = `title`, click = `plugin_run`).
Both paths run a command like the palette does: status-line output, re-render from `reply.view`, `playPluginViews(reply.views)`.

Op-hooks fire `pluginSyncHooks` at **two post-mutation points**: `OutlineView`'s `onCommit` (after an edit) and the `ToggleTodo` handler (`Cmd+T`).
`sync_hooks` dispatches **every** op since the host's last sweep, so one call also catches up structural ops (indent / move / delete) — mirrors the TUI's once-per-tick sweep.
Best-effort: a host with no op-hook plugins is a cheap no-op.

### `ui-render` overlays (sandboxed iframe)

A `ui-render` plugin emits HTML/JS via `ctx.ui.render(html)`.
The core gates these on the capability and surfaces them on `PluginRun.views`, propagated as `PluginRunReply.views` / `PluginSyncHooksReply.views`.
The desktop plays each as an **ephemeral, fully sandboxed `<iframe>` overlay**:

- `lib/plugin-views.ts` owns a Solid signal queue (`playPluginViews` enqueues, `dismissPluginView` pops).
- `components/PluginEffectLayer.tsx` (mounted once in `AppShell`) renders one iframe per entry: `position: fixed; inset: 0` fullscreen, transparent, `pointer-events: none` (click-through), `z-index: 9999`, auto-removed after 6s.
  Multiple views stack.
- **Security (load-bearing — never weaken):** the iframe is `sandbox="allow-scripts"` **without** `allow-same-origin`.
  The plugin JS runs in a null origin — no app DOM, cookies, `localStorage`, or credentialed fetch.
  HTML enters via `srcdoc`, never `innerHTML` on the host document.
  This is untrusted third-party code; the isolation is the whole point.

Played from three call sites: `PluginPalette` (after `pluginRun`), `OutlineView.onCommit`, and the `ToggleTodo` handler (after `pluginSyncHooks`).
The confetti example (`examples/confetti`, `op-hook` + `ui-render`) rides this: mark a block DONE → op → `sync_hooks` → its `onOp` emits the confetti HTML → `views` → overlay.

Frontend pieces: plugin DTOs + wrappers from `@outl/shared/api` (`lib/api.ts` keeps only `pluginKeybindings`); `lib/plugin-views.ts` + `components/PluginEffectLayer.tsx` (overlay queue).
The `⧉` button in `ChromeToggleBar` toggles `appState.pluginsOpen`; `components/PluginPalette.tsx` lists + runs commands.

### Content transformers (inline code-fence rendering)

A plugin can declare a transformer for a code-fence language (`mermaid`, …); matching fences render through it in `CodeFenceView` (`components/BlockRow.tsx`).
Registry + cache glue: `@outl/shared/plugins/transformer-registry` (shared with mobile); keeps `BlockRow` a renderer.
It owns a `lang → PluginTransformer` registry (Solid signal, loaded via `loadTransformers` — `AppShell.onMount` + re-run on `workspace-ready`; plugins load lazily, a boot fetch can be empty).
A `(blockId, body)` result cache (`runTransform`) re-runs plugin JS only when the body changes.
`kind: "text"` renders as plain whitespace-preserving text (no client-side markdown parse — a transformer wanting formatting emits `rich`).
`kind: "rich"` renders the HTML in an **inline** `<iframe>` (`RichFenceFrame`), sized via an optional `parent.postMessage({outlHeight})` handshake.
**Security (never weaken):** that iframe is `sandbox="allow-scripts"` **without** `allow-same-origin`, HTML via `srcdoc` — same isolation as the `ui-render` overlay, only inline + persistent instead of fullscreen + ephemeral.
`content-transformer:text` / `:rich` are in `client_capabilities()` (the host gates transformers by capability before listing them).

## Logging

`run()` in `src-tauri/src/lib.rs` installs a `tracing_subscriber` fmt subscriber writing to **stderr** as its first step (before rustls / Tauri setup).
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
Running `cargo tauri dev` from a terminal then shows the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") so device↔device sync is debuggable.
Init uses `.try_init()` so a double-init can't panic.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) for what the transport logs.

## Settings

Stored at `<app_config_dir>/settings.json`:

- macOS: `~/Library/Application Support/app.outl.desktop/`
- Linux: `~/.config/app.outl.desktop/`
- Windows: `%APPDATA%\app.outl.desktop\`

Schema (`crates/outl-desktop/src-tauri/src/settings.rs::Settings`):

```jsonc
{
  "last_workspace": "/Users/me/iCloud/outl",
  "vim_mode": false,
  "theme": "auto",       // "light" | "dark" | "auto"
  "font_size": 15,
  "sync_transport": "iroh",  // "iroh" (P2P, default) | "file" (iCloud/fs)
  "backlinks_order": "newest"  // "newest" (default) | "oldest" — read-only, see below
}
```

The Sync transport select in `SettingsModal` writes `sync_transport`.
`settings.rs` maps it to/from `[sync] transport` and preserves `relay_url` on save; takes effect on next launch.
`backlinks_order` is read-only here — `save` restores it from disk (same pattern as `[calendar]`) so the modal can't clobber the dedicated `set_backlinks_order` command's write.

The actor id (one per device) lives next to it as `actor` — a plain ULID.
Switching workspaces does not rotate it.

## Peers

Paired devices live in `<workspace>/.outl/peers.json` (per-graph), owned by `outl_sync_iroh::PeersStore` (see [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md)).
The desktop exposes two thin Tauri commands in `commands/peers.rs` — no business logic, they just load the store and project / mutate it:

| Command | Returns | Behaviour |
|---|---|---|
| `outl_peer_list` | `Vec<PeerDto>` (`node_id`, `alias`, `added_at`) | Loads `peers.json` (or default if absent) and lists every paired peer |
| `outl_peer_remove(id)` | `bool` | Removes peers whose `node_id` starts with `id` (prefix match); `true` if any were removed |

The path is `<workspace>/.outl/peers.json` (resolved from `AppState::storage_root` via `outl_sync_iroh::workspace_peers_path`) — the same per-graph location the CLI and the iroh transport read, not `~/.outl/` or `<app_config_dir>`.
Each command runs `migrate_global_peers_if_absent` first, so a user with a legacy global list keeps their peers on first open.
Only `identity.key` stays global (`~/.outl/`).

`commands/peers.rs` also exposes `outl_sync_now()` (reads `state.iroh_transport`, the `Arc<dyn SyncTransport>`, and calls the trait's `sync_now()`) — the force-sync trigger behind the Sync panel's Refresh.

### Sync panel dot + refresh (iroh-driven)

`components/SyncPanel.tsx` (the "Sync" section of `SettingsModal`) is the only place the desktop surfaces sync state; there is **no** always-on chrome dot (`StatusBar` / `ChromeToggleBar` carry none).
The panel header shows a small status dot derived from the shared `peersOnline(statuses())` helper (`@outl/shared/peers`) — green when at least one iroh peer is reachable, orange when none are (no peers paired, or all unreachable).
The **Refresh** button calls `forceSync()`: `syncNow()` (force a P2P pull) → `reloadWorkspace()` (re-render) → `refresh()` (re-read the device list + health for the dots).
`syncNow` / `reloadWorkspace` failures land on `appState.lastError` but never block the status read.
`syncNow()` + `peersOnline()` live in `@outl/shared` so desktop and mobile derive the dot + drive the refresh identically — see [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Deep links (`outl://`)

The desktop registers the `outl://` scheme so external launchers (the Raycast extension, shared links) jump straight to a page or daily note (issue #98).
The scheme contract and the shared parser live in `outl-actions` — see [`docs/clients.md` → Deep links](../../docs/clients.md#deep-links-outl) — so the desktop and mobile handlers can't drift.

Wiring (all in `src-tauri/src/lib.rs`):

- **Plugins.**
  `tauri-plugin-single-instance` is registered **first**.
  Its `deep-link` feature forwards an `outl://` URL opened while the app runs to the existing instance on Linux/Windows; the callback just focuses the `main` window.
  `tauri-plugin-deep-link` follows.
  The scheme is declared in `tauri.conf.json` under `plugins.deep-link.desktop.schemes` and granted via `deep-link:default` in `capabilities/default.json`.
- **Warm path** (`dispatch_deep_link`, fired by `on_open_url`) parses the URL with `outl_actions::parse_deep_link` — the one owner, this crate adds no parsing.
  It then **emits** `deep-link://navigate` with one of `{kind:"today"}` / `{kind:"daily",date}` / `{kind:"page",slug}` and focuses the window.
  A malformed URL is logged at `warn` and ignored — never a crash, never a stray page.
- **Cold path** (a URL that *launched* the app) can't emit — the frontend listener isn't mounted yet.
  So `setup()` buffers the parsed payload in a managed `PendingDeepLink(Mutex<Option<Value>>)` instead, and the `take_pending_deep_link` command drains it once on mount.
  Only the launch URL populates the buffer; the warm path never does, so a stale target can't replay on the next plain launch.
- **Frontend.**
  `AppShell` listens via `onDeepLinkNavigate` (`lib/events.ts`) for the warm path.
  On mount it calls `takePendingDeepLink()` (`lib/api.ts`) for the cold path — if a target is buffered it navigates there instead of loading today's journal (which would otherwise race and overwrite it).
  Both map onto the same `openTodayJournal` / `openJournalFor` / `openPageBySlug` commands the picker already calls, then `applyView`.
  The backend, not the frontend, owns parsing + window focus.

**Testing on macOS needs a bundled, installed app.**
macOS registers URL schemes only via LaunchServices from the bundle's `CFBundleURLTypes` (written by `tauri-plugin-deep-link` at `cargo tauri build`), so `cargo tauri dev` does **not** register `outl://`.
To test: `cargo tauri build`, copy the `.app` into `/Applications`, open it once so LaunchServices indexes it, then `open "outl://page/<slug>"`.
Linux/Windows register at runtime (`register_all()` in `setup`), so dev mode works there.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-desktop --all-targets -- -D warnings`
3. `cargo test -p outl-desktop`
4. `bun --filter outl-desktop test` (Vitest)
5. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click around the parts you touched.
6. If you touched anything in `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
