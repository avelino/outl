# TUI Manual

The `outl` terminal UI is the primary way to interact with an outl workspace.
It's journal-first, modal (Normal / Insert / Visual), and designed to feel familiar if you've used vim or any keyboard-driven outliner (Roam, Logseq, Obsidian).

## Running

```bash
outl --workspace ~/notes          # opens the TUI on ~/notes
outl --workspace ~/notes --theme dracula
outl tui ~/notes             # explicit subcommand form
cd ~/notes && outl           # no args: opens TUI in cwd
```

The TUI requires a real interactive terminal.
If stdout isn't a TTY (e.g. CI), it exits with a clear error instead of hanging.

## Config

The TUI reads two layers of TOML before launching:

1. **Global** — `~/.config/outl/config.toml` (the [`outl-config`](../crates/outl-config/) crate; XDG-style on every OS).
   Same file the desktop app's Settings modal writes to, so changing your theme in the desktop reflects on the next TUI launch.
   ```toml
   [theme]
   preset = "dracula"

   [editor]
   vim_mode = true
   font_size = 15
   ```
2. **Per-workspace** — `<workspace>/.outl/config.toml`.
   Workspace identity (`[workspace] actor_id = "..."`) lives here and can't move to global — it's per-device-per-workspace by design.
   A `[theme] preset` here overrides the global setting for this workspace only.

Theme precedence at startup (first hit wins): `--theme` CLI flag → per-workspace `[theme] preset` → global `[theme] preset` → built-in default (`outl`).

## Modes

### Normal

The default.
Move between blocks, open references, run commands.
No characters insert themselves — every key is a command.

| Key | Action |
|-----|--------|
| `i` | Edit current block (Insert mode) |
| `I` | Edit, cursor at start of block |
| `o` / `O` | New block below / above |
| `Enter` | Open `[[ref]]` / `#tag` / journal / block ref (`((blk-X))` / `!((blk-X))`) under cursor (otherwise edit). On a block ref it opens the source page and lands the cursor on the referenced block; orphan handles surface a status message and stay put. |
| `j` / `k` / `↑` / `↓` | Move between blocks |
| `h` / `l` / `←` / `→` | Move cursor inside the current block |
| `w` / `b` | Cursor to next / previous word |
| `0` / `$` | Cursor to start / end of block |
| `Tab` / `Shift-Tab` | Indent / outdent the current block |
| `K` / `J` (or `Alt+↑/↓`) | Move block up / down |
| `dd` | Delete the current block (chord) |
| `c` | Fold / unfold the current block. The bullet row shows `▼ ` (expanded) or `▶ ` (collapsed) when the block has children, two spaces otherwise. Children are hidden from the outline while collapsed and `j` / `k` skip past them, but the underlying tree is untouched. State is persisted as an `Op::SetCollapsed` in the op log — every device replays the same sequence, so the fold layout converges across iCloud / Syncthing peers without relying on file-level last-write-wins (which would lose concurrent flips). No-op on a block whose sidecar entry hasn't been written yet (save first). |
| `y r` | Yank the current block's ref handle (`((blk-XXXXXX))`) to the OS clipboard + `last_yanked_ref` (chord). On headless / no-clipboard environments it falls back to the status line only. |
| `Ctrl+Enter` / `Ctrl+T` | Cycle the block's TODO / DONE / none prefix (`Ctrl+T` is the portable fallback for tmux / Terminal.app, which collapse `Ctrl+Enter` into plain `Enter`) |
| `u` / `Ctrl+R` | Undo / redo |
| `V` | Enter Visual mode (multi-block select) |
| `t` / `Home` | Today's journal |
| `[` / `]` | Previous / next journal |
| `g j` | Jump to today (chord) |
| `Ctrl+P` | Quick switcher (fuzzy page/journal pick) |
| `/` | Slash command menu (Notion-style, fuzzy filter) |
| `:` | Command palette (vim-style) |
| `B` | Toggle the inline backlinks section below the outline |
| `?` | Toggle this help popup |
| `q q` / `Ctrl+C` | Quit — `q` alone arms a chord, second `q` confirms |

### Insert

Text input goes into the buffer.
Esc commits (writes back to the `.md`), Enter commits + creates a new block.

