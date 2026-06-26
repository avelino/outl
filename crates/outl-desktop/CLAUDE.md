# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

## Status

**Phase 6 — feature-complete v0.**
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
├── CLAUDE.md
├── package.json               # bun workspace, deps @outl/shared + tauri + dialog plugin
├── tsconfig.json
├── tsconfig.node.json
├── vite.config.ts
├── vitest.config.ts
├── index.html
├── src/                       # frontend (Solid)
│   ├── index.tsx              # mount
│   ├── App.tsx                # Onboarding / AppShell switch (first-run gate)
│   ├── styles.css             # Tailwind v4 entry + theme tokens
│   ├── setup.test.ts          # smoke (@outl/shared resolves)
│   ├── components/
│   │   ├── AppShell.tsx       # 3-pane grid
│   │   ├── Sidebar.tsx        # Today / Journals / Pages, filter input
│   │   ├── OutlineView.tsx    # editable outline (owns BlockCallbacks)
│   │   ├── BlockRow.tsx       # block render + textarea editor + CodeFenceView
│   │   ├── BacklinksPanel.tsx # right pane
│   │   ├── Picker.tsx         # Cmd+P quick switcher
│   │   ├── SettingsModal.tsx  # Cmd+, settings
│   │   ├── ChromeToggleBar.tsx# bottom-left cluster: sidebar / help toggles + SyncIndicator
│   │   ├── SyncIndicator.tsx  # always-visible sync dot → opens Settings → Sync
│   │   ├── Onboarding.tsx     # first-run flow (storage pick → optional pairing)
│   │   └── WorkspacePicker.tsx
│   └── lib/
│       ├── api.ts             # desktop-only commands (workspace, settings, exec)
│       ├── code-block.ts      # detect ```lang fences
│       ├── events.ts          # listen workspace-ready / peer-ops-changed
│       ├── shortcuts.ts       # Cmd+P/B/\\/,/T/[/] handler
│       └── store.ts           # Solid createStore (panel state, page view)
└── src-tauri/
    ├── Cargo.toml             # outl-desktop crate manifest
    ├── build.rs
    ├── tauri.conf.json        # identifier app.outl.desktop, 1280×800 window
    ├── capabilities/
    │   └── default.json       # core:default + dialog:default/allow-open
    ├── icons/                 # placeholder icons (mirror of mobile)
    └── src/
        ├── main.rs            # binary entry
        ├── lib.rs             # mod decls + run() (registers all 25 commands)
        ├── settings.rs        # settings.json IO + tests
        ├── state.rs           # AppState, PageView, WorkspaceSummary
        ├── helpers.rs         # parse_node_id, with_ws*, finish_in_page
        ├── workspace_open.rs  # open_workspace_at + spawn_workspace_opener
        ├── fs_watcher.rs      # notify + debouncer → peer-ops-changed
        └── commands/
            ├── mod.rs
            ├── workspace.rs   # set_workspace, current_workspace, reload, settings, stats
            ├── page.rs        # list / search / open / journal nav / resolve_ref
            ├── block.rs       # create / edit / todo / move / collapsed / paste
            ├── exec.rs        # run_code_block — thin Tauri adapter over outl_actions::exec::run_code_block (shared with mobile)
            └── peers.rs       # outl_peer_list / outl_peer_remove — read/edit <workspace>/.outl/peers.json via outl_sync_iroh::PeersStore
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

The "has the user onboarded" flag is a **per-install UI flag in `localStorage`** (`outl.onboarded`), **not** workspace state — it deliberately does NOT go through the op log (it must not converge across devices; each device onboards once).
It is intentionally not in `settings.json` either, since `settings.last_workspace` is the only first-run signal the backend tracks.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to mobile); only the chrome is desktop-local.
Pairing is **not** reimplemented — `Onboarding` renders the real `<SyncPanel />`.

### Sync status dot (always-visible)

