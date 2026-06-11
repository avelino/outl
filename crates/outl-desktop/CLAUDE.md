# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

## Status

**Phase 6 — feature-complete v0.** Outline edit, journal nav, picker (Cmd+P), backlinks panel, `outl-exec` code blocks, cross-platform FS watcher + auto-reload, settings modal, and the `desktop.yml` CI workflow are all in. Signed bundles, Homebrew cask, and graph view ride incrementally on top.

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

**This crate adds no business logic.** If a Tauri command does something that involves the workspace shape (edit, move, todo, journal render), it delegates to `outl-actions`.
If you find yourself writing a tree walk or an op-generating helper inside `src-tauri/src/lib.rs`, stop — move it to `outl-actions` instead.
The TUI and mobile clients need it too.

Same rule on the frontend: before writing a helper under `src/lib/`, check `@outl/shared` (see [`crates/outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md)). The renderer for inline tokens, paste detection, ref autocomplete, DTO types, and shared Tauri command wrappers all live there.

What this crate **does** own:

- Path discovery (file picker via `tauri-plugin-dialog`; persisted in settings JSON; cross-platform default).
- Cross-platform FS watcher (`notify` crate) that signals the frontend when peer `ops-*.jsonl` files grow — replaces the `NSMetadataQuery`/`NSFileCoordinator` dance the mobile crate has to do for iOS.
- Desktop-only Tauri command surface (workspace picker, settings IO). The code-execution command (`run_code_block`) is a **thin adapter** — the orchestration (flat-DFS walk, `.md` path resolution, `outl-exec` invocation, DTO build) lives in `outl_actions::exec` so the mobile client shares the exact same flow. The desktop adapter only parses NodeIds, locks the workspace, calls the action, and wraps the outcome with a refreshed `PageView`. Adding behaviour to `commands/exec.rs` is almost always a smell — promote it to `outl-actions` instead.
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
│   ├── App.tsx                # WorkspacePicker / AppShell switch
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
            └── exec.rs        # run_code_block — thin Tauri adapter over outl_actions::exec::run_code_block (shared with mobile)
```

## Blockquote chrome

A block whose `text` starts with the CommonMark `"> "` marker renders with a left border (`border-l-2 border-(--color-outl-fg-dimmer)/50`), a very faint tint (`bg-(--color-outl-fg-dimmer)/[0.06]`), a right-rounded corner (`rounded-r-md`), and **full body colour** — refs, tags, bold, code keep their normal palette so the styled-token affordance isn't lost.
The tint is intentionally ~6% alpha: enough to read as a soft box at a glance, low enough to not fight with surrounding outline rows.

The chrome wrapper sits one level above the bullet button — it envelops **both bullet *and* body** as one flex container.
That order (`│ ☐ body`) matches the TUI exactly and reads as "this is a quoted task" instead of "a task whose body happens to be a quote".
The fold chevron and indent guides stay *outside* the chrome so the gutter chrome doesn't end up boxed twice.
When the block isn't quoted, the wrapper degrades to a plain `flex min-w-0 flex-1 items-start` container, so non-quoted rows render byte-identical to before.

TUI has no per-line background available in ratatui, so it stays with just the `│ ` bar — the tint is a desktop/mobile addition that costs nothing to omit on terminal.
The detection uses `splitQuote` from `@outl/shared/markdown` (mirror of `outl_actions::quote::split_quote`); `stripQuoteFromTokens` drops the `> ` from the first `Plain` token before handing the list to `<MarkdownInline />` so the marker doesn't render twice.

Composition: the marker stacks with TODO/DONE the same way the TUI does (`> TODO foo` → quote chrome + checkbox).
Toggling the marker goes through the `toggleQuote(pageId, id)` wrapper in `@outl/shared/api/commands`, which calls the `toggle_quote` Tauri command (`src-tauri/src/commands/block.rs`), which delegates to `outl_actions::block::toggle_quote` — the same Rust function the mobile and TUI surfaces hit.
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
| Rust commands | `cargo test -p outl-desktop` | command shims, settings IO, fs_watcher (Phases 1+) |
| Frontend logic | `bun --filter outl-desktop test` | scaffold smoke (today), components + helpers (Phases 1+) |

Frontend suites today: `src/setup.test.ts` (scaffold smoke — `@outl/shared` alias resolves), `src/lib/chord-format.test.ts`, `src/lib/markdown-wrap.test.ts`, `src/lib/outline-walk.test.ts`, and `src/lib/action-handlers.test.ts` (regression tests for the `OpenRefUnderCursor` handler — Normal-mode `Enter` enters Insert on the selected block even when it carries a `[[ref]]`, and only a backlink-row selection opens the source page; pins #70).

## Shortcuts

The full catalog lives in **`crates/outl-shortcuts`** (single source of truth, also consumed by the TUI). The desktop fetches it via the `list_shortcut_bindings` Tauri command on boot and wires every `Action` through `lib/action-handlers.ts`.

Two of these chords also have **visible icon affordances** in a fixed bottom-left cluster (`components/ChromeToggleBar.tsx`, mounted by `AppShell`, VS Code activity-bar convention): the **sidebar toggle** (`◫`, mirrors `Cmd/Ctrl+Shift+E`) and the **shortcuts-help toggle** (`?`, mirrors `?` / `Cmd/Ctrl+/`). They carry no business logic — clicking flips the same `appState.sidebarOpen` / `appState.helpOpen` store signal the dispatcher flips, so button and keyboard stay in sync. The cluster floats over the main pane on an elevated, bordered surface (clear contrast against page content; active toggle inverts to the accent color), so the sidebar button stays reachable even after the left pane is hidden.

### OS-standard chrome (Global mode — fire in any context)

| Chord | Action |
|---|---|
| `Cmd/Ctrl+P` | Quick switcher (pages + journals, fuzzy) |
| `Cmd/Ctrl+J` | Open today's **j**ournal |
| `Cmd/Ctrl+T` | Toggle TODO / DONE on the focused / selected block (T for **t**ask) |
| `Cmd/Ctrl+Enter` | Toggle TODO / DONE on the focused / selected block (alt) |
| `Cmd/Ctrl+Shift+Enter` | Commit + create a sibling block below |
| `Cmd/Ctrl+X` | E**x**ecute the focused / selected code block (mirrors the TUI's `g x` chord) |
| `Cmd/Ctrl+[` / `]` | Previous / next journal day |
| `Cmd/Ctrl+Shift+E` | Toggle sidebar (mirrors VS Code's explorer chord) |
| `Cmd/Ctrl+Shift+B` | Toggle backlinks panel |
| `Cmd/Ctrl+,` | Open settings |

> **Why `Cmd+J` for the journal and not `Cmd+T`?** `T` is universally "task" in outliners (TUI's `Ctrl+T`, Logseq's `Cmd+T`, every Markdown task list shortcut). We don't make the user re-learn that. `J` for **journal** is unambiguous and lines up with the `g j` chord the TUI uses.
> **Why not `Cmd+B` / `Cmd+\`?** `Cmd+B` is **reserved for bold** in every popular markdown editor (Notion, Obsidian, Discord, Slack) — retraining users on a non-standard meaning is hostile. `Cmd+\` is **1Password's** global autofill chord on macOS; hijacking it breaks every user with 1Password installed.

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
| `Cmd/Ctrl+X` | Run the code block (mirrors TUI's `g x`) |
| `Tab` / `Shift-Tab` | Indent / outdent |
| `Esc` / blur | Commit |
| `Backspace` on empty | Delete the block |
| `[[` / `((` | Auto-close pair (`@outl/shared/autocomplete`) |
| `(` / `[` / `{` | Auto-pair with the matching closer, caret between (`autoPairBracket`, TUI parity); typing `)` / `]` / `}` over an identical closer steps past it instead of doubling |
| `Backspace` inside an empty pair | Collapses the whole pair — `[[]]` / `(())` (4 chars) and `()` / `[]` / `{}` (2 chars) — via `autoDeletePair` |

### `Enter` outside a textarea (Normal mode)

With a block selected and no textarea focused (the DOM fallback puts the dispatcher in Normal mode even with `vim_mode == false`), `Enter` resolves to the shared `OpenRefUnderCursor` action — but the desktop handler **always enters Insert on the selected block**.
The one exception: when the selection sits on a **backlink row** (read-only), `Enter` opens the source page and lands the cursor on the referencing block.

Why the divergence from the TUI: the TUI's Normal mode has a character cursor, so "open the ref under the cursor" is well-defined (`ref_at_cursor`) and falls back to Insert when the cursor isn't on a ref.
The desktop's Normal mode only has a selected block — an earlier handler approximated "under cursor" as "first `[[ref]]` in the block", which made every ref-carrying block impossible to edit via `Enter`.
On the desktop, **following a ref is the click on the token** (`onRefClick` in `OutlineView`); `Enter` means edit.

### `[[page]]` ref autocomplete

While the caret sits inside an open `[[…]]`, `BlockRow` shows a floating page-suggestion popup (`RefSuggestPopup`, anchored under the textarea). It reuses the shared `detectRefContext` / `applySuggestion` helpers (`@outl/shared/autocomplete`) and the `search_pages` command the `Cmd+P` picker already calls — no parallel implementation. `↑`/`↓` move the highlight, `Enter`/`Tab` accept (inserting the page title, or the ISO slug for journals), `Esc` closes the popup (a second `Esc` then commits the block), and clicking a row picks it (via `onMouseDown` + `preventDefault` so the textarea's blur-commit doesn't fire first). Block refs (`((…))`) are intentionally not suggested yet — separate feature.

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
  "font_size": 15
}
```

The actor id (one per device) lives next to it as `actor` — a plain
ULID. Switching workspaces does not rotate it.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-desktop --all-targets -- -D warnings`
3. `cargo test -p outl-desktop`
4. `bun --filter outl-desktop test` (Vitest)
5. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click around the parts you touched.
6. If you touched anything in `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
