//! Key-event routing: turn a [`crossterm::event::KeyEvent`] into a
//! method call on `crate::state::App`.
//!
//! Every function here is a `handle_*_key`: they pattern-match a key,
//! decide which action to invoke, and return. They never render; they
//! never read or write files directly; they delegate everything to
//! methods on `App` (defined in [`crate::actions`]).
//!
//! Keymap *documentation* (the table users care about) lives in
//! [`crate::keymap`].
//!
//! ## Multi-line editing inside a block (Insert mode)
//!
//! `Alt+Enter` inserts a soft newline — the cursor stays inside the
//! same block, so users can write multi-line bullets and fenced code
//! blocks (e.g. ` ```lisp `…` ``` `). Plain `Enter` commits the block
//! and creates a sibling below.
//!
//! Terminals that speak the kitty keyboard protocol (kitty, wezterm,
//! alacritty, Ghostty, foot, recent iTerm2) also accept `Shift+Enter`
//! for the same action; older terminals (Terminal.app, plain xterm)
//! collapse `Shift+Enter` into `Enter`, which is why `Alt+Enter` is
//! the portable fallback. The keyboard-enhancement push happens in
//! [`crate::runtime`].

use crate::actions::cycle_todo_inline;
use crate::state::{App, Mode, Overlay};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Is `cursor` (char index) sitting inside an *open* fenced code block?
///
/// "Open" here is from the *Enter-key perspective*: would pressing
/// Enter right now keep the cursor inside a fence?
///
/// Counts ` ``` ` markers on every line up to and **including** the
/// cursor's line. A `` ``` `` opener on the cursor's own line opens the
/// fence: the very next Enter starts the body. A `` ``` `` closer on
/// the cursor's line *also* counts — once the user types the closing
/// fence, the next Enter exits.
fn cursor_inside_open_fence(text: &str, cursor: usize) -> bool {
    let cursor_line = text.chars().take(cursor).filter(|&c| c == '\n').count();
    let mut open = false;
    for (idx, line) in text.split('\n').enumerate() {
        if idx > cursor_line {
            break;
        }
        if line.trim_start().starts_with("```") {
            open = !open;
        }
    }
    open
}

pub(crate) fn handle_visual_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('V') | KeyCode::Char('v') => app.mode = Mode::Normal,
        KeyCode::Char('d') | KeyCode::Char('x') => app.delete_visual_range(),
        KeyCode::Char('y') => app.yank_visual_range(),
        KeyCode::Tab => app.indent_visual_range(),
        KeyCode::BackTab => app.outdent_visual_range(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        _ => {}
    }
    Ok(())
}

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
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.selected = s.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                if s.selected + 1 < s.candidates.len() {
                    s.selected += 1;
                }
            }
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

