# Greeter

> **Capability:** `config-schema` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/greeter)

A `greet` slash command that toasts a friendly hello using a name you set in the
plugin's config.

## What it demonstrates

The `config-schema` capability — a user-editable setting validated by the host.
`config.schema.json` declares a single `name` string with a default of `friend`,
and `plugin.json` points to it via `contributes.configSchema`.
The `greet` command (a `slash-command` so there's something to trigger the read)
calls `ctx.config.get<T>()` to fetch the validated config and toasts the
greeting.
`ctx.config.get()` is ungated, so the plugin requests **no** permissions.

## The code

```ts
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
```

## Manifest

- **`capabilities`:** `["config-schema", "slash-command"]`
- **`permissions`:** `[]` — `ctx.config.get()` and `ctx.commands.register()` are ungated.
- **`contributes`:** `commands: [{ id: "greet", title: "Greet me" }]` plus `configSchema: "config.schema.json"`.

## Try it

```sh
outl -w <workspace> plugin install ./examples/greeter --yes
# Set your name in the plugin's config (default is "friend"), then open the
# slash menu and run "Greet me" (the `greet` command):
# 👋 Hello, Avelino! Your outline missed you.
```
