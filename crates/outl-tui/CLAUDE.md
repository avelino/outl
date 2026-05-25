# CLAUDE.md — outl-tui

The TUI. **Journal-first** — when you open `outl-tui`, you land on today's
journal. That's the spec; don't change it.

## Phase 1 scope

- **Read + write** outline (text editing inside blocks, block create /
  indent / outdent / delete).
- Two modes: `Normal` (navigate, block ops) and `Insert` (edit a single
  block's text).
- Quick switcher (`Ctrl+P`) for fuzzy page/journal jumping.
- Outline panel for current page with inline visible cursor.
- Help popup.
- Backlinks / tag panels and command palette are stubs (placeholder
  modules under `ui/`) — phase 3+.

## Modes

| State | What it does |
|-------|--------------|
| `Normal` | navigation + block-structure ops (no text typing) |
| `Insert { block_path, buffer, original_text }` | text edits go to the buffer; commit writes back to AST + disk |

`Esc` from Insert commits. The buffer carries the working text; on commit
we replace the AST node's `.text` and call `save()`, which writes the
`.md` and runs `outl_md::reconcile_md` to update the op log + sidecar.

## Navigation (Normal)

| Key | Action |
|-----|--------|
| `t` / `Home` | journal of today |
| `[` | previous journal |
| `]` | next journal |
| `g j` | jump to today (chord) |
| `Ctrl+P` | quick switcher (pages + journals, fuzzy) |
| `Tab` | indent current block |
| `Shift-Tab` | outdent current |
| `j` / `k` / arrows | move selection |
| `Enter` | open `[[ref]]` / `#tag` / journal under cursor, else Insert |
| `i` / `Enter` | enter Insert at end of current block |
| `I` | enter Insert at start of current block |
| `o` | new block below + Insert |
| `O` | new block above + Insert |
| `dd` | delete current block (chord) |
| `?` | toggle help popup |
| `q q` | quit (chord — single `q` arms; second `q` confirms) |
| `Ctrl-C` | quit (commits pending Insert first) |

## Insert mode keys

| Key | Action |
|-----|--------|
| `Esc` | commit (write buffer → AST → disk) |
| `Enter` | commit + new block below + continue editing |
| `Tab` | indent block (stay in Insert) |
| `Shift-Tab` | outdent block (stay in Insert) |
| `Backspace` on empty | delete block, move to previous |
| `Backspace` otherwise | delete previous char |
| chars / arrows / Home / End | normal text editing |
| `(`, `[`, `{` | auto-pair with closing |

## Visual conventions

- Selected block is highlighted with a colored bullet.
- In Insert mode, a `▏` caret marks cursor position inside the block.
- In Normal mode on the selected block, a block cursor (white bg)
  sits on the character under `cursor_col`.
- Other (non-focused) blocks render markdown prettily: `**bold**`
  shows as bold without asterisks, `*italic*` as italic, `~~strike~~`
  struck through, `` `code` `` in green, `[text](url)` blue-underlined,
  `[[ref]]` cyan-underlined (no brackets), `#tag` magenta-underlined.
- The selected/editing block renders **raw** (delimiters visible, dimmed)
  so cursor columns map 1:1 to source bytes.
- IDs are **never** shown.
- Mode tag (`NORMAL`/`INSERT`) appears in the header.

## Reuse across UI surfaces (Tauri, mobile)

The TUI is **the first UI surface**, not the only one. Every piece of
logic that's not strictly about ratatui rendering lives in `outl-md`
(or `outl-core`) so Tauri and the mobile apps can consume it later:

| Layer | Owns |
|-------|------|
| `outl-core` | Op log, CRDT, storage, workspace |
| `outl-md` | Parse/render, sidecar, matching, reconcile, **inline tokens (`InlineTok`, `RefTarget`)**, **slugify** |
| `outl-tui` | Terminal-specific: ratatui mapping, key handling, raw-mode lifecycle |
| `outl-desktop` (phase 5) | Tauri shell: maps `InlineTok` → React/HTML |
| `outl-mobile` (phase 6) | uniffi bridge: maps `InlineTok` → SwiftUI `AttributedString` / Compose `AnnotatedString` |

Pattern when adding a new feature:

1. If it's data or pure logic → put in `outl-md` (or `outl-core`).
2. If it's how it's drawn on a terminal → put in `outl-tui`.
3. Never write a function in `outl-tui` that a Tauri/mobile client
   would also need byte-for-byte. Extract upstream first.

## Persistence model

Editing is **AST-first**: edits mutate an in-memory `ParsedPage`. On
commit boundaries (Esc, Enter, dd, Tab/Shift-Tab, structural ops), the
TUI:

1. Renders the AST back to `.md` via `outl_md::render`.
2. Writes the `.md` file.
3. Calls `outl_md::reconcile_md` which runs matching → diff → applies
   ops to the workspace → updates the sidecar.

This means concurrent `outl serve` is OK — both go through the same
reconcile path; the sidecar `last_synced_hash` short-circuits no-ops.

## Layout

```
src/
├── main.rs              # binary entry (clap + outl_tui::run)
├── lib.rs               # exposes `run` so outl-cli can reuse the TUI
├── app.rs               # state, mode, key handling, render, AST helpers
├── edit_buffer.rs       # cursor + chars; isolated, well-tested
├── input.rs             # keymap documentation
├── editor.rs            # placeholder for phase 4 (block-level editor widgets)
└── ui/
    ├── pages.rs         # phase 3+ placeholder
    ├── journal.rs       # phase 3+ placeholder
    ├── outline.rs       # phase 3+ placeholder (logic lives in app::render_outline)
    ├── tags.rs          # phase 3+ placeholder
    ├── backlinks.rs     # phase 3+ placeholder
    └── command.rs       # phase 3+ placeholder
```

## Dependencies

- `ratatui` + `crossterm` (UI).
- `outl-core`, `outl-md` (workspace, parse/render/reconcile).
- `walkdir`, `toml`, `ulid`, `chrono`, `anyhow`.

## What this crate does NOT do

- ❌ Mutate the op log directly — every change goes through
  `outl_md::reconcile_md`, which routes ops through `Workspace`.
- ❌ Parse markdown by hand — use `outl-md`.
- ❌ Network — phase 2.
- ❌ Render outside the AST — the AST is the source of truth between
  the TUI and disk.

## Things to be careful about

- **Cursor accounting**: `EditBuffer.cursor` is a char index, not a
  byte offset. When converting to ratatui spans for rendering, use
  `byte_index_for_char` to slice the string correctly. Skipping this
  step crashes on multi-byte UTF-8.
- **Empty page**: `save()` always re-adds a single empty bullet when
  the page would otherwise be empty, so the cursor never has nowhere
  to go.
- **Chord state**: `pending_chord` is cleared on every key press. Don't
  let it persist past one event or `gj` becomes "g + (anything)".

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-tui --all-targets -- -D warnings`
3. `cargo test -p outl-tui` (lib + bin + e2e tests)
4. Manual smoke in a real terminal: `outl init /tmp/x && outl --path /tmp/x`
