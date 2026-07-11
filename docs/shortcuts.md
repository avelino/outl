# Shortcuts

Every keyboard shortcut outl ships with, across every client, in one table per concern.

## Source of truth

The desktop and TUI both pull their chord catalog from **`crates/outl-shortcuts`** (`src/defaults.rs::default_bindings()`).
The mobile app doesn't expose a keyboard surface (touch + on-screen keyboard); the rows below leave its column blank when there's nothing to bind.

> **One catalog, two adapters.**
> The TUI converts `crossterm::KeyEvent → Chord`, the desktop converts browser `KeyboardEvent → Chord`.
> Both then call the same `outl_shortcuts::lookup(mode, chord) → Action`.
> A chord change in `defaults.rs` lights up on both clients on the next build.

If a row below disagrees with what you observe in the app, **the code is right and this doc is stale** — file an issue or fix the row.

## How to read the tables

- **Chrome / Global** chords fire in every mode.
- **Normal / Insert / Visual / Overlay** mirror the vim modes.
  The desktop subscribes to `Normal`/`Visual` only while `editor.vim_mode = true` (see [`docs/config.md`](config.md)); chrome and `Insert` chords are always live.
- **`Cmd`** is the macOS modifier; **`Ctrl`** is the same chord on Linux / Windows / TUI.
  We list one form per row to keep the table readable.
- A chord in the form `q q` is a vim-style two-key sequence: press the first, then the second within ~1 s.

---

## Chrome (Global) — works in any mode

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Quick switcher (fuzzy pages + journals) | `Ctrl+P` | `Cmd/Ctrl+P` | tap toolbar |
| Open today's **j**ournal | `t` / `Home` | `Cmd/Ctrl+J` | toolbar |
| Toggle TODO / DONE on focused or selected block (T for **t**ask) | `Ctrl+T` / `Ctrl+Enter` | `Cmd/Ctrl+T` / `Cmd/Ctrl+Enter` | tap checkbox |
| Run code block under cursor / selected block (X for e**x**ecute) | `g x` chord / `:run` | `Cmd/Ctrl+Shift+X` (inside a textarea the Insert-mode strikethrough wins — commit first or use the Run button; plain `Cmd+X` is the OS cut / block cut) | tap "Run" button |
| Previous journal day | `[` | `Cmd/Ctrl+[` | swipe right |
| Next journal day | `]` | `Cmd/Ctrl+]` | swipe left |
| Toggle sidebar | `Ctrl+E` | `Cmd/Ctrl+Shift+E` | _(single pane)_ |
| Toggle backlinks panel | `Ctrl+B` | `Cmd/Ctrl+Shift+B` | inline below outline |
| Open settings | _via `:settings`_ | `Cmd/Ctrl+,` | gear icon |
| Toggle help overlay | `?` | `Cmd/Ctrl+/` | help button |
| Quit | `q q` (chord) / `Z Z` (vim alias) / `Ctrl+C` | `Cmd/Ctrl+Q` (OS) | — |

> **Why `Cmd+J` and not `Cmd+T` for today's journal?** Every outliner ecosystem uses `T` for *task* / TODO — TUI's `Ctrl+T`, Logseq's `Cmd+T`, the universal Markdown checkbox shortcut. Re-training that muscle memory would be hostile. `J` for **journal** is unambiguous and lines up with the TUI's `g j` chord.

**Defaults the user often asks about.**
Both clients ship with **sidebar and backlinks panel HIDDEN** (`show_sidebar: false`, `show_backlinks: false`).
Editor-hero on first launch — the user opts the panels in with the chord.
This matches Bear / Ulysses on the desktop and `outl-tui`'s historical behaviour.

### Why the shifted variants for sidebar / backlinks

