import type { PluginCommand, TemplateDto } from "@outl/shared/api/types";

/**
 * Reserved `plugin_id` for the native `/template <name>` slash entries.
 * Structural templates ship in the core (reachable via TUI/CLI/MCP), so
 * they must appear in the desktop slash menu WITHOUT a plugin. We reuse
 * the plugin-command popup by injecting synthetic {@link PluginCommand}
 * rows under this sentinel; `OutlineView`'s `onRunPluginCommand`
 * intercepts it and calls `instantiateTemplateAt` instead of `pluginRun`.
 * A real plugin can never claim this id (it isn't a loadable plugin
 * directory), so there's no collision.
 */
export const NATIVE_TEMPLATE_PLUGIN_ID = "@outl/template";

/**
 * Project the workspace's structural templates onto synthetic
 * {@link PluginCommand} rows for the slash menu. `command_id` carries the
 * template's invocation name (what `instantiateTemplateAt` needs);
 * `title` is a friendly label. Empty when the workspace has no template.
 */
export function templateSlashCommands(
  templates: TemplateDto[],
): PluginCommand[] {
  return templates.map((t) => ({
    plugin_id: NATIVE_TEMPLATE_PLUGIN_ID,
    command_id: t.name,
    title: `template: ${t.name}${t.duplicate ? " (duplicate name)" : ""}`,
  }));
}

/**
 * Rank plugin commands for the inline `/` slash menu against a query.
 *
 * Matching is keyed on the command **id** first (what the user types —
 * `/stats` — mirroring the TUI / CLI), then the human title. Ordering:
 *
 * 1. id starts with the query   (`/sta` → `stats`)
 * 2. id contains the query
 * 3. title contains the query
 *
 * An empty query returns every command (the bare `/` "show all" case).
 * Commands matching nothing are dropped. Ties keep input order (stable
 * sort), so two same-score commands stay in `pluginList()` order.
 */
export function rankSlashCommands(
  all: PluginCommand[],
  query: string,
): PluginCommand[] {
  const q = query.toLowerCase();
  return all
    .map((c, i) => {
      const id = c.command_id.toLowerCase();
      const title = c.title.toLowerCase();
      let score = -1;
      if (q === "") score = 0;
      else if (id.startsWith(q)) score = 3;
      else if (id.includes(q)) score = 2;
      else if (title.includes(q)) score = 1;
      return { c, score, i };
    })
    .filter((r) => r.score >= 0)
    .sort((a, b) => b.score - a.score || a.i - b.i)
    .map((r) => r.c);
}
