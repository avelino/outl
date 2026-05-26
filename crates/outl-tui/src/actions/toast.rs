//! Transient bottom-right notifications.
//!
//! Toasts are the "I want to see what happened" UI: a quick visual
//! confirmation for save, reload, undo, etc, without stealing the
//! footer's `status` line (which is reserved for chord prompts and
//! sticky warnings).
//!
//! API surface is small on purpose:
//! - [`App::toast`] — push a new notification with a kind + message.
//!   Default lifetime ~2.5 seconds.
//! - [`App::prune_toasts`] — called by the event loop each tick to
//!   evict expired entries so the stack doesn't grow unbounded.

use crate::state::{App, Toast, ToastKind};
use std::time::{Duration, Instant};

/// Default time a toast stays visible before the event loop sweeps
/// it out. Long enough to read a 4-word message; short enough that
/// stacking three save toasts doesn't drown out a real warning.
const TOAST_DEFAULT_LIFETIME_MS: u64 = 2_500;

/// Cap on how many toasts can coexist on screen at once. Anything
/// past this drops off the bottom of the stack — the newest is
/// always visible.
const TOAST_STACK_CAP: usize = 4;

impl App {
    /// Push a new toast with the default lifetime. Use the typed
    /// `kind` to drive the icon + color; the renderer maps semantics
    /// to colors consistently.
    pub(crate) fn toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        self.toast_for(kind, message, TOAST_DEFAULT_LIFETIME_MS);
    }

    /// Same as [`Self::toast`] but with a caller-supplied lifetime.
    /// Useful when the message needs a beat extra ("compiled in 4.2s,
    /// running…") or, conversely, should flash and disappear.
    pub(crate) fn toast_for(
        &mut self,
        kind: ToastKind,
        message: impl Into<String>,
        lifetime_ms: u64,
    ) {
        let toast = Toast {
            message: message.into(),
            kind,
            until: Instant::now() + Duration::from_millis(lifetime_ms),
        };
        self.toasts.push(toast);
        // Evict the oldest from the front so the stack is bounded.
        // `remove(0)` is O(n) but `n` is tiny (≤ TOAST_STACK_CAP).
        while self.toasts.len() > TOAST_STACK_CAP {
            self.toasts.remove(0);
        }
    }

    /// Sweep expired toasts. Called from the event loop on every
    /// tick (after `event::poll` returns, before draw). Returns the
    /// number removed so the caller can decide whether to force a
    /// redraw — the toast region looks wrong when an entry stays
    /// frozen past its expiry.
    pub(crate) fn prune_toasts(&mut self) -> usize {
        let now = Instant::now();
        let before = self.toasts.len();
        self.toasts.retain(|t| t.until > now);
        before - self.toasts.len()
    }
}
