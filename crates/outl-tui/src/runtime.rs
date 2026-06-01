//! TUI runtime — `pub fn run`, the event loop, terminal lifecycle, and
//! the panic-restore hook. The bits that turn a workspace path into a
//! running interactive program.
//!
//! State definitions live in [`crate::state`]; everything that touches
//! the `App` in response to a key event lives in [`crate::input`]; the
//! draw side lives in [`crate::view`].

use crate::input::{handle_insert_key, handle_normal_key, handle_overlay_key, handle_visual_key};
use crate::state::{App, Mode};
use crate::theme::{self, Theme};
use crate::view::render_app;
use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::fs;
use std::io::Stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often the event loop wakes up to check for external `.md`
/// edits when no keypress arrived. Short enough that `nvim :w` shows
/// up "instantly" by human standards, long enough not to thrash the
/// filesystem.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Shorter poll cadence used while a background workspace-index
/// rebuild is in flight. The worker finishes in tens of ms on a small
/// vault but the result only reaches the UI on the next event-loop
/// iteration — without this, the user waits up to `POLL_INTERVAL`
/// (750 ms) to see backlinks fill in after opening the app. ~60 fps
/// while we wait costs nothing on idle hardware and disappears the
/// moment the index lands.
const POLL_INTERVAL_PENDING_INDEX: Duration = Duration::from_millis(16);

/// Run the TUI against the workspace at `path`.
///
/// Picks the active theme from `.outl/config.toml`'s `[theme] preset`
/// field if present, falling back to the default-dark palette.
pub fn run(path: &Path) -> Result<()> {
    run_with_theme_override(path, None)
}

/// Variant of [`run`] that accepts a `--theme` override from the CLI.
/// Pass `Some(name)` to force a particular preset; `None` defers to the
/// config file (or default).
pub fn run_with_theme_override(path: &Path, theme_override: Option<&str>) -> Result<()> {
    if !is_tty() {
        return Err(anyhow::anyhow!(
            "outl-tui requires an interactive terminal (stdout is not a TTY)"
        ));
    }

    // Cage every dependency that uses `tracing` (Steel, wasmtime,
    // notify, ...). Without this they print INFO lines straight onto
    // the TUI canvas, which looks like the terminal exploded. We send
    // everything to a per-workspace log file so debugging is still
    // possible — and silence the terminal entirely.
    install_silent_log_subscriber(path);

    let workspace_root = path.to_path_buf();
    // `_lock` and `_actor_lock` live through the entire TUI run:
    // - `_lock` is the shared workspace flock on `<root>/.outl/.lock`.
    // - `_actor_lock` is the exclusive per-actor write flock on
    //   `<root>/ops/.lock-<actor>` and keeps another `outl` from
    //   stealing this process's actor mid-session.
    let (workspace, actor, cfg, _lock, _actor_lock) = open_workspace(&workspace_root)?;

    // No boot-time `apply_all_pages_md` here. The op log remains the
    // source of truth; `.md` is its projection. We deliberately skip
    // re-projecting at boot because peers and external editors (vim,
    // VS Code) may have written `.md` content the op log doesn't
    // know about yet — re-projecting blindly would clobber those
    // edits before the orphan scanner has a chance to fold them in
    // via `reconcile_md`.
    let theme = resolve_theme(theme_override, &cfg);
    let shared_workspace = cfg
        .get("workspace")
        .and_then(|w| w.get("storage"))
        .and_then(|s| s.as_str())
        == Some("jsonl");

    // Install the panic hook BEFORE switching to raw mode. If
    // anything panics from here on — bug in the render path, OOM —
    // the hook runs first, restores the terminal, then chains to the
    // default handler so the user still sees the panic message.
    install_panic_restore_hook();

    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering alt screen")?;

    // Ask the terminal to report enhanced key events (kitty keyboard
    // protocol). When supported, this lets us distinguish `Shift+Enter`
    // from `Enter`, `Ctrl+Enter` from `Enter`, and so on — essential
    // for multi-line editing inside a single block. Terminals that
    // don't support it (Terminal.app, plain xterm) silently ignore the
    // CSI sequence; we still have Alt+Enter as a portable fallback.
    let enhanced_keys = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced_keys {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
            )
        );
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("creating terminal")?;

    let result = event_loop(
        &mut terminal,
        workspace_root,
        workspace,
        actor,
        theme,
        shared_workspace,
    );

    if enhanced_keys {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

pub(crate) fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Install a process-wide panic hook that restores the terminal before
/// chaining to the previous handler.
///
/// Without this, a panic mid-render leaves the user staring at a
/// garbled terminal in raw mode with no cursor — they have to `reset`
/// blind to recover. Calling this is idempotent in spirit (only the
/// first call chains the real default hook).
fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore the terminal state. Errors are ignored — we're
        // already panicking; nothing useful to do with a second
        // failure.
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        let _ = execute!(std::io::stdout(), crossterm::cursor::Show);
        previous(info);
    }));
}

