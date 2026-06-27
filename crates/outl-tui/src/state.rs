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
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::{OutlineNode, ParsedPage};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

pub(crate) const HELP_HINT_NORMAL: &str =
    "i edit  o new  c fold  Enter open ref  K/J move  C-T TODO  u undo  ? help  q";
pub(crate) const HELP_HINT_INSERT: &str =
    "Esc commit  Enter new block  Ctrl+T TODO  Tab indent  Shift-Tab outdent";
pub(crate) const HELP_HINT_VISUAL: &str =
    "j/k extend  d/x delete  y yank  Tab indent  S-Tab outdent  Esc cancel";

/// Maximum entries in either history stack. Past this, oldest entries
/// drop off the front. Enough for many minutes of fluent editing without
/// becoming a memory issue.
pub(crate) const MAX_HISTORY: usize = 200;

// Re-exported from `outl-actions` so every client (TUI, mobile,
// future Tauri desktop) agrees on the literal wire form.
pub(crate) use outl_actions::{DONE_PREFIX, TODO_PREFIX};

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

/// Visual severity of a toast notification. Drives the icon and the
/// color the renderer applies — semantic, not free-form, so a
/// production-mode bug ("status was overwritten before the user could
/// read it") shows up consistently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Success,
    Info,
    Warning,
    Error,
}

/// One stacked notification rendered in the bottom-right corner.
///
/// Toasts are *additive*: pushing a new one doesn't clobber the
/// previous one's text the way `status` does. The renderer stacks
/// them above the footer until each one's `until` instant has
/// passed, then the event loop sweeps the expired ones out.
#[derive(Debug, Clone)]
pub(crate) struct Toast {
    pub(crate) message: String,
    pub(crate) kind: ToastKind,
    pub(crate) until: std::time::Instant,
}

/// Which section of the sidebar the user is browsing when focus is on
/// the sidebar. The three sections are stacked vertically; Tab cycles
/// forward, Shift-Tab backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarSection {
    Calendar,
    Pinned,
    Recent,
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
/// - `idx` indexes `app.backlinks_for_current()` (the workspace-driven
///   list produced by `outl_actions::backlinks_for_page`).
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

/// An armed Normal-mode op waiting for a follow-up character. Armed
/// by `r`, `f`, `F`; consumed by the next `Char(_)` keystroke. The
/// follow-up resolution lives in `input::normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingInputOp {
    /// `r{ch}` — replace the char under the cursor with `ch`.
    ReplaceChar,
    /// `f{ch}` — find next `ch` on the current block, forward.
    FindCharForward,
    /// `F{ch}` — find previous `ch`, backward.
    FindCharBackward,
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
///
/// Built-in commands carry names from the
/// [`crate::commands::CommandRegistry`]; plugin-contributed commands
/// carry owned strings plus a [`SlashOrigin::Plugin`] tag so
/// `accept_slash` routes them to the `PluginHost` instead of the
/// registry. Both kinds share this one filterable list.
#[derive(Debug, Clone)]
pub(crate) struct SlashCommand {
    /// Name as typed / displayed: `prop`, `search`, `run`, or a
    /// plugin command's title.
    pub(crate) name: String,
    /// One-liner shown next to the name.
    pub(crate) description: String,
    /// When true, accepting opens the vim command palette pre-filled
    /// with `<name> ` so the user can supply arguments
    /// (`prop priority high`). When false the command runs immediately.
    /// Always false for plugin commands (no arg surface in d0).
    pub(crate) needs_args: bool,
    /// Where this command runs when accepted.
    pub(crate) origin: SlashOrigin,
}

