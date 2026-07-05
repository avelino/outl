//! Date / time / week-tag insert commands.
//!
//! Sixteen single-purpose commands that all share the same shape: in
//! Insert mode they paste a formatted timestamp at the cursor; in
//! Normal mode they refuse politely with a status hint. The seven
//! `date-next-<weekday>` siblings come from a single macro.
//!
//! This module is **wiring only**: the date parsing / calendar
//! arithmetic lives in `outl_actions::dates` (`parse_date_arg`,
//! `week_tag`, `days_until_next_weekday`, `journal_ref`,
//! `journal_slug`) and "what is today" comes from
//! `outl_actions::clock`. The helpers below just anchor those pure
//! functions to the current clock reading.

use anyhow::Result;
use chrono::{Duration, Weekday};
use outl_actions::{clock, dates};

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
/// past, positive are future.
fn journal_link_offset(days_offset: i64) -> String {
    dates::journal_ref(clock::today() + Duration::days(days_offset))
}

/// `HH:MM` for the current local time. Plain text — not a journal ref.
fn time_text() -> String {
    clock::now_local().format("%H:%M").to_string()
}

/// `[[YYYY-MM-DD]] HH:MM` — the "stamp this moment" combo.
fn datetime_text() -> String {
    let now = clock::now_local();
    format!(
        "{} {}",
        dates::journal_ref(now.date_naive()),
        now.format("%H:%M")
    )
}

/// Plain `YYYY-MM-DD` (no brackets) for property values like
/// `due:: 2026-05-26`. ISO 8601 short date.
fn iso_date_offset(days_offset: i64) -> String {
    dates::journal_slug(clock::today() + Duration::days(days_offset))
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
                let days = dates::days_until_next_weekday(today, $weekday);
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
        match dates::parse_date_arg(args, today) {
            Some(d) => {
                insert_or_warn(app, "date", &dates::journal_ref(d));
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
        let s = dates::week_tag(clock::today());
        insert_or_warn(app, "week-num", &s);
        Ok(false)
    }
}

// ─── tests ────────────────────────────────────────────────────────────
// Only wiring is tested here: the clock-anchored helpers and the
// SlashCommand plumbing. The pure date logic (parse_date_arg, week_tag,
// days_until_next_weekday, journal labels) is tested in
// `outl_actions::dates`.

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

    #[test]
    fn iso_date_offset_has_no_brackets() {
        let s = iso_date_offset(0);
        assert_eq!(s.len(), 10, "expected YYYY-MM-DD, got {s:?}");
        assert!(!s.contains('['));
        assert!(chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").is_ok());
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
