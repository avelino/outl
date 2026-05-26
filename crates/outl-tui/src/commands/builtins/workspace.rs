//! Workspace / chrome commands: navigation, save, reload, help, quit.
//!
//! Anything that doesn't insert text and doesn't touch a single
//! block's properties — these are the "I want to do something with
//! the *editor*" verbs.

use anyhow::Result;

use super::super::SlashCommand;
use crate::state::App;

// ---------------------------------------------------------------------------
// today — jump to today's journal
// ---------------------------------------------------------------------------

pub struct TodayCommand;
impl SlashCommand for TodayCommand {
    fn name(&self) -> &'static str {
        "today"
    }
    fn description(&self) -> &'static str {
        "Jump to today's journal"
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.go_today()?;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// refresh — re-read the workspace from disk
// ---------------------------------------------------------------------------

pub struct RefreshCommand;
impl SlashCommand for RefreshCommand {
    fn name(&self) -> &'static str {
        "refresh"
    }
    fn description(&self) -> &'static str {
        "Re-read the workspace from disk (rebuilds index)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["reload", "r"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.refresh_workspace();
        app.status = "refreshed".into();
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// write — force-save the current page
// ---------------------------------------------------------------------------

pub struct WriteCommand;
impl SlashCommand for WriteCommand {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Save the current page to disk"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["w", "save"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.save();
        app.status = "saved".into();
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// help — toggle the help popup
// ---------------------------------------------------------------------------

pub struct HelpCommand;
impl SlashCommand for HelpCommand {
    fn name(&self) -> &'static str {
        "help"
    }
    fn description(&self) -> &'static str {
        "Toggle the help popup"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["h"]
    }
    fn execute(&self, app: &mut App, _args: &str) -> Result<bool> {
        app.show_help = true;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// quit — close the TUI
// ---------------------------------------------------------------------------

pub struct QuitCommand;
impl SlashCommand for QuitCommand {
    fn name(&self) -> &'static str {
        "quit"
    }
    fn description(&self) -> &'static str {
        "Close the TUI (commits any pending Insert first)"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["q", "exit"]
    }
    fn execute(&self, _app: &mut App, _args: &str) -> Result<bool> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// open — open (or create) a page by title
// ---------------------------------------------------------------------------

pub struct OpenCommand;
impl SlashCommand for OpenCommand {
    fn name(&self) -> &'static str {
        "open"
    }
    fn description(&self) -> &'static str {
        "Open (or create) a page by name — `open <name>`"
    }
    fn needs_args(&self) -> bool {
        true
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["o", "new", "n"]
    }
    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        if args.is_empty() {
            app.status = "usage: open <page name>".into();
            return Ok(false);
        }
        app.open_page_by_name(args)?;
        Ok(false)
    }
}
