//! Normal-mode key handler: outline navigation, structural ops,
//! chord recognition, mode switches, and the in-Normal sidebar +
//! help intercepts.
//!
//! Three layers of dispatch run before the main `match key.code`:
//! the help-popup intercept (swallows every key but tab switches),
//! the sidebar intercept (when focus is inside the sidebar), and
//! the chord accumulator (`d`/`g`/`y`/`q` arm for a follow-up key).
//! Everything past the chord block is bare-key handling.

use crate::state::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) fn handle_normal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // Help popup owns the keyboard exclusively while open. Any key
    // that isn't a tab switch / scroll / close is *swallowed* — we
    // don't want `j` to move the outline behind the popup while the
    // user thinks they're scrolling help.
    if app.show_help {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                if app.help_tab == 0 {
                    app.help_tab = crate::view::HELP_TABS.len() - 1;
                } else {
                    app.help_tab -= 1;
                }
                app.help_scroll = 0; // new tab → top
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
                app.help_tab = (app.help_tab + 1) % crate::view::HELP_TABS.len();
                app.help_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.help_scroll = app.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                app.help_scroll = app.help_scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                app.help_scroll = app.help_scroll.saturating_sub(10);
            }
            KeyCode::Char('g') | KeyCode::Home => {
                app.help_scroll = 0;
            }
            KeyCode::Char('G') | KeyCode::End => {
                // Big number — the renderer clamps against the
                // actual body length when it draws, so we don't need
                // to know the count here.
                app.help_scroll = u16::MAX / 2;
            }
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_help = false;
                app.help_scroll = 0;
            }
            _ => {
                // Swallow every other key. The popup has focus; the
                // outline behind it must not react.
            }
        }
        return Ok(false);
    }

    // Sidebar intercept: while focus is inside the sidebar, j/k
    // navigate the focused section, Tab cycles sections, Enter opens
    // the item, Esc returns focus to the outline (sidebar stays
    // visible). `\` always closes the sidebar entirely, handled
    // further down in the Normal handler.
    if app.sidebar_focus.is_some() {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                app.sidebar_move(1);
                return Ok(false);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.sidebar_move(-1);
                return Ok(false);
            }
            KeyCode::Char('g') => {
                app.sidebar_cursor = 0;
                return Ok(false);
            }
            KeyCode::Char('G') => {
                app.sidebar_move(i32::MAX / 2);
                return Ok(false);
            }
            KeyCode::Tab => {
                app.sidebar_cycle_section(true);
                return Ok(false);
            }
            KeyCode::BackTab => {
                app.sidebar_cycle_section(false);
                return Ok(false);
            }
            KeyCode::Enter => {
                app.sidebar_activate()?;
                return Ok(false);
            }
            KeyCode::Esc => {
                app.sidebar_blur();
                return Ok(false);
            }
            KeyCode::Char('\\') => {
                app.sidebar_close();
                return Ok(false);
            }
            _ => {}
        }
    }

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
            ('g', KeyCode::Char('p')) => {
                // `gp` = toggle the `pinned::` page property.
                // Mnemonic: "go pin". Chose a chord (not bare `P`)
                // because `P` is already paste-before in Normal
                // mode — overloading it would surprise yanker
                // muscle memory.
                app.toggle_pinned();
                return Ok(false);
            }
            ('y', KeyCode::Char('y')) => {
                app.yank_current();
                return Ok(false);
            }
            ('y', KeyCode::Char('r')) => {
                // `yr` — yank the **block ref handle** of the
                // currently selected block. Surfaces `((blk-XXXXXX))`
                // in the status line and stashes it in
                // `last_yanked_ref` so a later paste/insert command
                // can use it.
                app.yank_current_ref();
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
        // Fold / unfold the selected block. The renderer's triangle
        // marker (▶/▼) is the visual confirmation. No-op when the
        // block has no sidecar entry yet (see
        // `App::toggle_collapse_selected`).
        KeyCode::Char('c') => app.toggle_collapse_selected(),
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
        // Toggle the left sidebar (mini-calendar, pinned, recent).
        // Default off — `\` opts in. Opening jumps focus straight to
        // the first non-empty section (Pinned by default), so the
        // user can immediately `j/k` through items and `Enter` to
        // open — no extra Tab to "enter" the sidebar.
        KeyCode::Char('\\') => {
            if app.show_sidebar {
                app.sidebar_close();
            } else {
                app.sidebar_open_focused();
            }
        }
        // Enter Visual mode (vim-style: V selects entire blocks).
        KeyCode::Char('V') => app.enter_visual(),
        _ => {}
    }
    Ok(false)
}
