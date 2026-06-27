# Echo Sync

Example outl plugin. An **educational skeleton** of a sync transport — it shows
the exact interface a real transport plugs into, with no backend wired in.

Use it as the starting point for "I want outl to sync through *my* server / S3 /
whatever" — the structure is already here; you fill in `ctx.net`.

## What it does

Registers a sync transport via `ctx.sync.register({ push, pull })`:

- **`push(opsJsonl)`**: the host hands you the JSONL of locally-authored ops
  after edits. The skeleton just logs `[echo-sync] pushing N ops` (one op per
  line). A real transport would `ctx.net.fetch(backendUrl, { method: "POST",
  body: opsJsonl })`.
- **`pull()`**: the host calls this on a timer, expecting JSONL of remote ops
  back (or `null`). The skeleton returns `null` — nothing to apply.

## The contract: you transport bytes, the host owns the CRDT

A sync plugin **never** touches the tree. The host applies whatever JSONL `pull`
returns through the CRDT itself, with HLC ordering, so devices converge
deterministically. This is invariant #7 ("any state that must converge goes
through the op log") made pluggable — you only move bytes.

```json
"capabilities": ["sync-transport"],
"permissions": []
```

No permissions, because the skeleton makes no network calls. A real transport
adds `network:<your-backend-domain>`.

## Layout

```
echo-sync/
├── plugin.json     # manifest — declares the sync-transport capability
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

## Adapting it (real backend)

The `push`/`pull` bodies have the wiring spots marked in comments. To make it
real:

1. Add `network:<your-backend-domain>` to `permissions` in `plugin.json`.
2. Add a `config.schema.json` with a `url` field, read it with
   `ctx.config.get<{ url: string }>()`.
3. In `push`, `POST` `opsJsonl` to your backend.
4. In `pull`, `GET` remote ops and return the raw JSONL string (or `null`).

The host does the rest — it replays your bytes through the CRDT.

## License

MIT — see [LICENSE](./LICENSE).
