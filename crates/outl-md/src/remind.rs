//! The `remind::` block property: an English-shaped notification rule.
//!
//! ```text
//! - TODO #fup [[@joão]] about project abc [[2026-12-12]]
//!   remind:: 3pm every 1h until DONE
//! ```
//!
//! Grammar (see `docs/reminders.md` for the user-facing spec):
//!
//! ```ebnf
//! remind     ::= TIME ("every" INTERVAL)? ("until" STOP)? ("max" N)?
//! TIME       ::= H_AMPM | HHMM | HHMM_AMPM | "now"
//! INTERVAL   ::= N ("min" | "h" | "d")
//! STOP       ::= "DONE" | TIME | ISO_DATE
//! N          ::= 1..999
//! ```
//!
//! This module owns **syntax only**: text in, [`RemindRule`] out, plus the
//! [`ParseWarningKind`] records for anything it had to reject or clamp.
//! *When* a rule actually fires is `outl_actions::reminders` — one owner
//! for the schedule math, wrapped by every OS bridge.
//!
//! Parsing is permissive in the same sense as the rest of [`mod@crate::parse`]:
//! an invalid `remind::` never removes the property and never drops the
//! block. It just yields `None` (no scheduling) plus a warning the client
//! surfaces in the parse banner.

use crate::parse::ParseWarningKind;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Property key that carries a reminder rule on a block.
pub const REMIND_KEY: &str = "remind";

/// Smallest repeat interval we accept, in minutes. Anything below this
/// (`every 30s`) is rejected rather than clamped — a sub-minute nag is
/// never what the user meant, and silently rewriting it to 1min would
/// hide the typo.
pub const MIN_INTERVAL_MINUTES: u32 = 1;

/// Hard ceiling on `max N`. A rule asking for more is clamped down to
/// this and reported via [`ParseWarningKind::RemindMaxClamped`].
pub const MAX_FIRES_CAP: u32 = 10;

/// Where the first fire is anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemindAnchor {
    /// `now` — fire as soon as the rule is seen.
    Now,
    /// A wall-clock time of day, on the block's resolved date.
    At {
        /// Hour, 0-23.
        hour: u32,
        /// Minute, 0-59.
        minute: u32,
    },
}

/// When the repetition stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemindStop {
    /// `until DONE` — stop when the block's TODO flips to DONE. Also the
    /// implicit default when the user writes no `until` clause.
    Done,
    /// `until 6pm` — stop at a wall-clock time on the anchor's date.
    Time {
        /// Hour, 0-23.
        hour: u32,
        /// Minute, 0-59.
        minute: u32,
    },
    /// `until 2026-12-20` — stop at the end of that ISO date.
    Date(NaiveDate),
}

/// A parsed, validated `remind::` rule.
///
/// Every field is already clamped to the hard caps, so a consumer can
/// schedule straight off it without re-validating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemindRule {
    /// First fire.
    pub anchor: RemindAnchor,
    /// Repeat interval in minutes. `None` = single shot.
    pub every_minutes: Option<u32>,
    /// Stop condition. `None` means the implicit `until DONE`.
    pub until: Option<RemindStop>,
    /// Cap on the number of fires. `None` = uncapped (bounded by `until`).
    pub max_fires: Option<u32>,
}

impl RemindRule {
    /// The effective stop condition, resolving the implicit default.
    pub fn stop(&self) -> RemindStop {
        self.until.unwrap_or(RemindStop::Done)
    }

    /// Whether this rule repeats at all.
    pub fn repeats(&self) -> bool {
        self.every_minutes.is_some()
    }
}

/// Outcome of parsing one `remind::` value.
///
/// `rule` is `None` when the value could not be understood — scheduling
/// is disabled for that block, but the property text stays on disk
/// untouched so the user can fix the typo in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemindParse {
    /// The rule, when the value parsed.
    pub rule: Option<RemindRule>,
    /// Everything the parser rejected or clamped, in encounter order.
    pub warnings: Vec<ParseWarningKind>,
}

