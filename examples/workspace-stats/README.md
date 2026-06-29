# Workspace Stats

Plugin-exemplo do outl. Demonstra a capability **`slash-command`** apoiada em
queries read-only. O comando `stats` varre o workspace inteiro e dá um toast com
um resumo de uma linha: total de blocos, TODOs abertos, DONEs e nº de páginas.

Só lê — `read-page` é a única permission. Sem writes, então sem `write-page` /
`submit-op`.

## What it does

- **Command `Workspace statistics`** (`stats`): runs `ctx.blocks.query({})` and
  `ctx.page.list()`, counts blocks, `TODO`s, `DONE`s and pages, then toasts
  `📊 42 blocks · 12 TODO · 8 DONE · 5 pages`. Runs from the slash menu.

## Layout

```
workspace-stats/
├── plugin.json     # manifest — the contract with the host
├── package.json    # build deps + SDK (not shipped)
├── tsconfig.json
├── src/index.ts    # entry — calls definePlugin(...)
├── index.js        # the bundled output (esbuild, iife)
└── README.md
```

## Build

```sh
bun run build        # bundles src/index.ts -> index.js via esbuild
bun run typecheck    # tsc --noEmit
```

## License

MIT.
