/**
 * Journal slug + calendar math shared by every client.
 *
 * The calendar *chrome* stays per-client (mobile: bottom-sheet
 * `Calendar.tsx`; desktop: sidebar mini-grid) — only the pure
 * parsing / date arithmetic lives here, so the two grids can't
 * disagree on what `YYYY-MM-DD` means.
 *
 * Convention: `monthIndex` is **0-based** everywhere in this module
 * (JS `Date` convention). The slug's month is 1-based on the wire;
 * the conversion happens inside parse/format so callers never do
 * the `±1` themselves.
 */

/** Parsed journal slug. `monthIndex` is 0-based (JS `Date` convention). */
export interface JournalDateParts {
  year: number;
  monthIndex: number;
  day: number;
}

const SLUG_RE = /^(\d{4})-(\d{2})-(\d{2})$/;

/**
 * Parse a journal slug (`YYYY-MM-DD`) into its parts, or `null` for
 * anything that doesn't match the shape (page slugs, `null`, …).
 * Purely syntactic — `2026-13-99` parses; use [`journalSlugToDate`]
 * when you need a real calendar date.
 */
export function parseJournalSlug(
  slug: string | null | undefined,
): JournalDateParts | null {
  if (!slug) return null;
  const m = slug.match(SLUG_RE);
  if (!m) return null;
  return {
    year: Number(m[1]),
    monthIndex: Number(m[2]) - 1,
    day: Number(m[3]),
  };
}

/** Render a journal slug (`YYYY-MM-DD`). `monthIndex` is 0-based. */
export function formatJournalSlug(
  year: number,
  monthIndex: number,
  day: number,
): string {
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${year}-${pad(monthIndex + 1)}-${pad(day)}`;
}

/**
 * Parse a journal slug into a **local-time** `Date` (midnight).
 *
 * Parses the parts explicitly instead of `new Date("2026-06-02")`
 * because the string form is interpreted as midnight **UTC**, which
 * renders the previous day in negative-offset timezones.
 */
export function journalSlugToDate(slug: string | null | undefined): Date | null {
  const parts = parseJournalSlug(slug);
  if (!parts) return null;
  return new Date(parts.year, parts.monthIndex, parts.day);
}

/** Number of days in a month. `monthIndex` is 0-based. */
export function daysInMonth(year: number, monthIndex: number): number {
  return new Date(year, monthIndex + 1, 0).getDate();
}

/** English month names, indexable by `monthIndex`. */
export const MONTH_NAMES = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
] as const;

/**
 * Sunday-first single-letter weekday labels (iOS Calendar style).
 * Pairs with `Date#getDay()` used directly as the leading-pad count.
 * Used by the mobile bottom-sheet calendar.
 */
export const DAY_LABELS = ["S", "M", "T", "W", "T", "F", "S"] as const;

/**
 * Monday-first two-letter weekday labels — matches the TUI's
 * `Mo Tu We Th Fr Sa Su` header. Pairs with [`mondayIndex`].
 * Used by the desktop sidebar calendar.
 */
export const DAY_LABELS_MONDAY_FIRST = [
  "Mo",
  "Tu",
  "We",
  "Th",
  "Fr",
  "Sa",
  "Su",
] as const;

/**
 * Remap a JS `Date#getDay()` (0 = Sunday) onto a Monday-first index
 * (0 = Monday … 6 = Sunday) for grids using [`DAY_LABELS_MONDAY_FIRST`].
 */
export function mondayIndex(jsDay: number): number {
  return (jsDay + 6) % 7;
}

/** Year/month pair for calendar navigation. `monthIndex` is 0-based. */
export interface YearMonth {
  year: number;
  monthIndex: number;
}

/** The month before `(year, monthIndex)`, rolling the year at January. */
export function prevMonth(year: number, monthIndex: number): YearMonth {
  if (monthIndex === 0) return { year: year - 1, monthIndex: 11 };
  return { year, monthIndex: monthIndex - 1 };
}

/** The month after `(year, monthIndex)`, rolling the year at December. */
export function nextMonth(year: number, monthIndex: number): YearMonth {
  if (monthIndex === 11) return { year: year + 1, monthIndex: 0 };
  return { year, monthIndex: monthIndex + 1 };
}
