//! Date / time / week-tag insert commands.
//!
//! Sixteen single-purpose commands that all share the same shape: in
//! Insert mode they paste a formatted timestamp at the cursor; in
//! Normal mode they refuse politely with a status hint. Grouped here
//! because they share helpers (`insert_or_warn`, `journal_link_offset`,
//! `time_text`, …) and the seven `date-next-<weekday>` siblings come
//! from a single macro.

use anyhow::Result;
use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use outl_actions::clock;

use super::super::SlashCommand;
use crate::state::{App, Mode};

// ─── helpers ──────────────────────────────────────────────────────────

/// Insert `text` at the cursor if we're in Insert mode; otherwise
/// surface a status message — date commands only make sense while
/// the user is typing.
fn insert_or_warn(app: &mut App, command_name: &str, text: &str) {
    if let Mode::Insert { buffer, .. } = &mut app.mode {
        buffer.insert_str(text);
    } else {
        app.status = format!("/{command_name} only works in Insert mode (would insert {text})");
    }
}

/// `[[YYYY-MM-DD]]` for today + `days_offset`. Negative offsets are
/// past, positive are future. Extracted so the date-shifting logic is
/// unit-testable without standing up a full `App`.
fn journal_link_offset(days_offset: i64) -> String {
    let d = clock::today() + Duration::days(days_offset);
    format!("[[{}]]", d.format("%Y-%m-%d"))
}

/// `HH:MM` for the current local time. Plain text — not a journal ref.
fn time_text() -> String {
    clock::now_local().format("%H:%M").to_string()
}

/// `[[YYYY-MM-DD]] HH:MM` — the "stamp this moment" combo.
fn datetime_text() -> String {
    let now = clock::now_local();
    format!(
        "[[{}]] {}",
        now.date_naive().format("%Y-%m-%d"),
        now.format("%H:%M")
    )
}

/// Plain `YYYY-MM-DD` (no brackets) for property values like
/// `due:: 2026-05-26`. ISO 8601 short date.
fn iso_date_offset(days_offset: i64) -> String {
    let d = clock::today() + Duration::days(days_offset);
    d.format("%Y-%m-%d").to_string()
}

/// `#YYYY-Www` tag for the ISO week of `date` (e.g. `#2026-W21`).
/// Uses ISO 8601 week numbering — weeks start on Monday and `%G`
/// is the ISO week-numbering year (which can differ from `%Y` for
/// a handful of days near year boundaries).
fn week_tag(date: NaiveDate) -> String {
    format!("#{}", date.format("%G-W%V"))
}

/// Number of days from `today` to the next occurrence of `weekday`,
/// strictly in the future. If today already is that weekday, returns
/// 7 (next week's same day), not 0 — `/date-next-monday` on a Monday
/// should mean "next Monday", not "today".
fn days_until_next_weekday(today: NaiveDate, weekday: Weekday) -> i64 {
    let today_n = today.weekday().num_days_from_monday() as i64;
    let target_n = weekday.num_days_from_monday() as i64;
    let diff = (target_n - today_n).rem_euclid(7);
    if diff == 0 {
        7
    } else {
        diff
    }
}