/// Parse the value side of a `remind:: …` property line.
///
/// Never panics, never allocates on the happy path beyond the token
/// split. See the module docs for the grammar.
pub fn parse_remind(value: &str) -> RemindParse {
    let lowered = value.trim().to_ascii_lowercase();
    let tokens: Vec<&str> = lowered.split_whitespace().collect();
    let mut warnings = Vec::new();

    let Some((&first, mut rest)) = tokens.split_first() else {
        warnings.push(ParseWarningKind::RemindMissingAnchor);
        return RemindParse {
            rule: None,
            warnings,
        };
    };

    // The anchor is mandatory: `remind:: every 1h` has no start.
    if first == "every" || first == "until" || first == "max" {
        warnings.push(ParseWarningKind::RemindMissingAnchor);
        return RemindParse {
            rule: None,
            warnings,
        };
    }
    let anchor = match parse_anchor(first) {
        Some(a) => a,
        None => {
            warnings.push(ParseWarningKind::RemindInvalidTime);
            return RemindParse {
                rule: None,
                warnings,
            };
        }
    };

    let mut every_minutes = None;
    let mut until = None;
    let mut max_fires = None;

    while let Some((&keyword, tail)) = rest.split_first() {
        let Some((&arg, after)) = tail.split_first() else {
            // Dangling keyword with no argument.
            warnings.push(match keyword {
                "every" => ParseWarningKind::RemindInvalidInterval,
                "until" => ParseWarningKind::RemindInvalidStop,
                _ => ParseWarningKind::RemindInvalidTime,
            });
            return RemindParse {
                rule: None,
                warnings,
            };
        };
        match keyword {
            "every" => match parse_interval(arg) {
                Some(m) => every_minutes = Some(m),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidInterval);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            "until" => match parse_stop(arg) {
                Some(s) => until = Some(s),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidStop);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            "max" => match arg.parse::<u32>().ok().filter(|n| (1..=999).contains(n)) {
                Some(n) if n > MAX_FIRES_CAP => {
                    warnings.push(ParseWarningKind::RemindMaxClamped);
                    max_fires = Some(MAX_FIRES_CAP);
                }
                Some(n) => max_fires = Some(n),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidInterval);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            _ => {
                warnings.push(ParseWarningKind::RemindInvalidTime);
                return RemindParse {
                    rule: None,
                    warnings,
                };
            }
        }
        rest = after;
    }

    // `until 6pm` earlier than the anchor is unschedulable on the same
    // day. Drop the clause rather than the whole rule — the user still
    // gets their reminder, just without the stop.
    if let (
        RemindAnchor::At { hour, minute },
        Some(RemindStop::Time {
            hour: sh,
            minute: sm,
        }),
    ) = (anchor, until)
    {
        if (sh, sm) <= (hour, minute) {
            warnings.push(ParseWarningKind::RemindInvalidStop);
            until = None;
        }
    }

    RemindParse {
        rule: Some(RemindRule {
            anchor,
            every_minutes,
            until,
            max_fires,
        }),
        warnings,
    }
}

/// Pull the rule out of a block's property list, if it carries one.
///
/// Returns `None` when the block has no `remind::` at all — an invalid
/// rule also yields `None` on the rule side, with its warnings attached.
pub fn rule_from_properties(properties: &[(String, String)]) -> Option<RemindParse> {
    properties
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(REMIND_KEY))
        .map(|(_, v)| parse_remind(v))
}

fn parse_anchor(tok: &str) -> Option<RemindAnchor> {
    if tok == "now" {
        return Some(RemindAnchor::Now);
    }
    let (hour, minute) = parse_time(tok)?;
    Some(RemindAnchor::At { hour, minute })
}

fn parse_stop(tok: &str) -> Option<RemindStop> {
    if tok == "done" {
        return Some(RemindStop::Done);
    }
    if let Some((hour, minute)) = parse_time(tok) {
        return Some(RemindStop::Time { hour, minute });
    }
    NaiveDate::parse_from_str(tok, "%Y-%m-%d")
        .ok()
        .map(RemindStop::Date)
}