| Key | Action |
|-----|--------|
| `Esc` | Commit and return to Normal |
| `Enter` | Commit + new block below (soft newline inside open code fence — see below) |
| `Alt+Enter` / `Ctrl+J` | Soft newline (stays in same block) — portable across terminals |
| `Shift+Enter` | Soft newline — only on terminals that speak the kitty keyboard protocol |
| `Ctrl+Enter` / `Ctrl+T` | Cycle the block's TODO / DONE / none (stays in Insert; `Ctrl+T` works on terminals that collapse `Ctrl+Enter`) |
| `Tab` / `Shift-Tab` | Indent / outdent (stays in Insert) |
| `Backspace` on empty | Delete block, move to previous |
| `Left` at column 0 | Spill into the previous block (cursor at end) |
| `Right` at end of block | Spill into the next block (cursor at start) |
| `(`, `[`, `{` | Auto-pair with closing |
| `[[` | Page reference autocomplete (titles indexed across workspace) |
| `#` | Tag autocomplete |
| `((` | Block reference autocomplete — fuzzy-match on block text, inserts `((blk-XXXXXX))`. Empty query lists newest-first (NodeId descending = ULID time order) so the popup is deterministic and the same eight rows show on every keystroke. |
| `↑` / `↓` in popup | Navigate completion |
| `Enter` / `Tab` in popup | Accept completion |
| `Esc` in popup | Cancel completion |

### Multi-line blocks and fenced code

A single block can hold multiple lines (`Alt+Enter` / `Ctrl+J` / `Shift+Enter` on kitty terminals).
Used for paragraphs of prose inside one bullet and — most importantly — for fenced code blocks:

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
- ```lisp        ← typed `- `` ```lisp `, then Enter (+ 1 2)        ← typed body, Enter
  ```            ← typed closer, Enter
- next bullet    ← Enter here is a sibling again
```

