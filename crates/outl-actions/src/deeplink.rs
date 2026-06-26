//! `outl://` deep link parsing, shared by every GUI client.
//!
//! External launchers (the Raycast extension, future Alfred / browser
//! integrations, links shared into the mobile app) open outl at a
//! specific page or daily note through an `outl://` URL. The parsing
//! lives **here** — not in `outl-desktop` / `outl-mobile` — so the two
//! clients can't drift on the scheme contract. (Two parallel
//! implementations of one rule is the exact bug the shortcut catalog
//! already paid to remove; see the root `CLAUDE.md`.)
//!
//! Each client maps the parsed [`DeepLinkTarget`] onto its own `open_*`
//! command + window focus. This module never touches a `Workspace`,
//! storage, or any Tauri type — it's pure string → enum.
//!
//! ## Scheme contract
//!
//! | URL                       | [`DeepLinkTarget`]      |
//! |---------------------------|-------------------------|
//! | `outl://daily/today`      | [`DeepLinkTarget::Today`] |
//! | `outl://daily/2026-06-25` | [`DeepLinkTarget::Daily`] |
//! | `outl://page/<slug>`      | [`DeepLinkTarget::Page`] (`/` = nesting) |
//!
//! Anything else is an [`Err`] the client logs and ignores — never a
//! crash, never a stray page.

use chrono::NaiveDate;

use crate::page::is_valid_slug;

/// The URL scheme every outl deep link uses (without `://`).
pub const DEEP_LINK_SCHEME: &str = "outl";

/// A parsed `outl://` target, resolved by the client to a navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkTarget {
    /// `outl://daily/today` — open today's journal.
    Today,
    /// `outl://daily/<YYYY-MM-DD>` — open the daily for a specific date.
    Daily(NaiveDate),
    /// `outl://page/<slug>` — open a page by slug. The slug may be
    /// hierarchical (`ai-agent/learning`); each `/`-separated segment
    /// is validated independently.
    Page(String),
}

/// Why an `outl://` URL could not be parsed into a [`DeepLinkTarget`].
///
/// The client logs this and no-ops — a bad URL must never crash the app
/// or materialise a page.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    /// The URL did not start with `outl://`.
    #[error("not an outl:// URL")]
    WrongScheme,
    /// The path segment after `outl://` is not a known kind.
    #[error("unknown deep link kind `{0}`")]
    UnknownKind(String),
    /// The kind was recognised but the payload was empty / shapeless.
    #[error("malformed deep link payload")]
    Malformed,
    /// `outl://daily/<x>` where `<x>` is neither `today` nor an ISO date.
    #[error("invalid date `{0}` (expected YYYY-MM-DD)")]
    InvalidDate(String),
    /// `outl://page/<x>` where `<x>` is not a valid (hierarchical) slug.
    #[error("invalid page slug `{0}`")]
    InvalidSlug(String),
}

/// Parse an `outl://` URL into a navigation [`DeepLinkTarget`].
///
/// ```
/// use outl_actions::{parse_deep_link, DeepLinkTarget};
///
/// assert_eq!(parse_deep_link("outl://daily/today"), Ok(DeepLinkTarget::Today));
/// assert!(parse_deep_link("outl://page/ai-agent/learning").is_ok());
/// assert!(parse_deep_link("https://example.com").is_err());
/// ```
pub fn parse_deep_link(url: &str) -> Result<DeepLinkTarget, DeepLinkError> {
    let rest = url
        .strip_prefix("outl://")
        .ok_or(DeepLinkError::WrongScheme)?;

    // Split the kind off the front; keep every remaining `/` inside the
    // payload so hierarchical page slugs (`ai-agent/learning`) survive.
    let (kind, payload) = rest.split_once('/').unwrap_or((rest, ""));

    match kind {
        "daily" => parse_daily(payload),
        "page" => parse_page(payload),
        "" => Err(DeepLinkError::Malformed),
        other => Err(DeepLinkError::UnknownKind(other.to_string())),
    }
}

fn parse_daily(payload: &str) -> Result<DeepLinkTarget, DeepLinkError> {
    match payload {
        "" => Err(DeepLinkError::Malformed),
        "today" => Ok(DeepLinkTarget::Today),
        iso => NaiveDate::parse_from_str(iso, "%Y-%m-%d")
            .map(DeepLinkTarget::Daily)
            .map_err(|_| DeepLinkError::InvalidDate(iso.to_string())),
    }
}