pub(crate) fn handle_normal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Chord handling: a previous 'd' / 'g' / 'y' is pending.
    if let Some(pending) = app.pending_chord.take() {
        match (pending, key.code) {
            ('d', KeyCode::Char('d')) => {
                app.delete_current();
                return Ok(false);
            }
            ('g', KeyCode::Char('j')) => {
                app.go_today()?;
                return Ok(false);
            }
            ('g', KeyCode::Char('x')) => {
                // `gx` = run the code block under the cursor.
                // Mnemonic: "go execute".
                app.run_current_block();
                return Ok(false);
            }
            ('g', KeyCode::Char('g')) => {
                // `gg` = jump to the first block (vim convention).
                app.move_selection(i32::MIN / 2);
                return Ok(false);
            }
            ('y', KeyCode::Char('y')) => {
                app.yank_current();
                return Ok(false);
            }
            ('q', KeyCode::Char('q')) => {
                // Confirmed quit. The first `q` armed the chord; the
                // second within one keystroke window seals the deal.
                return Ok(true);
            }
            _ => {} // fall through to normal handling
        }
    }

    match key.code {
        KeyCode::Char('q') => {
            // Don't quit on a single `q` — too easy to hit by
            // accident, takes the user out of their editor with no
            // way to recover. Arm a chord instead; a *second* `q`
            // (or `:quit` / `Ctrl+C`) closes the TUI.
            app.pending_chord = Some('q');
            app.status = "press q again to quit".into();
            return Ok(false);
        }
        KeyCode::Char('?') => app.show_help = !app.show_help,
        // `Ctrl+T` is the portable alias for `Ctrl+Enter` (TODO toggle)
        // — tmux without `extended-keys` and Terminal.app collapse
        // `Ctrl+Enter` to plain `Enter`, so the chord we *want* never
        // arrives. Must come BEFORE the bare `t` arm.
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => app.toggle_todo(),
        // `t` and `Home` were sharing one binding: `t` jumps to today,
        // `Home` should move the cursor to the start of the current
        // block. Split them.
        KeyCode::Char('t') => app.go_today()?,
        KeyCode::Char('[') => app.shift_journal(-1)?,
        KeyCode::Char(']') => app.shift_journal(1)?,
        KeyCode::Char('g') => app.pending_chord = Some('g'),
        // Half-page jumps must be matched *before* the plain `d`/`u`
        // chord arms — match guards are tried in order and a guard-less
        // `Char('d')` arm would win otherwise.
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.move_selection((app.viewport_height.max(2) / 2) as i32)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.move_selection(-((app.viewport_height.max(2) / 2) as i32))
        }
        KeyCode::Char('d') => app.pending_chord = Some('d'),
        KeyCode::Char('y') => app.pending_chord = Some('y'),
        // Paste from the yank register. Plain `p`/`P` (no Ctrl, no
        // Alt) — Ctrl+P is the quick switcher and must beat this.
        KeyCode::Char('p') if key.modifiers.is_empty() => app.paste_after(),
        KeyCode::Char('P') if key.modifiers == KeyModifiers::SHIFT || key.modifiers.is_empty() => {
            app.paste_before()
        }
        KeyCode::Tab => {
            app.indent_current();
        }
        KeyCode::BackTab => {
            app.outdent_current();
        }
        // `Ctrl+Enter` toggles TODO. Must come BEFORE the plain `Enter`
        // arm or the open-ref handler eats the chord.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => app.toggle_todo(),
        // Enter is overloaded: if the cursor is sitting on a `[[ref]]`
        // / `#tag` / journal date, open it. Otherwise enter Insert
        // mode (the original behavior).
        KeyCode::Enter if !app.try_open_under_cursor()? => {
            app.enter_insert(false);
        }
        KeyCode::Enter => {}
        KeyCode::Char('i') => app.enter_insert(false),
        KeyCode::Char('I') => app.enter_insert(true),
        KeyCode::Char('o') => app.create_block_below(),
        KeyCode::Char('O') => app.create_block_above(),
        // Vertical navigation: blocks. `j`/`k` are vim conventions.
        // Alt + arrows drag the current block (more discoverable than capital K/J).
        KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => app.move_block_up(),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => app.move_block_down(),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        // Page-sized jumps. `viewport_height` is set by the renderer
        // on each draw, so it tracks the actual terminal size.
        KeyCode::PageDown => app.move_selection(app.viewport_height.max(1) as i32),
        KeyCode::PageUp => app.move_selection(-(app.viewport_height.max(1) as i32)),
        // `G` = last block (vim convention).
        KeyCode::Char('G') => app.move_selection(i32::MAX / 2),
        // Horizontal cursor inside the current block.
        KeyCode::Left | KeyCode::Char('h') => app.move_cursor_col(-1),
        KeyCode::Right | KeyCode::Char('l') => app.move_cursor_col(1),
        KeyCode::Char('0') | KeyCode::Home => app.cursor_to_home(),
        KeyCode::Char('$') | KeyCode::End => app.cursor_to_end(),
        KeyCode::Char('w') => app.cursor_word_right(),
        KeyCode::Char('b') => app.cursor_word_left(),
        // Block reordering (vim-ish: capital J/K drag the block).
        KeyCode::Char('K') => app.move_block_up(),
        KeyCode::Char('J') => app.move_block_down(),
        // History.
        KeyCode::Char('u') => app.undo(),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => app.redo(),
        // Overlays.
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.open_quick_switch();
            app.refresh_quick_switch();
        }
        // `/` opens the Notion-style slash command menu. Workspace
        // search lives there too (`/search`) — one extra keystroke,
        // full discoverability, future plugin commands appear
        // automatically.
        KeyCode::Char('/') => {
            app.open_slash();
            app.refresh_slash();
        }
        KeyCode::Char(':') => app.open_command(),
        // Walk through the last `/` search results without reopening
        // the overlay. `n` next, `N` previous (vim convention).
        KeyCode::Char('n') => app.search_next()?,
        KeyCode::Char('N') => app.search_prev()?,
        // Toggle backlinks panel.
        KeyCode::Char('B') => app.show_backlinks = !app.show_backlinks,
        // Enter Visual mode (vim-style: V selects entire blocks).
        KeyCode::Char('V') => app.enter_visual(),
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_insert_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Autocomplete intercepts navigation/Enter/Esc.
    if app.autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.autocomplete = None;
                return Ok(());
            }
            KeyCode::Enter => {
                app.accept_autocomplete();
                return Ok(());
            }
            KeyCode::Tab => {
                app.accept_autocomplete();
                return Ok(());
            }
            KeyCode::Up => {
                if let Some(ac) = &mut app.autocomplete {
                    ac.selected = ac.selected.saturating_sub(1);
                }
                return Ok(());
            }
            KeyCode::Down => {
                if let Some(ac) = &mut app.autocomplete {
                    if !ac.candidates.is_empty() && ac.selected + 1 < ac.candidates.len() {
                        ac.selected += 1;
                    }
                }
                return Ok(());
            }
            _ => {} // fall through to the regular handler
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.commit_insert();
        }
        // `Ctrl+Enter` toggles TODO directly inside the buffer — no
        // commit, no new block, cursor preserved relative to text.
        // `Ctrl+T` is the portable alias for terminals/multiplexers
        // (tmux, Terminal.app) that collapse `Ctrl+Enter` into plain
        // `Enter`.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                cycle_todo_inline(buffer);
            }
        }
        KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                cycle_todo_inline(buffer);
            }
        }
        // Soft newline inside the same block.
        //
        // Three ways to trigger it:
        //   1. `Shift+Enter` — kitty-protocol-aware terminals only
        //      (kitty, wezterm, alacritty, Ghostty, foot, iTerm2 ≥ 3.5).
        //      Older terminals (Terminal.app, plain xterm) collapse it
        //      into `Enter`.
        //   2. `Alt+Enter` / `Ctrl+J` — portable fallbacks. Alt depends
        //      on the terminal's "Option as Meta" setting on macOS;
        //      `Ctrl+J` (ASCII LF) always works.
        //   3. **Auto-detect**: plain `Enter` while inside an *open*
        //      fenced code block (a `` ``` `` opener without a matching
        //      closer above the cursor) — keeps users typing
        //      ```lisp\n(+ 1 2)\n``` without any modifier at all.
        //
        // The kitty-protocol push happens in [`crate::runtime`].
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.insert_char('\n');
            }
        }
        // `Ctrl+J` = ASCII LF, distinguishable from `Enter` (CR) in
        // every terminal. Last-resort portable soft newline. Must come
        // before `Ctrl+...` matches that handle other letters.
        KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.insert_char('\n');
            }
        }
        KeyCode::Enter => {
            // Plain Enter — `\n` inside an open code fence, sibling
            // block otherwise.
            let in_fence = matches!(&app.mode, Mode::Insert { buffer, .. }
                if cursor_inside_open_fence(&buffer.as_string(), buffer.cursor));
            if in_fence {
                if let Mode::Insert { buffer, .. } = &mut app.mode {
                    buffer.insert_char('\n');
                }
            } else {
                app.create_block_below();
            }
        }
        KeyCode::Tab => {
            app.indent_current();
        }
        KeyCode::BackTab => {
            app.outdent_current();
        }
        KeyCode::Backspace => {
            let should_delete = if let Mode::Insert { buffer, .. } = &mut app.mode {
                if buffer.cursor == 0 && buffer.is_empty() {
                    true
                } else {
                    buffer.delete_back();
                    false
                }
            } else {
                false
            };
            if should_delete {
                app.abort_insert();
                app.delete_current();
                app.move_selection(-1);
            }
        }
        KeyCode::Delete => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.delete_forward();
            }
        }
        KeyCode::Left => {
            // Inside the buffer (including across `\n` in a multi-line
            // block), just decrement the char cursor. At buffer start
            // (cursor == 0), spill into the *previous* block and place
            // the cursor at its end — so holding Left walks the user
            // back through the document like a single document.
            let at_start = matches!(&app.mode, Mode::Insert { buffer, .. } if buffer.cursor == 0);
            if at_start && app.selected > 0 {
                app.commit_insert();
                app.move_selection(-1);
                app.enter_insert(false); // cursor lands at end
            } else if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_left();
            }
        }
        KeyCode::Right => {
            // Symmetric of Left: at buffer end, spill into the next
            // block with the cursor at its start.
            let at_end = matches!(&app.mode, Mode::Insert { buffer, .. }
                if buffer.cursor == buffer.len());
            let last_idx = app.flat_len.saturating_sub(1);
            if at_end && app.selected < last_idx {
                app.commit_insert();
                app.move_selection(1);
                app.enter_insert(true); // cursor at start
            } else if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_right();
            }
        }
        KeyCode::Home => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_home();
            }
        }
        KeyCode::End => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                buffer.move_end();
            }
        }
        KeyCode::Char(ch) => {
            if let Mode::Insert { buffer, .. } = &mut app.mode {
                match ch {
                    '(' => buffer.insert_pair('(', ')'),
                    '[' => buffer.insert_pair('[', ']'),
                    '{' => buffer.insert_pair('{', '}'),
                    _ => buffer.insert_char(ch),
                }
            }
        }
        _ => {}
    }
    // After any key, re-check whether the cursor is inside a `[[` or `#`
    // token. Cheap because page count is small.
    app.maybe_update_autocomplete();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cursor_inside_open_fence;

    #[test]
    fn empty_buffer_is_not_inside_fence() {
        assert!(!cursor_inside_open_fence("", 0));
    }

    #[test]
    fn plain_text_is_not_inside_fence() {
        let t = "first line\nsecond line";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_inside_open_fence_after_opener() {
        // `` ```lisp\n<cursor> ``
        let t = "```lisp\n";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_inside_open_fence_after_opener_and_body() {
        let t = "```lisp\n(+ 1 2)\n";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_after_closed_fence_is_outside() {
        // Both opener and closer are above the cursor.
        let t = "```lisp\n(+ 1 2)\n```\n";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_on_opener_line_is_inside() {
        // User typed ` ```lisp ` and is about to press Enter to start
        // the body. From the Enter-key perspective, the fence is now
        // open: the next newline must be a soft newline.
        let t = "```lisp";
        assert!(cursor_inside_open_fence(t, t.chars().count()));
    }

    #[test]
    fn cursor_on_closer_line_is_outside() {
        // User just finished typing the closing `` ``` ``. From the
        // Enter-key perspective the closer already balanced the
        // opener — the next Enter should exit the block, not insert
        // another soft newline.
        let t = "```lisp\n(+ 1 2)\n```";
        assert!(!cursor_inside_open_fence(t, t.chars().count()));
    }
}
