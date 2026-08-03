//! Pure schedule math for `remind::`. **The single owner.**
//!
//! Every OS bridge — `UNUserNotificationCenter` on iOS/macOS, the
//! Windows `ScheduledToastNotification`, the systemd user timer on
//! Linux, the TUI's read-only overlay — asks *this* function when the
//! next fire is. None of them re-derive it. Two implementations of
//! "when does this nag me next" would drift, and the user would be the
//! one to find out, at 3am, on one device only.
//!
//! Everything here is pure: no clock, no workspace, no filesystem. The
//! caller passes `now`, so a test can sit at any instant and a bridge
//! can ask "what would fire at 07:00 tomorrow" without waiting.
//!
//! All times are **local wall clock** ([`NaiveDateTime`]). The user
//! wrote `3pm` meaning their 3pm; converting to UTC is the caller's
//! last step, after this function has decided.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use outl_md::remind::{RemindAnchor, RemindRule, RemindStop};

/// Everything the scheduler needs to know about one block's reminder
/// beyond the rule itself.
///
/// The split matters: `done` and `snoozed_until` come from state that
/// **converges** (block text and `Op::SnoozeRemind`), while
/// `fired_count` / `last_fired` come from the device-local fired cache.
/// Two devices can legitimately disagree about the last two — one was
/// asleep — and that is fine, because each schedules for itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReminderState {
    /// The block's TODO flipped to DONE — a hard stop for every rule,
    /// including one with an explicit `until 6pm`. Completing the task
    /// is the strongest possible "stop nagging me".
    pub done: bool,
    /// Fires already delivered **on this device**.
    pub fired_count: u32,
    /// When this device last delivered a fire for this block.
    pub last_fired: Option<NaiveDateTime>,
    /// Converged snooze instant (`Op::SnoozeRemind`), in local wall
    /// clock. Fires at-or-after this resume normally.
    pub snoozed_until: Option<NaiveDateTime>,
}

/// When this rule should next interrupt the user, or `None` if it
/// never should again.
///
/// `anchor_date` is the day the rule is anchored to — the block's
/// `[[YYYY-MM-DD]]` when it carries one, else today. A block with two
/// dates is scheduled twice, once per date; that decision belongs to
/// the scan, not here.
///
/// `quiet` is the device-local `(start, end)` window in minutes past
/// midnight (see `outl_config::RemindersCfg::quiet_window`). A fire
/// landing inside it is **pushed to the window's end**, never dropped —
/// a reminder the user asked for must not vanish because they were
/// asleep when it came due.
///
/// Returns `None` when the rule is finished: DONE, past its `until`,
/// out of `max` fires, or a single shot that already fired.
pub fn next_fire_at(
    rule: &RemindRule,
    anchor_date: NaiveDate,
    state: &ReminderState,
    quiet: Option<(u32, u32)>,
    now: NaiveDateTime,
) -> Option<NaiveDateTime> {
    if state.done {
        return None;
    }
    if let Some(max) = rule.max_fires {
        if state.fired_count >= max {
            return None;
        }
    }

    let candidate = match state.last_fired {
        // Nothing fired yet: aim at the anchor. An anchor already in
        // the past (the TODO was written after 3pm, or a peer's op
        // arrived late) fires immediately rather than waiting a day —
        // "overdue at creation" in the spec.
        None => {
            let first = first_fire(rule, anchor_date, now);
            first.max(now)
        }
        // Already fired: only a repeating rule has anything left.
        Some(last) => {
            let every = rule.every_minutes?;
            let next = last + Duration::minutes(i64::from(every));
            // A device that was off for six hours does not owe the
            // user six backlogged banners — it owes them one, now.
            next.max(now)
        }
    };

    let candidate = match state.snoozed_until {
        Some(until) if until > candidate => until,
        _ => candidate,
    };
    let candidate = push_out_of_quiet_hours(candidate, quiet);

    // The stop clause is checked *after* the snooze and quiet-hours
    // shifts: a fire pushed past `until 6pm` is genuinely over, and
    // re-firing it at 07:00 the next morning is not what was asked.
    match rule.stop() {
        RemindStop::Done => Some(candidate),
        RemindStop::Time { hour, minute } => {
            let stop = anchor_date.and_time(time_at(hour, minute));
            (candidate <= stop).then_some(candidate)
        }
        RemindStop::Date(d) => {
            let stop = d.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or(NaiveTime::MIN));
            (candidate <= stop).then_some(candidate)
        }
    }
}

