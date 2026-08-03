/**
 * `formatNextFire` / `groupReminders` — the two pure pieces of the
 * reminders UI both GUI clients render.
 *
 * They live in `@outl/shared` precisely because "in 3h" and "This
 * week" drift on the edge cases (exactly 60 minutes, midnight
 * rollover, a finished rule) long before anyone notices two clients
 * disagreeing.
 */

import { describe, expect, it } from "vitest";

import { formatNextFire, groupReminders } from "./commands";
import type { Reminder } from "./types";

const NOW = new Date("2026-12-12T09:00:00");

function reminder(nextFire: string | null, text = "task"): Reminder {
  return {
    block_id: `blk-${text}`,
    page_slug: "tasks",
    page_title: "tasks",
    text,
    rule: "10am",
    anchor_date: "2026-12-12",
    done: nextFire === null,
    next_fire: nextFire,
    snoozed_until: null,
  };
}

describe("formatNextFire", () => {
  it("shows an em dash for a rule that will never fire again", () => {
    expect(formatNextFire(null, NOW)).toBe("—");
  });

  it("shows an em dash rather than 'Invalid Date' for garbage", () => {
    expect(formatNextFire("not-a-date", NOW)).toBe("—");
  });

  it("collapses anything already due to 'now'", () => {
    expect(formatNextFire("2026-12-12T08:00:00", NOW)).toBe("now");
    expect(formatNextFire("2026-12-12T09:00:00", NOW)).toBe("now");
  });

  it("counts minutes under the hour and hours above it", () => {
    expect(formatNextFire("2026-12-12T09:20:00", NOW)).toBe("in 20min");
    expect(formatNextFire("2026-12-12T12:00:00", NOW)).toBe("in 3h");
  });

  it("switches to hours exactly at the 60-minute boundary", () => {
    expect(formatNextFire("2026-12-12T09:59:00", NOW)).toBe("in 59min");
    expect(formatNextFire("2026-12-12T10:00:00", NOW)).toBe("in 1h");
  });

  it("names tomorrow instead of counting hours across midnight", () => {
    expect(formatNextFire("2026-12-13T09:00:00", NOW)).toBe("tomorrow 09:00");
  });

  it("falls back to a date for anything further out", () => {
    expect(formatNextFire("2026-12-20T10:00:00", NOW)).toContain("10:00");
  });
});

describe("groupReminders", () => {
  it("buckets by day distance and puts finished rules last", () => {
    const groups = groupReminders(
      [
        reminder("2026-12-20T10:00:00", "later"),
        reminder(null, "finished"),
        reminder("2026-12-12T10:00:00", "today"),
        reminder("2026-12-14T10:00:00", "this week"),
        reminder("2026-12-13T10:00:00", "tomorrow"),
      ],
      NOW,
    );
    expect(groups.map((g) => g.label)).toEqual([
      "Today",
      "Tomorrow",
      "This week",
      "Later",
      "Done",
    ]);
    expect(groups[0].items.map((r) => r.text)).toEqual(["today"]);
    expect(groups[4].items.map((r) => r.text)).toEqual(["finished"]);
  });

  it("drops empty buckets so a client can map straight over it", () => {
    const groups = groupReminders([reminder("2026-12-12T10:00:00")], NOW);
    expect(groups).toHaveLength(1);
    expect(groups[0].label).toBe("Today");
  });

  it("counts an overdue fire as today, not as a past bucket", () => {
    // The Rust scheduler clamps an overdue fire to "now", but a list
    // rendered a few seconds later can still see a past instant.
    const groups = groupReminders([reminder("2026-12-11T10:00:00")], NOW);
    expect(groups[0].label).toBe("Today");
  });

  it("returns nothing for an empty list", () => {
    expect(groupReminders([], NOW)).toEqual([]);
  });
});
