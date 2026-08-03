//! The delivery loop: *which* reminders are due right now, and the
//! device-local memory of what this device already delivered.
//!
//! Split cleanly from [`crate::commands::reminders`]: that module
//! answers the frontend's questions, this one drives the OS
//! notification. Neither decides *when* a rule fires — both call
//! `outl_actions::reminders`.
//!
//! ## Why the fired log is a plain local file
//!
//! "I already buzzed you about this" is true of one device, not of the
//! workspace. Putting it in the op log would mean the phone firing
//! silences the laptop, which is the opposite of what a reminder is
//! for. So it lives at `<root>/.outl/reminders-fired.json` — a
//! **dotfile**, deliberately, so iCloud drops it and iroh never ships
//! it (same policy as the snapshot cache and the op-log index).
//!
//! Entries older than [`FIRED_TTL_DAYS`] are pruned on every save, so
//! the file can't grow without bound on a long-lived vault.
//!
//! Losing this file is harmless: the worst case is one duplicate
//! notification per active rule, never a missed one.

use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDate, NaiveDateTime};
use outl_actions::clock;
use outl_actions::reminders::{scan_reminders, FiredLog, FiredRecord, Reminder};
use outl_core::id::NodeId;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::commands::reminders::ReminderDto;
use crate::helpers::with_ws;
use crate::host::AppHost;

/// How long a fired record is kept before pruning.
pub const FIRED_TTL_DAYS: i64 = 7;

/// One delivered fire, in the on-disk shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FiredEntry {
    block: String,
    /// `YYYY-MM-DD` anchor date — one block with two dates is two
    /// independent schedules and must not share a counter.
    date: String,
    count: u32,
    /// Local ISO datetime of the last delivery.
    last: String,
}

/// `<root>/.outl/reminders-fired.json`.
pub fn fired_log_path(root: &Path) -> PathBuf {
    root.join(".outl").join("reminders-fired.json")
}

/// Read the device-local fired log. A missing / unreadable / malformed
/// file yields an empty log rather than an error: at worst the user
/// gets one duplicate banner, and refusing to schedule because a cache
/// file is corrupt would be strictly worse.
pub fn load_fired_log(root: &Path) -> FiredLog {
    let Ok(bytes) = std::fs::read(fired_log_path(root)) else {
        return FiredLog::new();
    };
    let entries: Vec<FiredEntry> = match serde_json::from_slice(&bytes) {
        Ok(e) => e,
        Err(e) => {
            warn!("reminders-fired.json is unreadable ({e}); starting fresh");
            return FiredLog::new();
        }
    };
    let mut out = FiredLog::new();
    for e in entries {
        let (Ok(block), Ok(date), Ok(last)) = (
            parse_block(&e.block),
            NaiveDate::parse_from_str(&e.date, "%Y-%m-%d"),
            NaiveDateTime::parse_from_str(&e.last, "%Y-%m-%dT%H:%M:%S"),
        ) else {
            continue;
        };
        out.insert(
            (block, date),
            FiredRecord {
                count: e.count,
                last,
            },
        );
    }
    out
}

/// Persist the fired log, pruning anything older than
/// [`FIRED_TTL_DAYS`]. Best-effort: a write failure is logged, never
/// propagated — it costs at most a duplicate notification.
pub fn save_fired_log(root: &Path, log: &FiredLog, now: NaiveDateTime) {
    let cutoff = now - Duration::days(FIRED_TTL_DAYS);
    let entries: Vec<FiredEntry> = log
        .iter()
        .filter(|(_, rec)| rec.last >= cutoff)
        .map(|((block, date), rec)| FiredEntry {
            block: block.to_string(),
            date: date.format("%Y-%m-%d").to_string(),
            count: rec.count,
            last: rec.last.format("%Y-%m-%dT%H:%M:%S").to_string(),
        })
        .collect();
    let path = fired_log_path(root);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("could not create {}: {e}", parent.display());
            return;
        }
    }
    match serde_json::to_vec(&entries) {
        Ok(bytes) => {
            if let Err(e) = outl_md::write_atomic(&path, &bytes) {
                warn!("could not write the fired log: {e}");
            }
        }
        Err(e) => warn!("could not serialize the fired log: {e}"),
    }
}

