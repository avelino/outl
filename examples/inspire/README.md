# Inspire

Example outl plugin. The **canonical template for any external integration**:
a single `inspire` command fetches a random quote over HTTP and shows it as a
notification.

If you want a plugin that talks to a GPT API, a webhook, or any REST backend,
start here — the only things that change are the URL, the headers, and how you
shape the response.

## What it does

- **Command `Inspire me`** (`inspire`): calls
  `ctx.net.fetch("https://api.quotable.io/random", { timeoutMs: 8000 })`,
  parses the JSON body (`{ content, author }`), and notifies
  `💬 <content> — <author>`. On failure it checks `r.ok` and surfaces the error.

## The one idea: `ctx.net`

`network` is a **permission**, not a capability. You declare the exact domain
you'll hit:

```json
"capabilities": ["slash-command"],
"permissions": ["network:api.quotable.io"]
```

`network:*` is **rejected** — scope to a domain (`api.quotable.io`) or a
leading-label wildcard (`*.quotable.io`). The host checks every request against
the approved set; a URL outside it comes back as `{ ok: false }` (it does **not**
throw), so always check `r.ok`.

## Layout

```
inspire/
├── plugin.json     # manifest — declares the network permission + command
├── package.json    # build deps + SDK (not shipped)
├── tsconfig.json
├── src/index.ts    # entry — calls definePlugin(...)
├── index.js        # the shipped bundle (esbuild iife)
├── README.md
└── LICENSE
```

## Build

```sh
bunx --bun esbuild src/index.ts \
  --bundle --format=iife --platform=neutral --target=es2022 \
  --outfile=index.js --alias:@outl/plugin-sdk=../../plugin-sdk/src/index.ts
```

`--format=iife` is required — the host runs the plugin as a single
self-contained script, no module resolution.

## Adapting it (GPT / any API)

Swap the URL and add auth read from config:

```ts
const { apiKey } = ctx.config.get<{ apiKey: string }>();
const r = await ctx.net.fetch("https://api.openai.com/v1/chat/completions", {
  method: "POST",
  headers: { Authorization: `Bearer ${apiKey}`, "content-type": "application/json" },
  body: JSON.stringify({ /* ... */ }),
  timeoutMs: 30_000,
});
```

Then change the permission to `network:api.openai.com` and add the `apiKey`
field to a `config.schema.json` so the host renders a form for it.

## License

MIT — see [LICENSE](./LICENSE).
