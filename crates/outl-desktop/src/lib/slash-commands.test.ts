import { describe, expect, it } from "vitest";

import { rankSlashCommands } from "./slash-commands";
import type { PluginCommand } from "./api";

const cmd = (command_id: string, title: string): PluginCommand => ({
  plugin_id: `app.outl.examples.${command_id}`,
  command_id,
  title,
});

const ALL: PluginCommand[] = [
  cmd("stats", "Workspace statistics"),
  cmd("greet", "Greet me"),
  cmd("pick", "Pick a random task"),
  cmd("todo-archive-done", "Archive DONE blocks"),
];

describe("rankSlashCommands", () => {
  it("ranks the id-prefix match on top — the `/sta` report", () => {
    const out = rankSlashCommands(ALL, "sta");
    expect(out[0].command_id).toBe("stats");
  });

  it("matches by full id and keeps it first", () => {
    const out = rankSlashCommands(ALL, "stats");
    expect(out[0].command_id).toBe("stats");
  });

  it("returns every command for an empty query (bare `/`)", () => {
    expect(rankSlashCommands(ALL, "")).toHaveLength(ALL.length);
  });

  it("falls back to a title match when the id does not match", () => {
    // No id contains "archive"; only the title "Archive DONE blocks".
    const out = rankSlashCommands(ALL, "archive");
    expect(out.map((c) => c.command_id)).toContain("todo-archive-done");
  });

  it("drops commands that match neither id nor title", () => {
    expect(rankSlashCommands(ALL, "zzzz")).toEqual([]);
  });

  it("ranks id-substring above title-substring", () => {
    // Query "e": id "greet" (substring) must outrank a title-only hit.
    const out = rankSlashCommands(ALL, "gree");
    expect(out[0].command_id).toBe("greet");
  });
});