/// Parse a date argument: ISO absolute (`2026-06-15`) or relative
/// (`+3d`, `-2w`, `+1m`). Returns `None` on unrecognized input so
/// the command can surface a usage hint.
fn parse_date_arg(arg: &str, today: NaiveDate) -> Option<NaiveDate> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    // Absolute ISO date wins first — `2026-06-15` is unambiguous.
    if let Ok(d) = NaiveDate::parse_from_str(arg, "%Y-%m-%d") {
        return Some(d);
    }
    // Relative: `+Nd`, `-Nw`, `+Nm` (also bare `Nd` / `Nw` / `Nm`,
    // treated as positive — nice when the user forgets the sign).
    let (sign, rest) = if let Some(rest) = arg.strip_prefix('+') {
        (1i64, rest)
    } else if let Some(rest) = arg.strip_prefix('-') {
        (-1i64, rest)
    } else {
        (1i64, arg)
    };
    let last = rest.chars().last()?;
    if !"dwm".contains(last) {
        return None;
    }
    let num_str = &rest[..rest.len() - last.len_utf8()];
    let n: i64 = num_str.parse().ok()?;
    let signed = sign * n;
    match last {
        'd' => Some(today + Duration::days(signed)),
        'w' => Some(today + Duration::weeks(signed)),
        'm' => {
            // Months need calendar-aware arithmetic — `chrono::Months`
            // clamps on overflow (Jan 31 + 1 month → Feb 28/29).
            let months = u32::try_from(signed.unsigned_abs()).ok()?;
            if signed >= 0 {
                today.checked_add_months(Months::new(months))
            } else {
                today.checked_sub_months(Months::new(months))
            }
        }
        _ => None,
    }
}

// ─── date-today / tomorrow / yesterday ────────────────────────────────

pub struct DateTodayCommand;
impl SlashCommand for DateTodayCommand {
    fn name(&self) -> &'static str {
        "date-today"
    }
    fn description(&self) -> &'static str {
        "Insert today's journal ref — `[[YYYY-MM-DD]]`"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dt"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "date-today", &journal_link_offset(0));
        Ok(false)
    }
}

pub struct DateTomorrowCommand;
impl SlashCommand for DateTomorrowCommand {
    fn name(&self) -> &'static str {
        "date-tomorrow"
    }
    fn description(&self) -> &'static str {
        "Insert tomorrow's journal ref — `[[YYYY-MM-DD]]`"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dtm"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "date-tomorrow", &journal_link_offset(1));
        Ok(false)
    }
}

pub struct DateYesterdayCommand;
impl SlashCommand for DateYesterdayCommand {
    fn name(&self) -> &'static str {
        "date-yesterday"
    }
    fn description(&self) -> &'static str {
        "Insert yesterday's journal ref — `[[YYYY-MM-DD]]`"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dy"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "date-yesterday", &journal_link_offset(-1));
        Ok(false)
    }
}

// ─── time-now / datetime-now ──────────────────────────────────────────

pub struct TimeNowCommand;
impl SlashCommand for TimeNowCommand {
    fn name(&self) -> &'static str {
        "time-now"
    }
    fn description(&self) -> &'static str {
        "Insert the current local time — `HH:MM` (no brackets)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["now", "tn"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "time-now", &time_text());
        Ok(false)
    }
}

pub struct DateTimeNowCommand;
impl SlashCommand for DateTimeNowCommand {
    fn name(&self) -> &'static str {
        "datetime-now"
    }
    fn description(&self) -> &'static str {
        "Insert journal ref + time — `[[YYYY-MM-DD]] HH:MM`"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dtn", "stamp"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "datetime-now", &datetime_text());
        Ok(false)
    }
}

// ─── date-next-week / date-last-week ──────────────────────────────────

pub struct DateNextWeekCommand;
impl SlashCommand for DateNextWeekCommand {
    fn name(&self) -> &'static str {
        "date-next-week"
    }
    fn description(&self) -> &'static str {
        "Insert next week's journal ref (today + 7 days)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dnw"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "date-next-week", &journal_link_offset(7));
        Ok(false)
    }
}

pub struct DateLastWeekCommand;
impl SlashCommand for DateLastWeekCommand {
    fn name(&self) -> &'static str {
        "date-last-week"
    }
    fn description(&self) -> &'static str {
        "Insert last week's journal ref (today − 7 days)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["dlw"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "date-last-week", &journal_link_offset(-7));
        Ok(false)
    }
}

