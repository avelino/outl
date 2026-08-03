//! `remind::` command bodies — shared by desktop and mobile.
//!
//! Every one of these is a thin shell over `outl_actions::reminders`.
//! The scheduling decision (*when does this fire*) is made there, once,
//! for every client; this module only translates DTOs and routes the
//! mutation through the usual `finish_in_page` path so the `.md` and
//! sidecar stay in step.
//!
//! Delivering the notification is **not** here: that's per-OS and lives
//! in each client's Tauri layer. What is shared is the answer to "what
//! should fire and when", which both clients read from
//! [`list_reminders`].

use chrono::Duration;
use outl_actions::reminders::{scan_reminders, snooze, FiredLog, Reminder};
use outl_actions::{clock, set_property};
use outl_core::property::PropValue;
use serde::{Deserialize, Serialize};

use crate::helpers::{finish_in_page, parse_node_id, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

/// Wire shape of one scheduled reminder.
///
/// Times are ISO-8601 **local** strings (`2026-12-12T15:00:00`), not
/// epoch numbers: the frontend renders them verbatim in the user's
/// wall clock, and re-deriving a local time from an epoch in JS would
/// re-introduce the timezone bug `outl_actions::clock` exists to fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderDto {
    pub block_id: String,
    pub page_slug: String,
    pub page_title: String,
    pub text: String,
    /// The `remind::` value verbatim, e.g. `"3pm every 1h until DONE"`.
    pub rule: String,
    /// `YYYY-MM-DD` the rule is anchored to.
    pub anchor_date: String,
    pub done: bool,
    /// Local ISO datetime of the next fire, or `null` when finished.
    pub next_fire: Option<String>,
    /// Local ISO datetime the snooze runs until, or `null`.
    pub snoozed_until: Option<String>,
}

impl From<Reminder> for ReminderDto {
    fn from(r: Reminder) -> Self {
        Self {
            block_id: r.block_id.to_string(),
            page_slug: r.page_slug,
            page_title: r.page_title,
            text: r.text,
            rule: r.rule_text,
            anchor_date: r.anchor_date.format("%Y-%m-%d").to_string(),
            done: r.done,
            next_fire: r
                .next_fire
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()),
            snoozed_until: r
                .snoozed_until_ms
                .and_then(outl_actions::reminders::epoch_ms_to_local_naive)
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()),
        }
    }
}

/// Device-local delivery preferences, surfaced so the frontend can
/// show "reminders are off" instead of an empty list the user can't
/// explain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderSettingsDto {
    pub enabled: bool,
    /// `"22:00-07:00"` or `""`.
    pub quiet_hours: String,
}

/// Every reminder in the workspace, soonest first.
///
/// Read-only, so it passes an empty [`FiredLog`]: the list answers
/// "what is scheduled", not "what has this device already delivered".
/// The delivery loop passes its real fired cache.
pub fn list_reminders<S: AppHost>(state: &S) -> Result<Vec<ReminderDto>, String> {
    let cfg = outl_config::load();
    let quiet = cfg.reminders.quiet_window();
    let now = clock::now_local().naive_local();
    with_ws(state, |ws| {
        Ok(scan_reminders(ws, &FiredLog::new(), quiet, now)
            .into_iter()
            .map(ReminderDto::from)
            .collect())
    })
}

/// This device's reminder settings. Reads `config.toml`, so it needs
/// no workspace and no host.
pub fn reminder_settings() -> ReminderSettingsDto {
    let cfg = outl_config::load();
    ReminderSettingsDto {
        enabled: cfg.reminders.enabled,
        quiet_hours: cfg.reminders.quiet_hours.unwrap_or_default(),
    }
}

/// Silence a block's reminder for `minutes` from now.
///
/// Goes through `Op::SnoozeRemind`, so the same block goes quiet on
/// every paired device — snoozing on the phone must not leave the
/// laptop buzzing.
///
/// Takes no page id because it touches no `.md`: the snooze lives only
/// in the op log, by design (writing it into the markdown would put a
/// device-local *time* into the user's clean notes).
pub fn snooze_reminder<S: AppHost>(state: &S, block_id: &str, minutes: i64) -> Result<(), String> {
    let node = parse_node_id(block_id)?;
    let until = clock::now_local().naive_local() + Duration::minutes(minutes.max(1));
    let until_ms = outl_actions::reminders::local_naive_to_epoch_ms(until);
    let hlc = state.hlc().clone();
    with_ws_mut(state, |ws| {
        snooze(ws, &hlc, node, until_ms).map_err(|e| e.to_string())
    })
}

/// Clear a block's snooze so it resumes on its normal schedule.
pub fn clear_reminder_snooze<S: AppHost>(state: &S, block_id: &str) -> Result<(), String> {
    let node = parse_node_id(block_id)?;
    let hlc = state.hlc().clone();
    with_ws_mut(state, |ws| {
        snooze(ws, &hlc, node, None).map_err(|e| e.to_string())
    })
}

/// Set (or clear, with an empty `rule`) a block's `remind::` property
/// and return the refreshed page.
///
/// Editing the rule resets the schedule from scratch — which falls out
/// for free, since the schedule is derived from the rule on every scan
/// rather than cached anywhere.
pub fn set_block_remind<S: AppHost>(
    state: &S,
    page_id: &str,
    block_id: &str,
    rule: &str,
) -> Result<PageView, String> {
    let page = parse_node_id(page_id)?;
    let node = parse_node_id(block_id)?;
    let value = {
        let trimmed = rule.trim();
        (!trimmed.is_empty()).then(|| PropValue::Text(trimmed.to_string()))
    };
    let hlc = state.hlc().clone();
    finish_in_page(state, page, |ws| {
        set_property(ws, &hlc, node, outl_md::remind::REMIND_KEY, value)
    })
}
