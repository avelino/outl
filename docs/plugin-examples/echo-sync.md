# Echo Sync

> **Capability:** `sync-transport` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/echo-sync)

An educational skeleton of a sync transport.
It shows the exact interface a real transport plugs into — `ctx.sync.register({ push, pull })` — with no backend wired in.
Use it as the starting point for "I want outl to sync through *my* server".

## What it demonstrates

The `push` / `pull` contract, and the strict division of labor between a sync plugin and the host.

A sync plugin **only transports bytes**.
It never touches the CRDT or the materialized tree.
The host drives the cadence:

- It calls `push(opsJsonl)` with the JSONL of locally-authored ops after edits — one op per line, so counting lines is counting ops.
- It calls `pull()` on a timer and expects JSONL of remote ops back, or `null` when there is nothing new.

Whatever JSONL `pull` returns, the host applies it through the CRDT itself, with HLC ordering, so two devices converge deterministically.
The plugin never parses or trusts those bytes into the tree.
This is the op-log convergence invariant made pluggable: any state that must converge between devices flows through the op log, and a transport is just the wire.

In this skeleton there is no backend, so `push` only logs how many ops it *would* ship and `pull` returns `null`.
The two spots where a real transport wires `ctx.net` plus a configured backend URL are marked in the source comments.

## The code

```ts
import { definePlugin, type PluginContext, type SyncTransport } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    const transport: SyncTransport = {
      // Called by the host with the JSONL of locally-authored ops to ship.
      // One op per line, so counting lines = counting ops.
      push(opsJsonl: string): void {
        const count = countOps(opsJsonl);
        ctx.log.info(`[echo-sync] pushing ${count} op(s)`);

        // A real transport ships the bytes to its backend, e.g.:
        //
        //   const backend = ctx.config.get<{ url: string }>().url;
        //   const r = ctx.net.fetch(backend, {
        //     method: "POST",
        //     headers: { "content-type": "application/x-ndjson" },
        //     body: opsJsonl,
        //     timeoutMs: 10_000,
        //   });
        //   if (!r.ok) ctx.log.error("[echo-sync] push failed");
        //
        // (requires `network:<backend-domain>` in plugin.json and an
        //  `url` config key via `configSchema`.)
      },

      // Called by the host on a timer. Return JSONL of remote ops to apply, or
      // `null` when there's nothing new. The skeleton has no backend, so: null.
      pull(): string | null {
        ctx.log.info("[echo-sync] pull: no backend wired, nothing to apply");

        // A real transport fetches remote ops and returns the raw JSONL:
        //
        //   const backend = ctx.config.get<{ url: string }>().url;
        //   const r = ctx.net.fetch(`${backend}/since`, { timeoutMs: 10_000 });
        //   return r.ok ? r_body /* JSONL string */ : null;
        //
        // The host applies the returned JSONL through the CRDT itself — you
        // never parse or trust it into the tree yourself.
        return null;
      },
    };

    ctx.sync.register(transport);
    ctx.log.info("[echo-sync] transport registered (skeleton — no backend)");
  },
});

/** Count ops in a JSONL blob: one op per non-empty line. */
function countOps(jsonl: string): number {
  return jsonl.split("\n").filter((line) => line.trim().length > 0).length;
}
```

## Manifest

```json
{
  "capabilities": ["sync-transport"],
  "permissions": []
}
```

The capability `sync-transport` is what lets the plugin call `ctx.sync.register`.
Permissions are empty because the skeleton makes no network calls.
A real transport adds `network:<your-backend-domain>` here, and there are no `contributes` — a transport has no command, it just registers and lets the host drive it.

## Try it

```sh
outl -w <workspace> plugin install ./examples/echo-sync --yes
# edit a few blocks, then watch the plugin log:
#   [echo-sync] pushing N op(s)
#   [echo-sync] pull: no backend wired, nothing to apply
```

## Adapting it

To make it a real transport:

1. Add `network:<your-backend-domain>` to `permissions` in `plugin.json`.
2. Add a `config.schema.json` with a `url` field and read it via `ctx.config.get<{ url: string }>()`.
3. In `push`, `POST` `opsJsonl` to your backend with `ctx.net.fetch`.
4. In `pull`, `GET` the remote ops and return the raw JSONL string, or `null` when there is nothing new.

The host does the rest — it replays your bytes through the CRDT, so you never reason about merge or conflict yourself.
