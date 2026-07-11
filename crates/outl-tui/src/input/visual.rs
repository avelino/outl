//! Visual mode key handler.
//!
//! Visual mode operates on a contiguous range of outline blocks. Keys
//! that aren't `d`/`x`/`y`/`Tab`/`BackTab` either move the selection
//! (extending the range) or exit to Normal.

use crate::state::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn handle_visual_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        // Exit Visual via `exit_visual` so `last_visual` is captured —
        // a subsequent `gv` in Normal mode restores this range.
        KeyCode::Esc | KeyCode::Char('V') | KeyCode::Char('v') => app.exit_visual(),
        KeyCode::Char('d') | KeyCode::Char('x') => app.delete_visual_range(),
        KeyCode::Char('y') => app.yank_visual_range(),
        // `Tab` / `Shift-Tab` indent / outdent — vim ergonomics use
        // `>` / `<` for the same effect. Both fire the same range op
        // so muscle memory works either way; vim purists get `>`/`<`
        // without losing the `Tab` discoverability.
        KeyCode::Tab | KeyCode::Char('>') => app.indent_visual_range(),
        KeyCode::BackTab | KeyCode::Char('<') => app.outdent_visual_range(),
        // `Alt`+arrows drag the whole range among its siblings —
        // mirrors the single-block `Alt`+arrows in Normal mode. The
        // plain arrows below extend the selection, so `Alt` is what
        // separates "reorder the range" from "grow the range".
        KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => app.move_up_visual_range(),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => app.move_down_visual_range(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        _ => {}
    }
    Ok(())
}