`<SyncIndicator />` sits in the bottom-left `<ChromeToggleBar />` cluster so the mesh state is glanceable without opening Settings (GUI users expect that, the way mobile shows a dot in its toolbar).
Green = at least one iroh peer reachable, orange = none, dim = first probe still running; clicking opens Settings → Sync (the full `<SyncPanel />`).
It derives reachability from `peerStatus()` → `peersOnline()` (`@outl/shared/peers`) — the **same source** the Sync panel and the mobile dot use, so the three never disagree.
It re-probes on a slow interval and immediately on `peer-ops-changed` (a delivery proves the mesh is up).
Do not add a second reachability path; `peersOnline` is the one owner.

## Blockquote chrome

A block whose `text` starts with the CommonMark `"> "` marker renders with a left border (`border-l-2 border-(--color-outl-fg-dimmer)/50`),
a very faint tint (`bg-(--color-outl-fg-dimmer)/[0.06]`),
a right-rounded corner (`rounded-r-md`),
and **full body colour** — refs, tags, bold, code keep their normal palette so the styled-token affordance isn't lost.
The tint is intentionally ~6% alpha: enough to read as a soft box at a glance, low enough to not fight with surrounding outline rows.

The chrome wrapper sits one level above the bullet button — it envelops **both bullet *and* body** as one flex container.
That order (`│ ☐ body`) matches the TUI exactly and reads as "this is a quoted task" instead of "a task whose body happens to be a quote".
The fold chevron and indent guides stay *outside* the chrome so the gutter chrome doesn't end up boxed twice.
When the block isn't quoted, the wrapper degrades to a plain `flex min-w-0 flex-1 items-start` container, so non-quoted rows render byte-identical to before.

TUI has no per-line background available in ratatui, so it stays with just the `│ ` bar — the tint is a desktop/mobile addition that costs nothing to omit on terminal.
The detection uses `splitQuote` from `@outl/shared/markdown` (mirror of `outl_actions::quote::split_quote`);
`stripQuoteFromTokens` drops the `> ` from the first `Plain` token before handing the list to `<MarkdownInline />` so the marker doesn't render twice.

Composition: the marker stacks with TODO/DONE the same way the TUI does (`> TODO foo` → quote chrome + checkbox).
Toggling the marker goes through the `toggleQuote(pageId, id)` wrapper in `@outl/shared/api/commands`,
which calls the `toggle_quote` Tauri command (`src-tauri/src/commands/block.rs`),
which delegates to `outl_actions::block::toggle_quote` — the same Rust function the mobile and TUI surfaces hit.
**No string surgery on the TS side** — the prefix arithmetic owns the rule and stays in one place.

## Theme tokens

`src/lib/palette.ts::applyPaletteToRoot` writes two CSS custom-property namespaces on every theme switch:

- **`--color-outl-*`** — the canonical set.
  New desktop code uses only these (`bg-(--color-outl-bg-elev)`, `border-(--color-outl-fg)/15`, etc.).
- **`--color-ios-*` / `--color-iosd-*`** — legacy names still consumed by `@outl/shared/markdown::MarkdownInline`.
  They are mapped from the active palette so the shared renderer stays client-agnostic until it migrates.

`src/styles.css` provides boot-default values for both namespaces (the `outl` brand palette) so the page isn't flash-unstyled before `applyPaletteToRoot` runs.
`color-scheme` (`light` / `dark`) is also set from the palette's `bg` luminance so native controls (scrollbars, `<select>`) follow the active preset.

