# @outl/plugin-sdk

TypeScript SDK for authoring [outl](https://outl.app) plugins.

**Types + one helper, nothing else.** Zero runtime dependencies. The SDK never talks to Tauri, the filesystem, or the network — the real `PluginContext` is injected by the outl runtime (a Boa JS engine in the Rust `outl-plugins` crate) when it calls your plugin's `activate(ctx)`.

Documentation: **<https://outl.app/docs/query#plugin-sdk-api-outlquery>**

## Install

```sh
npm install @outl/plugin-sdk
# or
bun add @outl/plugin-sdk
```

## Quick start

```ts
import { definePlugin } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx) {
    ctx.commands.register("say-hi", () => ctx.ui.notify("hi from my plugin"));

    // React to every op applied to the log (needs the `read-op-log` permission).
    ctx.ops.onOp((op) => {
      if (op.actor?.startsWith("plugin:")) return; // ignore your own writes
      ctx.log.info(`op ${op.kind} on ${op.node}`);
    });
  },
});
```

All plugin **metadata** — id, version, permissions, `contributes` — lives in `plugin.json`, never in code. `definePlugin` carries **behavior only**, so there's exactly one source of truth per fact.

## Mental model

You think in **blocks and ops**, never in pixels, CRDT internals, or `.md` files. Every mutation you trigger (`ctx.blocks.move`, `ctx.blocks.edit`, …) becomes a host call routed through `outl-actions` → `Workspace::apply` → the op log, stamped `plugin:<id>@<device>`. The op log stays the single source of truth; the SDK is just a typed door to it.

Blocks execute **describe → apply**: reads (`query` / `get`) see a snapshot from the start of the turn, and writes are buffered and applied by the host *after* your handler returns. A block you `edit`/`create` this turn is **not** visible to a later `query` in the same turn — collect what you need first, then mutate.

## The `ctx` surface

Each namespace is gated by a permission declared in `plugin.json`; calling into one you didn't request rejects at the host boundary.

| Namespace | What it does | Permission |
|-----------|--------------|------------|
| `ctx.blocks` | Query, get, edit, create, move, toggle TODO, delete, `appendTree` | `read-page` / `write-page` / `submit-op` |
| `ctx.page` | List, create, `appendTree` (seed a fresh page's first blocks) | `read-page` / `write-page` |
| `ctx.template` | List and instantiate structural templates | `read-page` / `write-page` |
| `ctx.ops` | `onOp` hook fired for every applied op (local + synced) | `read-op-log` |
| `ctx.commands` | Register slash-menu / keybinding handlers | — (declared in `plugin.json`) |
| `ctx.config` | Read the user's validated config for this plugin | — |
| `ctx.storage` | Per-plugin local key/value store (**does not sync**) | `storage:local` |
| `ctx.secrets` | Read this plugin's secrets from the OS keychain | `secrets` |
| `ctx.net` | `fetch` with a required `timeoutMs` | `network:<domain>` |
| `ctx.content` | Register a code-fence transformer for a language | `content-transformer:*` |
| `ctx.sync` | Register a sync transport (ship/receive op JSONL) | `sync-transport` |
| `ctx.ui` | `notify` toast; `render` sandboxed HTML overlay (GUI only) | — / `ui-render` |
| `ctx.log` | Structured logging into the client's plugin log | — |

## Structured query — `outl.query`

The workspace query engine is also exposed as a structured API. Pass a plain object instead of the DSL string; both paths converge on the same engine:

```ts
const tasks = outl.query({ status: "todo", tag: "ops", sort: "page", limit: 50 });
for (const t of tasks) {
  console.log(`${t.status === "done" ? "[x]" : "[ ]"} ${t.text} — (${t.page})`);
}
```

Full field and result-shape reference: <https://outl.app/docs/query#plugin-sdk-api-outlquery>.

## Links

- Plugin API reference: <https://outl.app/docs/plugin-api>
- Plugin architecture: <https://outl.app/docs/plugin-architecture>
- Tutorial: <https://outl.app/docs/plugin-tutorial>
- Source: <https://github.com/avelino/outl>

## License

MIT