| Chord | Why _not_ |
|---|---|
| `Cmd+B` | Reserved for **bold** in Insert mode — every popular markdown editor (Notion, Obsidian, Discord, Slack, Typora) treats it that way. Hijacking would be hostile. |
| `Cmd+\` | macOS **1Password** global autofill. Stealing it breaks every 1Password user. |

So `Cmd+Shift+E` (VS Code's "Show Explorer") and `Cmd+Shift+B` are the canonical chrome chords on the desktop.
The TUI mirrors the spirit with `Ctrl+E` / `Ctrl+B` (most terminals collapse `Ctrl+Shift+letter` into `Ctrl+letter`, so both forms work identically).

---

## Inline markdown — Insert mode (textarea focused)

Wrap the current selection (or insert the delimiter pair around the caret).
Mirrors the convention every markdown editor on the planet ships.

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Bold (`**…**`) | type `**` | `Cmd/Ctrl+B` | toolbar B |
| Italic (`_…_`) | type `_` | `Cmd/Ctrl+I` | toolbar I |
| Inline code (`` `…` ``) | type `` ` `` | `Cmd/Ctrl+E` | toolbar `<>` |
| Strikethrough (`~~…~~`) | type `~~` | `Cmd/Ctrl+Shift+X` | toolbar S |
| Link (`[label](url)`) | type `[` | `Cmd/Ctrl+K` | toolbar 🔗 |

> outl ships `_…_` as the canonical italic. The parser still accepts `*…*` for compatibility, but `.md` projections emit underscores.

---

## Outline navigation — Normal mode

The desktop honours `Normal`/`Visual` only while `editor.vim_mode = true`.
The TUI is vim-style by definition.

| Action | TUI | Desktop (vim on) | Mobile |
|---|---|---|---|
| Selection down | `j` / `↓` | `j` / `↓` | tap block |
| Selection up | `k` / `↑` | `k` / `↑` | tap block |
| Enter Insert at end of block | `i` | `i` | tap block |
| Enter Insert at start of block | `I` | `I` | tap at start |
| Enter Insert one char past cursor (vim append) | `a` | `a` *(= `i`, no char cursor)* | — |
| Enter Insert at end of block (vim `A`) | `A` | `A` | — |
| Substitute block (clear + Insert at col 0; `S` / `cc`) | `S` | `S` | — |
| Substitute char under cursor (= `xi`) | `s` | — *(char cursor only)* | — |
| Yank current block to register + OS clipboard (`Y`, alias of `y y`) | `Y` | `Y` | — |
| Paste OS clipboard **with** formatting (outline structure / multi-paragraph split) | `p` | `Cmd/Ctrl+V` | paste |
| Paste OS clipboard **without** formatting (raw text, single block) | `P` | `Cmd/Ctrl+Shift+V` | — |
| Open `[[ref]]` / `#tag` / `((blk-…))` under cursor | `Enter` | `Enter` | tap |
| New block below + Insert | `o` | `o` / `Cmd/Ctrl+Shift+Enter` *(no vim needed)* | toolbar `+` |
| New block above + Insert (creates a sibling *before* the selected block) | `O` | `O` | — |
| Indent block | `Tab` | `Tab` | drag right |
| Outdent block | `Shift+Tab` | `Shift+Tab` | drag left |
| Move block up among siblings | `K` | `Cmd/Ctrl+Shift+↑` | drag |
| Move block down among siblings | `J` | `Cmd/Ctrl+Shift+↓` | drag |
| Cut block + subtree (move-by-id; paste keeps `((blk-…))` refs) | — | `Cmd/Ctrl+X` | — |
| Copy block + subtree (paste duplicates with fresh ids) | — | `Cmd/Ctrl+C` | — |
| Paste block after the selection (cut → move, copy → duplicate) | — | `Cmd/Ctrl+V` | — |
| Cancel a pending cut | — | `Esc` | — |
| Delete block (chord) | `d d` | `d d` | swipe left |
| Fold / unfold (toggle collapsed) | `c` | `c` | tap bullet |
| Unfold all on the page (chord) | `z R` | `z R` | — |
| Fold all on the page (chord) | `z M` | `z M` | — |
| Center viewport on cursor (chord) | `z z` | `z z` | — |
| Last block (jump) | `G` | `G` | — |
| First block (chord) | `g g` | `g g` | — |
| Reselect last Visual range (chord) | `g v` | `g v` | — |
| Search workspace for word / block text — forward | `*` | `*` *(seeds picker)* | — |
| Search workspace for word / block text — backward | `#` | `#` *(seeds picker)* | — |
| Undo last committed block mutation | `u` | `u` / `Cmd/Ctrl+Z` | toolbar |
| Redo | `Ctrl+R` | `Ctrl+R` / `Cmd/Ctrl+Shift+Z` | toolbar |
| Yank block ref → clipboard (chord) | `y r` | `y r` | — |
| Enter Visual | `v` | `v` | — |
| Open command palette | `:` | `:` | — |
| Open slash menu | `/` | `/` | `/` |

