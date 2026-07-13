//! Built-in variable substitution for template instantiation.
//!
//! Templates can carry placeholder tokens that are replaced at
//! instantiation time with date/page/time values. The set of
//! supported tokens is intentionally small and closed — custom
//! logic belongs in a callable template's code block, not in more
//! tokens.
//!
//! | Token | Replaced with |
//! |---|---|
//! | `{{date}}` | Date of the target page (journal) or today |
//! | `{{today}}` | Today's date (ISO) |
//! | `{{yesterday}}` | Yesterday's date |
//! | `{{tomorrow}}` | Tomorrow's date |
//! | `{{page}}` | Slug of the target page |
//! | `{{time}}` | Current wall-clock `HH:MM` |

use chrono::{DateTime, FixedOffset, NaiveDate};

use crate::clock;
use crate::dates::journal_slug;

/// Context carried into [`substitute_vars`] to resolve built-in
/// tokens.
#[derive(Debug, Clone)]
pub(crate) struct VarContext {
    /// Slug of the page the template is being instantiated into.
    pub page_slug: String,
    /// Date of the target page when it is a journal (`YYYY-MM-DD`).
    /// Falls back to today for non-journal pages.
    pub page_date: NaiveDate,
    /// Wall-clock now — the single anchor for `{{today}}`,
    /// `{{yesterday}}`, `{{tomorrow}}`, and `{{time}}`, so a frozen
    /// context yields deterministic output (no second clock read that
    /// could cross a midnight boundary mid-substitution).
    pub now: DateTime<FixedOffset>,
}

impl VarContext {
    /// Build a context from the target page slug and an optional
    /// journal date. When `page_date` is `None`, today is used.
    ///
    /// "Now"/"today" come from [`crate::clock`], not `chrono::Local`,
    /// so template dates honour the configured `[calendar] timezone`
    /// and match the journal date instead of reading UTC inside
    /// containers/Crostini (issue #107).
    pub fn new(page_slug: &str, page_date: Option<NaiveDate>) -> Self {
        Self {
            page_slug: page_slug.to_string(),
            page_date: page_date.unwrap_or_else(clock::today),
            now: clock::now_local(),
        }
    }
}

/// Replace every known `{{token}}` in `text` using `ctx`.
///
/// Unknown tokens are left verbatim so the user sees them and can
/// fix the template instead of silently swallowing typos. Every date
/// derives from `ctx.now`, so the result is deterministic for a given
/// context.
pub(crate) fn substitute_vars(text: &str, ctx: &VarContext) -> String {
    let today = ctx.now.date_naive();
    let yesterday = today - chrono::Duration::days(1);
    let tomorrow = today + chrono::Duration::days(1);

    text.replace("{{date}}", &journal_slug(ctx.page_date))
        .replace("{{today}}", &journal_slug(today))
        .replace("{{yesterday}}", &journal_slug(yesterday))
        .replace("{{tomorrow}}", &journal_slug(tomorrow))
        .replace("{{page}}", &ctx.page_slug)
        .replace("{{time}}", &ctx.now.format("%H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> VarContext {
        VarContext {
            page_slug: "2026-07-08".to_string(),
            page_date: NaiveDate::from_ymd_opt(2026, 7, 8).unwrap(),
            now: DateTime::parse_from_rfc3339("2026-07-10T14:32:00-03:00").unwrap(),
        }
    }

    #[test]
    fn substitutes_date() {
        // `{{date}}` is the target page's date, independent of `now`.
        let result = substitute_vars("Meeting on {{date}}", &ctx());
        assert_eq!(result, "Meeting on 2026-07-08");
    }

    #[test]
    fn substitutes_today_from_ctx_now() {
        // Deterministic: `{{today}}` is `ctx.now`'s date, not a live
        // clock read — a frozen context yields a fixed value.
        let result = substitute_vars("Today is {{today}}", &ctx());
        assert_eq!(result, "Today is 2026-07-10");
    }

    #[test]
    fn substitutes_relative_dates_from_ctx_now() {
        let result = substitute_vars("Y: {{yesterday}} T: {{tomorrow}}", &ctx());
        assert_eq!(result, "Y: 2026-07-09 T: 2026-07-11");
    }

    #[test]
    fn substitutes_page_slug() {
        let result = substitute_vars("Page: {{page}}", &ctx());
        assert_eq!(result, "Page: 2026-07-08");
    }

    #[test]
    fn substitutes_time() {
        let result = substitute_vars("At {{time}}", &ctx());
        assert!(result.contains("14:32"));
    }

    #[test]
    fn leaves_unknown_tokens_verbatim() {
        let result = substitute_vars("Hello {{name}}!", &ctx());
        assert_eq!(result, "Hello {{name}}!");
    }

    #[test]
    fn substitutes_multiple_tokens_in_one_line() {
        let result = substitute_vars("[[{{date}}] {{page}} {{time}}", &ctx());
        assert!(result.starts_with("[[2026-07-08] 2026-07-08"));
    }

    #[test]
    fn no_tokens_returns_unchanged() {
        let result = substitute_vars("Just plain text", &ctx());
        assert_eq!(result, "Just plain text");
    }
}
