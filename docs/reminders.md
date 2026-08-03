# Reminders (`remind::`)

A `[[2026-12-12]]` in a block gets you a backlink on that day's journal.
That's great for **recall** and useless for **interruption** — you still have to open the app on the right day.

`remind::` is the opt-in that turns a block into something the OS will tell you about.

```markdown
- TODO #fup [[@joão]] about project abc [[2026-12-12]]
  remind:: 3pm every 1h until DONE
```

Reads as English: *remind me at 3pm, every hour, until it's done.*

## Explicit opt-in, always

A `[[date]]` alone **never** schedules a notification.
Plenty of people use `[[date]]` purely for backlinking, and notifications are noisy — the moment a link becomes a buzz, the linking stops.
No `remind::`, no interruption.

| block has a date | has `remind::` | what happens |
|---|---|---|
| `[[2026-12-12]]` | no | nothing — backlink only |
| `[[2026-12-12]]` | `remind:: 10am` | one fire on 2026-12-12 at 10:00 |
| `[[2026-12-12]]` | `remind:: 10am every 1h` | repeats until DONE |
| no date | `remind:: 10am` | fires **today** at 10:00 |
| no date | no | nothing |

## Syntax

```ebnf
remind     ::= TIME ("every" INTERVAL)? ("until" STOP)? ("max" N)?

TIME       ::= "now" | "10am" | "3pm" | "15:00" | "1:30pm"
INTERVAL   ::= N ("min" | "h" | "d")          # 30min, 1h, 2d
STOP       ::= "DONE" | TIME | ISO_DATE       # until DONE, until 6pm, until 2026-12-20
N          ::= 1..999
```

Case-insensitive — `3PM EVERY 1H UNTIL DONE` parses the same as the lowercase form.

| written | means |
|---|---|
| `remind:: 10am` | one fire at 10:00 |
| `remind:: 10am every 1h` | from 10:00, hourly, until DONE |
| `remind:: 10am every 1h until 6pm` | stops at 18:00 on the anchor day |
| `remind:: 10am every 1h max 5` | at most 5 fires |
| `remind:: 3pm every 30min until DONE` | the typical "nag me" |
| `remind:: now every 15min until DONE` | start immediately, loop |

**`until DONE` is the implicit default.**
Writing no `until` clause means the same thing.

**A 24-hour time needs the colon.**
`15:00` works; a bare `15` is too ambiguous to guess between "3pm" and "the 15th", so it's rejected.

### Caps

| cap | value | what happens past it |
|---|---|---|
| `every` floor | 1 minute | rejected (`every 30s` is never what you meant, and silently rewriting it would hide the typo) |
| `max` ceiling | 10 fires | clamped down, with a warning |
| `until TIME` | must be after the anchor | the clause is dropped, the rest of the rule still schedules |

### When a rule doesn't parse

Nothing is lost.
The property stays on disk verbatim, the block is untouched, and the rule simply doesn't schedule — the parse banner shows which line to fix.
This is the same permissive recovery the rest of the outl dialect uses (see [Markdown dialect](markdown-format.md)).

The warnings you can see: `remind_missing_anchor`, `remind_invalid_time`, `remind_invalid_interval`, `remind_invalid_stop`, `remind_max_clamped`.
`outl doctor` lists them per file.

## What fires, and when

1. The first fire lands on the anchor — the rule's `TIME` on the block's `[[date]]`, or today when it carries none.
2. With `every`, the next fire is one interval after the last one.
3. A rule whose anchor is **already past** when the block is written fires immediately, then follows `every`.

Two behaviours worth knowing:

- **A device that was asleep owes you one banner, not a backlog.**
  Close the laptop at 10:00 on an `every 1h` rule and open it at 18:00: you get one reminder, not eight.
- **Two dates in one block schedule twice.**
  `[[2026-12-12]] and [[2026-12-15]]` fires on both — you wrote both dates on purpose.

### What cancels a reminder

| you do | effect |
|---|---|
| flip `TODO` → `DONE` | every pending fire is cancelled, including on a rule with an explicit `until 6pm` |
| delete the block | cancelled |
| edit the `remind::` value | rescheduled from scratch |
| edit the block's `[[date]]` | rescheduled |

### Snooze

Snoozing writes an `Op::SnoozeRemind` into the op log, so **it converges**: silencing a nag on your phone silences the same block on your laptop.
Presets are 1 hour, tomorrow, and next week; the desktop panel and the mobile sheet also offer "Resume" to clear it early.

