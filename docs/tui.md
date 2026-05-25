# TUI Manual

The `outl` terminal UI is the primary way to interact with an outl
workspace. It's journal-first, modal (Normal / Insert / Visual), and
designed to feel familiar if you've used vim or any keyboard-driven
outliner (Roam, Logseq, Obsidian).

## Running

```bash
outl --path ~/notes          # opens the TUI on ~/notes
outl --path ~/notes --theme dracula
outl tui ~/notes             # explicit subcommand form
cd ~/notes && outl           # no args: opens TUI in cwd
```

The TUI requires a real interactive terminal. If stdout isn't a TTY
(e.g. CI), it exits with a clear error instead of hanging.

## Modes

### Normal

The default. Move between blocks, open references, run commands. No
characters insert themselves — every key is a command.

| Key | Action |
|-----|--------|
| `i` | Edit current block (Insert mode) |
| `I` | Edit, cursor at start of block |
| `o` / `O` | New block below / above |
| `Enter` | Open `[[ref]]` / `#tag` / journal under cursor (otherwise edit) |
| `j` / `k` / `↑` / `↓` | Move between blocks |
| `h` / `l` / `←` / `→` | Move cursor inside the current block |
| `w` / `b` | Cursor to next / previous word |
| `0` / `$` | Cursor to start / end of block |
| `Tab` / `Shift-Tab` | Indent / outdent the current block |
| `K` / `J` (or `Alt+↑/↓`) | Move block up / down |
| `dd` | Delete the current block (chord) |
| `Ctrl+Enter` | Cycle the block's TODO / DONE / none prefix |
| `u` / `Ctrl+R` | Undo / redo |
| `V` | Enter Visual mode (multi-block select) |
| `t` / `Home` | Today's journal |
| `[` / `]` | Previous / next journal |
| `g j` | Jump to today (chord) |
| `Ctrl+P` | Quick switcher (fuzzy page/journal pick) |
| `/` | Workspace-wide search |
| `:` | Command palette |
| `B` | Toggle the backlinks panel |
| `?` | Toggle this help popup |
| `q q` / `Ctrl+C` | Quit — `q` alone arms a chord, second `q` confirms |

### Insert

Text input goes into the buffer. Esc commits (writes back to the
`.md`), Enter commits + creates a new block.

| Key | Action |
|-----|--------|
| `Esc` | Commit and return to Normal |
| `Enter` | Commit + new block below (soft newline inside open code fence — see below) |
| `Alt+Enter` / `Ctrl+J` | Soft newline (stays in same block) — portable across terminals |
| `Shift+Enter` | Soft newline — only on terminals that speak the kitty keyboard protocol |
| `Ctrl+Enter` | Cycle the block's TODO / DONE / none (stays in Insert) |
| `Tab` / `Shift-Tab` | Indent / outdent (stays in Insert) |
| `Backspace` on empty | Delete block, move to previous |
| `Left` at column 0 | Spill into the previous block (cursor at end) |
| `Right` at end of block | Spill into the next block (cursor at start) |
| `(`, `[`, `{` | Auto-pair with closing |
| `[[` | Page reference autocomplete (titles indexed across workspace) |
| `#` | Tag autocomplete |
| `↑` / `↓` in popup | Navigate completion |
| `Enter` / `Tab` in popup | Accept completion |
| `Esc` in popup | Cancel completion |

### Multi-line blocks and fenced code

A single block can hold multiple lines (`Alt+Enter` / `Ctrl+J` /
`Shift+Enter` on kitty terminals). Used for paragraphs of prose
inside one bullet and — most importantly — for fenced code blocks:

```
- ```lisp
  (+ 1 2)
  ```
```

