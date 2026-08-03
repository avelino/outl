//! `remind::` — block-level reminder rules and their scheduling.
//!
//! Three pieces, deliberately separated by what they depend on:
//!
//! | Module | Depends on | Owns |
//! |---|---|---|
//! | [`schedule`] | nothing (pure) | *when* a rule fires next |
//! | [`scan`] | workspace + disk | *which* blocks have rules |
//! | this module | workspace + HLC | the snooze mutation, time conversion |
//!
//! **Every client wraps this crate; no client re-derives the math.**
//! The iOS `UNCalendarNotificationTrigger` registration, the macOS
//! launch agent, the Windows scheduled toast, the systemd timer and
//! the TUI overlay all call [`scan_reminders`] / [`next_fire_at`].
//! A second implementation of "when does this nag me" is exactly the
//! kind of drift that reaches the user before it reaches a test.
//!
//! ## What converges and what doesn't
//!
//! | State | Converges? | Where it lives |
//! |---|---|---|
//! | the `remind::` rule, the block's `[[date]]` | yes | block text / props → op log |
//! | TODO → DONE | yes | text prefix → `Op::Edit` |
//! | snooze | yes | [`Op::SnoozeRemind`] |
//! | "this device already fired it" | **no** | [`FiredLog`], client-owned cache |
//! | quiet hours, enabled flag | **no** | `outl_config::RemindersCfg` |
//!
//! Root `CLAUDE.md` invariant #7 is why the snooze is an op and the
//! fired log is not: snoozing on the phone must silence the laptop,
//! but the phone having buzzed must not stop the laptop from buzzing.

pub mod fired;
pub mod scan;
pub mod schedule;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

pub use fired::{fired_log_path, load_fired_log, save_fired_log, take_due, FIRED_TTL_DAYS};
pub use scan::{scan_reminders, FiredLog, FiredRecord, Reminder, Urgency};
pub use schedule::{next_fire_at, ReminderState};

use crate::clock;
use crate::error::ActionError;

/// Silence `node`'s reminder until `until_ms` (Unix epoch
/// milliseconds).
///
/// Goes through the op log so the snooze converges: tapping "Snooze
/// 1h" on the phone stops the same block nagging on the desktop.
/// Passing `None` clears the snooze — the "resume now" path, and what
/// a reschedule after a rule edit uses.
pub fn snooze(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    until_ms: Option<u64>,
) -> Result<(), ActionError> {
    let ts = hlc.next();
    workspace.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::SnoozeRemind {
            node,
            until_ms,
            old_until_ms: None,
        },
    })?;
    Ok(())
}

/// The snooze options every client offers, in the order they render.
///
/// **One owner, in Rust.** Two of the three aren't fixed offsets —
/// "tomorrow morning" is a wall time, not `now + 24h` — so a client
/// holding its own list of minute offsets gets them subtly wrong (the
/// first version of this shipped `+1440min`, which snoozes to 3am if
/// you tapped it at 3am). The GUIs read the labels off
/// [`SnoozePreset::all`] through a command and send back the `id`; the
/// TUI matches on the enum directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoozePreset {
    /// An hour from now.
    OneHour,
    /// 09:00 tomorrow, the "deal with it in the morning" option.
    TomorrowMorning,
    /// 09:00 seven days out.
    NextWeek,
}

impl SnoozePreset {
    /// Every preset, in render order.
    pub fn all() -> [SnoozePreset; 3] {
        [
            SnoozePreset::OneHour,
            SnoozePreset::TomorrowMorning,
            SnoozePreset::NextWeek,
        ]
    }

