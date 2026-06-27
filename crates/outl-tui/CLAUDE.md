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
  The `y r` chord plus `/refer` and `/refer-embed` slash commands copy the current block's handle to the **OS clipboard** (via `arboard`) and stash it in `App::last_yanked_ref` for in-app paste;
  the `((` autocomplete fuzzy-matches block text in Insert.
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

- **`c` is fold, not vim's "change".**
  Bullet row shows `▼ `/`▶ ` for parents (two-space gap on leaves so columns stay flush).
  Hidden subtrees are skipped by `j`/`k`.
  Persisted as `Op::SetCollapsed` in the op log — converges across devices through the CRDT, no per-file last-write-wins.
- **`z M` / `z R` skip-leaves contract.**
  `z M` (fold-all) walks the AST in DFS and only emits `Op::SetCollapsed(true)` for blocks with `!children.is_empty()`.
  Foldar leaf é invisível hoje, mas o op fica no log: adicionar children embaixo depois faria eles aparecerem colapsados (future-surprise).
  `z R` (unfold-all) caminha todos os ids porque descolapsar leaf não tem efeito futuro.
  `outl_actions::set_block_collapsed` **sempre** escreve op no log (mesmo quando o valor já bate) — o `Ok(false)` retornado é só telemetria pra UI, não "log untouched".
  A CRDT precisa de cada flip pra convergir flips concorrentes via HLC.
- **`y r` clipboard fallback.**
  `y r` (chord) copies `((blk-XXXXXX))` to the OS clipboard via `arboard` **and** stashes it in `last_yanked_ref`.
  Status flips to `yanked … (clipboard unavailable)` on headless / no-display environments — the in-app yank register still works.
- **`Enter` is overloaded.**
  Open `[[ref]]` / `#tag` / journal / block ref (`((blk-X))` / `!((blk-X))`) under cursor, else enter Insert.
  On a block ref it jumps to the source page and positions the cursor on the referenced block; orphan handles surface a status message and stay put.
- **Quit is chord-only.**
  `q q` arms + confirms (single `q` is too easy to hit by accident).
  `Z Z` is the vim "save and quit" alias — outl auto-commits on every Normal boundary, so it reduces to `q q`.
  `Ctrl+C` commits any in-flight Insert before quitting.
- **`r` / `f` / `F` use `state.pending_input_op`.**
  A one-shot enum (mutually exclusive with `pending_chord`) consumed by the next `Char(c)` keystroke.
  Any other keystroke cancels.
- **`*` / `#` reuse the workspace search.** `search_word_under_cursor` extracts the word the cursor sits on (whitespace-bounded), opens the `/` overlay machinery, accepts the first hit, persists the rest into `last_search` so `n` / `N` walk them.
- **Visual range capture.**
  Every Visual exit (`Esc`, `y`, `d`) routes through `remember_visual_range` so `g v` can restore the last range.

## Insert mode

Full Insert key list is in [`docs/shortcuts.md`](../../docs/shortcuts.md).
TUI-specific contracts worth remembering:

- **`Esc` commits** through `commit_insert` (writes buffer → AST → disk via `outl_md::reconcile_md`).
  Aborting without commit is `abort_insert` — wired to nothing today; we never lose user keystrokes silently.
- **`Enter` always commits and continues.**
  It's commit + new block below + park in Insert on the new block.
  The Insert-mode commit path also drains `pending_reload` (peer-ops poller held it back during the edit).
- **`Backspace` on an empty block deletes the block** and moves selection to the previous one — the only structural mutation that can happen from Insert.
- **Autocomplete triggers** are pure trigger-detection inside the buffer (`[[`, `#`, `((`, `/`); they own the keystream while their popup is open.
  See `actions/autocomplete.rs` for the trigger detector contract.

## Visual conventions

