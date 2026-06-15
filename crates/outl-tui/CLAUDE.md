# CLAUDE.md — outl-tui

The TUI.
**Journal-first** — when you open `outl-tui`, you land on today's journal.
That's the spec; don't change it.

## Phase 1 scope

- **Read + write** outline (text editing inside blocks, block create / indent / outdent / delete).
- Two modes: `Normal` (navigate, block ops) and `Insert` (edit a single block's text).
- Quick switcher (`Ctrl+P`) for fuzzy page/journal jumping.
- Outline panel for current page with inline visible cursor.
- Inline backlinks rendered below the outline (`B` toggles, `j/k` crosses the separator).
- Block references and embeds: `((blk-XXXXXX))` resolves to the source block's text + page icon.
  `!((blk-XXXXXX))` (when the block contains a single embed token) expands the source block **and its children** read-only below the carrying block.
  `Enter` on either form opens the source page and lands the cursor on the referenced block.
  The `y r` chord plus `/refer` and `/refer-embed` slash commands copy the current block's handle to the **OS clipboard** (via `arboard`) and stash it in `App::last_yanked_ref` for in-app paste; the `((` autocomplete fuzzy-matches block text in Insert.
- Help popup.

## Modes

| State | What it does |
|-------|--------------|
| `Normal` | navigation + block-structure ops (no text typing) |
| `Insert { block_path, buffer, original_text }` | text edits go to the buffer; commit writes back to AST + disk |

`Esc` from Insert commits.
The buffer carries the working text; on commit we replace the AST node's `.text` and call `save()`, which writes the `.md` and runs `outl_md::reconcile_md` to update the op log + sidecar.

## Navigation (Normal)

The full chord catalog (TUI + desktop side-by-side) lives in [`docs/shortcuts.md`](../../docs/shortcuts.md).
This section captures only the **architectural / TUI-specific behaviour** a contributor needs before touching `input/normal.rs`:

- **`c` is fold, not vim's "change".** Bullet row shows `▼ `/`▶ ` for parents (two-space gap on leaves so columns stay flush). Hidden subtrees are skipped by `j`/`k`. Persisted as `Op::SetCollapsed` in the op log — converges across devices through the CRDT, no per-file last-write-wins.
- **`y r` clipboard fallback.** `y r` (chord) copies `((blk-XXXXXX))` to the OS clipboard via `arboard` **and** stashes it in `last_yanked_ref`. Status flips to `yanked … (clipboard unavailable)` on headless / no-display environments — the in-app yank register still works.
- **`Enter` is overloaded.** Open `[[ref]]` / `#tag` / journal / block ref (`((blk-X))` / `!((blk-X))`) under cursor, else enter Insert. On a block ref it jumps to the source page and positions the cursor on the referenced block; orphan handles surface a status message and stay put.
- **Quit is chord-only.** `q q` arms + confirms (single `q` is too easy to hit by accident). `Z Z` is the vim "save and quit" alias — outl auto-commits on every Normal boundary, so it reduces to `q q`. `Ctrl+C` commits any in-flight Insert before quitting.
- **`r` / `f` / `F` use `state.pending_input_op`.** A one-shot enum (mutually exclusive with `pending_chord`) consumed by the next `Char(c)` keystroke. Any other keystroke cancels.
- **`*` / `#` reuse the workspace search.** `search_word_under_cursor` extracts the word the cursor sits on (whitespace-bounded), opens the `/` overlay machinery, accepts the first hit, persists the rest into `last_search` so `n` / `N` walk them.
- **Visual range capture.** Every Visual exit (`Esc`, `y`, `d`) routes through `remember_visual_range` so `g v` can restore the last range.

## Insert mode

Full Insert key list is in [`docs/shortcuts.md`](../../docs/shortcuts.md).
TUI-specific contracts worth remembering:

- **`Esc` commits** through `commit_insert` (writes buffer → AST → disk via `outl_md::reconcile_md`). Aborting without commit is `abort_insert` — wired to nothing today; we never lose user keystrokes silently.
- **`Enter` always commits and continues.** It's commit + new block below + park in Insert on the new block. The Insert-mode commit path also drains `pending_reload` (peer-ops poller held it back during the edit).
- **`Backspace` on an empty block deletes the block** and moves selection to the previous one — the only structural mutation that can happen from Insert.
- **Autocomplete triggers** are pure trigger-detection inside the buffer (`[[`, `#`, `((`, `/`); they own the keystream while their popup is open. See `actions/autocomplete.rs` for the trigger detector contract.

## Visual conventions

- Selected block is highlighted with a colored bullet.
- In Insert mode, a `▏` caret marks cursor position inside the block.
- In Normal mode on the selected block, a block cursor (white bg) sits on the character under `cursor_col`.
- Other (non-focused) blocks render markdown prettily: `**bold**` shows as bold without asterisks, `*italic*` as italic, `~~strike~~` struck through, `` `code` `` in green, `[text](url)` blue-underlined, `[[ref]]` cyan-underlined (no brackets), `#tag` magenta-underlined, `((blk-XXXXXX))` resolves to the source block's text + page icon (orphan handles render dimmed), `:shortcode:` renders as the unicode glyph (`:tada:` → 🎉; unknown shortcodes never tokenize so the literal stays visible).
- The selected/editing block keeps `:` (dim) + shortcode (raw) + `:` (dim) so cursor columns match source bytes 1:1 — the pretty render is the only place the glyph appears.
- `!((blk-XXXXXX))` — when a block contains a single embed token (whitespace OK around it) — expands the source block **and its children** read-only below the carrying block.
  Conventions:
  - Every embed row carries a `↳ ` prefix (root + descendants) so the expansion reads as one cohesive block.
  - Descendants get `2 * (depth + 1)` spaces of padding before `↳ ` so children align under the source root's *text*, not under its `↳ `.
  - Outer indent (`│ ` guides) matches the carrying block's outline depth.
  - TODO/DONE checkboxes, page refs and tags render with their normal styling inside the expansion (via `render_pretty_block_text`).
  - Recursion is capped at depth 4 to break embed cycles.
  - Expansion runs in every render mode — but the carrying block's first row keeps the raw `!((…))` literal under the cursor so column-byte alignment holds.
- The selected/editing block renders **raw** (delimiters visible, dimmed) so cursor columns map 1:1 to source bytes — including the literal `((blk-XXXXXX))` and `!((blk-XXXXXX))` forms.
- A block whose text starts with the CommonMark `"> "` prefix renders with a left `│ ` bar (dimmed, `theme.dim`) and full body colour — the `│` is enough of a cue, dimming refs / tags / bold would erase their affordance. Same affordance as `TODO`/`DONE`. The chrome lives in **`view::inline::render_pretty_block_text_impl`** and is the **only owner** of the bar + checkbox + token rendering pipeline; the outline view's `BlockRowKind::Bullet if single_line_pretty` branch delegates to it directly (one owner, every caller wraps). The bar composes with the TODO checkbox (`│ ☐ foo`) and `view::inline::split_block_prefixes` accepts the prefixes in **either order**, so `"> TODO foo"` and `"TODO > foo"` render the same.
- IDs are **never** shown.
- Mode tag (`NORMAL`/`INSERT`) appears in the header.

## Reuse across UI surfaces (Tauri, mobile)

The TUI is **the first UI surface**, not the only one.
Every piece of logic that's not strictly about ratatui rendering lives in `outl-md` (or `outl-core`) so Tauri and the mobile apps can consume it later:

| Layer | Owns |
|-------|------|
| `outl-core` | Op log, CRDT, storage, workspace |
| `outl-md` | Parse/render, sidecar, matching, reconcile, **inline tokens (`InlineTok`, `RefTarget`)**, **slugify** |
| `outl-actions` | UI-agnostic workspace operations (edit, indent, move, toggle TODO, page model, backlinks). **TUI now imports from here** — `cycle_todo`, `split_todo`, and `TodoState` live in `outl-actions`. |
| `outl-tui` | Terminal-specific: ratatui mapping, key handling, raw-mode lifecycle |
| `outl-mobile` (shipping today) | Tauri 2 + Solid: consumes `InlineTok` / `RefTarget` from `outl-md` and renders to JSX. Shares every workspace operation with the TUI via `outl-actions`. |
| `outl-desktop` (shipping today) | Tauri 2 + Solid for macOS / Linux / Windows. Shares the entire `outl-actions` surface plus the `@outl/shared` TS lib (DTOs, `MarkdownInline`, paste / autocomplete helpers, command wrappers) with mobile. Adds OS-standard shortcuts, `outl-exec` code-block execution, and a cross-platform FS watcher (`notify`) that emits `peer-ops-changed` so the frontend reloads automatically when a peer's `ops-*.jsonl` arrives. |

Pattern when adding a new feature:

1. **Grep first.** Before writing a helper here, `rg "fn <name>"` / `rg "struct <Name>"` across `crates/outl-core`, `crates/outl-md`, `crates/outl-actions`.
   The thing you're about to write probably exists upstream — wrap it instead of cloning the logic.
2. If it's data or pure logic → put in `outl-md` (or `outl-core`).
3. If it's a workspace mutation two clients would call the same way → put it in `outl-actions`.
4. If it's how it's drawn on a terminal → put in `outl-tui`.
5. Never write a function in `outl-tui` that a Tauri/mobile client would also need byte-for-byte.
   Extract upstream first.

**Concrete example:** `EditBuffer::move_up` / `move_down` (cursor nav across `\n` inside a multi-line block) are TUI-specific primitives, but the `(line, col) ↔ char_idx` math underneath isn't — it's shared with how the renderer maps a cursor onto a [`outl_md::view::BlockRow`].
Both directions live in `outl_md::view::{char_to_line_col, line_col_to_char}` and the `EditBuffer` methods are thin wrappers.
**Don't** add a `line_start_and_column` helper here; extract the inverse to `outl-md` if it's missing and wrap from here.

## Persistence model

Editing is **AST-first**: edits mutate an in-memory `ParsedPage`.
On commit boundaries (Esc, Enter, dd, Tab/Shift-Tab, structural ops), the TUI:

1. Renders the AST back to `.md` via `outl_md::render`.
2. Writes the `.md` file.
3. Calls `outl_md::reconcile_md` which runs matching → diff → applies ops to the workspace → updates the sidecar.

This means concurrent `outl serve` is OK — both go through the same reconcile path; the sidecar `last_synced_hash` short-circuits no-ops.

## Layout

```
src/
├── main.rs              # binary entry (clap + outl_tui::run)
├── lib.rs               # exposes `run` so outl-cli can reuse the TUI
├── app.rs               # thin re-export shim + cross-module tests
├── state.rs             # plain data: App, Mode, Focus, Overlay, snapshots
├── actions/             # impl App { ... } blocks, one per concern
│   ├── lifecycle.rs     # load / save / external-edit polling / new
│   ├── nav.rs           # page/journal jumps, cursor, ref open, Focus-aware move
│   ├── block.rs         # Insert mode, create/indent/outdent/delete blocks
│   ├── history.rs       # undo / redo snapshots
│   ├── visual.rs        # Visual mode + range ops
│   ├── yank.rs          # yank register, in-app paste of yanked blocks
│   ├── paste.rs         # external-clipboard paste (bracketed paste → outl_actions::paste_markdown)
│   ├── exec.rs          # run code block via outl_exec
│   └── overlay.rs       # quick switcher, search, palette, autocomplete
├── input.rs             # key → action routing
├── view.rs              # render_app orchestrator; thin
├── view/
│   ├── inline.rs        # span-level markdown (highlight + pretty)
│   ├── outline.rs       # outline rendering (render_outline, render_block, …)
│   ├── overlays.rs      # every modal popup
│   ├── warnings_banner.rs # yellow banner above the outline when the current page has ParseWarnings
│   └── backlinks.rs     # inline backlinks section (below outline, ─ rule)
├── outline_ops.rs       # one-line re-export shim — helpers moved to outl_md::outline_ops so the mobile client can share them
├── edit_buffer.rs       # cursor + chars; isolated, well-tested
├── editor.rs            # placeholder for phase 4 (block-level editor widgets)
└── ui/                  # legacy placeholders; logic lives in view/
```

## Dependencies

- `ratatui` + `crossterm` (UI).
- `outl-core`, `outl-md` (workspace, parse/render/reconcile).
- `arboard` (OS clipboard for `y r` / `/refer` / `/refer-embed`; degrades to status-line-only on headless).
- `walkdir`, `toml`, `ulid`, `chrono`, `anyhow`.

## Peer sync coordination

The TUI is a peer in a multi-device workspace.
Two threads' worth of sync logic live here on top of `outl_actions::SyncEngine`:

| Thread / path | Responsibility |
|---------------|----------------|
| `spawn_jsonl_poller` (worker thread, ~2s tick) | Calls `engine.snapshot_peers()` (own `ops-<actor>.jsonl` filtered out) and signals the main loop when a peer file grew. |
| `poll_jsonl_updates` (main loop, per tick) | Drains the signal. **In Insert mode**, sets `pending_reload = true` and returns — the in-flight `ParsedPage` would be clobbered by a reload mid-edit. Outside Insert, calls `engine.reload_workspace()` and `engine.reproject_page()`. |
| `commit_insert` | After the user's edit lands in the op log, drains `pending_reload` and runs the deferred reload. The CRDT merges peer ops with the freshly-committed local edit. |
| `spawn_orphan_md_scanner` (worker thread, 10s tick) | Calls `engine.scan_for_orphans()` to find `.md` files whose sidecar is missing or stale (Roam import, peer-shipped projection without sidecar, vim edits). Signals the main loop, which runs `outl_md::reconcile::reconcile_md` on each path (also deferred during Insert mode). |

The two filters that make this safe:

- `snapshot_peers` (not `snapshot`) — never react to your own jsonl growing, or every save closes a reload-race loop.
- `pending_reload` flag — never swap the workspace while an Insert buffer has unsaved keystrokes; the unsaved buffer is not in any op log yet and the CRDT can't help with state it doesn't know about.

Mobile shares the same `SyncEngine` but does not need the `pending_reload` flag: every mutation is one atomic Tauri command, so there's no multi-keystroke window.
Different policy, same engine.

## What this crate does NOT do

- ❌ Mutate the op log directly — every change goes through `outl_md::reconcile_md`, which routes ops through `Workspace`.
- ❌ Parse markdown by hand — use `outl-md`.
- ❌ Re-implement workspace operations that another client needs — put them in `outl-actions` first.
- ❌ Render outside the AST — the AST is the source of truth between the TUI and disk.

## Things to be careful about

- **Cursor accounting**: `EditBuffer.cursor` is a char index, not a byte offset.
  When converting to ratatui spans for rendering, use `byte_index_for_char` to slice the string correctly.
  Skipping this step crashes on multi-byte UTF-8.
- **Empty page**: `save()` always re-adds a single empty bullet when the page would otherwise be empty, so the cursor never has nowhere to go.
- **Chord state**: `pending_chord` is cleared on every key press.
  Don't let it persist past one event or `gj` becomes "g + (anything)".

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-tui --all-targets -- -D warnings`
3. `cargo test -p outl-tui` (lib + bin + e2e tests)
4. **`RUSTDOCFLAGS="-D warnings" cargo doc -p outl-tui --no-deps`** — CI runs this and it catches things `clippy` won't.
   Most common bite: ``[`SomeType`]`` in a `//!` module doc where the type is `pub(crate)` triggers `rustdoc::private_intra_doc_links`.
   Drop the brackets, keep the backticks: `` `SomeType` ``.
5. Manual smoke in a real terminal: `outl init /tmp/x && outl --path /tmp/x`
