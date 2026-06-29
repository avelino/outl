# Random Task

An example outl plugin: pick one open **TODO** at random and tell you to focus on it.

It's the smallest demo of the **`keybinding`** capability:

- **`keybinding`** — a chord (`Ctrl+Shift+R`) bound to a command, so it fires without opening the slash menu.
- **`slash-command`** — the same `pick` command is also reachable from the slash menu / palette.

A keybinding always points at a registered command: the chord just dispatches the `pick` command declared in `plugin.json`.
That's why both capabilities are listed — the command is the thing that exists, the keybinding is one way to trigger it.

## What it does

`pick` runs `ctx.blocks.query({ todo: "TODO" })`, chooses a random block, and shows `👉 Focus on: <text>` (or `🎉 No open tasks!` when nothing is open).
It's **read-only** — no block is mutated — which keeps the example free of any describe→apply ordering concern.

The host JS engine (Boa) ships a normal `Math.random()`, so `Math.random()` is all it takes to pick the index.

## Layout

```
random-task/
├── plugin.json     # manifest — commands + keybinding
├── package.json    # build deps + SDK (not shipped)
├── tsconfig.json
├── src/index.ts    # entry — calls definePlugin(...)
├── index.js        # bundled output (IIFE)
└── README.md
```

## Build + install

```bash
bun install                          # from the repo root, once
cd examples/random-task && bun run build   # bundles src/index.ts → index.js (IIFE)

outl -w <workspace> plugin install ./examples/random-task --yes
```

Then press `Ctrl+Shift+R` (or run `pick` from the slash menu) to get a random task to focus on.

## License

MIT.
