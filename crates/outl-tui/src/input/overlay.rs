//! Overlay key router and the four overlay-specific handlers.
//!
//! Overlays are modal: while one is open, Normal/Insert/Visual mode
//! handlers don't run — every key goes through here.
//!
//! - **QuickSwitch** — fuzzy page/journal picker (`Ctrl+P`).
//! - **Search** — workspace text search (via `/search` slash command).
//! - **Command** — vim-style `:command` palette.
//! - **Slash** — Notion-style `/` menu, surface for built-in and
//!   plugin commands.

use crate::state::{App, Overlay};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Route a keystroke to whichever overlay is currently open.
///
/// Returns `Ok(true)` when the caller should exit the event loop.
pub(crate) fn handle_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match &app.overlay {
        Some(Overlay::QuickSwitch(_)) => handle_quick_switch_key(app, key),
        Some(Overlay::Search(_)) => handle_search_overlay_key(app, key),
        Some(Overlay::Command(_)) => handle_command_overlay_key(app, key),
        Some(Overlay::Slash(_)) => handle_slash_overlay_key(app, key),
        Some(Overlay::Error(_)) => {
            // Modal error popup: any key dismisses. Special-case Ctrl+C
            // so it still quits the whole TUI.
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(true);
            }
            app.overlay = None;
            Ok(false)
        }
        None => Ok(false),
    }
}

fn handle_quick_switch_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => app.accept_quick_switch()?,
        KeyCode::Up => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.selected = qs.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                if qs.selected + 1 < qs.candidates.len() {
                    qs.selected += 1;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.query.pop();
            }
            app.refresh_quick_switch();
        }
        KeyCode::Char(c) => {
            if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
                qs.query.push(c);
            }
            app.refresh_quick_switch();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_search_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => app.accept_search()?,
        KeyCode::Up => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.selected = s.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                if s.selected + 1 < s.hits.len() {
                    s.selected += 1;
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.query.pop();
            }
            app.refresh_search();
        }
        KeyCode::Char(c) => {
            if let Some(Overlay::Search(ref mut s)) = app.overlay {
                s.query.push(c);
            }
            app.refresh_search();
        }
        _ => {}
    }
    Ok(false)
}

fn handle_command_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => {
            let buf = if let Some(Overlay::Command(ref c)) = app.overlay {
                c.buffer.clone()
            } else {
                return Ok(false);
            };
            app.overlay = None;
            return run_command(app, &buf);
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Command(ref mut c)) = app.overlay {
                c.buffer.pop();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(Overlay::Command(ref mut c)) = app.overlay {
                c.buffer.push(ch);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_slash_overlay_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => return app.accept_slash(),
        KeyCode::Up => {
            app.slash_select_prev();
        }
        KeyCode::Down => {
            app.slash_select_next();
        }
        KeyCode::Backspace => {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query.pop();
            }
            app.refresh_slash();
        }
        KeyCode::Char(c) => {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query.push(c);
            }
            app.refresh_slash();
        }
        _ => {}
    }
    Ok(false)
}

/// Execute a `:command` from the command bar. Returns `Ok(true)` when
/// the command quits the app.
///
/// Routes everything through the `command_registry`. The vim palette
/// and the `/` slash menu share that registry, so a plugin that
/// registers a new command shows up in both surfaces without code
/// duplication here.
fn run_command(app: &mut App, line: &str) -> Result<bool> {
    let registry = app.command_registry.clone();
    registry.dispatch(app, line)
}
