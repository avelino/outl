import type { PluginCommand } from "./api";

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