/// The rule's very first fire, before any snooze / quiet-hours shift.
fn first_fire(rule: &RemindRule, anchor_date: NaiveDate, now: NaiveDateTime) -> NaiveDateTime {
    match rule.anchor {
        RemindAnchor::Now => now,
        RemindAnchor::At { hour, minute } => anchor_date.and_time(time_at(hour, minute)),
    }
}

/// Shift `at` out of the quiet window, to the window's end.
///
/// A window whose start is after its end (`22:00-07:00`) wraps
/// midnight; that is the common case, not the exception.
fn push_out_of_quiet_hours(at: NaiveDateTime, quiet: Option<(u32, u32)>) -> NaiveDateTime {
    let Some((start, end)) = quiet else {
        return at;
    };
    let minutes = at.time().hour() * 60 + at.time().minute();
    let end_time = time_at(end / 60, end % 60);

    if start < end {
        // Same-day window, e.g. 13:00-14:00.
        if minutes >= start && minutes < end {
            return at.date().and_time(end_time);
        }
        return at;
    }
    // Wrapping window, e.g. 22:00-07:00.
    if minutes >= start {
        // Late evening — resume on the following morning.
        return (at.date() + Duration::days(1)).and_time(end_time);
    }
    if minutes < end {
        // Small hours — resume later the same morning.
        return at.date().and_time(end_time);
    }
    at
}