// ─── date-next-<weekday> ──────────────────────────────────────────────
// Seven sibling commands share one body via a macro.
macro_rules! next_weekday_command {
    ($struct_name:ident, $name:literal, $weekday:expr, $label:literal, $aliases:expr) => {
        pub struct $struct_name;
        impl SlashCommand for $struct_name {
            fn name(&self) -> &'static str {
                $name
            }
            fn description(&self) -> &'static str {
                concat!("Insert next ", $label, "'s journal ref — `[[YYYY-MM-DD]]`")
            }
            fn aliases(&self) -> &'static [&'static str] {
                $aliases
            }
            fn inserts_inline(&self) -> bool {
                true
            }
            fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
                let today = clock::today();
                let days = days_until_next_weekday(today, $weekday);
                insert_or_warn(app, $name, &journal_link_offset(days));
                Ok(false)
            }
        }
    };
}

next_weekday_command!(
    DateNextMondayCommand,
    "date-next-monday",
    Weekday::Mon,
    "Monday",
    &["dnmon"]
);
next_weekday_command!(
    DateNextTuesdayCommand,
    "date-next-tuesday",
    Weekday::Tue,
    "Tuesday",
    &["dntue"]
);
next_weekday_command!(
    DateNextWednesdayCommand,
    "date-next-wednesday",
    Weekday::Wed,
    "Wednesday",
    &["dnwed"]
);
next_weekday_command!(
    DateNextThursdayCommand,
    "date-next-thursday",
    Weekday::Thu,
    "Thursday",
    &["dnthu"]
);
next_weekday_command!(
    DateNextFridayCommand,
    "date-next-friday",
    Weekday::Fri,
    "Friday",
    &["dnfri"]
);
next_weekday_command!(
    DateNextSaturdayCommand,
    "date-next-saturday",
    Weekday::Sat,
    "Saturday",
    &["dnsat"]
);
next_weekday_command!(
    DateNextSundayCommand,
    "date-next-sunday",
    Weekday::Sun,
    "Sunday",
    &["dnsun"]
);

// ─── date (flexible offset / absolute) ────────────────────────────────

pub struct DateCommand;
impl SlashCommand for DateCommand {
    fn name(&self) -> &'static str {
        "date"
    }
    fn description(&self) -> &'static str {
        "Insert a journal ref by offset or ISO date — `date +3d` · `date -2w` · `date +1m` · `date 2026-06-15`"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        let today = clock::today();
        match parse_date_arg(args, today) {
            Some(d) => {
                let s = format!("[[{}]]", d.format("%Y-%m-%d"));
                insert_or_warn(app, "date", &s);
            }
            None => {
                app.status = "usage: date +Nd | -Nw | +Nm | YYYY-MM-DD".into();
            }
        }
        Ok(false)
    }
}

// ─── iso-date-{today,tomorrow,yesterday} ──────────────────────────────

pub struct IsoDateTodayCommand;
impl SlashCommand for IsoDateTodayCommand {
    fn name(&self) -> &'static str {
        "iso-date-today"
    }
    fn description(&self) -> &'static str {
        "Insert today's date as plain ISO — `YYYY-MM-DD` (no brackets, for `due::` etc)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["isod"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "iso-date-today", &iso_date_offset(0));
        Ok(false)
    }
}

pub struct IsoDateTomorrowCommand;
impl SlashCommand for IsoDateTomorrowCommand {
    fn name(&self) -> &'static str {
        "iso-date-tomorrow"
    }
    fn description(&self) -> &'static str {
        "Insert tomorrow's date as plain ISO — `YYYY-MM-DD` (no brackets)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["isodtm"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "iso-date-tomorrow", &iso_date_offset(1));
        Ok(false)
    }
}

pub struct IsoDateYesterdayCommand;
impl SlashCommand for IsoDateYesterdayCommand {
    fn name(&self) -> &'static str {
        "iso-date-yesterday"
    }
    fn description(&self) -> &'static str {
        "Insert yesterday's date as plain ISO — `YYYY-MM-DD` (no brackets)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["isody"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        insert_or_warn(app, "iso-date-yesterday", &iso_date_offset(-1));
        Ok(false)
    }
}

// ─── week-num (`#YYYY-Www`) ───────────────────────────────────────────
// Inserted as a `#tag` (not `[[ref]]`) so it routes through the
// existing tag indexing — weekly notes show up in backlinks for the
// tag page if it exists.