- Selected block is highlighted with a colored bullet.
- In Insert mode, a `▏` caret marks cursor position inside the block.
- In Normal mode on the selected block, a block cursor (white bg) sits on the character under `cursor_col`.
- Other (non-focused) blocks render markdown prettily: `**bold**` shows as bold without asterisks,
  `*italic*` as italic,
  `~~strike~~` struck through,
  `` `code` `` in green,
  `[text](url)` blue-underlined,
  `[[ref]]` cyan-underlined (no brackets),
  `#tag` magenta-underlined,
  `((blk-XXXXXX))` resolves to the source block's text + page icon (orphan handles render dimmed),
  `:shortcode:` renders as the unicode glyph (`:tada:` → 🎉; unknown shortcodes never tokenize so the literal stays visible).
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
- A block whose text starts with the CommonMark `"> "` prefix renders with a left `│ ` bar (dimmed, `theme.dim`) and full body colour — the `│` is enough of a cue, dimming refs / tags / bold would erase their affordance.
  Same affordance as `TODO`/`DONE`.
  The chrome lives in **`view::inline::render_pretty_block_text_impl`** and is the **only owner** of the bar + checkbox + token rendering pipeline;
  the outline view's `BlockRowKind::Bullet if single_line_pretty` branch delegates to it directly (one owner, every caller wraps).
  The bar composes with the TODO checkbox (`│ ☐ foo`) and `view::inline::split_block_prefixes` accepts the prefixes in **either order**, so `"> TODO foo"` and `"TODO > foo"` render the same.