> **About `a` / `*` / `#` on the desktop.** The desktop's Normal mode has only a selected block id — no character cursor inside the block. So `a` collapses to `i` (the textarea's own caret takes over), and `*` / `#` seed the picker with the first few words of the selected block's text instead of doing a word-under-cursor search. The catalog still ships these chords so muscle memory from the TUI carries over.

> **`Cmd+X` / `Cmd+C` / `Cmd+V` are mode-aware on the desktop.** Inside a block editor (Insert mode, a `<textarea>` is focused) they are the OS-native text cut / copy / paste — the chords aren't in the catalog there, so the keystroke reaches the webview untouched. In **view mode** (Normal, nothing focused) they act on the whole selected block + its subtree: cut marks it to *move by id* (the paste emits a single `Op::Move`, so `((blk-…))` refs and backlinks survive — and the target may live on another page, moving the block across pages), copy snapshots it as markdown (the paste duplicates with fresh ids). This is also why **run code block** moved off `Cmd+X` to `Cmd+Shift+X` (view mode): a text-editing app has to let the OS-wide cut win.

> **`Cmd/Ctrl+Shift+Enter` works without vim mode.**
> Unlike `o`, the chord is not vim-gated: with no textarea focused the desktop falls into Normal dispatch regardless of the `vim_mode` setting, so every user can append a block from view mode.
> Inside a block editor the same chord commits the current edit first (Insert-mode `CommitAndContinue`).
> Both `Cmd+Shift+Enter` (macOS) and `Ctrl+Shift+Enter` (Windows / Linux) are bound in each mode.

### Cursor inside a block (Normal)

These rely on a **character cursor inside the selected block**.
The TUI ships it natively; the desktop has only a selected block id, so the char-cursor ops surface a status-line nudge instead of firing.

| Action | TUI | Desktop (vim on) |
|---|---|---|
| Char left / right | `h` / `l` (or arrows) | `h` / `l` |
| Word right / left | `w` / `b` | `w` / `b` |
| Word end forward (vim `e`) | `e` | — |
| Start / end of block text | `0` / `$` (or Home/End) | `0` / `$` |
| Find char forward / backward (next typed char) | `f{ch}` / `F{ch}` | — |
| Delete char under / before cursor | `x` / `X` | — |
| Delete to end of block (`D`) / change to end (`C`) | `D` / `C` | — |
| Replace char under cursor with next typed char | `r{ch}` | — |
| Toggle case of char under cursor; advance | `~` | — |

---

## Insert mode (text editing)

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Commit + exit Insert | `Esc` | `Esc` / blur | blur |
| Newline inside the block (multi-line text) | `Shift+Enter` | `Shift+Enter` | `Enter` |
| Commit + new block below | `Enter` | `Enter` | `Enter` |
| Commit + new block, caret-aware (caret at col 0 → *before* the block / vim `O`; past col 0 → *below*) | — | `Cmd/Ctrl+Shift+Enter` | — |
| Indent (stay in Insert) | `Tab` | `Tab` | drag |
| Outdent (stay in Insert) | `Shift+Tab` | `Shift+Tab` | drag |
| Delete block on empty | `Backspace` on empty | `Backspace` on empty | — |
| Auto-pair `(` `[` `{` `[[` `((` | yes | yes | yes |
| Ref autocomplete | `[[` triggers picker | `[[` triggers picker | `[[` triggers picker |
| Tag autocomplete | `#` triggers picker | `#` triggers picker | — |
| Block ref autocomplete | `((` triggers picker | `((` triggers picker | — |
| Slash command autocomplete | `/` | `/` | — |
| Toggle TODO/DONE on current | `Ctrl+T` / `Ctrl+Enter` | `Cmd/Ctrl+T` / `Cmd/Ctrl+Enter` | tap checkbox / long-press menu |
| Cut / copy / paste **text** (native) | — | `Cmd/Ctrl+X` / `Cmd/Ctrl+C` / `Cmd/Ctrl+V` | native |
| Run code block | `g x` chord | _(commit with `Esc`, then `Cmd/Ctrl+Shift+X`)_ | tap "Run" |

---

## Visual mode (range)

TUI + desktop; mobile has no Visual equivalent yet.

