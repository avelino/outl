import { describe, expect, it } from "vitest";

import {
  DAY_LABELS,
  DAY_LABELS_MONDAY_FIRST,
  MONTH_NAMES,
  daysInMonth,
  formatJournalSlug,
  journalSlugToDate,
  mondayIndex,
  nextMonth,
  parseJournalSlug,
  prevMonth,
} from "./index";

describe("parseJournalSlug", () => {
  it("parses a valid slug into 0-based monthIndex parts", () => {
    expect(parseJournalSlug("2026-07-04")).toEqual({
      year: 2026,
      monthIndex: 6,
      day: 4,
    });
  });

  it("rejects non-journal slugs and empty input", () => {
    expect(parseJournalSlug("ai-agent/learning")).toBeNull();
    expect(parseJournalSlug("2026-7-4")).toBeNull(); // unpadded
    expect(parseJournalSlug("2026-07-04-extra")).toBeNull();
    expect(parseJournalSlug("")).toBeNull();
    expect(parseJournalSlug(null)).toBeNull();
    expect(parseJournalSlug(undefined)).toBeNull();
  });

  it("roundtrips through formatJournalSlug", () => {
    for (const slug of ["2026-01-01", "2026-12-31", "1999-02-28"]) {
      const p = parseJournalSlug(slug);
      expect(p).not.toBeNull();
      if (!p) continue;
      expect(formatJournalSlug(p.year, p.monthIndex, p.day)).toBe(slug);
    }
  });
});

describe("formatJournalSlug", () => {
  it("zero-pads month and day", () => {
    expect(formatJournalSlug(2026, 0, 5)).toBe("2026-01-05");
    expect(formatJournalSlug(2026, 11, 31)).toBe("2026-12-31");
  });
});

describe("journalSlugToDate", () => {
  it("builds a local-time date (not midnight UTC)", () => {
    const d = journalSlugToDate("2026-06-02");
    expect(d).not.toBeNull();
    // Local getters must return the slug's own parts regardless of the
    // host timezone — the whole point of parsing parts explicitly.
    expect(d?.getFullYear()).toBe(2026);
    expect(d?.getMonth()).toBe(5);
    expect(d?.getDate()).toBe(2);
  });

  it("returns null for non-journal slugs", () => {
    expect(journalSlugToDate("not-a-date")).toBeNull();
    expect(journalSlugToDate(null)).toBeNull();
  });
});

describe("daysInMonth", () => {
  it("knows 31/30-day months", () => {
    expect(daysInMonth(2026, 0)).toBe(31); // January
    expect(daysInMonth(2026, 3)).toBe(30); // April
  });

  it("handles February: leap vs non-leap", () => {
    expect(daysInMonth(2026, 1)).toBe(28);
    expect(daysInMonth(2024, 1)).toBe(29); // divisible by 4
    expect(daysInMonth(2000, 1)).toBe(29); // divisible by 400
    expect(daysInMonth(1900, 1)).toBe(28); // divisible by 100, not 400
  });
});

describe("prevMonth / nextMonth", () => {
  it("steps within the same year", () => {
    expect(prevMonth(2026, 6)).toEqual({ year: 2026, monthIndex: 5 });
    expect(nextMonth(2026, 6)).toEqual({ year: 2026, monthIndex: 7 });
  });

  it("rolls the year at the boundaries", () => {
    expect(prevMonth(2026, 0)).toEqual({ year: 2025, monthIndex: 11 });
    expect(nextMonth(2026, 11)).toEqual({ year: 2027, monthIndex: 0 });
  });

  it("prev and next are inverses", () => {
    const start = { year: 2026, monthIndex: 0 };
    const forward = nextMonth(start.year, start.monthIndex);
    expect(prevMonth(forward.year, forward.monthIndex)).toEqual(start);
  });
});

describe("labels + mondayIndex", () => {
  it("exposes 12 month names and 7 weekday labels per convention", () => {
    expect(MONTH_NAMES).toHaveLength(12);
    expect(DAY_LABELS).toHaveLength(7);
    expect(DAY_LABELS_MONDAY_FIRST).toHaveLength(7);
  });

  it("mondayIndex remaps getDay() onto Monday-first", () => {
    expect(mondayIndex(1)).toBe(0); // Monday
    expect(mondayIndex(0)).toBe(6); // Sunday
    expect(mondayIndex(6)).toBe(5); // Saturday
  });
});
