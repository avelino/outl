//! The reminders overlay: author a `remind::` rule, inspect every
//! scheduled one, snooze or complete from the list.
//!
//! **The TUI never delivers a notification.** A terminal session has
//! no background presence, so there is nothing to fire into when the
//! user closes the pane. What it does have is the fastest authoring
//! surface in the product, so `g r` / `g R` write the rule and `g n`
//! shows what the phone and the laptop will deliver.
//!
//! Every schedule question routes to `outl_actions::reminders` — the
//! same function the GUI clients call. Nothing about *when* a rule
//! fires is decided in this file.

use chrono::Duration;
use outl_actions::reminders::{scan_reminders, snooze_until, FiredLog};
use outl_core::property::PropValue;

use crate::state::{App, Focus, Mode, Overlay, RemindersState, ToastKind};

/// The "nag me" preset behind `g R`, spelled in the `remind::`
/// grammar. The desktop hardcodes the same string in
/// `action-handlers.ts` — a constant can't cross the Rust/TS boundary
/// any more than a DTO field can, and a Tauri round-trip to fetch a
/// literal would be worse than the duplication.
const NAG_PRESET: &str = "now every 1h until DONE";

/// Default rule `g r` writes — one fire, this morning. Small on
/// purpose: the user is expected to edit the property line right
/// after, and a bare anchor is the shortest rule that does something.
const DEFAULT_RULE: &str = "9am";

impl App {
    /// `g r` — attach a starter `remind::` to the selected block.
    ///
    /// Idempotent in spirit: on a block that already has a rule this
    /// reports the existing one instead of overwriting it, so the
    /// chord can't silently discard a carefully typed schedule.
    pub(crate) fn insert_remind(&mut self) {
        self.write_remind(DEFAULT_RULE, false);
    }

    /// `g R` — the "nag me" preset, in one chord.
    ///
    /// Unlike `g r` this **does** overwrite: asking for the nag preset
    /// is an explicit escalation of whatever was there.
    pub(crate) fn insert_remind_nag(&mut self) {
        self.write_remind(NAG_PRESET, true);
    }

    fn write_remind(&mut self, rule: &str, overwrite: bool) {
        if !matches!(self.focus, Focus::Outline) || !matches!(self.mode, Mode::Normal) {
            return;
        }
        let Some(&node) = self.id_by_flat.get(self.selected) else {
            // Same guard the collapse chord uses: a brand-new bullet
            // has no sidecar entry until the next save, and a property
            // needs a node id to hang on.
            self.toast(
                ToastKind::Info,
                "block has no sidecar entry yet; save first",
            );
            return;
        };
        if !overwrite {
            if let Some(PropValue::Text(existing)) = self
                .workspace
                .tree()
                .property(node, outl_md::remind::REMIND_KEY)
                .cloned()
            {
                self.toast(ToastKind::Info, format!("already reminding: {existing}"));
                return;
            }
        }
        let hlc = self.hlc.clone();
        match outl_actions::set_property(
            &mut self.workspace,
            &hlc,
            node,
            outl_md::remind::REMIND_KEY,
            Some(PropValue::Text(rule.to_string())),
        ) {
            Ok(()) => {
                // The property lives in the op log now; re-read the page
                // so the `.md` projection (and the rendered property
                // line) reflects it in this frame.
                self.load_current_no_autorun();
                self.toast(ToastKind::Info, format!("remind:: {rule}"));
            }
            Err(e) => self.toast(ToastKind::Error, format!("could not set remind:: — {e}")),
        }
    }

    /// `g n` — open the reminders overlay.
    pub(crate) fn open_reminders(&mut self) {
        self.show_reminders(0);
    }

    /// Move the overlay cursor. `delta` is signed; the cursor clamps
    /// rather than wrapping, matching every other TUI list.
    pub(crate) fn move_reminders_cursor(&mut self, delta: i32) {
        let Some(Overlay::Reminders(ref mut r)) = self.overlay else {
            return;
        };
        if r.all.is_empty() {
            return;
        }
        let last = r.all.len() - 1;
        let next = (r.selected as i32 + delta).clamp(0, last as i32);
        r.selected = next as usize;
    }

    /// `s` in the overlay — snooze the highlighted reminder one hour.
    ///
    /// Writes `Op::SnoozeRemind`, so the user's phone goes quiet too.
    pub(crate) fn snooze_selected_reminder(&mut self) {
        let Some(node) = self.selected_reminder_block() else {
            return;
        };
        let until = outl_actions::clock::now_local().naive_local() + Duration::hours(1);
        let hlc = self.hlc.clone();
        match snooze_until(&mut self.workspace, &hlc, node, until) {
            Ok(()) => {
                self.toast(ToastKind::Info, "snoozed 1h (every device)");
                self.refresh_reminders();
            }
            Err(e) => self.toast(ToastKind::Error, format!("could not snooze — {e}")),
        }
    }

    /// `Enter` in the overlay — jump to the reminder's page.
    pub(crate) fn open_selected_reminder(&mut self) {
        let slug = match &self.overlay {
            Some(Overlay::Reminders(r)) => r.all.get(r.selected).map(|x| x.page_slug.clone()),
            _ => None,
        };
        let Some(slug) = slug else { return };
        self.overlay = None;
        if let Err(e) = self.open_page_by_name(&slug) {
            self.toast(ToastKind::Error, format!("could not open {slug} — {e}"));
        }
    }

    /// Re-scan after a mutation so the overlay reflects what landed.
    fn refresh_reminders(&mut self) {
        let Some(Overlay::Reminders(r)) = &self.overlay else {
            return;
        };
        self.show_reminders(r.selected);
    }

    /// Scan the workspace and (re)open the overlay on it.
    ///
    /// `want` is the row to land on; it is clamped, because a snooze
    /// re-sorts the list and the old index may now point past the end.
    /// The scan passes an empty [`FiredLog`] — the overlay answers
    /// "what is scheduled", never "what did this device deliver".
    fn show_reminders(&mut self, want: usize) {
        let quiet = outl_config::load().reminders.quiet_window();
        let now = outl_actions::clock::now_local().naive_local();
        let all = scan_reminders(&self.workspace, &FiredLog::new(), quiet, now);
        let selected = want.min(all.len().saturating_sub(1));
        self.overlay = Some(Overlay::Reminders(RemindersState { all, selected }));
    }

    fn selected_reminder_block(&self) -> Option<outl_core::id::NodeId> {
        match &self.overlay {
            Some(Overlay::Reminders(r)) => r.all.get(r.selected).map(|x| x.block_id),
            _ => None,
        }
    }
}
