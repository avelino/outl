# Page Pulse

An example outl plugin: a quick **pulse** of the workspace — total blocks, open TODOs, and DONEs.

It's the smallest demo of the **`toolbar-button`** capability:

- **`toolbar-button`** — a 💓 glyph in the GUI client's chrome that runs a command on tap.
- **`slash-command`** — the same `pulse` command is also reachable from the slash menu / palette.

A toolbar button always points at a registered command: tapping it dispatches the `pulse` command declared in `plugin.json`.
That's why both capabilities are listed — the command is the thing that exists, the button is one way to trigger it.

## What it does

`pulse` runs `ctx.blocks.query({})` (an empty filter matches every block), counts the TODOs and DONEs, and shows `💓 <n> blocks · <t> open · <d> done`.
It's **read-only** — no block is mutated.

## Where it runs

`toolbar-button` is a GUI capability: the button shows on **desktop** and **mobile** (which have chrome to host it).
On the **TUI/CLI** there's no toolbar surface, but the `pulse` command is still reachable from the slash menu.

## Layout

```
page-pulse/
├── plugin.json     # manifest — commands + toolbar
├── package.json    # build deps + SDK (not shipped)
├── tsconfig.json
├── src/index.ts    # entry — calls definePlugin(...)
├── index.js        # bundled output (IIFE)
└── README.md
```

## Build + install

```bash
bun install                         # from the repo root, once
cd examples/page-pulse && bun run build   # bundles src/index.ts → index.js (IIFE)

outl -w <workspace> plugin install ./examples/page-pulse --yes
```

Then tap the 💓 button in the toolbar (or run `pulse` from the slash menu) to read the pulse.

## License

MIT.
