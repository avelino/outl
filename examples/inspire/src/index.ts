/**
 * Inspire — example outl plugin (network).
 *
 * The smallest possible plugin that reaches the outside world: a single
 * `inspire` command fetches a random quote over HTTP and shows it as a
 * notification. It is the **canonical template for any external integration** —
 * swap the URL (and add an auth header from `ctx.config`) and you have a GPT
 * call, a webhook, a REST sync, whatever.
 *
 *   - `slash-command` — the `inspire` command (declared in plugin.json).
 *   - `network:api.quotable.io` permission — the host gates every request host
 *     against the approved domains. A URL outside them is refused with
 *     `{ ok: false }` (it does NOT throw), so we always check `r.ok`.
 *
 * `network` is a *permission*, not a capability — that's why `capabilities`
 * lists only `slash-command`.
 */

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
