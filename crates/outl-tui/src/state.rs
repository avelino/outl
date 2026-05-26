//! Plain state types for the TUI — no behaviour, no rendering.
//!
//! Everything in here is a data definition: the `App` struct, mode and
//! overlay enums, history snapshots, the autocomplete and search
//! payloads. The actions that mutate them live in [`crate::actions`];
//! the key handlers that decide *when* to call them live in
//! [`crate::input`]; the rendering lives in [`crate::view`].
//!
//! Visibility is `pub(crate)` because every other module in this crate
//! reads or writes these types, but nothing outside `outl-tui` should.

use crate::edit_buffer::EditBuffer;
use crate::theme::Theme;
use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::{OutlineNode, ParsedPage};
use std::path::PathBuf;
use std::time::SystemTime;

pub(crate) const HELP_HINT_NORMAL: &str =
    "i edit  o new  h/l cursor  Enter open ref  K/J move  C-T TODO  u undo  ? help  q";
pub(crate) const HELP_HINT_INSERT: &str =
    "Esc commit  Enter new block  Ctrl+T TODO  Tab indent  Shift-Tab outdent";
pub(crate) const HELP_HINT_VISUAL: &str =
    "j/k extend  d/x delete  y yank  Tab indent  S-Tab outdent  Esc cancel";

/// Maximum entries in either history stack. Past this, oldest entries
/// drop off the front. Enough for many minutes of fluent editing without
/// becoming a memory issue.
pub(crate) const MAX_HISTORY: usize = 200;

pub(crate) const TODO_PREFIX: &str = "TODO ";
pub(crate) const DONE_PREFIX: &str = "DONE ";

/// Where Insert-mode edits get committed.
///
/// `CurrentPage` is the historical path: mutate `App.page` and save
/// through `current_path()`. `SourcePage` is the cross-page route used
/// when the user edits a backlink in place — the buffer commits
/// directly into the source page's `.md` without changing `App.view`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EditTarget {
    CurrentPage,
    SourcePage {
        path: PathBuf,
        /// Working copy of the source page's AST. Mutated by the
        /// buffer commit, then written back to `path` via
        /// `App::save_page`.
        page: ParsedPage,
    },
}

/// Insert vs Normal vs Visual.
///
/// Visual is a Normal sibling that adds a range anchor: every block
/// between `anchor` and the current selection is considered selected
/// for batch operations (delete, indent, outdent).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Mode {
    Normal,
    /// Editing the block at `path`. `block_path` is interpreted
    /// against `App.page` when `target == CurrentPage`, otherwise
    /// against `target.page` (the loaded source AST).
    Insert {
        target: EditTarget,
        block_path: Vec<usize>,
        buffer: EditBuffer,
        original_text: String,
    },
    /// Multi-block selection anchored at `anchor` (flat index). The
    /// other end of the range is `App::selected`.
    Visual {
        anchor: usize,
    },
}

/// Which view is current.
#[derive(Debug, Clone)]
pub(crate) enum View {
    Journal(NaiveDate),
    Page(PathBuf),
}

/// Where the cursor lives — inside the current page's outline (default)
/// or inside the inline backlinks section below it.
///
/// We carry focus as a *peer of `Mode`* rather than rewriting
/// `selected: usize` because the vast majority of action methods
/// (delete, indent, yank, visual range, undo snapshots) operate on the
/// current page only. Splitting `selected` into a Selection enum would
/// touch ~40 call sites for a feature that's really a different mode
/// with a different commit target.
///
/// `Focus::Backlink { idx, sub_path }`:
/// - `idx` indexes `app.index.backlinks(current_slug)`.
/// - `sub_path` is a DFS path *inside* `Backlink.source_block` — empty
///   means the source block itself; `[0]` its first child; etc.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Focus {
    #[default]
    Outline,
    Backlink {
        idx: usize,
        sub_path: Vec<usize>,
    },
}

/// One reversible step.
#[derive(Debug, Clone)]
pub(crate) struct HistorySnapshot {
    pub(crate) page: ParsedPage,
    pub(crate) selected: usize,
    pub(crate) cursor_col: usize,
    pub(crate) view_path: PathBuf,
}

/// Modal overlays shown above the main panes.
///
/// Mutually exclusive — only one at a time. Each one captures the key
/// stream while it's open; Esc closes any of them.
#[derive(Debug)]
pub(crate) enum Overlay {
    /// Fuzzy-search popup for opening pages and journals by title.
    QuickSwitch(QuickSwitchState),
    /// Workspace-wide search across all blocks.
    Search(SearchState),
    /// Vim-style command line (`:open`, `:theme`, `:q`...).
    Command(CommandState),
    /// Error / warning shown to the user. Modal — any key dismisses.
    /// Use this when the failure is detailed enough that the status
    /// line would truncate it (compile errors, traps, multi-line
    /// stderr).
    Error(ErrorState),
    /// Notion-style slash command menu: filterable list of actions
    /// the user can trigger. Discoverable in a way `:` (vim command
    /// palette) isn't — every command shows up with a description.
    Slash(SlashState),
}