/// `NaiveTime` from an already-validated `(hour, minute)` pair. The
/// parser guarantees the range, so the fallback is unreachable in
/// practice; it exists so this stays `unwrap`-free.
fn time_at(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(NaiveTime::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_md::remind::parse_remind;

    fn rule(s: &str) -> RemindRule {
        parse_remind(s).rule.expect("rule parses")
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        date(y, m, d).and_time(time_at(h, min))
    }

    const DAY: (i32, u32, u32) = (2026, 12, 12);

    fn anchor() -> NaiveDate {
        date(DAY.0, DAY.1, DAY.2)
    }

    #[test]
    fn first_fire_lands_on_the_anchor() {
        let now = at(2026, 12, 12, 9, 0);
        assert_eq!(
            next_fire_at(
                &rule("10am"),
                anchor(),
                &ReminderState::default(),
                None,
                now
            ),
            Some(at(2026, 12, 12, 10, 0))
        );
    }

    #[test]
    fn overdue_at_creation_fires_immediately() {
        // The user wrote the TODO at 15:30 with `remind:: 10am`. The
        // anchor is already gone; nagging tomorrow is not the intent.
        let now = at(2026, 12, 12, 15, 30);
        assert_eq!(
            next_fire_at(
                &rule("10am"),
                anchor(),
                &ReminderState::default(),
                None,
                now
            ),
            Some(now)
        );
    }

    #[test]
    fn single_shot_never_fires_twice() {
        let now = at(2026, 12, 12, 11, 0);
        let state = ReminderState {
            fired_count: 1,
            last_fired: Some(at(2026, 12, 12, 10, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am"), anchor(), &state, None, now),
            None
        );
    }

    #[test]
    fn repeat_schedules_one_interval_after_the_last_fire() {
        let now = at(2026, 12, 12, 10, 5);
        let state = ReminderState {
            fired_count: 1,
            last_fired: Some(at(2026, 12, 12, 10, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am every 1h"), anchor(), &state, None, now),
            Some(at(2026, 12, 12, 11, 0))
        );
    }

    #[test]
    fn a_device_that_slept_owes_one_fire_not_a_backlog() {
        // Laptop was closed from 10:00 to 18:00 on a `every 1h` rule.
        // Eight banners at once is punishment, not a reminder.
        let now = at(2026, 12, 12, 18, 0);
        let state = ReminderState {
            fired_count: 1,
            last_fired: Some(at(2026, 12, 12, 10, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am every 1h"), anchor(), &state, None, now),
            Some(now)
        );
    }

    #[test]
    fn done_stops_every_rule_including_an_explicit_until() {
        let now = at(2026, 12, 12, 9, 0);
        let state = ReminderState {
            done: true,
            ..Default::default()
        };
        for r in ["10am", "10am every 1h", "10am every 1h until 6pm"] {
            assert_eq!(next_fire_at(&rule(r), anchor(), &state, None, now), None);
        }
    }

    #[test]
    fn max_fires_caps_the_repeat() {
        let now = at(2026, 12, 12, 14, 0);
        let state = ReminderState {
            fired_count: 5,
            last_fired: Some(at(2026, 12, 12, 14, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am every 1h max 5"), anchor(), &state, None, now),
            None
        );
    }

    #[test]
    fn until_time_ends_the_repeat() {
        let state = ReminderState {
            fired_count: 8,
            last_fired: Some(at(2026, 12, 12, 17, 30)),
            ..Default::default()
        };
        let now = at(2026, 12, 12, 17, 35);
        // 17:30 + 1h = 18:30, past the 18:00 stop.
        assert_eq!(
            next_fire_at(
                &rule("10am every 1h until 6pm"),
                anchor(),
                &state,
                None,
                now
            ),
            None
        );
    }

    #[test]
    fn until_date_ends_the_repeat_at_end_of_day() {
        let r = rule("10am every 1d until 2026-12-13");
        let inside = ReminderState {
            fired_count: 1,
            last_fired: Some(at(2026, 12, 12, 10, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&r, anchor(), &inside, None, at(2026, 12, 12, 11, 0)),
            Some(at(2026, 12, 13, 10, 0))
        );
        let outside = ReminderState {
            fired_count: 2,
            last_fired: Some(at(2026, 12, 13, 10, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&r, anchor(), &outside, None, at(2026, 12, 13, 11, 0)),
            None
        );
    }

    #[test]
    fn snooze_pushes_the_next_fire_out() {
        let now = at(2026, 12, 12, 9, 0);
        let state = ReminderState {
            snoozed_until: Some(at(2026, 12, 12, 14, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am"), anchor(), &state, None, now),
            Some(at(2026, 12, 12, 14, 0))
        );
    }

    #[test]
    fn an_expired_snooze_does_not_delay_anything() {
        let now = at(2026, 12, 12, 9, 0);
        let state = ReminderState {
            snoozed_until: Some(at(2026, 12, 12, 8, 0)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(&rule("10am"), anchor(), &state, None, now),
            Some(at(2026, 12, 12, 10, 0))
        );
    }

    #[test]
    fn quiet_hours_push_a_late_night_fire_to_the_morning() {
        // 22:00-07:00. A 23:00 fire resumes at 07:00 the next day.
        let quiet = Some((22 * 60, 7 * 60));
        let now = at(2026, 12, 12, 20, 0);
        assert_eq!(
            next_fire_at(
                &rule("11pm"),
                anchor(),
                &ReminderState::default(),
                quiet,
                now
            ),
            Some(at(2026, 12, 13, 7, 0))
        );
    }

    #[test]
    fn quiet_hours_push_a_small_hours_fire_to_the_same_morning() {
        let quiet = Some((22 * 60, 7 * 60));
        let now = at(2026, 12, 12, 2, 0);
        assert_eq!(
            next_fire_at(
                &rule("3am"),
                anchor(),
                &ReminderState::default(),
                quiet,
                now
            ),
            Some(at(2026, 12, 12, 7, 0))
        );
    }

    #[test]
    fn quiet_hours_leave_a_daytime_fire_alone() {
        let quiet = Some((22 * 60, 7 * 60));
        let now = at(2026, 12, 12, 9, 0);
        assert_eq!(
            next_fire_at(
                &rule("10am"),
                anchor(),
                &ReminderState::default(),
                quiet,
                now
            ),
            Some(at(2026, 12, 12, 10, 0))
        );
    }

    #[test]
    fn a_same_day_quiet_window_does_not_wrap() {
        // 13:00-14:00 — lunch. A 13:30 fire moves to 14:00 today, not
        // tomorrow.
        let quiet = Some((13 * 60, 14 * 60));
        let now = at(2026, 12, 12, 12, 0);
        assert_eq!(
            next_fire_at(
                &rule("1:30pm"),
                anchor(),
                &ReminderState::default(),
                quiet,
                now
            ),
            Some(at(2026, 12, 12, 14, 0))
        );
    }

    #[test]
    fn now_anchor_fires_at_now() {
        let now = at(2026, 12, 12, 16, 42);
        assert_eq!(
            next_fire_at(
                &rule("now every 15min until DONE"),
                anchor(),
                &ReminderState::default(),
                None,
                now
            ),
            Some(now)
        );
    }

    #[test]
    fn a_fire_pushed_past_until_by_quiet_hours_is_over() {
        // `until 11pm` with a 22:00 quiet start: the 22:30 fire would
        // land at 07:00 tomorrow, well past the stop. Firing it then
        // is not what the user asked for.
        let quiet = Some((22 * 60, 7 * 60));
        let state = ReminderState {
            fired_count: 1,
            last_fired: Some(at(2026, 12, 12, 21, 30)),
            ..Default::default()
        };
        assert_eq!(
            next_fire_at(
                &rule("9pm every 1h until 11pm"),
                anchor(),
                &state,
                quiet,
                at(2026, 12, 12, 21, 35)
            ),
            None
        );
    }
}