/// Dispatch target for an accepted slash command.
#[derive(Debug, Clone)]
pub(crate) enum SlashOrigin {
    /// A shipped command in the [`crate::commands::CommandRegistry`],
    /// dispatched by `name`.
    Builtin,
    /// A plugin-contributed command, dispatched to the `PluginHost` by
    /// `(plugin_id, command_id)`.
    Plugin {
        plugin_id: String,
        command_id: String,
    },
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
    /// The full candidate set captured when the switcher opens.
    /// Treated as read-only after `open_quick_switch` populates it:
    /// `refresh_quick_switch` always filters *from here* into
    /// `candidates` and never writes back, so deleting query
    /// characters can always restore candidates a longer query had
    /// dropped.
    pub(crate) all_candidates: Vec<SwitchCandidate>,
    /// The filtered + scored subset currently shown, recomputed from
    /// `all_candidates` on every keystroke.
    pub(crate) candidates: Vec<SwitchCandidate>,
    pub(crate) selected: usize,
    /// One-slot cache for the preview pane so the renderer doesn't
    /// re-read the highlighted page from disk on every frame. Keyed
    /// by the candidate's `key` (slug or ISO date); the renderer
    /// invalidates whenever the cached key doesn't match the
    /// currently selected candidate. `RefCell` because the
    /// `render_app` path holds `QuickSwitchState` by shared
    /// reference — interior mutability lets us refresh the slot
    /// without rippling `&mut` through every view function.
    pub(crate) preview_cache: std::cell::RefCell<Option<(String, String)>>,
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
    /// User typed `((`; pick a block by its text. Candidates are
    /// **handles** (`blk-XXXXXX`); the popup looks each handle's
    /// `BlockEntry.text` up in the index for display.
    BlockRef,
    /// User typed `/` at the start of a word; pick a slash command.
    /// On accept, the trigger + query are removed from the buffer
    /// and the command runs (or pops the `:` palette for arg entry).
    SlashCommand,
    /// User typed `@` at the start of a word; pick a **person**.
    /// Candidates are page titles where `type:: person` is set
    /// (filtered through `WorkspaceIndex::pages_by_type`).
    /// On accept the trigger + query are replaced with
    /// `[[@<title>]]` — a regular wikilink whose target carries the
    /// `@` prefix as a visual mention affordance.
    /// Unlike `Tag`, the query **allows spaces** so composite names
    /// like `@Thiago Avelino` work.
    Mention,
    /// User typed `:` at the start of a word or after whitespace,
    /// followed by `[a-z]`. Candidates are shortcodes ranked by
    /// `outl_md::emoji::search`; the popup row shows the glyph on the
    /// left and `:shortcode:` on the right. On accept the trigger +
    /// query are replaced with the canonical `:shortcode:` form (the
    /// `.md` always stores the shortcode literal, never the glyph —
    /// see `docs/markdown-format.md` § "Emoji shortcodes").
    Emoji,
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
    /// Selected help tab when `show_help` is true. `h/l` cycle
    /// horizontally between sections (Normal / Insert / Visual /
    /// Overlays / Dates) — splits the wall-of-text help into
    /// glance-able panes.
    pub(crate) help_tab: usize,
    /// Vertical scroll offset inside the active help tab. `j/k` and
    /// `↓/↑/PgUp/PgDn/g/G` drive this while the popup is open.
    /// Resets to 0 whenever the user switches tabs so the new tab's
    /// content starts from the top, not at a stale offset.
    pub(crate) help_scroll: u16,
    pub(crate) pending_chord: Option<char>,
    /// Armed Normal-mode op waiting for the next keystroke. `r`, `f`,
    /// `F` arm this; the next `Char(c)` consumes it and applies the
    /// corresponding action. Mutually exclusive with `pending_chord`
    /// — both are armed by a single keystroke and the next key
    /// resolves exactly one of them.
    pub(crate) pending_input_op: Option<PendingInputOp>,
    /// First chord of a two-chord **plugin** keybinding, buffered while
    /// we wait on the second keypress. Separate from `pending_chord`
    /// (which is the native vim chord accumulator) so plugin sequences
    /// like `Ctrl+T A` never interfere with — and are never shadowed
    /// by — the built-in `d`/`g`/`y`/`z` chords. Only ever set when the
    /// first chord is a strict prefix of some registered plugin binding
    /// and matches no native action; see `input/normal.rs`.
    pub(crate) pending_plugin_chord: Option<outl_shortcuts::Chord>,
    pub(crate) status: String,

    /// Last Visual range, captured every time the user leaves Visual
    /// mode. vim's `gv` re-enters Visual with this range. `None` means
    /// no Visual session has happened yet in this app instance.
    pub(crate) last_visual: Option<(usize, usize)>,

    /// Non-fatal parser recoveries for the currently-loaded `.md`.
    ///
    /// Populated by [`App::load_current_no_autorun`] from
    /// `ParsedPage.warnings`. Drives the inline banner above the
    /// outline + the chip in the status line so the user knows
    /// outl had to recover lines from a file that doesn't fully
    /// match the dialect (typical case: leading `# heading`,
    /// free paragraph, imported markdown).
    ///
    /// The view is **non-destructive**: blocks built from a
    /// recovered line render normally; the next save normalises
    /// the file to `- <raw>`. Nothing is deleted on the user's
    /// behalf — they decide when to clean up.
    pub(crate) parse_warnings: Vec<outl_md::ParseWarning>,

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

    /// Last `((blk-XXXXXX))` token yanked via the `yr` chord. Lets the
    /// user pick a block ref handle without leaving the keyboard:
    /// `yr` to capture, then type / paste manually somewhere else.
    /// `None` means the chord was never invoked in this session.
    pub(crate) last_yanked_ref: Option<String>,

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