/// One entry in the slash menu.
#[derive(Debug, Clone)]
pub(crate) struct SlashCommand {
    /// Name as typed: `prop`, `search`, `run`, ...
    pub(crate) name: &'static str,
    /// One-liner shown next to the name.
    pub(crate) description: &'static str,
    /// When true, accepting opens the vim command palette pre-filled
    /// with `<name> ` so the user can supply arguments
    /// (`prop priority high`). When false the command runs immediately.
    pub(crate) needs_args: bool,
}

/// State of the slash overlay.
#[derive(Debug)]
pub(crate) struct SlashState {
    pub(crate) query: String,
    /// Available commands. Filtered by fuzzy match against `query`.
    pub(crate) candidates: Vec<SlashCommand>,
    /// Index of the highlighted candidate.
    pub(crate) selected: usize,
}

/// Payload for [`Overlay::Error`].
#[derive(Debug)]
pub(crate) struct ErrorState {
    /// Short header — e.g. "rust failed to compile".
    pub(crate) title: String,
    /// Full body. Multi-line OK; the renderer wraps and scrolls.
    pub(crate) body: String,
}

/// One entry in the quick switcher result list.
#[derive(Debug, Clone)]
pub(crate) struct SwitchCandidate {
    /// Display label (title for pages, ISO date for journals).
    pub(crate) label: String,
    /// Slug or date, used to derive the filename on open.
    pub(crate) key: String,
    /// Where to open it.
    pub(crate) kind: SwitchKind,
    /// Last fuzzy score (for sort stability).
    pub(crate) score: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchKind {
    Page,
    Journal,
}

#[derive(Debug)]
pub(crate) struct QuickSwitchState {
    pub(crate) query: String,
    pub(crate) candidates: Vec<SwitchCandidate>,
    pub(crate) selected: usize,
}

/// One search hit — a single block matching the query.
#[derive(Debug, Clone)]
pub(crate) struct SearchHit {
    /// Display label (page title or journal date).
    pub(crate) page_label: String,
    /// `icon::` of the source page, if set — rendered before the
    /// label in the search overlay.
    pub(crate) page_icon: Option<String>,
    /// Which file the hit lives in.
    pub(crate) md_path: PathBuf,
    /// Snippet around the match.
    pub(crate) snippet: String,
    /// Block index inside the parsed page (DFS preorder).
    pub(crate) block_index: usize,
    /// Fuzzy score for ranking.
    pub(crate) score: i32,
}

#[derive(Debug)]
pub(crate) struct SearchState {
    pub(crate) query: String,
    pub(crate) hits: Vec<SearchHit>,
    pub(crate) selected: usize,
}

#[derive(Debug)]
pub(crate) struct CommandState {
    /// Current command buffer (without the leading `:`).
    pub(crate) buffer: String,
}

/// Persisted search-result navigation after the search overlay closes.
#[derive(Debug, Clone)]
pub(crate) struct LastSearch {
    pub(crate) hits: Vec<SearchHit>,
    /// Index of the most recently shown hit. `n` advances, `N` retreats.
    pub(crate) cursor: usize,
}

pub(crate) fn hit_count(ls: &Option<LastSearch>) -> usize {
    ls.as_ref().map(|x| x.hits.len()).unwrap_or(0)
}

/// Inline autocomplete trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutocompleteKind {
    /// User typed `[[`; pick a page title.
    PageRef,
    /// User typed `#`; pick a tag (any existing page name).
    Tag,
    /// User typed `/` at the start of a word; pick a slash command.
    /// On accept, the trigger + query are removed from the buffer
    /// and the command runs (or pops the `:` palette for arg entry).
    SlashCommand,
}

#[derive(Debug)]
pub(crate) struct AutocompleteState {
    pub(crate) kind: AutocompleteKind,
    /// Chars typed after the trigger (the query so far).
    pub(crate) query: String,
    /// Filtered candidates (page entries).
    pub(crate) candidates: Vec<String>,
    /// Highlighted index in `candidates`.
    pub(crate) selected: usize,
}

/// Application state.
pub(crate) struct App {
    pub(crate) workspace_root: PathBuf,
    pub(crate) workspace: Workspace,
    pub(crate) hlc: HlcGenerator,
    pub(crate) orphans_log: PathBuf,

