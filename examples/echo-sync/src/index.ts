/**
 * Echo Sync — example outl plugin (sync-transport).
 *
 * An **educational skeleton**, not a working sync. It exists to show the exact
 * shape a sync transport plugs into: `ctx.sync.register({ push, pull })`. The
 * host drives the cadence — it calls `push` with the JSONL of locally-authored
 * ops after edits, and calls `pull` on a timer expecting JSONL of remote ops
 * back (or `null`).
 *
 * A plugin only **transports bytes**. It never touches the CRDT or the tree:
 * the host applies whatever JSONL `pull` returns through the CRDT itself, with
 * HLC ordering, so two devices converge deterministically. That's invariant #7
 * in CLAUDE.md ("any state that must converge goes through the op log") made
 * pluggable.
 *
 * This skeleton has no backend: `push` just logs how many ops it *would* ship,
 * and `pull` returns `null` (nothing to apply). The comments mark exactly where
 * a real transport wires `ctx.net` + a configured backend URL.
 *
 *   - `sync-transport` capability — required to call `ctx.sync.register`.
 *   - no permissions — the skeleton makes no network calls. A real transport
 *     would add `network:<your-backend-domain>`.
 */

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
