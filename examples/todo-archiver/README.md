# TODO Archiver

Plugin-exemplo do outl. Serve de **template canônico** pra escrever um plugin e
de teste de fumaça do pipe inteiro: `op-hook` + `slash-command` + `keybinding` +
`config-schema`, tudo num plugin pequeno e funcional.

Move blocos `DONE` pra uma página de arquivo, mantendo as páginas de trabalho
focadas no que ainda está aberto.

## What it does

- **Command `Archive DONE blocks`** (`todo-archive-done`): finds every `DONE`
  block in the workspace and moves it to the configured archive page. Skips
  blocks already on that page. Runs from the slash menu or via `Ctrl+Shift+A`.
- **Op hook**: logs each block that transitions into `DONE` (ignores the
  plugin's own archive moves, so there's no feedback loop).
- **Config**: a single `archivePage` setting (default `archive`).

## Layout

```
todo-archiver/
├── plugin.json          # manifest — the contract with the host
├── package.json         # build deps + SDK (not shipped)
├── tsconfig.json
├── config.schema.json   # JSON Schema for the user-editable config
├── src/index.ts         # entry — calls definePlugin(...)
├── README.md
└── LICENSE
```

The author builds a single bundled `index.js`; only the install shape
(`plugin.json`, `index.js`, `config.schema.json`, `README.md`) is shipped — no
`node_modules`, no runtime resolution.

## Build

```sh
bun install          # from the repo root (bun workspaces)
bun run build        # bundles src/index.ts -> index.js via esbuild
bun run typecheck    # tsc --noEmit
```

## Config

```jsonc
{
  // Slug of the page DONE blocks are moved to.
  "archivePage": "archive"
}
```

## How a mutation flows

`ctx.blocks.move(id, { toPage })` is **not** a direct edit. It becomes a host
call → `outl-actions` → `Workspace::apply` → the op log, stamped
`plugin:app.outl.examples.todo-archiver@<device>`. The `.md` files stay 100% clean and
the op log is the audit trail. The author thinks in blocks and ops, never in
CRDT internals or markdown.

## License

MIT — see [LICENSE](./LICENSE).