    pub(crate) view: View,
    /// In-memory AST for the current view. The "truth" for what's drawn.
    pub(crate) page: ParsedPage,
    /// Flat index of the currently selected block in the outline (DFS preorder).
    pub(crate) selected: usize,
    /// Number of blocks (cached for selection bounds).
    pub(crate) flat_len: usize,
    /// Char-index cursor within the selected block in Normal mode.
    /// Lets users navigate inside a line and trigger "open under cursor"
    /// on a `[[ref]]`, `#tag`, or `[[YYYY-MM-DD]]`. Resets to 0 whenever
    /// the selected block changes.
    pub(crate) cursor_col: usize,

    /// All pages on disk. Used by the quick switcher (`Ctrl+P`) and
    /// `collect_switch_candidates` — the sidebar that used to render
    /// this list was removed; the data is still tracked for navigation.
    pub(crate) page_list: Vec<PathBuf>,

    pub(crate) mode: Mode,
    pub(crate) show_help: bool,
    pub(crate) pending_chord: Option<char>,
    pub(crate) status: String,

    /// Modal overlay (quick switcher, search, command palette). `None`
    /// in regular Normal/Insert mode.
    pub(crate) overlay: Option<Overlay>,

    /// Inline autocomplete state inside Insert mode (triggered by `[[`
    /// or `#`). Mutually exclusive with `overlay` because Insert mode
    /// itself can't have overlays.
    pub(crate) autocomplete: Option<AutocompleteState>,

    /// Persisted last-search results so `n`/`N` can navigate hits
    /// after the search overlay is closed.
    pub(crate) last_search: Option<LastSearch>,

    /// Yank register — one or more blocks copied via `yy` (Normal) or
    /// `y` (Visual). `p` / `P` paste these.
    pub(crate) yank_register: Vec<OutlineNode>,

    /// Derived index of pages, titles and backlinks. Used by the
    /// backlinks panel, quick switcher, autocomplete, icon decoration.
    ///
    /// Built off the critical path: `App::new` starts with an empty
    /// index and spawns a background thread that scans the whole
    /// workspace. The thread's result lands here via
    /// [`App::poll_index_updates`] — TUI is interactive in the
    /// meantime, just without backlinks/icons until the build finishes.
    pub(crate) index: WorkspaceIndex,

    /// In-flight workspace-index build, if any. Set by
    /// [`App::spawn_index_rebuild`], drained by
    /// [`App::poll_index_updates`]. `None` when the index is up to
    /// date.
    pub(crate) index_rx: Option<std::sync::mpsc::Receiver<WorkspaceIndex>>,

    /// Whether to draw the inline backlinks section below the outline.
    /// Toggled with `B`. Default `true` so users discover the feature.
    pub(crate) show_backlinks: bool,

    /// Where the cursor lives — outline (default) or inside the inline
    /// backlinks section. Reset to `Focus::Outline` whenever the view
    /// changes (page open, journal navigation, etc).
    pub(crate) focus: Focus,

    /// First visible line of the outline (vertical scroll offset). The
    /// view module keeps this in sync with `selected` — when navigation
    /// pushes the selection off-screen, this advances so the cursor
    /// stays inside the viewport.
    pub(crate) scroll_y: u16,
    /// Last rendered viewport height for the outline area, in lines.
    /// `Ctrl+D` / `Ctrl+U` / `PgDn` / `PgUp` use this to jump by a
    /// page or half-page. Updated by [`crate::view::render_main`].
    pub(crate) viewport_height: u16,

    /// Last-seen mtime of the page file on disk. Set by `load_current`
    /// and `save`; consulted by the polling loop so external edits
    /// (e.g. `nvim` saving the same `.md`) are picked up automatically.
    pub(crate) last_mtime: Option<SystemTime>,

    /// Snapshots of the page AST taken before each structural mutation.
    /// `u` pops back one step; `Ctrl+R` re-pushes. Bounded — see
    /// [`MAX_HISTORY`] — so a long session can't blow memory.
    pub(crate) undo: Vec<HistorySnapshot>,
    /// Forward history: filled by `undo`, drained by `redo`.
    pub(crate) redo: Vec<HistorySnapshot>,

    /// Active color palette. Mutable so commands like `:theme dracula`
    /// can swap it at runtime without restarting the binary.
    pub(crate) theme: Theme,

    /// Registered code-block runtimes (`lisp`, `echo`, …). Used by
    /// `gx` / `:run`. Built once at startup; cheap to clone but we keep
    /// one instance per App.
    pub(crate) exec_registry: RuntimeRegistry,

    /// Registered slash / palette commands (`prop`, `search`, …).
    /// Built once at startup with the shipped commands; plugins
    /// (future) will append to it. Driving registry for both `/`
    /// and `:` overlays.
    pub(crate) command_registry: crate::commands::CommandRegistry,
}
