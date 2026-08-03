//! Turn "what came due" into the DTO the GUI clients ship to the OS.
//!
//! The fired log and the due-scan itself live in
//! `outl_actions::reminders::fired` — every client delivers (the TUI
//! through an OSC 9 escape), so that logic cannot sit behind the Tauri
//! layer. What is left here is the DTO mapping and pulling the config
//! + workspace off the [`AppHost`].

use outl_actions::clock;
use outl_actions::reminders::take_due as actions_take_due;

use crate::commands::reminders::ReminderDto;
use crate::helpers::with_ws;
use crate::host::AppHost;

/// Everything due at-or-before now, as DTOs, with this device's fired
/// log already updated.
///
/// Returns empty — doing no work beyond the config read — when the
/// device has reminders switched off, so a caller can poll
/// unconditionally on a timer.
pub fn take_due<S: AppHost>(state: &S) -> Vec<ReminderDto> {
    let cfg = outl_config::load();
    if !cfg.reminders.enabled {
        return Vec::new();
    }
    let Ok(root) = state.storage_root() else {
        return Vec::new();
    };
    let now = clock::now_local().naive_local();
    with_ws(state, |ws| {
        Ok(actions_take_due(
            ws,
            &root,
            cfg.reminders.quiet_window(),
            now,
        ))
    })
    .unwrap_or_default()
    .into_iter()
    .map(ReminderDto::from)
    .collect()
}
