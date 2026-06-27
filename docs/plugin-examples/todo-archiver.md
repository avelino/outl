# TODO Archiver

> **Capabilities:** `op-hook` + `slash-command` + `keybinding` + `config-schema` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/todo-archiver)

Moves every **DONE** block to a configurable archive page, keeping working pages focused on what's still open.

## What it demonstrates

The combo example — one plugin wiring four capabilities at once:

- **`op-hook`** — observes ops as they land (logs DONE transitions).
- **`slash-command`** — the `todo-archive-done` command does the archiving.
- **`keybinding`** — the same command bound to a chord (`Ctrl+Shift+A`, declared in `plugin.json`).
- **`config-schema`** — a user-editable `archivePage` setting, read with `ctx.config.get`.

It never touches the CRDT or `.md` directly: every mutation goes through the typed host context, which routes to the op log.

## The code

```ts
import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("todo-archive-done", () => {
      const archivePage = ctx.config.get<{ archivePage: string }>()?.archivePage ?? "archive";
      ctx.page.create(archivePage); // idempotent on the slug

      const done = ctx.blocks.query({ todo: "DONE" });
      const movable = done.filter((b) => b.page !== archivePage);
      for (const block of movable) {
        ctx.blocks.move(block.id, { toPage: archivePage });
      }
      ctx.ui.notify(`Archived ${movable.length} block(s) to "${archivePage}"`);
    });
  },
});
```

See [`src/index.ts`](https://github.com/avelino/outl/tree/main/examples/todo-archiver/src/index.ts) for the full version (with the op-hook and config helpers).

## Manifest

```jsonc
"capabilities": ["op-hook", "slash-command", "keybinding", "config-schema"],
"permissions":  ["read-page", "write-page", "submit-op", "storage:local"],
"contributes": {
  "commands":    [{ "id": "todo-archive-done", "title": "Archive DONE blocks" }],
  "keybindings": [{ "command": "todo-archive-done", "key": "Ctrl+Shift+A" }],
  "configSchema": "config.schema.json"
}
```

## Try it

```sh
outl -w <workspace> plugin install ./examples/todo-archiver --yes
outl -w <workspace> plugin run app.outl.examples.todo-archiver todo-archive-done
```

In the TUI, type `/` and pick the command; in the desktop palette (`⧉`) or with the chord; or set `config.archivePage` in `installed.json` to change the destination.
