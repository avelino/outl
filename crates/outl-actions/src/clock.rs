//! Process-wide clock for "now" and "today" in the user's configured
//! timezone.
//!
//! Every client renders a journal date and a status-line clock. Those
//! used to call `chrono::Local::now()` directly, which trusts the
//! operating system's local timezone. That trust breaks in containers
//! and Chrome OS **Crostini**, where the OS clock runs in UTC even
//! though the user's real timezone isn't — so the journal lands on the
//! wrong day near midnight and the clock reads an hour off (issue
//! #107).
//!
//! The fix is opt-in and lives in one place. A client reads
//! `config.toml`'s `[calendar] timezone` (an IANA name like
//! `"Europe/London"`) and calls [`init`] once at boot. From then on,
//! [`now_local`] and [`today`] resolve in that zone — DST-aware, via
//! the `chrono-tz` database. With no timezone configured (the default
//! for a normally set-up machine) the clock stays on the OS local
//! timezone, so nothing changes for the 99% case.
//!
//! Call sites use [`now_local`] / [`today`] instead of
//! `chrono::Local::now()` so there is a single source of truth for
//! "what time is it for this user".

use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, Local, NaiveDate, Utc};
use chrono_tz::Tz;

/// The resolved timezone, set once at boot by [`init`].
///
/// `OnceLock<Option<Tz>>`: the outer `OnceLock` tracks "has a client
/// initialized the clock yet", the inner `Option` is "did that client
/// configure a zone". Before init (or when init resolved to local),
/// the clock falls back to [`Local`].
static CONFIGURED_TZ: OnceLock<Option<Tz>> = OnceLock::new();

/// Initialize the clock from the user's configured timezone.
///
/// `tz` is the raw `[calendar] timezone` value from `config.toml` — an
/// IANA name (`"Europe/London"`), or `None`/empty to use the OS local
/// timezone. An unparseable name is logged and treated as unset, so a
/// typo degrades to the previous behaviour instead of crashing.
///
/// Call once, early in each client's boot, before the first
/// [`now_local`] / [`today`]. Idempotent: the first call wins and later
/// calls are ignored (so a re-entrant boot path can't flip the zone out
/// from under code that already read it).
pub fn init(tz: Option<&str>) {
    let _ = CONFIGURED_TZ.set(resolve(tz));
}

/// Parse a configured timezone string into a [`Tz`].
///
/// `None`/empty/whitespace → `None` (use OS local). An unknown name →
/// `None` plus a `warn!`, so the clock falls back rather than failing.
fn resolve(tz: Option<&str>) -> Option<Tz> {
    let name = tz.map(str::trim).filter(|s| !s.is_empty())?;
    match name.parse::<Tz>() {
        Ok(tz) => Some(tz),
        Err(_) => {
            tracing::warn!("unknown [calendar] timezone {name:?} in config; using OS local time");
            None
        }
    }
}

/// The current instant in the configured timezone, or the OS local
/// timezone when none is set.
///
/// Returns a [`FixedOffset`] datetime so the offset is captured at the
/// call site — formatting `%H:%M` or taking `date_naive()` reflects the
/// user's wall clock, not UTC.
pub fn now_local() -> DateTime<FixedOffset> {
    match configured() {
        Some(tz) => Utc::now().with_timezone(&tz).fixed_offset(),
        None => Local::now().fixed_offset(),
    }
}

/// Today's date in the configured timezone, or the OS local timezone
/// when none is set. This is the journal's "today".
pub fn today() -> NaiveDate {
    now_local().date_naive()
}

/// The resolved zone, if a client initialized the clock with one.
fn configured() -> Option<Tz> {
    CONFIGURED_TZ.get().copied().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn resolve_parses_iana_name() {
        assert_eq!(resolve(Some("Europe/London")), Some(Tz::Europe__London));
        // Surrounding whitespace from a hand-edited TOML is tolerated.
        assert_eq!(resolve(Some("  Europe/London  ")), Some(Tz::Europe__London));
    }

    #[test]
    fn resolve_unset_or_empty_is_none() {
        assert_eq!(resolve(None), None);
        assert_eq!(resolve(Some("")), None);
        assert_eq!(resolve(Some("   ")), None);
    }

    #[test]
    fn resolve_unknown_name_falls_back_to_none() {
        // A typo must not crash the clock — it degrades to OS local.
        assert_eq!(resolve(Some("Mars/Phobos")), None);
    }

    #[test]
    fn london_zone_is_dst_aware() {
        // The #107 repro: at 20:24 UTC in summer, London is on BST (+01),
        // so the wall clock reads 21:24 — not 20:24.
        let tz = resolve(Some("Europe/London")).expect("known zone");
        let summer = Utc.with_ymd_and_hms(2026, 6, 28, 20, 24, 0).unwrap();
        assert_eq!(
            summer.with_timezone(&tz).format("%H:%M").to_string(),
            "21:24"
        );

        // In winter there's no DST: London is on GMT (+00), stays 20:24.
        let winter = Utc.with_ymd_and_hms(2026, 1, 15, 20, 24, 0).unwrap();
        assert_eq!(
            winter.with_timezone(&tz).format("%H:%M").to_string(),
            "20:24"
        );
    }

    #[test]
    fn date_can_differ_from_utc_across_midnight() {
        // 23:30 in São Paulo (UTC-3) is already 02:30 UTC the next day.
        // "today" must be the user's local date, not UTC's.
        let tz = resolve(Some("America/Sao_Paulo")).expect("known zone");
        let utc = Utc.with_ymd_and_hms(2026, 6, 29, 2, 30, 0).unwrap();
        let local = utc.with_timezone(&tz);
        assert_eq!(
            local.date_naive(),
            NaiveDate::from_ymd_opt(2026, 6, 28).unwrap()
        );
    }
}