    /// Stable wire id. The GUIs round-trip this, so renaming one is a
    /// breaking change to the command surface, not a copy edit.
    pub fn id(self) -> &'static str {
        match self {
            SnoozePreset::OneHour => "1h",
            SnoozePreset::TomorrowMorning => "tomorrow",
            SnoozePreset::NextWeek => "next-week",
        }
    }

    /// Parse a wire id back. Unknown ids yield `None` rather than
    /// falling back to a default — silently snoozing for a different
    /// duration than the button said is worse than doing nothing.
    pub fn from_id(id: &str) -> Option<SnoozePreset> {
        SnoozePreset::all().into_iter().find(|p| p.id() == id)
    }

    /// Label as the user reads it on the button.
    pub fn label(self) -> &'static str {
        match self {
            SnoozePreset::OneHour => "1 hour",
            SnoozePreset::TomorrowMorning => "Tomorrow 9am",
            SnoozePreset::NextWeek => "Next week",
        }
    }

    /// Resolve to a wall-clock instant relative to `now`.
    ///
    /// The morning presets land on 09:00 rather than the current time
    /// of day, which is the whole point of picking them: a reminder
    /// snoozed at 23:40 should come back after breakfast, not at 23:40
    /// tomorrow.
    pub fn resolve(self, now: NaiveDateTime) -> NaiveDateTime {
        let morning = |days: i64| {
            (now.date() + chrono::Duration::days(days))
                .and_hms_opt(9, 0, 0)
                .unwrap_or(now)
        };
        match self {
            SnoozePreset::OneHour => now + chrono::Duration::hours(1),
            SnoozePreset::TomorrowMorning => morning(1),
            SnoozePreset::NextWeek => morning(7),
        }
    }
}

/// Snooze `node` until `at` in local wall clock — the shape every
/// client's "Snooze 1h / tomorrow 9am / next week" menu produces.
pub fn snooze_until(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    at: NaiveDateTime,
) -> Result<(), ActionError> {
    snooze(workspace, hlc, node, local_naive_to_epoch_ms(at))
}

/// Local wall clock → Unix epoch milliseconds, resolved in the
/// configured timezone ([`clock`]), not `chrono::Local`.
///
/// The offset is resolved **at `at`**, not at "now". Reusing the
/// current offset shifts a target across a DST boundary by the
/// transition: "snooze until next week" set the evening before a
/// spring-forward came back an hour late.
///
/// Returns `None` for a wall time that doesn't exist locally (the hour
/// a spring-forward skips). The caller treats that as "don't snooze to
/// a time that never happens" rather than guessing an hour. An
/// ambiguous time (the repeated hour in autumn) resolves to the first
/// occurrence, so the reminder comes back early rather than late.
pub fn local_naive_to_epoch_ms(at: NaiveDateTime) -> Option<u64> {
    let millis = match clock::timezone() {
        Some(tz) => tz.from_local_datetime(&at).earliest()?.timestamp_millis(),
        None => Local
            .from_local_datetime(&at)
            .earliest()?
            .timestamp_millis(),
    };
    u64::try_from(millis).ok()
}

/// Unix epoch milliseconds → local wall clock, in the configured
/// timezone. The inverse of [`local_naive_to_epoch_ms`], and likewise
/// resolved at the target instant so a stored snooze reads back as the
/// wall time it was set to even across a DST change.
pub fn epoch_ms_to_local_naive(ms: u64) -> Option<NaiveDateTime> {
    let millis = i64::try_from(ms).ok()?;
    let instant = Utc.timestamp_millis_opt(millis).single()?;
    Some(match clock::timezone() {
        Some(tz) => instant.with_timezone(&tz).naive_local(),
        None => instant.with_timezone(&Local).naive_local(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// The DST case the review caught: a snooze set before a
    /// transition must read back as the wall time it was set to, not
    /// shifted by the offset that happened to be current when it was
    /// stored.
    ///
    /// Runs against the process clock (OS local in CI, which usually
    /// has no transition), so it asserts the invariant that holds
    /// either way: round-tripping an instant on both sides of a
    /// March/November boundary is lossless.
    #[test]
    fn conversion_round_trips_across_a_dst_boundary() {
        for (m, d) in [(3, 8), (3, 10), (11, 1), (11, 3)] {
            let at = NaiveDate::from_ymd_opt(2026, m, d)
                .expect("valid date")
                .and_hms_opt(9, 0, 0)
                .expect("valid time");
            let Some(ms) = local_naive_to_epoch_ms(at) else {
                // A wall time that doesn't exist locally is a
                // legitimate `None`; nothing to round-trip.
                continue;
            };
            assert_eq!(
                epoch_ms_to_local_naive(ms),
                Some(at),
                "9am on {m}/{d} must read back as 9am"
            );
        }
    }

    #[test]
    fn epoch_conversion_round_trips() {
        let at = NaiveDate::from_ymd_opt(2026, 12, 12)
            .expect("valid date")
            .and_hms_opt(15, 30, 0)
            .expect("valid time");
        let ms = local_naive_to_epoch_ms(at).expect("representable");
        assert_eq!(epoch_ms_to_local_naive(ms), Some(at));
    }
}