pub struct WeekNumCommand;
impl SlashCommand for WeekNumCommand {
    fn name(&self) -> &'static str {
        "week-num"
    }
    fn description(&self) -> &'static str {
        "Insert current ISO week as a tag — `#YYYY-Www`"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["wn", "week"]
    }
    fn inserts_inline(&self) -> bool {
        true
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        let s = week_tag(clock::today());
        insert_or_warn(app, "week-num", &s);
        Ok(false)
    }
}

// ─── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Strip the `[[ ... ]]` wrap and parse the inner ISO date.
    fn parse_link(s: &str) -> chrono::NaiveDate {
        assert!(
            s.starts_with("[[") && s.ends_with("]]"),
            "not a link: {s:?}"
        );
        let inner = &s[2..s.len() - 2];
        chrono::NaiveDate::parse_from_str(inner, "%Y-%m-%d").unwrap_or_else(|e| {
            panic!("inner {inner:?} not ISO date: {e}");
        })
    }

    #[test]
    fn today_link_format_matches_journal_slug() {
        let s = journal_link_offset(0);
        assert_eq!(s.len(), 14, "expected `[[YYYY-MM-DD]]`, got {s:?}");
        parse_link(&s); // panics if format is off
    }

    #[test]
    fn tomorrow_minus_today_is_one_day() {
        let today = parse_link(&journal_link_offset(0));
        let tomorrow = parse_link(&journal_link_offset(1));
        assert_eq!(tomorrow - today, Duration::days(1));
    }

    #[test]
    fn yesterday_minus_today_is_negative_one_day() {
        let today = parse_link(&journal_link_offset(0));
        let yesterday = parse_link(&journal_link_offset(-1));
        assert_eq!(yesterday - today, Duration::days(-1));
    }

    #[test]
    fn time_text_is_plain_hhmm() {
        let s = time_text();
        assert_eq!(s.len(), 5, "expected HH:MM, got {s:?}");
        assert!(!s.contains('['), "time should not be a link");
        assert_eq!(s.chars().nth(2), Some(':'));
    }

    #[test]
    fn datetime_text_has_link_then_time() {
        let s = datetime_text();
        assert_eq!(s.len(), 20, "expected `[[YYYY-MM-DD]] HH:MM`, got {s:?}");
        let (link, time) = s.split_once(' ').expect("space between link and time");
        parse_link(link);
        assert_eq!(time.len(), 5);
        assert_eq!(time.chars().nth(2), Some(':'));
    }

    /// All commands marked `inserts_inline = true` write at the
    /// cursor — the slash dispatcher relies on this flag to skip the
    /// `commit_insert()` step. If anyone adds a new inline command,
    /// this guard catches a missing override.
    #[test]
    fn date_commands_advertise_inline_inserts() {
        assert!(DateTodayCommand.inserts_inline());
        assert!(DateTomorrowCommand.inserts_inline());
        assert!(DateYesterdayCommand.inserts_inline());
        assert!(TimeNowCommand.inserts_inline());
        assert!(DateTimeNowCommand.inserts_inline());
        assert!(DateNextWeekCommand.inserts_inline());
        assert!(DateLastWeekCommand.inserts_inline());
        assert!(DateNextMondayCommand.inserts_inline());
        assert!(DateNextFridayCommand.inserts_inline());
        assert!(DateNextSundayCommand.inserts_inline());
        assert!(DateCommand.inserts_inline());
        assert!(IsoDateTodayCommand.inserts_inline());
        assert!(IsoDateTomorrowCommand.inserts_inline());
        assert!(IsoDateYesterdayCommand.inserts_inline());
        assert!(WeekNumCommand.inserts_inline());
    }

    #[test]
    fn next_week_is_seven_days_ahead() {
        let today = parse_link(&journal_link_offset(0));
        let nw = parse_link(&journal_link_offset(7));
        assert_eq!(nw - today, Duration::days(7));
    }

    #[test]
    fn last_week_is_seven_days_back() {
        let today = parse_link(&journal_link_offset(0));
        let lw = parse_link(&journal_link_offset(-7));
        assert_eq!(today - lw, Duration::days(7));
    }

    // -- days_until_next_weekday ---------------------------------------------

    #[test]
    fn next_weekday_on_same_weekday_jumps_seven() {
        // 2026-05-25 is a Monday.
        let mon = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        assert_eq!(days_until_next_weekday(mon, Weekday::Mon), 7);
    }

    #[test]
    fn next_weekday_in_same_week() {
        // Monday → Friday is 4 days.
        let mon = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        assert_eq!(days_until_next_weekday(mon, Weekday::Fri), 4);
    }

    #[test]
    fn next_weekday_wraps_across_weekend() {
        // Friday → Monday is 3 days.
        let fri = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        assert_eq!(days_until_next_weekday(fri, Weekday::Mon), 3);
    }

    #[test]
    fn next_weekday_one_day_forward() {
        // Tuesday → Wednesday is 1 day.
        let tue = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        assert_eq!(days_until_next_weekday(tue, Weekday::Wed), 1);
    }

    // -- parse_date_arg ------------------------------------------------------

    #[test]
    fn parse_absolute_iso_date() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        let d = parse_date_arg("2026-06-15", today).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
    }

    #[test]
    fn parse_positive_days_offset() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        let d = parse_date_arg("+3d", today).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 5, 29).unwrap());
    }

    #[test]
    fn parse_negative_weeks_offset() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        let d = parse_date_arg("-2w", today).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 5, 12).unwrap());
    }

    #[test]
    fn parse_months_offset_clamps_in_short_months() {
        // Jan 31 + 1 month → Feb 28 (or 29 in leap years). chrono::Months
        // clamps to the last valid day, which is exactly what we want.
        let today = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let d = parse_date_arg("+1m", today).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    #[test]
    fn parse_bare_offset_is_positive() {
        // No sign means `+` — a UX kindness.
        let today = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        let d = parse_date_arg("5d", today).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 5, 31).unwrap());
    }

    #[test]
    fn parse_rejects_garbage() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
        assert!(parse_date_arg("nope", today).is_none());
        assert!(parse_date_arg("+3x", today).is_none());
        assert!(parse_date_arg("", today).is_none());
        assert!(parse_date_arg("2026-13-99", today).is_none());
    }

    // -- iso_date / week_tag -------------------------------------------------

    #[test]
    fn iso_date_offset_has_no_brackets() {
        let s = iso_date_offset(0);
        assert_eq!(s.len(), 10, "expected YYYY-MM-DD, got {s:?}");
        assert!(!s.contains('['));
        assert!(NaiveDate::parse_from_str(&s, "%Y-%m-%d").is_ok());
    }

    #[test]
    fn week_tag_format_is_hash_year_w_week() {
        // 2026-05-25 is Monday of ISO week 22 of year 2026.
        let mon = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        assert_eq!(week_tag(mon), "#2026-W22");
    }

    #[test]
    fn week_tag_uses_iso_week_year_at_boundary() {
        // 2025-12-31 (Wed) is ISO week 1 of 2026 — `%G` (ISO year)
        // differs from `%Y` (calendar year) here. Confirms we used %G.
        let last_of_year = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert_eq!(week_tag(last_of_year), "#2026-W01");
    }

    // -- non-date commands kept lean -----------------------------------------

    #[test]
    fn non_date_commands_stay_non_inline() {
        // Sentinel — anything not in this module should keep its
        // default `inserts_inline == false`. We re-assert here so
        // a refactor that flips the default doesn't slip through.
        use super::super::exec::SearchCommand;
        use super::super::workspace::{OpenCommand, TodayCommand};
        assert!(!TodayCommand.inserts_inline());
        assert!(!OpenCommand.inserts_inline());
        assert!(!SearchCommand.inserts_inline());
    }
}
