# Getting started

A minute from clone to first journal entry.

## Install

Right now outl is built from source. Released binaries land with
phase 4.

```bash
git clone https://github.com/avelino/outl.git
cd outl
cargo build --release
```

You need Rust 1.88+. `rust-toolchain.toml` pins the version, so
`rustup` will pick it up automatically.

Drop the binary anywhere on your `PATH`:

```bash
cp target/release/outl ~/.local/bin/
```

(Or use `cargo install --path crates/outl-cli` if that's your
flavor.)

## Create a workspace

A workspace is just a directory. Pick a path, point `outl init` at
it:

```bash
outl init ~/notes
```

You'll get:

```
~/notes/
├── .outl/
│   ├── log.db          # the op log (SQLite)
│   ├── config.toml     # workspace identity + settings
│   ├── peers.toml      # P2P peers (phase 2+)
│   └── orphans.log     # log of unmatched blocks during external edits
├── pages/              # your named pages live here
├── journals/
│   └── 2026-05-25.md   # today's journal, seeded
└── templates/
    └── journal.md      # template applied to new journals
```

## Open the TUI

```bash
outl --path ~/notes
```

It lands you on today's journal. Press `?` to see every keymap.

Or, if you `cd ~/notes`, just `outl` works — no subcommand means
"open the TUI here."

## First moves

| You want to... | Do this |
|----------------|---------|
| Start typing | `i` (edit current block) or `o` (new block below) |
| Open `[[Avelino]]` (creating the page if needed) | type `[[`, autocomplete, press `Enter` over the link |
| Jump to today | `t` |
| Yesterday / tomorrow | `[` / `]` |
| Find any page or journal | `Ctrl+P` (fuzzy switcher) |
| Search the whole workspace | `/` |
| Run a command | `:` then `theme dracula`, `open Foo`, `q`, etc. |
| Quit | `q` or `Ctrl+C` |

Pages you reference but haven't created yet are *real* the moment you
press `Enter` on the link — outl creates `pages/<slug>.md` with
`title:: <Name>` automatically.

## Try a theme

Six built-in palettes:

```bash
outl --path ~/notes --theme dracula
outl --path ~/notes --theme nord
outl --path ~/notes --theme monokai
outl --path ~/notes --theme solarized-dark
outl --path ~/notes --theme light
outl --path ~/notes --theme default-dark
```

To pin a theme per workspace, edit `~/notes/.outl/config.toml`:

```toml
[theme]
preset = "dracula"
```

Or switch at runtime: open the command palette with `:`, type
`theme nord`, hit Enter.

## Edit `.md` externally

Open any file in `~/notes/pages/` with VS Code, vim, Obsidian —
whatever. The file is plain markdown:

```markdown
title:: Avelino

- some block
- another block with [[ref]] and #tag
```

There are no `id::` lines, no UUIDs, no HTML comments. When you save
and reopen the TUI, outl matches each block back to its sidecar entry
and rebuilds the op log.

## Next steps

- The [TUI manual](tui.md) — every key, every overlay, persistence
  rules, gotchas.
- [Why outl](why-outl.md) — the pitch vs. Roam and Logseq.
- [Sync, done right](sync.md) — what makes the algorithm interesting.
- The [Roadmap](roadmap.md) — what's coming.