    /// Cached backlinks for the currently-opened page, keyed by slug.
    ///
    /// `outl_actions::backlinks_for_page` is `O(blocks in workspace)`
    /// per call. Render frames and backlink-panel keystrokes both
    /// query the same slug repeatedly, so we cache the last result
    /// here. `None` means "stale, recompute on next read" — the
    /// invalidation points are `save`, `save_page_with`,
    /// `reload_workspace_from_disk`, and any path that swaps the
    /// open view (`load_current`, `go_today`, `shift_journal`, …).
    ///
    /// Wrapped in `RefCell` so the read-only render path
    /// ([`App::backlinks_for_current`]) can populate it on a cache
    /// miss without needing `&mut App`. See
    /// [`App::invalidate_backlinks_cache`].
    pub(crate) backlinks_cache: std::cell::RefCell<Option<(String, Vec<outl_actions::Backlink>)>>,

    /// `true` when this workspace is using the JSONL backend (shared
    /// across devices via iCloud / Syncthing / etc.). Set at boot
    /// from `[workspace].storage`; controls whether the peer-ops
    /// poller actually fires. A SQLite workspace that happens to
    /// have an `ops/` directory on disk (manual mkdir, leftover
    /// from a partial migration) must NOT be polled — opening
    /// `JsonlStorage` against it would replace the materialised
    /// workspace with an empty one and the user would see every
    /// block disappear.
    pub(crate) shared_workspace: bool,

    /// Receives a `()` notification whenever a peer (mobile, another
    /// TUI) writes new ops into `<root>/ops/`. Drained by
    /// [`App::poll_jsonl_updates`], which reopens the workspace and
    /// refreshes the in-memory `ParsedPage`.
    ///
    /// `None` for workspaces that use the SQLite backend (no shared
    /// op log to watch).
    pub(crate) jsonl_rx: Option<std::sync::mpsc::Receiver<()>>,

    /// Active sync transport, if one is wired in. `None` means the
    /// default filesystem/iCloud behaviour: `spawn_jsonl_poller`
    /// falls back to `outl_actions::FileSyncTransport` (poll `ops/`
    /// every 2 s). `Some(_)` (e.g. `IrohSyncTransport`) takes over
    /// both change detection (`start`) and the post-commit announce
    /// hook (`announce_local_ops`). Behind an `Arc` so the poller
    /// thread can hold its own handle.
    pub(crate) sync_transport: Option<std::sync::Arc<dyn outl_actions::SyncTransport>>,

    /// Set by [`App::poll_jsonl_updates`] when the poller fires while
    /// the user is in [`Mode::Insert`]. We can't safely reopen the
    /// workspace mid-edit (the in-flight `ParsedPage` would be lost),
    /// so we remember that a peer wrote ops and apply the reload
    /// during the next `commit_insert` once the buffer is flushed.
    pub(crate) pending_reload: bool,

    /// Receives the list of `.md` files the background scanner
    /// flagged as out-of-sync with the op log — either no sidecar
    /// (just imported / dropped in by vim) or `last_synced_hash`
    /// doesn't match.
    ///
    /// Drained by [`App::poll_orphan_md_updates`] which runs
    /// `reconcile_md` on the main thread, creating the missing
    /// ops + sidecar so future renders see the file properly.
    pub(crate) orphan_md_rx: Option<std::sync::mpsc::Receiver<Vec<std::path::PathBuf>>>,

    /// Whether to draw the inline backlinks section below the outline.
    /// Toggled with `B`. Default `true` so users discover the feature.
    pub(crate) show_backlinks: bool,

    /// Whether the left sidebar (mini-calendar + pinned + recent) is
    /// visible. Default `false` so first-time users land on the
    /// classic single-pane layout — the toggle (`\`) opt-ins those who
    /// want it. Persisted across the session in memory only.
    pub(crate) show_sidebar: bool,

    /// Sidebar focus state. Tracks which section the user is browsing
    /// when the sidebar has focus (vs. the outline). `None` means the
    /// outline owns the keyboard; `Some(_)` means the next `j/k/Enter`
    /// goes to the sidebar list.
    pub(crate) sidebar_focus: Option<SidebarSection>,

    /// Sidebar cursor inside the currently focused section (0-based).
    /// Only meaningful when `sidebar_focus.is_some()`.
    pub(crate) sidebar_cursor: usize,

    /// LRU of recently-opened paths. Newest first; bounded to a small
    /// window so the sidebar's `Recent` section stays scannable.
    /// In-memory only for now — persisting to `.outl/state.toml` is on
    /// the roadmap (Phase 2 stretch).
    pub(crate) recent_paths: Vec<PathBuf>,

