# Greeter

Plugin-exemplo do outl. Demonstra a capability **`config-schema`**: lê um setting
`name` editável pelo usuário (validado pelo host contra `config.schema.json`) e
um comando `greet` dá um toast com um olá amigável usando ele.

Sem permissions — não lê páginas nem o op log. `ctx.config.get()` não tem gate,
então um array de `permissions` vazio é o correto.

## What it does

- **Command `Greet me`** (`greet`): reads `ctx.config.get().name` and toasts
  `👋 Hello, <name>! Your outline missed you.`. Runs from the slash menu.
- **Config**: a single `name` setting (default `friend`).

## Layout

```
greeter/
├── plugin.json         # manifest — the contract with the host
├── package.json        # build deps + SDK (not shipped)
├── tsconfig.json
├── config.schema.json  # JSON Schema for the user-editable config
├── src/index.ts        # entry — calls definePlugin(...)
├── index.js            # the bundled output (esbuild, iife)
└── README.md
```

## Build

```sh
bun run build        # bundles src/index.ts -> index.js via esbuild
bun run typecheck    # tsc --noEmit
```

## Config

```jsonc
{
  // Name the greeting addresses you by.
  "name": "friend"
}
```

## License

MIT.
