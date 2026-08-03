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

pub mod scan;
pub mod schedule;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

pub use scan::{scan_reminders, FiredLog, FiredRecord, Reminder};
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

/// Local wall clock → Unix epoch milliseconds, resolving the offset
/// through the configured timezone ([`clock`]), not `chrono::Local`.
///
/// Returns `None` for a wall time that doesn't exist locally (the
/// hour a DST spring-forward skips). The caller treats that as "don't
/// snooze to a time that never happens" rather than guessing an hour.
pub fn local_naive_to_epoch_ms(at: NaiveDateTime) -> Option<u64> {
    let offset = *clock::now_local().offset();
    let dt = offset.from_local_datetime(&at).single()?;
    u64::try_from(dt.timestamp_millis()).ok()
}

/// Unix epoch milliseconds → local wall clock, in the configured
/// timezone. The inverse of [`local_naive_to_epoch_ms`].
pub fn epoch_ms_to_local_naive(ms: u64) -> Option<NaiveDateTime> {
    let millis = i64::try_from(ms).ok()?;
    let utc: DateTime<Local> = Local.timestamp_millis_opt(millis).single()?;
    let offset = *clock::now_local().offset();
    Some(utc.with_timezone(&offset).naive_local())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

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