**Auto-fence**: while typing inside an *open* code fence (the opener
` ``` ` is above the cursor but no closer has been typed yet), plain
`Enter` is treated as a soft newline. This lets you type a fenced
block naturally without remembering the soft-newline combo:

```
- ```lisp        ← typed `- `` ```lisp `, then Enter
  (+ 1 2)        ← typed body, Enter
  ```            ← typed closer, Enter
- next bullet    ← Enter here is a sibling again
```

The on-disk format is plain CommonMark — see
[`docs/markdown-format.md`](markdown-format.md#multi-line-block-text-continuation-lines).

### Visual

A range of blocks is highlighted. `j` / `k` extends the range; the
common Normal-mode keys for editing aren't available — Visual is for
batch operations.

| Key | Action |
|-----|--------|
| `Esc` / `v` / `V` | Cancel, back to Normal |
| `j` / `k` / `↑` / `↓` | Extend the range |
| `d` / `x` | Delete the selected range |
| `Tab` / `Shift-Tab` | Batch indent / outdent the selected range |

## Overlays

Three modal popups can appear over the main panes. They steal the
keystream while open; `Esc` always closes them.

### Quick Switcher (`Ctrl+P`)

Fuzzy search across page titles, slugs, and journal dates. Today's
date is always present even if the journal file doesn't exist yet.

### Search (`/`)

Workspace-wide block search using the same fuzzy matcher. Hits show
the source page label + a snippet of the block; `Enter` jumps to it.

### Command palette (`:`)

Vim-style command bar. Supported commands:

| Command | Action |
|---------|--------|
| `:q` / `:quit` | Quit |
| `:w` / `:write` / `:save` | Force re-save current page |
| `:open <name>` / `:o <name>` | Open page by name |
| `:new <name>` / `:n <name>` | Create page (or open if exists) |
| `:theme <name>` | Swap the active theme (see `outl theme list`) |
| `:today` | Jump to today's journal |
| `:help` / `:h` | Open the help popup |

Unknown commands surface in the status line as `unknown command:
:<line>`.

## Panels

```
┌─outl · default-dark ────────────────────────────────────┬─Backlinks────┐
│ Journal · Sunday, 2026-05-24                            │ Project X    │
├─────────────────────────────────────────────────────────┤   • led by … │
│ - first block                                           │              │
│ - second block with [[Avelino]] and #tag                │ Ideas        │
│   - nested                                              │   • saw …    │
│                                                         │              │
├──┌NORMAL─┐ i edit  o new  K/J move … ───────────────────┴──────────────┤
│  └───────┘                                                             │
└────────────────────────────────────────────────────────────────────────┘
```

- **Outline** — the current view (journal or named page). Markdown
  renders inline (bold/italic/code/strike); the selected/editing block
  is shown raw so cursor columns align with source bytes.
- **Backlinks** — every block in any other page that contains
  `[[this]]` or `#this`. Toggle with `B`. Self-references are
  excluded.
- **Status / hint** — mode badge, contextual key reminder, backlink
  count, status messages.

There is *no* pages sidebar. Use `Ctrl+P` (quick switcher) to jump to
any page or journal by fuzzy title — the sidebar was redundant with
that and ate horizontal space on narrow terminals.

## Behavior worth knowing

- **Autosave**: every commit (Esc from Insert, structural ops, history
  navigation) writes the `.md` to disk and reconciles into the op log.
  Concurrent `outl serve` is safe — both routes go through
  `outl_md::reconcile_md`.
- **No IDs on disk**: every block has a stable ULID, but it lives in
  the `.outl` sidecar file, not in your markdown. `outl serve` /
  `outl-tui` rebuild that sidecar after every change.
- **External edits hot-reload**: when another editor writes the
  currently-open `.md`, the TUI picks it up automatically within
  about a second. If you're in Insert mode, it refuses to clobber
  your in-flight edit and writes a warning on the status line —
  finish typing, press `Esc` to commit, then `Ctrl+L` to reload.
- **Undo bounded**: 200 most recent snapshots. Older edits drop off
  the front. Each snapshot remembers selection + cursor so undo lands
  you where you were.
- **Empty pages keep a bullet**: deleting the last block silently
  re-adds an empty `- ` so your cursor always has somewhere to go.
- **Slugified filenames**: `[[Avelino]]` lives in
  `pages/avelino.md` with `title:: Avelino` set automatically on
  first open.

## Code-block execution

Fenced code blocks can be run in place. The result lands as a
`> **result:**` subblock right below the source, and re-running
updates the same subblock idempotently.

| Key | Action |
|-----|--------|
| `g x` | Run the code block under the cursor |
| `:run` (also `:x`) | Same, via the command palette |

```
- ```lisp
  (map (lambda (x) (* x x)) (list 1 2 3 4))
  ```
  - > **result:** `(1 4 9 16)`
```

Built-in languages (each behind a Cargo feature, so you can strip
what you don't need):

| Tag | Engine | Notes |
|-----|--------|-------|
| ` ```lisp ` | [Steel](https://github.com/mattwparas/steel) | Scheme R5RS-ish |
| ` ```js ` | [Boa](https://boajs.dev) | ES2015+, `console.log` captured |
| ` ```python ` | [RustPython](https://rustpython.github.io) | Py3 subset, no native ext |
| ` ```lua ` | [mlua](https://github.com/mlua-rs/mlua) | Lua 5.4 vendored |
| ` ```echo ` | builtin | Returns source verbatim — debug only |

Adding another language is one file under `crates/outl-exec/src/runtimes/`
plus a feature flag. See [`docs/exec.md`](exec.md) (forthcoming) for
the contract and `outl-exec/src/runtimes/lisp.rs` as the canonical
template.

## Theming

See [`docs/theming.md`](theming.md) for the palette spec, preset list,
and how to set a theme via config or CLI.

## What's NOT in the TUI yet

Phase 1 lands the core editor and most-used surfaces. Some things are
explicitly deferred:

- **Embed `((block-id))`** — recognized as opaque text for now;
  inline render of the target block is phase 3.
- **`{{query: ...}}`** — inline saved queries; phase 3.
- **Visual mode batch indent / yank / paste** — only delete is wired
  today.
- **Graph view** — phase 5 desktop has it; the TUI may grow one but
  not a priority.
- **Live collaboration / P2P sync** — phase 2.