/// Resolve the active theme.
///
/// Precedence (first hit wins):
/// 1. `--theme <preset>` CLI override.
/// 2. `[theme] preset = "..."` in `config.toml`.
/// 3. [`theme::default_theme`].
///
/// An unknown name falls through silently to the default. The caller can
/// surface the choice via the status line if it cares.
fn resolve_theme(cli_override: Option<&str>, cfg: &toml::Value) -> Theme {
    if let Some(name) = cli_override {
        if let Some(t) = theme::by_name(name) {
            return t;
        }
    }
    if let Some(preset) = cfg
        .get("theme")
        .and_then(|t| t.get("preset"))
        .and_then(|v| v.as_str())
    {
        if let Some(t) = theme::by_name(preset) {
            return t;
        }
    }
    theme::default_theme()
}

fn open_workspace(
    root: &Path,
) -> Result<(
    Workspace,
    ActorId,
    toml::Value,
    outl_core::WorkspaceLock,
    outl_core::ActorWriteLock,
)> {
    // Shared workspace lock — every well-behaved `outl` opener piles
    // on. Concurrent TUI + MCP server + sink-outl plugin is the
    // supported case; per-actor write isolation comes from the
    // ActorWriteLock below.
    let lock = outl_core::WorkspaceLock::acquire(root)
        .with_context(|| format!("could not acquire workspace lock at {}", root.display()))?;

    let cfg_path = root.join(".outl").join("config.toml");
    let cfg = fs::read_to_string(&cfg_path).with_context(|| {
        format!(
            "no outl workspace at {} — run `outl init` first",
            root.display()
        )
    })?;
    let cfg: toml::Value = toml::from_str(&cfg).context("parsing config.toml")?;
    let actor_str = cfg
        .get("workspace")
        .and_then(|w| w.get("actor_id"))
        .and_then(|a| a.as_str())
        .context("workspace.actor_id missing from config.toml")?;
    let actor_ulid = ulid::Ulid::from_string(actor_str).context("actor_id is not a valid ULID")?;
    let config_actor = ActorId(actor_ulid);

    let ops_dir = root.join("ops");
    fs::create_dir_all(&ops_dir)
        .with_context(|| format!("creating ops dir at {}", ops_dir.display()))?;

    // Exclusive per-actor write lock. Falls back to an ephemeral
    // actor when another `outl` already owns the config-default one
    // — that's how a TUI + MCP server share the same workspace
    // without racing on `ops-<config_actor>.jsonl`.
    let (actor_lock, actor) = outl_core::resolve_write_actor(&ops_dir, config_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", ops_dir.display()))?;
    if actor != config_actor {
        tracing::info!(
            "another outl process owns the config actor {config_actor}; this TUI writes under ephemeral actor {actor}"
        );
    }

    let storage = open_storage(root, actor, &cfg)?;
    let ws = Workspace::open_with_storage(actor, storage, Some(root.to_path_buf()))?;
    Ok((ws, actor, cfg, lock, actor_lock))
}

/// Open the workspace's op log.
///
/// Storage is always [`JsonlStorage`] rooted at `<root>/ops/`, one
/// `ops-<actor>.jsonl` file per device. Cross-device sync is the
/// responsibility of whatever filesystem-level transport the user
/// chose (iCloud Drive, Syncthing, shared NFS) — the directory is
/// created on open if missing because syncing platforms sometimes
/// garbage-collect empty dirs. Not named `.ops/` because iCloud
/// skips dotted paths.
///
/// `cfg` used to drive a `[workspace].storage` switch; the field is
/// now a no-op kept for backward-compat with older configs that
/// declared `storage = "jsonl"` or `"sqlite"`. Both paths land here.
fn open_storage(root: &Path, actor: ActorId, _cfg: &toml::Value) -> Result<Box<dyn Storage>> {
    let ops_dir = root.join("ops");
    fs::create_dir_all(&ops_dir)
        .with_context(|| format!("creating ops dir at {}", ops_dir.display()))?;
    let storage = JsonlStorage::open(ops_dir, actor)
        .with_context(|| format!("opening jsonl storage at {}", root.display()))?;
    Ok(Box::new(storage))
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace_root: PathBuf,
    workspace: Workspace,
    actor: ActorId,
    theme: Theme,
    shared_workspace: bool,
) -> Result<()> {
    let mut app = App::new(workspace_root, workspace, actor, theme, shared_workspace)?;
    loop {
        // Pick up the background index build if it finished since the
        // last frame. Non-blocking; costs ~one channel try_recv.
        app.poll_index_updates();
        // Pick up any peer ops the jsonl poller saw arrive via iCloud
        // (or another sync transport). Reopens the workspace from
        // disk so the merged op log shows up in the next render.
        app.poll_jsonl_updates();
        // Reconcile `.md` files dropped in by importers (Roam, Logseq)
        // or edited externally (vim, VS Code). The scanner picks them
        // up in the background; we emit Create/Move/Edit ops here on
        // the main thread.
        app.poll_orphan_md_updates();
        // Sweep expired toasts so they don't linger on screen past
        // their lifetime. Cheap O(n) over the small toast stack.
        app.prune_toasts();

        terminal.draw(|f| render_app(f, &mut app)).context("draw")?;
        // Wait for a keystroke for up to POLL_INTERVAL. If nothing
        // arrives, take that opportunity to check whether the `.md`
        // changed under us (external editor saved). This is the
        // simplest hot-reload path — no filesystem watcher, no
        // background thread — and good enough at human latency.
        //
        // While a background index rebuild is in flight, shorten the
        // timeout so the freshly-built `WorkspaceIndex` shows up in
        // the UI within ~16 ms of arriving (instead of waiting up to
        // 750 ms for the next external-edit poll).
        let poll_timeout = if app.has_pending_index() {
            POLL_INTERVAL_PENDING_INDEX
        } else {
            POLL_INTERVAL
        };
        if !event::poll(poll_timeout).unwrap_or(false) {
            app.check_external_changes();
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Before processing the keystroke, sync with disk. If a
        // background editor wrote between polls, we want the keystroke
        // to act on the *new* page state, not the stale one.
        app.check_external_changes();
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Commit any pending insert before exiting.
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            }
            return Ok(());
        }

        // Universal: if the help popup is up, intercept the obvious
        // "close it" keys before they reach any mode-specific handler.
        // The popup is a `bool` flag (not an `Overlay`) so the overlay
        // close path doesn't catch it. Also resets `help_scroll` so
        // reopening starts from the top instead of a stale offset.
        if app.show_help
            && matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            )
        {
            app.show_help = false;
            app.help_scroll = 0;
            continue;
        }

        // Universal `Ctrl+S` = save. Works in any mode.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            } else {
                app.save();
            }
            app.toast(crate::state::ToastKind::Success, "saved");
            continue;
        }

        // Universal `Ctrl+L` = refresh workspace (re-read from disk).
        // Useful when another editor changes files behind us.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
            if matches!(app.mode, Mode::Insert { .. }) {
                app.commit_insert();
            }
            app.refresh_workspace();
            app.status = "refreshed".into();
            continue;
        }

        // Overlays steal the keystream while open.
        if app.overlay.is_some() {
            if handle_overlay_key(&mut app, key)? {
                return Ok(());
            }
            continue;
        }

        match app.mode {
            Mode::Normal => {
                if handle_normal_key(&mut app, key)? {
                    return Ok(());
                }
            }
            Mode::Insert { .. } => handle_insert_key(&mut app, key)?,
            Mode::Visual { .. } => handle_visual_key(&mut app, key)?,
        }
    }
}