The on-disk format is plain CommonMark — see [`docs/markdown-format.md`](markdown-format.md#multi-line-block-text-continuation-lines).

### Visual

A range of blocks is highlighted.
`j` / `k` extends the range; the common Normal-mode keys for editing aren't available — Visual is for batch operations.

| Key | Action |
|-----|--------|
| `Esc` / `v` / `V` | Cancel, back to Normal |
| `j` / `k` / `↑` / `↓` | Extend the range |
| `d` / `x` | Delete the selected range |
| `Tab` / `Shift-Tab` | Batch indent / outdent the selected range |

## Overlays

Three modal popups can appear over the main panes.
They steal the keystream while open; `Esc` always closes them.

### Quick Switcher (`Ctrl+P`)

Fuzzy search across page titles, slugs, and journal dates.
Today's date is always present even if the journal file doesn't exist yet.

### Slash menu (`/`) and Command palette (`:`)

Two surfaces over the **same** command registry — pick whichever matches your muscle memory:

- **`/`** (Normal mode) opens a Notion-style filterable list.
  Each entry shows its name + description.
  Inside Insert mode, typing `/` triggers inline autocomplete with the same list — pick a command with `Tab`/`Enter` without leaving the buffer.
- **`:`** is the vim command line.
  Same registry, same args, same aliases — `/q` and `:q` are interchangeable.

Unknown commands surface in the status line as `unknown command: <name>`.

#### Workspace / navigation

| Command | Aliases | Action |
|---------|---------|--------|
| `open <name>` | `o`, `new`, `n` | Open (or create) page by name |
| `today` | — | Jump to today's journal |
| `search` | `s`, `find` | Workspace-wide block search |
| `quit` | `q`, `exit` | Close the TUI |
| `write` | `w`, `save` | Force-save current page |
| `refresh` | `r`, `reload` | Re-read workspace from disk |
| `theme <preset>` | — | Swap the active theme |
| `help` | `h` | Toggle help popup |

#### Properties

| Command | Aliases | Action |
|---------|---------|--------|
| `prop-block <key> <value>` | `prop` | Set property on current block (empty value deletes) |
| `prop-page <key> <value>` | — | Set page-level property (`title::`, `icon::`, …) |

#### Block references

| Command | Aliases | Action |
|---------|---------|--------|
| `refer` | — | Copy `((blk-XXXXXX))` of the current block to the OS clipboard + `last_yanked_ref`. Same as the `y r` chord. |
| `refer-embed` | — | Copy the embed form `!((blk-XXXXXX))` of the current block to the OS clipboard + `last_yanked_ref`. |

> **Clipboard fallback**: `y r` / `/refer` / `/refer-embed` use [`arboard`](https://crates.io/crates/arboard) to talk to the OS clipboard.
> The status line reads `copied … to clipboard` on success and `yanked … (clipboard unavailable)` on terminals / SSH sessions without a clipboard backend — the token still lives in `last_yanked_ref` so the in-app paste path keeps working.

#### Code execution

| Command | Aliases | Action |
|---------|---------|--------|
| `run` | `x`, `execute` | Run the code block under the cursor |

#### Date & time inserters

These write text **at the cursor** (Insert mode only).
They skip the auto-commit step the other commands do, so your in-flight edit stays alive while the text lands.

| Command | Aliases | Inserts |
|---------|---------|---------|
| `date-today` | `dt` | `[[YYYY-MM-DD]]` (today) |
| `date-tomorrow` | `dtm` | `[[YYYY-MM-DD]]` (today + 1) |
| `date-yesterday` | `dy` | `[[YYYY-MM-DD]]` (today − 1) |
| `date-next-week` | `dnw` | `[[YYYY-MM-DD]]` (today + 7) |
| `date-last-week` | `dlw` | `[[YYYY-MM-DD]]` (today − 7) |
| `date-next-monday` | `dnmon` | next Monday's journal ref |
| `date-next-tuesday` | `dntue` | next Tuesday's journal ref |
| `date-next-wednesday` | `dnwed` | next Wednesday's journal ref |
| `date-next-thursday` | `dnthu` | next Thursday's journal ref |
| `date-next-friday` | `dnfri` | next Friday's journal ref |
| `date-next-saturday` | `dnsat` | next Saturday's journal ref |
| `date-next-sunday` | `dnsun` | next Sunday's journal ref |
| `date <arg>` | — | flexible — see below |
| `iso-date-today` | `isod` | `YYYY-MM-DD` (no brackets, for `due::` etc) |
| `iso-date-tomorrow` | `isodtm` | `YYYY-MM-DD` |
| `iso-date-yesterday` | `isody` | `YYYY-MM-DD` |
| `time-now` | `now`, `tn` | `HH:MM` (no brackets, plain time) |
| `datetime-now` | `dtn`, `stamp` | `[[YYYY-MM-DD]] HH:MM` (journal ref + time) |
| `week-num` | `wn`, `week` | `#YYYY-Www` (ISO week as a tag) |

##### `/date <arg>`

| Input | Resolves to |
|-------|-------------|
| `/date +3d` | today + 3 days |
| `/date -2w` | today − 2 weeks |
| `/date +1m` | today + 1 month (Jan 31 + 1m → Feb 28/29 — clamped to last day of month) |
| `/date 5d` | bare `Nd`/`Nw`/`Nm` is treated as positive |
| `/date 2026-06-15` | absolute ISO date |

Garbage input (`/date nope`, `/date +3x`, invalid date) shows `usage: date +Nd | -Nw | +Nm | YYYY-MM-DD` on the status line.

> **Weekday math:** `date-next-<weekday>` always jumps to the **next** occurrence of that weekday, strictly in the future.
> Running it on the same weekday adds 7 days, not 0 — `date-next-monday` on a Monday means "next Monday.
> **ISO week year:** `week-num` uses `%G-W%V` (ISO 8601), not `%Y-W%V`.
> The ISO year can differ from the calendar year on a few days around year boundaries — e.g. 2025-12-31 (Wednesday) belongs to ISO week `2026-W01`, not `2025-W01`.

## Panels

```
┌─outl · default-dark ───────────────────────────────────────────────────┐
│ Page · Avelino                                                         │
├────────────────────────────────────────────────────────────────────────┤
│ - I am the author                                                      │
│ - some other note                                                      │
│ ───────────────────────────────────────────────────────────────────────│
│  Backlinks · 2 ref(s)                                                  │
│                                                                        │
│ 📄  Project X                                                          │
│ - led by [[Avelino]]                                                   │
│   - milestone A                                                        │
│   - milestone B                                                        │
│                                                                        │
│ 📅  2026-05-24                                                         │
│ - meeting with [[Avelino]] about Q4                                    │
├──┌NORMAL─┐ i edit  o new  K/J move …  ⇇ 2 backlinks ───────────────────┤
│  └───────┘                                                             │
└────────────────────────────────────────────────────────────────────────┘
```

- **Outline** — the current view (journal or named page).
  Markdown renders inline (bold/italic/code/strike); the selected/editing block is shown raw so cursor columns align with source bytes.
  Block references (`((blk-XXXXXX))`) resolve to the source block's text plus its page icon; orphaned handles render dimmed.
  Embeds (`!((blk-XXXXXX))`) — when the block contains a single embed token (whitespace OK) — render the source block **and its children** expanded read-only below the carrying block.
  Every embed row carries a `↳ ` prefix (root + descendants), so the expansion reads as one cohesive block; descendants are indented by `2 * (depth + 1)` spaces before their `↳ ` so children align under the source's *text*, not under the parent's `↳ `.
  TODO/DONE checkboxes, page refs, and tags render with their normal styling inside the expansion.
  Recursion is capped at depth 4 to break embed cycles.
  The cursor-bearing block always keeps the raw `((…))` / `!((…))` literal on its first row so column counting stays exact.
- **Backlinks (inline)** — rendered below the outline, separated by a full-width `─` rule.
  Every block in any other page that contains `[[this]]` or `#this` shows up with its children, grouped by source page.
  `j`/`k` navigation crosses the separator transparently: from the last outline block, `j` lands you on the first backlink; `k` from the first backlink walks back into the outline.
  Toggle the section with `B`.
  Self-references are excluded.
  Press `i` / `Enter` on a backlink to jump to its source page positioned on the referencing block (in-place editing lands in a follow-up).
- **Status / hint** — mode badge, contextual key reminder, backlink count, status messages.

There is *no* pages sidebar.
Use `Ctrl+P` (quick switcher) to jump to any page or journal by fuzzy title — the sidebar was redundant with that and ate horizontal space on narrow terminals.

## Parser-warning banner

When you open a `.md` that the outl parser had to recover from (a leading `# heading`, a free paragraph between bullets, imported markdown that doesn't fit the dialect), the TUI shows a yellow banner above the outline:

```
┌─ ⚠ 3 line(s) outside outl dialect — preserved as blocks ─┐
│ line 1: # 2026-06-08 (+2 more)                           │
└──────────────────────────────────────────────────────────┘
```

- Every offending line is preserved as a regular block — nothing is dropped on parse, and the next save normalises the file to the dialect (`- <raw>`).
- The status line also carries a short `⚠ N line(s) outside outl dialect — preserved` hint for terminals where the banner is not visible.
- On a clean file the banner collapses to zero height; the layout looks identical to before the feature landed.
- Source of truth: `ParsedPage.warnings` (`outl_md::ParseWarning`).
  Mobile + desktop render the same data via `<ParseWarningsBanner>` from `@outl/shared`, and `outl doctor` lists every page with active warnings.
- The behaviour is intentionally non-blocking: you can keep editing, save, navigate away.
  Use the warning as a hint to clean the file at your pace — outl never deletes content on your behalf.

## Behavior worth knowing

- **Autosave**: every commit (Esc from Insert, structural ops, history navigation) writes the `.md` to disk and reconciles into the op log.
  Concurrent `outl serve` is safe — both routes go through `outl_md::reconcile_md`.
- **No IDs on disk**: every block has a stable ULID, but it lives in the `.outl` sidecar file, not in your markdown.
  `outl serve` / `outl-tui` rebuild that sidecar after every change.
- **External edits hot-reload**: when another editor writes the currently-open `.md`, the TUI picks it up automatically within about a second.
  If you're in Insert mode, it refuses to clobber your in-flight edit and writes a warning on the status line — finish typing, press `Esc` to commit, then `Ctrl+L` to reload.
- **Undo bounded**: 200 most recent snapshots.
  Older edits drop off the front.
  Each snapshot remembers selection + cursor so undo lands you where you were.
- **Empty pages keep a bullet**: deleting the last block silently re-adds an empty `- ` so your cursor always has somewhere to go.
- **Slugified filenames**: `[[Avelino]]` lives in `pages/avelino.md` with `title:: Avelino` set automatically on first open.

## Code-block execution

Fenced code blocks can be run in place.
The result lands as a `> **result:**` subblock right below the source, and re-running updates the same subblock idempotently.

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

- **`{{query: ...}}`** — inline saved queries; phase 3.
- **Visual mode batch indent / yank / paste** — only delete is wired
  today.
- **Graph view** — phase 5 desktop has it; the TUI may grow one but
  not a priority.
- **Live collaboration / P2P sync** — phase 2.
