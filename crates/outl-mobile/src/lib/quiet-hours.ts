/**
 * Split / join for the `[reminders] quiet_hours` string.
 *
 * The wire format is a single `"22:00-07:00"` (what
 * `outl_config::RemindersCfg::quiet_window` parses), but a phone has
 * no good way to type that: hyphen and colon each need a keyboard
 * layout switch. Mobile renders two native `<input type="time">`
 * pickers instead, so it needs to take the string apart and put it
 * back together.
 *
 * Lives in the mobile client, not `@outl/shared`, because the desktop
 * edits the same setting as a plain text field. If it ever moves to
 * pickers too, promote this then — shipping it shared today would be
 * speculative.
 */

/**
 * `"22:00-07:00"` -> `["22:00", "07:00"]`.
 *
 * Anything the backend wouldn't accept comes back as a pair of empty
 * strings, so the pickers render blank rather than showing half of a
 * malformed value as if it were configured.
 */
export function splitQuietHours(raw: string): [string, string] {
  const parts = raw.split("-");
  if (parts.length !== 2) return ["", ""];
  const from = parts[0].trim();
  const to = parts[1].trim();
  if (!isTime(from) || !isTime(to)) return ["", ""];
  return [from, to];
}

/**
 * Put one end back and rebuild the wire string.
 *
 * Returns `""` unless **both** ends are set: a half-filled window is
 * not a window, and persisting `"22:00-"` would only give the backend
 * something unparseable to drop on the next read. Clearing either
 * picker is therefore how you turn quiet hours off.
 */
export function withQuietEnd(
  raw: string,
  which: 0 | 1,
  value: string,
): string {
  const parts = splitQuietHours(raw);
  parts[which] = isTime(value) ? value : "";
  return parts[0] && parts[1] ? `${parts[0]}-${parts[1]}` : "";
}

/** `HH:MM`, 24-hour, as an `<input type="time">` reports it. */
function isTime(v: string): boolean {
  const m = /^(\d{2}):(\d{2})$/.exec(v);
  if (!m) return false;
  return Number(m[1]) < 24 && Number(m[2]) < 60;
}
