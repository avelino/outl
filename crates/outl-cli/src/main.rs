//! `outl` — the CLI binary.
//!
//! Thin shell over `outl-core`, `outl-md`, and `outl-tui`. See
//! `crates/outl-cli/CLAUDE.md`.
//!
//! UX:
//!
//! - `outl` with no subcommand opens the TUI in the current directory.
//! - `outl --path <dir>` opens the TUI in `<dir>` (global flag, works
//!   with any subcommand that needs a workspace path).
//! - Subcommands cover workspace lifecycle and one-shot operations; the
//!   global `--path` is used when the subcommand omits its positional
//!   `<path>` argument.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

mod cmd;
mod sync_engine;
mod workspace_layout;

#[derive(Parser, Debug)]
#[command(
    name = "outl",
    about = "Local-first outliner with markdown as source of truth.",
    long_about = "Local-first outliner with markdown as source of truth.\n\
                  \n\
                  Running `outl` with no subcommand opens the TUI in the workspace at \
                  `--path` (default: current directory).",
    version
)]
struct Cli {
    /// Workspace path. Used by every subcommand that needs one; defaults
    /// to the current directory. Subcommand-level positional path, when
    /// provided, takes precedence.
    #[arg(short = 'p', long, global = true)]
    path: Option<PathBuf>,

    /// TUI theme preset (default-dark, light, dracula, solarized-dark,
    /// nord, monokai). Overrides `[theme] preset` in workspace
    /// `config.toml` for this run.
    #[arg(long, global = true, value_name = "PRESET")]
    theme: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Increase verbosity. Pass multiple times for more detail.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Open the TUI on the workspace (default: `--path` or current dir).
    Tui {
        /// Workspace path. Overrides the global `--path`.
        path: Option<PathBuf>,
    },
    /// Initialize a new workspace at the given path.
    Init {
        /// Workspace path. Created if it does not exist. Overrides `--path`.
        path: Option<PathBuf>,
    },
    /// Run the file watcher; keep the workspace in sync.
    Serve {
        /// Workspace path. Overrides the global `--path`.
        path: Option<PathBuf>,
        /// Reconcile every `.md` once and exit (no file watcher).
        #[arg(long)]
        once: bool,
    },
    /// Export the workspace to another format (phase 4 placeholder).
    Export {
        /// Format to export to.
        #[arg(long, default_value = "hugo")]
        to: String,
    },
    /// Check workspace integrity.
    Doctor {
        /// Workspace path. Overrides the global `--path`.
        path: Option<PathBuf>,
    },
    /// Resolve orphan matches via the TUI.
    Reconcile {
        /// Workspace path. Overrides the global `--path`.
        path: Option<PathBuf>,
    },
    /// Inspect or list theme presets.
    Theme {
        #[command(subcommand)]
        sub: Option<ThemeSubcommand>,
    },
    /// Copy ops from the local SQLite log into a shared `.ops/` JSONL
    /// log so peers (mobile, future desktop) can read them via iCloud /
    /// Syncthing / shared folder. Run this once after moving an
    /// existing TUI-only workspace into a synced directory.
    MigrateToShared {
        /// Workspace path. Overrides the global `--path`.
        path: Option<PathBuf>,
    },
    /// Import a graph from another outliner.
    Import {
        /// Source format: `logseq` (directory) or `roam` (JSON file).
        format: String,
        /// Path to the Logseq graph directory or the Roam backup file.
        src: PathBuf,
        /// Destination workspace. Created if it doesn't exist yet.
        dst: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum ThemeSubcommand {
    /// Print every available preset, one per line.
    List,
    /// Describe a specific preset (palette + style names).
    Show {
        /// Preset name (case- and separator-insensitive).
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // The TUI installs its own silent subscriber that captures
    // dependency logs (Steel, wasmtime, ...) into
    // `<workspace>/.outl/tui.log`. If we install a stderr subscriber
    // here first, the TUI's `try_init` is a no-op and every dep log
    // ends up *on top of* the rendered UI. So defer: TUI runs install
    // their own; everything else (serve / doctor / reconcile / ...)
    // keeps the stderr subscriber the user expects on a CLI command.
    let is_tui = matches!(cli.command, None | Some(Command::Tui { .. }));
    if !is_tui {
        init_tracing(cli.verbose);
    }

    match cli.command {
        None => {
            let p = resolve_path(cli.path.as_ref(), None)?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Tui { path }) => {
            let p = resolve_path(cli.path.as_ref(), path.as_ref())?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Init { path }) => {
            let p = resolve_init_path(cli.path.as_ref(), path.as_ref())?;
            cmd::init::run(&p)
        }
        Some(Command::Serve { path, once }) => {
            let p = resolve_path(cli.path.as_ref(), path.as_ref())?;
            cmd::serve::run(&p, once)
        }
        Some(Command::Export { to }) => cmd::export::run(&to),
        Some(Command::Doctor { path }) => {
            let p = resolve_path(cli.path.as_ref(), path.as_ref())?;
            cmd::doctor::run(&p)
        }
        Some(Command::Reconcile { path }) => {
            let p = resolve_path(cli.path.as_ref(), path.as_ref())?;
            cmd::reconcile::run(&p)
        }
        Some(Command::Theme { sub }) => cmd::theme::run(sub.as_ref()),
        Some(Command::Import { format, src, dst }) => cmd::import::run(&format, &src, &dst),
        Some(Command::MigrateToShared { path }) => {
            let p = resolve_path(cli.path.as_ref(), path.as_ref())?;
            cmd::migrate_to_shared::run(&p)
        }
    }
}

/// Resolve which workspace path to operate on.
///
/// Precedence: subcommand-positional > global `--path` > current dir.
fn resolve_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = local {
        return Ok(p.clone());
    }
    if let Some(p) = global {
        return Ok(p.clone());
    }
    std::env::current_dir().with_context(|| "reading current directory")
}

/// If `path` has no `.outl/` directory yet, prompt the user for
/// permission to initialize one. If they say no, error out cleanly.
///
/// When stdin isn't a TTY (e.g. piped) we don't prompt — instead we
/// error with the same "run `outl init`" message we used before. This
/// keeps scripted callers predictable.
fn ensure_workspace_or_prompt(path: &Path) -> Result<()> {
    let outl_dir = path.join(".outl");
    if outl_dir.exists() {
        return Ok(());
    }

    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        anyhow::bail!(
            "no outl workspace at {} — run `outl init {}` first",
            path.display(),
            path.display()
        );
    }

    use std::io::{BufRead, Write};
    eprintln!("No outl workspace at {}.", path.display());
    eprint!("Initialize a new workspace here? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .with_context(|| "reading prompt response")?;
    let answer = line.trim().to_lowercase();
    if answer == "y" || answer == "yes" {
        cmd::init::run(path)?;
        Ok(())
    } else {
        anyhow::bail!("aborted — no workspace initialized at {}", path.display());
    }
}

/// Same as [`resolve_path`] but errors out when neither flag nor positional
/// was given (init refuses to create a workspace at the cwd by accident).
fn resolve_init_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    match local.or(global) {
        Some(p) => Ok(p.clone()),
        None => Err(anyhow::anyhow!(
            "`outl init` needs an explicit path: pass a positional argument or `--path <DIR>`"
        )),
    }
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .try_init();
}