**When `@outl/shared/markdown::MarkdownInline` migrates to `--color-outl-*`**, the `--color-ios-*` writes in `applyPaletteToRoot` and the legacy block in `styles.css` can both be removed.
See [`outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md#theming-note) for the migration plan.

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
| Rust commands | `cargo test -p outl-desktop` | command shims, settings IO, fs_watcher (Phases 1+), surgical undo invalidation across a peer reload (`helpers::invalidate_changed_history` — only pages whose projection changed lose their stacks) |
| Frontend logic | `bun --filter outl-desktop test` | scaffold smoke (today), components + helpers (Phases 1+) |

Frontend suites today: `src/setup.test.ts` (scaffold smoke — `@outl/shared` alias resolves),
`src/lib/chord-format.test.ts`,
`src/lib/markdown-wrap.test.ts`,
`src/lib/outline-walk.test.ts`,
and `src/lib/action-handlers.test.ts` (regression tests for the `OpenRefUnderCursor` handler — Normal-mode `Enter` enters Insert on the selected block even when it carries a `[[ref]]`,
and only a backlink-row selection opens the source page; pins #70).

## Shortcuts

The full catalog lives in **`crates/outl-shortcuts`** (single source of truth, also consumed by the TUI).
The desktop fetches it via the `list_shortcut_bindings` Tauri command on boot and wires every `Action` through `lib/action-handlers.ts`.

Two of these chords also have **visible icon affordances** in a fixed bottom-left cluster (`components/ChromeToggleBar.tsx`, mounted by `AppShell`, VS Code activity-bar convention):
the **sidebar toggle** (`◫`, mirrors `Cmd/Ctrl+Shift+E`) and the **shortcuts-help toggle** (`?`, mirrors `?` / `Cmd/Ctrl+/`).
They carry no business logic — clicking flips the same `appState.sidebarOpen` / `appState.helpOpen` store signal the dispatcher flips, so button and keyboard stay in sync.
The cluster floats over the main pane on an elevated, bordered surface (clear contrast against page content; active toggle inverts to the accent color), so the sidebar button stays reachable even after the left pane is hidden.

### OS-standard chrome (Global mode — fire in any context)

| Chord | Action |
|---|---|
| `Cmd/Ctrl+P` | Quick switcher (pages + journals, fuzzy) |
| `Cmd/Ctrl+J` | Open today's **j**ournal |
| `Cmd/Ctrl+T` | Toggle TODO / DONE on the focused / selected block (T for **t**ask) |
| `Cmd/Ctrl+Enter` | Toggle TODO / DONE on the focused / selected block (alt) |
| `Cmd/Ctrl+Shift+Enter` | Commit + create a sibling block below |
| `Cmd/Ctrl+Shift+X` | E**x**ecute the focused / selected code block (mirrors the TUI's `g x` chord). Inside a textarea the Insert-mode strikethrough binding wins (mode-specific beats Global) — commit first or use the per-block run button. |
| `Cmd/Ctrl+[` / `]` | Previous / next journal day |
| `Cmd/Ctrl+Shift+E` | Toggle sidebar (mirrors VS Code's explorer chord) |
| `Cmd/Ctrl+Shift+B` | Toggle backlinks panel |
| `Cmd/Ctrl+,` | Open settings |

> **Why `Cmd+J` for the journal and not `Cmd+T`?** `T` is universally "task" in outliners (TUI's `Ctrl+T`, Logseq's `Cmd+T`, every Markdown task list shortcut). We don't make the user re-learn that. `J` for **journal** is unambiguous and lines up with the `g j` chord the TUI uses.
> **Why not `Cmd+B` / `Cmd+\`?** `Cmd+B` is **reserved for bold** in every popular markdown editor (Notion, Obsidian, Discord, Slack) — retraining users on a non-standard meaning is hostile. `Cmd+\` is **1Password's** global autofill chord on macOS; hijacking it breaks every user with 1Password installed.
> **Why did `Cmd+X` stop running code blocks?** It shadowed the OS-universal **cut** inside every textarea (the dispatcher `preventDefault`s matched Global chords even in Insert mode). Clipboard muscle memory beats the e**x**ecute mnemonic in a text-editing app, so run-code moved to `Cmd+Shift+X` and plain `Cmd+X` now falls through to the webview's native cut. See issue #80.

### Undo / redo (Normal mode — fire when no textarea is focused)

| Chord | Action |
|---|---|
| `Cmd/Ctrl+Z` | Undo the last committed block mutation on the current page |
| `Cmd/Ctrl+Shift+Z` | Redo it |
| `u` / `Ctrl+R` | Same actions, vim spelling (TUI parity) |

Deliberately **Normal**, not Global: with a textarea focused the chord falls through to the webview (the in-flight draft is the textarea's own undo domain), and a Global binding would `preventDefault` it away.
History is **block-level**: each mutation that goes through `finish_in_page` (edit, create, indent / outdent, move, delete, TODO / quote toggle, paste)
pushes the page's pre-mutation `.md` render onto a bounded per-page stack (`outl_actions::history::HistoryStacks`);
undo restores the snapshot through `outl_md::reconcile_md`, so the restore is itself ops in the log — the op log stays the source of truth, nothing is rewritten.
Fold toggles (`set_block_collapsed`) bypass `finish_in_page` and are not undoable, matching their "view state, not content" semantics.
Invalidation is **surgical**: a workspace **switch** clears every stack,
but a peer-driven **reload** (`peer-ops-changed` → `reload_workspace`) drops only the stacks of pages whose projection actually changed across the reload — restoring one of those would silently revert the peer's edits.
Pages the peer didn't touch keep their full undo depth.
(The first cut cleared everything on every reload, which capped `Cmd+Z` at one step whenever the TUI was open on the same workspace — every TUI write fires `peer-ops-changed`.)

### Inline markdown (Insert mode — fire when a textarea is focused)

Wrap the current selection (or insert the delimiter pair around the caret) — mirrors the convention every popular markdown editor ships.

| Chord | Action | Output |
|---|---|---|
| `Cmd/Ctrl+B` | Bold | `**text**` |
| `Cmd/Ctrl+I` | Italic | `_text_` |
| `Cmd/Ctrl+E` | Inline code | `` `text` `` |
| `Cmd/Ctrl+Shift+X` | Strikethrough | `~~text~~` |
| `Cmd/Ctrl+K` | Link | `[text](url)` — `url` is pre-selected |

Implementation lives in `lib/markdown-wrap.ts`: each handler reads `document.activeElement`, splices the value, dispatches an `input` event so `<BlockRow />`'s Solid signal stays in sync, then repositions the caret / selection.

### Block-editor chords (inside a block's textarea)

| Chord | Action |
|---|---|
| `Enter` | Insert a `\n` inside the current block (multi-line text) |
| `Cmd/Ctrl+Shift+Enter` | Commit + create a sibling below + edit it |
| `Cmd/Ctrl+T` / `Cmd/Ctrl+Enter` | Toggle TODO / DONE on this block |
| `Cmd/Ctrl+X` | Native cut — falls through to the webview (no catalog binding matches inside a textarea) |
| `Cmd/Ctrl+Z` | Falls through to the webview too, but native per-keystroke undo is still broken: the controlled `value={draft()}` binding resets the textarea's undo stack on every keystroke (tracked as follow-up in issue #80) |
| `Tab` / `Shift-Tab` | Indent / outdent |
| `Esc` / blur | Commit |
| `Backspace` on empty | Delete the block |
| `[[` / `((` | Auto-close pair (`@outl/shared/autocomplete`) |
| `(` / `[` / `{` | Auto-pair with the matching closer, caret between (`autoPairBracket`, TUI parity); typing `)` / `]` / `}` over an identical closer steps past it instead of doubling |
| `Backspace` inside an empty pair | Collapses the whole pair — `[[]]` / `(())` (4 chars) and `()` / `[]` / `{}` (2 chars) — via `autoDeletePair` |

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

- **Visual mode is real**: `Mode::"vim-visual"` in the store with `visualAnchorId` + `lastVisualRange`.
  `<BlockRow />`'s `isInVisualRange()` paints the range at 18% accent opacity (distinct from the 6% single-row selection tint).
  Every Visual exit (`Esc`, `y`, `d`) captures the range to `lastVisualRange` so `g v` can restore it.

- **`*` / `#` is not vim-pure.**
  Without a char cursor, "word under cursor" isn't defined — we seed the picker with the first 4 words of the selected block's text instead.
  Document this in `docs/shortcuts.md` so users aren't surprised.

- **Range ops walk bottom-up + tolerate id-already-gone.**
  `DeleteRange` iterates `[hi → lo]` so children go before parents (the parent's move-to-trash would otherwise pull a still-targeted descendant out from under us).
  NodeIds are stable across the CRDT (`deleteBlock` is `Move(node, TRASH)`, not a re-keying), so the id snapshot taken before the loop stays valid;
  we only have to swallow individual failures (`safeCall` writes them to the status line) when a peer ate the same id concurrently or the range straddled a parent + descendants.

- **`UnfoldAll` / `FoldAll` walk via `flattenAll` / `flattenParents`, never `flattenVisible`.**
  The whole point of `zR` is to expand subtrees currently hidden under a collapsed parent.
  The visible-only walk would silently no-op on every descendant of a folded node, so this is a real bug if `applyCollapsedToAll` reaches for the wrong helper.
  **`zM` (fold-all) uses `flattenParents`**: foldar leaf hoje é invisível,
  mas `outl_actions::set_block_collapsed` **sempre** escreve `Op::SetCollapsed` no log (a CRDT precisa de cada flip pra convergir),
  então adicionar children embaixo de uma "leaf que foi foldada" faz eles aparecerem colapsados — future-surprise real.
  **`zR` (unfold-all) usa `flattenAll`**: descolapsar leaf não tem efeito futuro.
  Mirror exato de `outl-tui`'s `collect_collapse_candidates` pra a contagem de ops bater entre os clients.

- **`A` (`EnterInsertAtEnd`) routes through `appState.caretIntent`.**
  The textarea is mounted by Solid's `<Show>` swap;
  poking it via `queueMicrotask` + `document.querySelector` after flipping `editingBlockId` was racey (the DOM node wasn't guaranteed to exist by the next microtask).
  The handler now sets `caretIntent: "end"` *before* `editingBlockId`;
  `<BlockRow />`'s own `createEffect` reads the intent on mount, applies `setSelectionRange`, and clears the signal.
  Same hook applies for any future caret-intent gestures (`B`/`b`-style "land at start of word", etc).

- **Visual highlight uses a memoised `Set<id>` at the parent, not a per-row predicate.**
  `<OutlineView />` builds `visualSet = createMemo(() => visualRangeSet(...))` once per outline/anchor/cursor/mode change and passes it down as a prop;
  `<BlockRow />` answers `props.visualSet?.has(id) ?? false` in O(1).
  The earlier shape called `isInVisualRange(id, anchor, cursor, outline)` per row, which rebuilt `flattenVisible(blocks)` from scratch each call — N rows × N DFS = O(N²) per Visual extension keystroke (visibly laggy from ~500 blocks on).
  The predicate `isInVisualRange` still exists in `lib/outline-walk.ts` but only for the unit-test suite; **no render path should call it**.
  Outside vim-visual mode, `visualSet` is `null` so every row short-circuits before touching the Set.

- **Char-cursor nudge is one shared handler.**
  All 10 char-cursor catalog entries (`x` `X` `D` `C` `s` `r` `~` `e` `f` `F`) point at `charCursorNudge`.
  One source of truth means the message can't drift between catalog entries.

- **`p` / `P` paste handlers are not wired yet.** `Y` / `YankRange` fill `appState.yankRegister`, but pasting N blocks has a design call (siblings? children? after / before?) that's deliberately deferred.

- **Path to enable char-cursor ops.**
  Add a visible Normal-mode caret painted by `<BlockRow />` (model change), then move the 10 blocked handlers to real implementations.
  Separate PR.

### `Enter` outside a textarea (Normal mode)

With a block selected and no textarea focused (the DOM fallback puts the dispatcher in Normal mode even with `vim_mode == false`),
`Enter` resolves to the shared `OpenRefUnderCursor` action — but the desktop handler **always enters Insert on the selected block**.
The one exception: when the selection sits on a **backlink row** (read-only), `Enter` opens the source page and lands the cursor on the referencing block.

Why the divergence from the TUI: the TUI's Normal mode has a character cursor, so "open the ref under the cursor" is well-defined (`ref_at_cursor`) and falls back to Insert when the cursor isn't on a ref.
The desktop's Normal mode only has a selected block — an earlier handler approximated "under cursor" as "first `[[ref]]` in the block", which made every ref-carrying block impossible to edit via `Enter`.
On the desktop, **following a ref is the click on the token** (`onRefClick` in `OutlineView`); `Enter` means edit.

### `:shortcode:` emoji autocomplete

While the caret sits inside an open `:shortcode` trigger, `BlockRow` shows a floating popup (`EmojiSuggestPopup`, anchored under the textarea — same pattern as `RefSuggestPopup`).
It reuses `detectEmojiContext` / `applyEmojiSuggestion` from `@outl/shared/autocomplete` and the `searchEmojis` command (`outl_emoji_search` Tauri side, backed by `outl_md::emoji::search`).
`↑`/`↓` move the highlight,
`Enter`/`Tab` accept (inserting the canonical `:shortcode:` form into the buffer — the `.md` stores the shortcode literal, never the codepoint),
`Esc` closes the popup (a second `Esc` then commits the block),
and clicking a row picks it (via `onMouseDown` + `preventDefault`).
The emoji popup takes precedence over the ref popup at the same caret;
the two never co-exist because `detectEmojiContext` only triggers on word-initial `:[a-z]`.
No keyboard shortcut lives in `outl-shortcuts` for this — it's pure trigger-detection inside the textarea.

### `[[page]]` ref autocomplete

While the caret sits inside an open `[[…]]`, `BlockRow` shows a floating page-suggestion popup (`RefSuggestPopup`, anchored under the textarea).
It reuses the shared `detectRefContext` / `applySuggestion` helpers (`@outl/shared/autocomplete`) and the `search_pages` command the `Cmd+P` picker already calls — no parallel implementation.
`↑`/`↓` move the highlight,
`Enter`/`Tab` accept (inserting the page title, or the ISO slug for journals),
`Esc` closes the popup (a second `Esc` then commits the block),
and clicking a row picks it (via `onMouseDown` + `preventDefault` so the textarea's blur-commit doesn't fire first).
Block refs (`((…))`) are intentionally not suggested yet — separate feature.

### Clicking external `[label](url)` links

`<MarkdownInline />` renders external markdown links clickable when given an `onLinkClick(href)` prop.
`OutlineView` wires it to `openExternalUrl` (`@outl/shared/api/commands`),
which scheme-guards to `http(s)`/`mailto` and opens in the system browser via **`tauri-plugin-opener`** (registered in `src-tauri/src/lib.rs`;
the capability grants a scoped `opener:allow-open-url` for `http`/`https`/`mailto` in `capabilities/default.json`).
Failures (malformed URL, disallowed scheme) land on the status line via `appState.lastError`.
The `[[ref]]` / `#tag` click handlers are unchanged (they navigate the workspace, not the browser).
The opener call lives in the shared wrapper — not a custom Tauri command — so mobile can opt in later by registering the same plugin and passing `onLinkClick`.
Backlink rows stay inert (the whole row is already a navigate-to-source button; nesting a second click target would conflict).

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
  "sync_transport": "iroh"  // "iroh" (P2P, default) | "file" (iCloud/fs)
}
```

The Sync transport select in `SettingsModal` writes `sync_transport`.
`settings.rs` maps it to/from `[sync] transport` and preserves `relay_url` on save; takes effect on next launch.

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
macOS does not register URL schemes at runtime — only via LaunchServices from the app bundle's `CFBundleURLTypes` (which `tauri-plugin-deep-link` writes from `schemes` at `cargo tauri build` time).
So `cargo tauri dev` does **not** register `outl://`; `open outl://…` then shows the Finder "no application set to open the URL" dialog.
To test: `cargo tauri build`, copy the `.app` into `/Applications` (the scheme registration requires it to live there), open it once so LaunchServices indexes it, then `open "outl://page/<slug>"`.
Linux/Windows register at runtime (`register_all()` in `setup`), so dev mode works there.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-desktop --all-targets -- -D warnings`
3. `cargo test -p outl-desktop`
4. `bun --filter outl-desktop test` (Vitest)
5. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click around the parts you touched.
6. If you touched anything in `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