/// Wire up `tracing` so nothing prints onto the TUI canvas, but logs
/// stay debuggable.
///
/// Order matters here:
/// 1. **`tracing-log` bridge** — Steel and a few other deps emit via
///    the older `log` crate. Without the bridge `tracing-subscriber`
///    never sees those records and the default `log` impl falls back
///    to printing on stderr — straight onto our TUI canvas. Install
///    the bridge *before* the subscriber so the subscriber catches
///    every record.
/// 2. **`tracing-subscriber`** with a file appender at
///    `<workspace>/.outl/tui.log`. `tail -f` it when something looks
///    off.
/// 3. Best-effort installation: if a global was already set (rare,
///    but possible when a dep front-runs us), `try_init` swallows
///    the error — the TUI keeps running with whatever subscriber
///    was registered first.
fn install_silent_log_subscriber(workspace_root: &Path) {
    use std::fs::OpenOptions;
    use tracing_log::LogTracer;
    use tracing_subscriber::{fmt, EnvFilter};

    // Bridge `log` → `tracing`. Idempotent: re-init returns Err which
    // we ignore.
    let _ = LogTracer::init();

    let log_path = workspace_root.join(".outl").join("tui.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(workspace_root));
    let file = OpenOptions::new().create(true).append(true).open(&log_path);

    // `RUST_LOG` still wins if the user wants verbose output — useful
    // when reporting a bug. Default = warn; everything noisier gets
    // dropped on the floor.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let builder = fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true);

    let result = match file {
        Ok(f) => builder.with_writer(f).try_init(),
        // No log file? Still install a sink so the canvas stays clean.
        Err(_) => builder.with_writer(std::io::sink).try_init(),
    };
    // Errors here mean a global subscriber was set by an earlier
    // caller — fine, just move on.
    let _ = result;
}