fn parse_page(payload: &str) -> Result<DeepLinkTarget, DeepLinkError> {
    // Tolerate a trailing slash (`outl://page/foo/`), but nothing else.
    let slug = payload.trim_end_matches('/');
    if slug.is_empty() {
        return Err(DeepLinkError::Malformed);
    }
    // Validate each segment with the page model's own rule. `is_valid_slug`
    // rejects `/`, control chars, and `..`, so applying it per segment
    // both allows nesting and blocks path traversal.
    if slug.split('/').all(is_valid_slug) {
        Ok(DeepLinkTarget::Page(slug.to_string()))
    } else {
        Err(DeepLinkError::InvalidSlug(slug.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_today() {
        assert_eq!(
            parse_deep_link("outl://daily/today"),
            Ok(DeepLinkTarget::Today)
        );
    }

    #[test]
    fn parses_daily_iso() {
        assert_eq!(
            parse_deep_link("outl://daily/2026-06-25"),
            Ok(DeepLinkTarget::Daily(
                NaiveDate::from_ymd_opt(2026, 6, 25).unwrap()
            ))
        );
    }

    #[test]
    fn parses_flat_page() {
        assert_eq!(
            parse_deep_link("outl://page/inbox"),
            Ok(DeepLinkTarget::Page("inbox".into()))
        );
    }

    #[test]
    fn parses_hierarchical_page() {
        assert_eq!(
            parse_deep_link("outl://page/ai-agent/learning"),
            Ok(DeepLinkTarget::Page("ai-agent/learning".into()))
        );
    }

    #[test]
    fn tolerates_trailing_slash_on_page() {
        assert_eq!(
            parse_deep_link("outl://page/inbox/"),
            Ok(DeepLinkTarget::Page("inbox".into()))
        );
    }

    #[test]
    fn rejects_wrong_scheme() {
        assert_eq!(
            parse_deep_link("https://example.com"),
            Err(DeepLinkError::WrongScheme)
        );
        assert_eq!(
            parse_deep_link("file:///etc/passwd"),
            Err(DeepLinkError::WrongScheme)
        );
        // A bare slug with no scheme is not a deep link.
        assert_eq!(
            parse_deep_link("daily/today"),
            Err(DeepLinkError::WrongScheme)
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert_eq!(
            parse_deep_link("outl://search/foo"),
            Err(DeepLinkError::UnknownKind("search".into()))
        );
        assert_eq!(
            parse_deep_link("outl://block/01ABC"),
            Err(DeepLinkError::UnknownKind("block".into()))
        );
    }

    #[test]
    fn rejects_empty_payloads() {
        assert_eq!(parse_deep_link("outl://"), Err(DeepLinkError::Malformed));
        assert_eq!(
            parse_deep_link("outl://daily"),
            Err(DeepLinkError::Malformed)
        );
        assert_eq!(
            parse_deep_link("outl://daily/"),
            Err(DeepLinkError::Malformed)
        );
        assert_eq!(
            parse_deep_link("outl://page"),
            Err(DeepLinkError::Malformed)
        );
        assert_eq!(
            parse_deep_link("outl://page/"),
            Err(DeepLinkError::Malformed)
        );
    }

    #[test]
    fn rejects_invalid_dates() {
        // Month 13 — chrono rejects it, so the strict-validation lesson holds.
        assert_eq!(
            parse_deep_link("outl://daily/2026-13-01"),
            Err(DeepLinkError::InvalidDate("2026-13-01".into()))
        );
        // Day 32.
        assert!(matches!(
            parse_deep_link("outl://daily/2026-01-32"),
            Err(DeepLinkError::InvalidDate(_))
        ));
        // Not a date at all.
        assert!(matches!(
            parse_deep_link("outl://daily/yesterday"),
            Err(DeepLinkError::InvalidDate(_))
        ));
    }

    #[test]
    fn rejects_path_traversal_slug() {
        // `..` and absolute escapes must never resolve to a page.
        assert!(matches!(
            parse_deep_link("outl://page/../../etc/passwd"),
            Err(DeepLinkError::InvalidSlug(_))
        ));
        assert!(matches!(
            parse_deep_link("outl://page/foo/../bar"),
            Err(DeepLinkError::InvalidSlug(_))
        ));
    }

    #[test]
    fn rejects_empty_inner_segment() {
        // `foo//bar` has an empty middle segment — invalid.
        assert!(matches!(
            parse_deep_link("outl://page/foo//bar"),
            Err(DeepLinkError::InvalidSlug(_))
        ));
    }

    #[test]
    fn rejects_leading_slash_segment() {
        // `outl://page//foo` → payload `/foo` → empty first segment.
        // `is_valid_slug("")` is false, so the whole slug is rejected
        // instead of resolving to a stray `foo` page.
        assert!(matches!(
            parse_deep_link("outl://page//foo"),
            Err(DeepLinkError::InvalidSlug(_))
        ));
    }
}