/// `10am`, `3pm`, `15:00`, `1:30pm`, `9:05`.
fn parse_time(tok: &str) -> Option<(u32, u32)> {
    if let Some(body) = tok.strip_suffix("am") {
        let (h, m) = split_hm(body)?;
        if !(1..=12).contains(&h) {
            return None;
        }
        return Some((if h == 12 { 0 } else { h }, m));
    }
    if let Some(body) = tok.strip_suffix("pm") {
        let (h, m) = split_hm(body)?;
        if !(1..=12).contains(&h) {
            return None;
        }
        return Some((if h == 12 { 12 } else { h + 12 }, m));
    }
    // 24h form requires the colon; a bare `15` is too ambiguous to guess.
    let (h, m) = tok.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// `10` -> (10, 0); `1:30` -> (1, 30). Used for the am/pm branch only.
fn split_hm(body: &str) -> Option<(u32, u32)> {
    match body.split_once(':') {
        Some((h, m)) => {
            let hour: u32 = h.parse().ok()?;
            let minute: u32 = m.parse().ok()?;
            (minute < 60).then_some((hour, minute))
        }
        None => body.parse::<u32>().ok().map(|h| (h, 0)),
    }
}

/// `30min`, `1h`, `2d`.
fn parse_interval(tok: &str) -> Option<u32> {
    let (digits, unit) = tok.split_at(tok.find(|c: char| !c.is_ascii_digit())?);
    let n: u32 = digits.parse().ok()?;
    if !(1..=999).contains(&n) {
        return None;
    }
    let minutes = match unit {
        "min" | "m" => n,
        "h" => n.checked_mul(60)?,
        "d" => n.checked_mul(60 * 24)?,
        _ => return None,
    };
    (minutes >= MIN_INTERVAL_MINUTES).then_some(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(s: &str) -> RemindRule {
        parse_remind(s).rule.expect("rule should parse")
    }

    #[test]
    fn bare_time_is_a_single_shot() {
        let r = rule("10am");
        assert_eq!(
            r.anchor,
            RemindAnchor::At {
                hour: 10,
                minute: 0
            }
        );
        assert_eq!(r.every_minutes, None);
        assert_eq!(r.stop(), RemindStop::Done);
        assert!(!r.repeats());
    }

    #[test]
    fn every_clause_sets_the_interval() {
        assert_eq!(rule("10am every 1h").every_minutes, Some(60));
        assert_eq!(rule("3pm every 30min").every_minutes, Some(30));
        assert_eq!(rule("15:00 every 2d").every_minutes, Some(2880));
    }

    #[test]
    fn until_done_is_the_implicit_default() {
        assert_eq!(rule("10am every 1h").stop(), RemindStop::Done);
        assert_eq!(rule("10am every 1h until done").stop(), RemindStop::Done);
    }

    #[test]
    fn until_accepts_time_and_iso_date() {
        assert_eq!(
            rule("10am every 1h until 6pm").until,
            Some(RemindStop::Time {
                hour: 18,
                minute: 0
            })
        );
        assert_eq!(
            rule("10am every 1d until 2026-12-20").until,
            Some(RemindStop::Date(
                NaiveDate::from_ymd_opt(2026, 12, 20).expect("valid date")
            ))
        );
    }

    #[test]
    fn now_anchor_parses() {
        assert_eq!(rule("now every 15min until DONE").anchor, RemindAnchor::Now);
    }

    #[test]
    fn am_pm_edge_hours_map_to_24h() {
        assert_eq!(rule("12am").anchor, RemindAnchor::At { hour: 0, minute: 0 });
        assert_eq!(
            rule("12pm").anchor,
            RemindAnchor::At {
                hour: 12,
                minute: 0
            }
        );
        assert_eq!(
            rule("1:30pm").anchor,
            RemindAnchor::At {
                hour: 13,
                minute: 30
            }
        );
    }

    #[test]
    fn missing_anchor_is_rejected() {
        let p = parse_remind("every 1h");
        assert!(p.rule.is_none());
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindMissingAnchor]);
        assert_eq!(
            parse_remind("").warnings,
            vec![ParseWarningKind::RemindMissingAnchor]
        );
    }

    #[test]
    fn invalid_time_is_rejected() {
        for bad in ["25:00", "10:70", "13am", "0pm", "banana"] {
            let p = parse_remind(bad);
            assert!(p.rule.is_none(), "{bad} should not parse");
            assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidTime]);
        }
    }

    #[test]
    fn sub_minute_interval_is_rejected() {
        let p = parse_remind("10am every 30s");
        assert!(p.rule.is_none());
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidInterval]);
    }

    #[test]
    fn unparseable_stop_is_rejected() {
        let p = parse_remind("10am until yesterday");
        assert!(p.rule.is_none());
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidStop]);
    }

    #[test]
    fn max_above_the_cap_is_clamped_not_rejected() {
        let p = parse_remind("10am every 1h max 50");
        assert_eq!(p.rule.expect("still schedules").max_fires, Some(10));
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindMaxClamped]);
        assert_eq!(rule("10am every 1h max 5").max_fires, Some(5));
    }

    #[test]
    fn until_before_the_anchor_drops_the_clause() {
        let p = parse_remind("3pm every 1h until 10am");
        let r = p.rule.expect("rule survives, stop does not");
        assert_eq!(r.until, None);
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidStop]);
    }

    #[test]
    fn dangling_keyword_does_not_panic() {
        assert!(parse_remind("10am every").rule.is_none());
        assert!(parse_remind("10am until").rule.is_none());
        assert!(parse_remind("10am max").rule.is_none());
    }

    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(
            rule("3PM EVERY 1H UNTIL DONE").anchor,
            RemindAnchor::At {
                hour: 15,
                minute: 0
            }
        );
    }

    #[test]
    fn rule_is_read_off_the_property_list() {
        let props = vec![
            ("id".to_string(), "x".to_string()),
            ("remind".to_string(), "10am".to_string()),
        ];
        assert!(rule_from_properties(&props)
            .expect("has remind")
            .rule
            .is_some());
        assert!(rule_from_properties(&[]).is_none());
    }
}