- IDs are **never** shown.
- Mode tag (`NORMAL`/`INSERT`) appears in the header.
- **Block text word-wraps to the pane width** (issue #99).
  Terminals don't reflow, and `Paragraph::wrap` can't be used because it expands lines *after* layout and would desync the `selected_line` scroll index.
  So `view::wrap::push_wrapped` emits the wrapped `Line`s up front: the first visual row keeps the bullet/fold `head`, continuations re-indent under the text column, and the `│ ` indent rails repeat on every row.
  Wrapping runs on the already-styled `Span`s (post-tokenization), so a break never splits a `**bold**` token back into literal asterisks.
  **Cursor rows (Insert / Normal-selected) wrap too** — `emit_row_with_cursor` bakes the caret / block cursor into the row's `Span`s *before* `push_wrapped` runs.
  Reflowing just carries the cursor onto its wrapped visual row: the char offset was already consumed turning it into a span, so there's nothing left to desync.
  The earlier "cursor rows pass width `0`" workaround was the actual #99 regression: the selected block stayed on one overflowing line and only wrapped once the cursor left it (`viewing mode won't wrap until I navigate away`).
  `text_width == 0` is still the "don't wrap" sentinel, but only headless renders pass it now.

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

1. **Grep first.**
   Before writing a helper here, `rg "fn <name>"` / `rg "struct <Name>"` across `crates/outl-core`, `crates/outl-md`, `crates/outl-actions`.
   The thing you're about to write probably exists upstream — wrap it instead of cloning the logic.
2. If it's data or pure logic → put in `outl-md` (or `outl-core`).
3. If it's a workspace mutation two clients would call the same way → put it in `outl-actions`.
4. If it's how it's drawn on a terminal → put in `outl-tui`.
5. Never write a function in `outl-tui` that a Tauri/mobile client would also need byte-for-byte.
   Extract upstream first.

**Concrete example:** `EditBuffer::move_up` / `move_down` (cursor nav across `\n` inside a multi-line block) are TUI-specific primitives,
but the `(line, col) ↔ char_idx` math underneath isn't — it's shared with how the renderer maps a cursor onto a [`outl_md::view::BlockRow`].
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
│   ├── plugins.rs       # outl_plugins::PluginHost wiring (load, slash dispatch, op-hook sweep)
│   └── overlay.rs       # quick switcher, search, palette, autocomplete
├── input/               # key → action routing (normal/insert/visual/overlay)
│   ├── chord_adapter.rs # crossterm KeyEvent → outl_shortcuts::Chord
│   └── plugin_chord.rs  # plugin keybinding dispatch (Normal-only, never shadows native)
├── view.rs              # render_app orchestrator; thin
├── view/
│   ├── inline.rs        # span-level markdown (highlight + pretty)
│   ├── outline.rs       # outline rendering (render_outline, render_block, …)
│   ├── wrap.rs          # width-aware word wrap of styled spans (push_wrapped)
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
- `outl-plugins` (JS plugin runtime — `PluginHost`, `load_installed`; default `js`/Boa feature on).
- `walkdir`, `toml`, `ulid`, `chrono`, `anyhow`.

## Plugins

JS plugins are loaded at boot from `<root>/.outl/plugins/` into an `outl_plugins::PluginHost` held directly in `App.plugin_host` (`Option`, single-threaded — no `Arc`/`Mutex`, the Boa context is `!Send`).
Boot / slash / op-hook / content-transform wiring lives in `actions/plugins.rs`; keybinding dispatch lives in `input/plugin_chord.rs`.
The five touch points are:

- **Boot** (`App::load_plugins`, called at the end of `App::new`).
  Declares the client capabilities the TUI honors (`slash-command`, `op-hook`, `keybinding`, `content-transformer:text`, `toolbar-button`), runs `load_installed`, then `mark_synced` so pre-existing ops don't fire hooks on startup.
  A `toolbar-button` has no chrome bar in a terminal, so its command is surfaced in the **slash menu** instead (deduped against `slash-command` entries) — a runnable command is never dropped just because its only affordance was a GUI button.
  `ctx.net`, `ctx.storage`, and the gas limits are host-level (the engine), so they work in the TUI with no per-capability wiring — only HTML surfaces (`ui-render`, `content-transformer:rich`) stay undeclared, since a terminal can't draw them.
  Best-effort: a load failure toasts a warning and the TUI runs normally; a workspace with no plugins is unchanged.
  **`content-transformer:rich` is deliberately *not* declared** — `rich` output is HTML for a GUI iframe, meaningless in a terminal; the host filters those out of `host.transformers()` automatically.
- **Slash commands** (`App::slash_candidates` in `actions/overlay.rs`).
  The slash menu concatenates `host.commands()` onto the built-in registry list; each plugin command carries a `SlashOrigin::Plugin { plugin_id, command_id }` tag (vs `SlashOrigin::Builtin`).
  `accept_slash` routes a plugin pick to `App::run_plugin_command`, which surfaces `notify`/error output as toasts and re-projects if it mutated.
- **Keybindings** (`input/plugin_chord.rs::try_plugin_binding`, called first inside `handle_normal_key`).
  A plugin's `contributes.keybindings[].key` is parsed by `outl-plugins` into an `outl_shortcuts::ChordSequence`; `input/chord_adapter.rs` maps the live `crossterm::KeyEvent` into the same `outl_shortcuts::Chord` so we can compare them.
  A matching single-chord binding runs `App::run_plugin_command` immediately.
  A two-chord binding (`Ctrl+G A`) buffers the first chord in `App::pending_plugin_chord` (a **separate** field from the native `pending_chord` vim accumulator so the two never interfere) and fires on the second key.
  **Plugin chords are scoped to Normal mode** — they're `Mode::Global` in the catalog, but the TUI deliberately won't steal keys mid-edit.
  They **never shadow a native action**: `native_normal_chord` mirrors what `handle_normal_key` consumes, so a plugin can't rebind `j`, `dd`, `Ctrl+T`, `Ctrl+P`, etc. (use a free chord like `Ctrl+G` or a two-chord sequence).
  No host / no bindings / a key with no `Chord` form all short-circuit to native handling.
- **Op hooks** (`App::run_plugin_op_hooks`).
  Called once per iteration at the **single post-mutation point** in `runtime.rs`'s event loop (after the mode key handler, before the next draw).
  Deferred while in `Mode::Insert` (same reason as `pending_reload`: a hook-driven `load_current` would clobber the in-flight buffer; the edit isn't in the op log until commit anyway).
- **Content transformers** (`App::recompute_transforms`).
  **Pre-compute, not render-time.**
  When a block's text is a single closed code fence (`` ```<lang> `` … `` ``` ``) whose language a loaded `text` transformer claims, its body runs through `host.transform_block` *at load time*.
  The result is cached in `App::transform_cache`, keyed by `NodeId`.
  The render walk (`view/outline.rs`) only has `&App`, and `transform_block` is `&mut self` (it runs Boa) — so the transform **cannot** happen during render.
  It's done in `recompute_transforms`, called from `load_current_no_autorun` (every reparse), after `load_plugins` at boot, and on the reproject paths (plugin + peer mutations).
  The render path is then a pure `HashMap` lookup: a read-only block with a cache hit renders the transformed text/markdown (`RenderMode::Transformed`) in place of the raw fence; the bullet stays.
  **A block under the cursor (Insert / Normal-selected) always renders the raw fence source** so the user edits what they see — the cursor cases win over a cache hit.
  Lang match: the fence's raw info-string first (custom langs like `mermaid`), then the canonical alias via `outl_md::lang::canonical` (so a transformer registered as `rust` fires on `` ```rs ``).
  Best-effort: a plugin error or `Ok(None)` (declined) leaves the block to render as a raw fence — never crashes.

A plugin mutation lands in the op log via `outl-actions` but does **not** write `.md`, so `reproject_after_plugin` runs `outl_actions::apply_all_pages_md` (a plugin can touch any page) then `load_current`.
If a plugin declares a capability the TUI lacks, the host filters it; `host.missing_capabilities(id)` lists the gap.

## Peer sync coordination

The TUI is a peer in a multi-device workspace.
Two threads' worth of sync logic live here on top of `outl_actions::SyncEngine`:

| Thread / path | Responsibility |
|---------------|----------------|
| `wire_sync_transport` (startup, before the poller) | Optionally builds an `outl_sync_iroh::IrohSyncTransport` from the per-device `~/.outl/identity.key` + the per-workspace `<workspace>/.outl/peers.json` (one-time migration copies any legacy global peer list in) and stores it in `App::sync_transport`. Gated on `[sync] transport = "iroh"` in the global `~/.config/outl/config.toml` (read via `outl_config::load()` at startup and threaded into `App::new`) — which is the **default**, so a fresh config boots on iroh; the explicit `transport = "file"` opt-out leaves it `None`. Any failure degrades to the filesystem poller (best-effort, never aborts startup). |
| `spawn_jsonl_poller` (startup) | Runs **both** change-detection backends, not one or the other. `outl_actions::FileSyncTransport.start(...)` always runs (polls peer `ops-<actor>.jsonl` every ~2 s, own file filtered out, signals on growth); when `App::sync_transport` is set (iroh) `transport.start(...)` runs too (QUIC → local `ops/` → signal). They cover different paths and neither subsumes the other: iroh signals only on its OWN wire receipts, the poller signals on ANY disk write — including ops a **co-resident** process (a desktop / MCP / CLI sharing this workspace's `ops/`) wrote. That co-resident case is load-bearing: the desktop and the TUI share `~/.outl/identity.key` (one node id per device), so the relay routes a mobile peer's inbound to whichever endpoint holds the route (usually the desktop); the desktop writes the received ops to the shared `ops/`, but the TUI's iroh transport never saw them on the wire, so without the always-on poller the TUI stayed blind to them (the "TUI ↔ mobile doesn't sync" bug). Reopen is idempotent, so the occasional overlap of both signals just reconfirms convergence. |
| `save` / `save_page_with` post-commit | After the local edit lands in the op log, calls `transport.announce_local_ops(slug, hlc)` when a transport is set. **Both** commit paths announce: `save` (the hot path — current page) and `save_page_with` (cross-page backlink edits / Insert into a source page), so a backlink edit wakes peers too instead of only converging on the catch-up re-sync. No-op for `FileSyncTransport`; `IrohSyncTransport` gossips the new HLC so peers pull over QUIC. |
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
