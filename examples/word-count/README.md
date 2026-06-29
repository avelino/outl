# Word Count

Plugin-exemplo do outl. Demonstra a capability **`op-hook`** sozinha: observa o
op stream e, a cada edição de bloco, conta as palavras e dá um toast na primeira
vez que o bloco cruza um marco (50 / 100 / 250 / 500 palavras).

É read-only — não muta nada. A única permission é `read-op-log`, que é o que
`ctx.ops.onOp` exige.

## What it does

- **Op hook**: on every `Edit` op, counts the words in `op.text`. The first time
  a block crosses 50, 100, 250 or 500 words, it toasts `📝 100 words in this block`.
- No commands, no config, no writes.

## Layout

```
word-count/
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