    /// Stack of transient notifications shown in the bottom-right
    /// corner. Each one carries its own expiry; the event loop sweeps
    /// the expired ones out so they don't accumulate. Independent
    /// from `status`, which still drives the single-line footer
    /// message (e.g. chord prompts).
    pub(crate) toasts: Vec<Toast>,

    /// Block ids currently rendered collapsed (children hidden) in
    /// the outline. Hydrated from `workspace.tree().is_collapsed(_)`
    /// on every `load_current`, so the local mirror always tracks
    /// the op log's authoritative view.
    ///
    /// Source of truth is the op log (`Op::SetCollapsed`); this
    /// `HashSet` is the fast-path the renderer + nav helpers consult
    /// to skip hidden subtrees. Toggling goes through
    /// `outl_actions::toggle_block_collapsed`, which generates an
    /// `Op::SetCollapsed` and applies it via `Workspace::apply`. We
    /// patch this `HashSet` optimistically and re-sync from the
    /// workspace after the apply returns.
    pub(crate) collapsed: HashSet<NodeId>,

    /// Flat DFS-preorder map from the outline's flat index (the
    /// `cursor` the render walk maintains, also `App.selected`) to
    /// the block's `NodeId`. Populated on every `load_current` from
    /// the freshly-read sidecar's `blocks` list (which is itself in
    /// DFS preorder by construction).
    ///
    /// The render walk needs an id-by-position lookup to consult
    /// `collapsed`; `outl-md::parse::OutlineNode` does not carry the
    /// id, and going through the workspace index every step would be
    /// O(N) per render. This vector is cheap to (re)build and stays
    /// in sync with `page.blocks` because both are reconstructed
    /// together in `load_current_no_autorun`.
    pub(crate) id_by_flat: Vec<NodeId>,

    /// `hidden_by_collapse[i]` is `true` when the i-th flat block
    /// has at least one ancestor whose id is in [`Self::collapsed`].
    /// Used by `step_forward`/`step_backward` to skip past folded
    /// subtrees so `j`/`k` never leave the user pointing at an
    /// invisible block.
    ///
    /// Recomputed whenever `collapsed` changes
    /// (`recompute_hidden_by_collapse`).
    pub(crate) hidden_by_collapse: Vec<bool>,

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

    /// Wall-clock instant of the last successful save (or initial
    /// load). The header renders `⟳ {N}s ago` against this so the user
    /// has a glanceable freshness indicator. `None` means "never saved
    /// in this session" — the header chip is hidden in that case.
    pub(crate) last_saved_at: Option<Instant>,

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
    /// contribute their commands through the separate `plugin_host`
    /// (see `slash_candidates`). Driving registry for both `/`
    /// and `:` overlays.
    pub(crate) command_registry: crate::commands::CommandRegistry,

    /// JS plugin runtime, loaded from `<root>/.outl/plugins/` at boot.
    ///
    /// `None` only on the rare path where the host couldn't be built at
    /// all; a host that simply found no plugins is still `Some` and
    /// empty. Plugins are **best-effort**: a load failure never blocks
    /// the TUI. The host contributes slash commands (concatenated into
    /// the slash menu) and runs `onOp` hooks after every mutation via
    /// [`App::run_plugin_op_hooks`].
    ///
    /// Not `Send` (the Boa context is single-thread), which is fine —
    /// the TUI is single-threaded on the main loop, so the host lives
    /// directly in `App` with no `Arc`/`Mutex`.
    pub(crate) plugin_host: Option<outl_plugins::PluginHost>,

    /// Pre-computed content-transformer output for the current view,
    /// keyed by block `NodeId`. A block whose text is a single code
    /// fence (`` ```<lang> `` … `` ``` ``) whose `lang` a loaded plugin
    /// claims as a `text` transformer gets its body run through
    /// [`outl_plugins::PluginHost::transform_block`] **at load time**
    /// (in `recompute_transforms`), and the resulting text/markdown is
    /// stashed here.
    ///
    /// Why pre-compute: `transform_block` is `&mut self` (it runs the
    /// Boa engine) and the outline render walk only has `&App`, so the
    /// transform cannot happen during render. It would also be wrong to
    /// run JS once per frame. This map turns the render path into a pure
    /// `HashMap` lookup keyed by `id_by_flat[cursor]`.
    ///
    /// Only `kind == "text"` results land here — `rich` (HTML for a GUI
    /// iframe) has no meaning in a terminal and is ignored. Rebuilt
    /// whenever the AST is (every `load_current_no_autorun`); a block
    /// being edited (cursor on it) renders raw regardless of an entry
    /// here, so the user always sees and edits the real fence source.
    pub(crate) transform_cache: HashMap<NodeId, String>,
}
