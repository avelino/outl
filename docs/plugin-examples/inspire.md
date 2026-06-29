# Inspire

> **Capability:** `network` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/inspire)

A one-command plugin that reaches the outside world.
The `inspire` command fetches a random quote over HTTP and shows it as a notification.
It is the canonical template for **any** external integration from a plugin — swap the URL and add an auth header and you have a GPT call, a webhook, or a REST sync.

## What it demonstrates

`ctx.net.fetch` and how the host gates it.

Network is a **permission**, not a capability.
The plugin declares the exact domain it will hit, and the host checks every request against the approved set.
A URL outside the approved domains is refused with `{ ok: false }` — it does **not** throw — so the handler always checks `r.ok` before reading the body.

A bare `network:*` is rejected by the host.
You scope to a domain (`network:api.quotable.io`) or a leading-label wildcard (`network:*.quotable.io`).

The fetch is blocking under the hood (it runs on the plugin's own thread), which is why `timeoutMs` is required: no unbounded calls.

## The code

```ts
import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

/** Public quote API. One specific domain — `network:*` is rejected by the host. */
const QUOTE_URL = "https://api.quotable.io/random";

/** Shape of the JSON body the quote API returns. */
interface Quote {
  content: string;
  author: string;
}

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("inspire", async () => {
      // The fetch is gated by the `network:api.quotable.io` permission. A host
      // outside the approved set comes back as `{ ok: false }`, never a throw.
      const r = await ctx.net.fetch(QUOTE_URL, { timeoutMs: 8000 });

      if (!r.ok) {
        const reason = `HTTP ${r.status}`;
        ctx.log.error(`[inspire] fetch failed: ${reason}`);
        ctx.ui.notify(`Could not reach the quote service (${reason})`);
        return;
      }

      const quote = await r.json<Quote>();
      ctx.ui.notify(`💬 ${quote.content} — ${quote.author}`);
    });
  },
});
```

## Manifest

```json
{
  "capabilities": ["slash-command"],
  "permissions": ["network:api.quotable.io"],
  "contributes": {
    "commands": [{ "id": "inspire", "title": "Inspire me" }]
  }
}
```

Note that `capabilities` lists only `slash-command`.
The network access lives entirely in `permissions`, scoped to the one domain the plugin talks to.

## Try it

```sh
outl -w <workspace> plugin install ./examples/inspire --yes
# then run the `inspire` command from the slash menu / command palette
```

## Adapting it

Point it at a different API and read the credentials from config.

```ts
const { apiKey } = ctx.config.get<{ apiKey: string }>();
const r = await ctx.net.fetch("https://api.openai.com/v1/chat/completions", {
  method: "POST",
  headers: { Authorization: `Bearer ${apiKey}`, "content-type": "application/json" },
  body: JSON.stringify({ /* ... */ }),
  timeoutMs: 30_000,
});
```

Two manifest changes follow from that: change the permission to `network:api.openai.com`, and add an `apiKey` field to a `config.schema.json` so the host renders a settings form and validates the value before `ctx.config.get` hands it back.