On the desktop, `Shift+↑` / `Shift+↓` start (and keep growing) a contiguous selection **without** vim mode — the non-vim multi-select entry.
It flips the client into Visual and pops a floating **batch toolbar** (`N selected` + Indent / Outdent / Move up / Move down / Delete / Done) so the range ops are reachable by mouse; the toolbar fires the same actions the chords do.
Deleting a range that contains any block with nested children asks for confirmation first.

| Action | TUI | Desktop |
|---|---|---|
| Start / extend selection down | `j` / `↓` | `Shift+↓` (any mode) · `j` / `↓` (vim) |
| Start / extend selection up | `k` / `↑` | `Shift+↑` (any mode) · `k` / `↑` (vim) |
| Yank range | `y` | `y` |
| Delete range (confirms if any block has children) | `d` / `x` | `d` / `x` · `Delete` / `Backspace` · toolbar **Delete** |
| Indent range (vim `>`) | `Tab` / `>` | `>` · toolbar **Indent** |
| Outdent range (vim `<`) | `Shift+Tab` / `<` | `<` · toolbar **Outdent** |
| Move range up among siblings | `Alt+↑` | `Cmd/Ctrl+Shift+↑` · toolbar **↑** |
| Move range down among siblings | `Alt+↓` | `Cmd/Ctrl+Shift+↓` · toolbar **↓** |
| Leave Visual (captures range so a follow-up `g v` restores it) | `Esc` | `Esc` · toolbar **Done** |

---

## Page operations

`g d` (Normal mode, "go delete") is the canonical chord for page deletion.
It lives in the shared `outl-shortcuts` catalog, same `g<action>` family as `g j` (today) / `g x` (execute) / `g p` (pin).
The chord deletes the focused page (sidebar-highlighted row when the sidebar has focus on the TUI, otherwise the current page).
Each client confirms before invoking `outl_actions::page::delete`.
Journals are refused everywhere.
Clients also expose the action through their native page-list affordance.

| Action | TUI | Desktop | Mobile |
|---|---|---|---|
| Delete the focused page (with confirmation) | `g d` (Normal mode) deletes the current page; sidebar `d` deletes the focused sidebar row | `g d` (Normal mode) + hover `×` button on the sidebar row | long-press the row in the page switcher |

The desktop routes `g d` through the same `DeletePage` handler in `action-handlers.ts` (same `window.confirm` + `deletePage(slug)` flow as the `×` button).
Mobile has no keyboard surface.
Long-press in the page switcher remains the only trigger on touch devices.

---

## Overlays (picker, palette, settings, help)

| Action | TUI | Desktop |
|---|---|---|
| Highlight next | `↓` / `Tab` / `Ctrl+J` | `↓` |
| Highlight previous | `↑` / `Shift+Tab` / `Ctrl+K` | `↑` |
| Confirm | `Enter` | `Enter` |
| Close overlay | `Esc` | `Esc` |

The picker (`Cmd+P` / `Ctrl+P`) fuzzy-matches pages and journals together; type a date in ISO (`2026-06-04`) or natural (`today`, `yesterday`) to jump.

---

## Where each chord lives in the code

| Layer | File | What it owns |
|---|---|---|
| Canonical catalog | `crates/outl-shortcuts/src/defaults.rs` | Every `(mode, chord, action, description)` row. |
| `Action` enum | `crates/outl-shortcuts/src/action.rs` | The named operation each chord resolves to. |
| TUI input adapter | `crates/outl-tui/src/input/*.rs` | `crossterm::KeyEvent → Chord`. |
| Desktop input adapter | `crates/outl-desktop/src/lib/shortcuts.ts` | `KeyboardEvent → Chord`. |
| Desktop dispatcher | `crates/outl-desktop/src/lib/action-handlers.ts` | `Action → Tauri command`. |
| Mobile toolbar / gestures | `crates/outl-mobile/src/components/` | Per-component on-screen handlers. |

A chord change is a single line in `defaults.rs` plus, if the action is new, a row in `action.rs` and a handler in each client.
See [`crates/outl-shortcuts/CLAUDE.md`](../crates/outl-shortcuts/CLAUDE.md) for the full add-a-binding checklist.

---

## Help overlay vs. this doc

In the TUI, press `?` (Normal mode) to see the live chord table baked into the binary — it's generated from the same `default_bindings()` table this doc describes.
In the desktop, `Cmd+/` opens the same overlay.
If you need to look something up while typing, the in-app overlay is faster than this page.

This doc exists so a contributor (or a user shopping for outl) can see every shortcut without launching the app.