/// Everything due to fire at-or-before `now`, with the fired log
/// already updated and persisted.
///
/// Call this on a timer (and after every sync, since a peer's op can
/// make something newly due). Returns an empty vec — doing no work
/// beyond the config read — when the device has reminders switched
/// off, so a caller can poll unconditionally.
pub fn take_due<S: AppHost>(state: &S) -> Vec<ReminderDto> {
    let cfg = outl_config::load();
    if !cfg.reminders.enabled {
        return Vec::new();
    }
    let Ok(root) = state.storage_root() else {
        return Vec::new();
    };
    let now = clock::now_local().naive_local();
    let mut fired = load_fired_log(&root);

    let due: Vec<Reminder> = with_ws(state, |ws| {
        Ok(scan_reminders(
            ws,
            &fired,
            cfg.reminders.quiet_window(),
            now,
        ))
    })
    .unwrap_or_default()
    .into_iter()
    .filter(|r| r.next_fire.is_some_and(|t| t <= now))
    .collect();

    if due.is_empty() {
        return Vec::new();
    }
    for r in &due {
        let entry = fired
            .entry((r.block_id, r.anchor_date))
            .or_insert(FiredRecord {
                count: 0,
                last: now,
            });
        entry.count += 1;
        entry.last = now;
    }
    save_fired_log(&root, &fired, now);
    due.into_iter().map(ReminderDto::from).collect()
}

fn parse_block(s: &str) -> Result<NodeId, ()> {
    use std::str::FromStr;
    ulid::Ulid::from_str(s).map(NodeId).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .expect("valid date")
            .and_hms_opt(h, min, 0)
            .expect("valid time")
    }

    #[test]
    fn fired_log_round_trips() {
        let tmp = TempDir::new().expect("tempdir");
        let block = NodeId::new();
        let date = NaiveDate::from_ymd_opt(2026, 12, 12).expect("valid date");
        let mut log = FiredLog::new();
        log.insert(
            (block, date),
            FiredRecord {
                count: 3,
                last: at(2026, 12, 12, 15, 0),
            },
        );
        save_fired_log(tmp.path(), &log, at(2026, 12, 12, 16, 0));
        assert_eq!(load_fired_log(tmp.path()), log);
    }

    #[test]
    fn entries_past_the_ttl_are_pruned_on_save() {
        let tmp = TempDir::new().expect("tempdir");
        let stale = NodeId::new();
        let fresh = NodeId::new();
        let date = NaiveDate::from_ymd_opt(2026, 12, 1).expect("valid date");
        let mut log = FiredLog::new();
        log.insert(
            (stale, date),
            FiredRecord {
                count: 1,
                last: at(2026, 12, 1, 10, 0),
            },
        );
        log.insert(
            (fresh, date),
            FiredRecord {
                count: 1,
                last: at(2026, 12, 12, 10, 0),
            },
        );
        save_fired_log(tmp.path(), &log, at(2026, 12, 12, 11, 0));

        let back = load_fired_log(tmp.path());
        assert_eq!(back.len(), 1);
        assert!(back.contains_key(&(fresh, date)));
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_not_as_an_error() {
        // One duplicate banner beats refusing to schedule at all.
        let tmp = TempDir::new().expect("tempdir");
        let path = fired_log_path(tmp.path());
        std::fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
        std::fs::write(&path, b"{not json").expect("write");
        assert!(load_fired_log(tmp.path()).is_empty());
    }

    #[test]
    fn the_log_never_lands_on_the_sync_surface() {
        // A `.`-prefixed directory is what keeps iCloud from shipping
        // this device-local cache to every other device.
        let path = fired_log_path(Path::new("/w"));
        assert!(path.starts_with("/w/.outl"), "{}", path.display());
    }
}
