# Template Stamper

> **Capability:** `slash-command` · **Permissions:** `read-page`, `write-page`

A minimal plugin that registers a `/stamp` slash command to instantiate a structural template under the currently focused block.

## What it demonstrates

- `ctx.template.list()` — read available templates from the workspace.
- `ctx.template.instantiate(name, blockId)` — deep-copy a template's subtree under a target block.

## The code

```ts
import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("stamp", async () => {
      const templates = await ctx.template.list();
      if (templates.length === 0) {
        ctx.ui.notify("No templates found. Add `template:: name` to a page.");
        return;
      }

      // Pick the first template (a real plugin would show a picker).
      const tpl = templates[0];
      const blocks = await ctx.blocks.query({});
      const target = blocks[0];

      if (!target) {
        ctx.ui.notify("No block to stamp under.");
        return;
      }

      await ctx.template.instantiate(tpl.name, target.id);
      ctx.ui.notify(`Stamped template "${tpl.name}" under block ${target.id}.`);
    });
  },
});
```

## Manifest

```json
{
  "id": "template-stamper",
  "name": "Template Stamper",
  "version": "0.1.0",
  "permissions": ["read-page", "write-page"],
  "capabilities": ["slash-command"],
  "contributes": {
    "commands": [{ "id": "stamp", "title": "Stamp a template" }]
  }
}
```

## Try it

```sh
outl plugin install ./template-stamper
outl plugin enable template-stamper
# In the TUI, type: /stamp
```

## Adapting it

- **Show a picker:** iterate `ctx.template.list()` and let the user choose via `ctx.ui.render()` (desktop/mobile only).
- **Auto-stamp on page create:** subscribe to `ctx.ops.onOp`, watch for `Create` ops on pages matching a pattern, and call `ctx.template.instantiate()` automatically.
- **Callable templates:** combine with `ctx.blocks` to write a ` ```call:<name> ` block and trigger execution.

See [Templates](../docs/templates.md) for the full template engine guide.
