/**
 * Greeter — example outl plugin.
 *
 * Demonstrates the `config-schema` capability: it reads a user-editable `name`
 * setting (validated by the host against `config.schema.json`) and a `greet`
 * slash command toasts a friendly hello using it.
 *
 * No permissions — it neither reads pages nor the op log. `ctx.config.get()` is
 * ungated, so an empty `permissions` array is correct.
 */

import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

/** Shape of this plugin's config, mirrored by `config.schema.json`. */
interface GreeterConfig {
  name: string;
}

/** Fallback name when the user hasn't set one (matches the schema default). */
const DEFAULT_NAME = "friend";

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("greet", () => {
      // The host already validated this against config.schema.json, so the
      // value is safe to trust — we just guard the empty-string case.
      const cfg = ctx.config.get<Partial<GreeterConfig>>();
      const name = cfg?.name?.trim() || DEFAULT_NAME;

      ctx.ui.notify(`👋 Hello, ${name}! Your outline missed you.`);
    });
  },
});