### Quiet hours

Device-local, off by default:

```toml
[reminders]
enabled = true
quiet_hours = "22:00-07:00"
```

A fire landing inside the window is **pushed to the window's end**, never dropped — you asked for it, you get it, just not at 3am.
A window that wraps midnight is the normal case and is handled; so is a same-day window like `13:00-14:00`.

One exception: a fire pushed past its own `until` is genuinely over.
`remind:: 9pm every 1h until 11pm` with quiet hours starting at 22:00 stops at 21:00 — waking you at 07:00 for an 11pm deadline is not what the rule said.

`enabled = false` (the default) means this device delivers nothing.
The rules still parse and still show up in the reminders list; they just don't interrupt.
Turning it on is what triggers the OS notification-permission prompt, so it has to be an explicit act.

## Where you see them

| client | list | author | delivery |
|---|---|---|---|
| **TUI** | `g n` overlay | `g r`, `g R` | **none** — a terminal session has no background presence |
| **Desktop** | `Cmd/Ctrl+Shift+R` panel | `Cmd+R`, `g r` / `g R` in Normal | OS notification while the app runs |
| **Mobile** | bell icon in the header | long-press a block → *Remind me…* | iOS notification while the app runs |

Chords are in the shared catalog, so they can't drift — see [Shortcuts](shortcuts.md).

> **Why `g n` and not `Ctrl+R` in the TUI?**
> `Ctrl+R` is already Redo, and a terminal can't distinguish `Ctrl+R` from `Ctrl+Shift+R`.
> The `g` family (`g j`, `g x`, `g d`) is the honest home for it there; the desktop still takes `Cmd/Ctrl+Shift+R`.

## Background delivery — what ships today

**Today: reminders fire whenever the app is running**, foreground or backgrounded, on macOS / Linux / Windows / iOS.
The app polls every 30 seconds; the backend keeps a device-local "already fired" log (`<root>/.outl/reminders-fired.json`, 7-day TTL) so polling twice never double-buzzes and losing the file costs you at most one duplicate.

**Not yet: delivery with the app fully closed.**
That needs per-OS scheduling registered ahead of time, and each platform wants something different:

- **iOS** — `UNCalendarNotificationTrigger` requests registered in advance (the system caps pending requests at 64), re-filled from a `BGAppRefreshTask`.
- **macOS** — a small launch agent on a `StartCalendarInterval`, rather than keeping the app resident in the tray.
- **Windows** — `ScheduledToastNotification`, or a Task Scheduler helper.
- **Linux** — a systemd user timer firing a helper binary.

All four are tracked as follow-ups to [issue #63](https://github.com/avelino/outl/issues/63).
Until they land, a reminder for a day you never open outl will not reach you — worth knowing before you rely on it for something that matters.

## What converges and what doesn't

This is the [invariant #7](../CLAUDE.md) line, drawn explicitly:

| state | converges? | lives in |
|---|---|---|
| the `remind::` rule, the block's `[[date]]` | ✅ | block text + properties → op log |
| `TODO` / `DONE` | ✅ | text prefix → op log |
| snooze | ✅ | `Op::SnoozeRemind` |
| "this device already fired it" | ❌ | `<root>/.outl/reminders-fired.json`, local, 7-day TTL |
| quiet hours, the enabled flag | ❌ | `~/.config/outl/config.toml`, per device |

The split is the whole point: snoozing on one device must silence every device, but one device having buzzed must not stop another from buzzing.

## For contributors

`outl_actions::reminders::next_fire_at` is the **single owner** of the schedule math — pure, clock-free, takes `now` as a parameter.
Every surface (the TUI overlay, the desktop panel, the mobile sheet, each OS bridge) calls it.
A second opinion in TypeScript or Swift about when a reminder fires is exactly the drift that reaches the user before it reaches a test.

The pieces:

| crate | owns |
|---|---|
| `outl-md` | `remind::` syntax → `RemindRule`, plus the `ParseWarningKind` variants |
| `outl-core` | `Op::SnoozeRemind` and the tree's snooze table |
| `outl-actions` | `next_fire_at` (pure) + `scan_reminders` (workspace + disk) + `snooze` |
| `outl-config` | `[reminders]`, device-local |
| `outl-tauri-shared` | the DTOs, the commands, and the fired-log runtime both GUI clients share |
