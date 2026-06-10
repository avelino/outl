//! `outl` — the CLI binary.
//!
//! Thin shell over `outl-core`, `outl-md`, `outl-actions`, and
//! `outl-tui`. See `crates/outl-cli/CLAUDE.md` and `docs/cli.md`.
//!
//! UX:
//!
//! - `outl` with no subcommand opens the TUI in the current directory.
//! - `outl --workspace <dir>` opens the TUI in `<dir>` (global flag, works
//!   with any subcommand that needs a workspace path).
//! - Subcommands cover workspace lifecycle, machine-shaped operations
//!   (page/block/daily/search/query/export), and the `mcp serve` shim
//!   that lets Claude Desktop reach the same handlers over stdio.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

mod cmd;
mod human;
mod mcp;
mod output;
mod sync_engine;
mod workspace_layout;
mod ws;

#[derive(Parser, Debug)]
#[command(
    name = "outl",
    about = "Local-first outliner with markdown as source of truth.",
    long_about = "Local-first outliner with markdown as source of truth.\n\
                  \n\
                  Running `outl` with no subcommand opens the TUI in the workspace at \
                  `--workspace` (default: current directory).",
    version
)]
struct Cli {
    /// Workspace path. Used by every subcommand that needs one;
    /// defaults to the current directory. Subcommand-level positional
    /// path, when provided, takes precedence.
    #[arg(short = 'w', long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,

    /// TUI theme preset (default-dark, light, logseq-light, dracula,
    /// solarized-dark, nord, monokai). Overrides `[theme] preset` in
    /// workspace `config.toml` for this run.
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
    /// Open the TUI on the workspace (default: `--workspace` or current dir).
    Tui {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
    },
    /// Initialize a new workspace at the given path.
    Init {
        /// Workspace path. Created if it does not exist. Overrides `--workspace`.
        path: Option<PathBuf>,
    },
    /// Run the file watcher; keep the workspace in sync.
    Serve {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Reconcile every `.md` once and exit (no file watcher).
        #[arg(long)]
        once: bool,
    },
    /// Check workspace integrity.
    Doctor {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Emit the report as the JSON envelope instead of a human view.
        #[arg(long)]
        json: bool,
    },
    /// Resolve orphan matches via the TUI.
    Reconcile {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
    },
    /// Inspect or list theme presets.
    Theme {
        #[command(subcommand)]
        sub: Option<ThemeSubcommand>,
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
    /// Page-level operations.
    Page {
        #[command(subcommand)]
        sub: cmd::page::PageCommand,
    },
    /// Block-level operations.
    Block {
        #[command(subcommand)]
        sub: cmd::block::BlockCommand,
    },
    /// Daily journal operations.
    Daily {
        #[command(subcommand)]
        sub: cmd::daily::DailyCommand,
    },
    /// Full-text search.
    Search(cmd::search::SearchArgs),
    /// Structured query over pages.
    Query(cmd::query::QueryArgs),
    /// Backlinks and reference lookups.
    Backlinks {
        #[command(subcommand)]
        sub: cmd::backlinks::BacklinksCommand,
    },
    /// Apply a list of write ops sequentially in one workspace session.
    /// Reads `{"ops": [...]}` from stdin by default.
    Batch(cmd::batch::BatchArgs),
    /// Tag listing and lookups.
    Tag {
        #[command(subcommand)]
        sub: cmd::tag::TagCommand,
    },
    /// Render a page in a target format (hugo / md / json).
    Export {
        #[command(subcommand)]
        sub: Option<cmd::export_v2::ExportCommand>,
        /// Legacy placeholder for `--to <fmt>` shape; only `hugo` was
        /// ever accepted. Kept so prior scripts don't break.
        #[arg(long)]
        to: Option<String>,
    },
    /// Workspace summary (path, actor, counts).
    Workspace {
        #[command(subcommand)]
        sub: WorkspaceSubcommand,
    },
    /// Run the MCP (Model Context Protocol) server over stdio. Wire
    /// this into `claude_desktop_config.json` to expose every CLI
    /// subcommand as an MCP tool.
    Mcp {
        #[command(subcommand)]
        sub: McpSubcommand,
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

#[derive(Subcommand, Debug)]
pub enum WorkspaceSubcommand {
    /// Workspace info — path, actor, counts.
    Info(cmd::workspace_info::WorkspaceInfoArgs),
}

#[derive(Subcommand, Debug)]
pub enum McpSubcommand {
    /// Start the MCP stdio server. Targets the workspace at the global
    /// `--workspace` (or current directory if unset).
    Serve {},
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
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Tui { path }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Init { path }) => {
            let p = resolve_init_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::init::run(&p)
        }
        Some(Command::Serve { path, once }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::serve::run(&p, once)
        }
        Some(Command::Doctor { path, json }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            if json {
                std::process::exit(cmd::doctor::run_json(&p));
            }
            cmd::doctor::run(&p)
        }
        Some(Command::Reconcile { path }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::reconcile::run(&p)
        }
        Some(Command::Theme { sub }) => cmd::theme::run(sub.as_ref()),
        Some(Command::Import { format, src, dst }) => cmd::import::run(&format, &src, &dst),
        Some(Command::Page { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::page::run(&sub, &p));
        }
        Some(Command::Block { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::block::run(&sub, &p));
        }
        Some(Command::Daily { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::daily::run(&sub, &p));
        }
        Some(Command::Search(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::search::run(&args, &p));
        }
        Some(Command::Query(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::query::run(&args, &p));
        }
        Some(Command::Backlinks { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::backlinks::run(&sub, &p));
        }
        Some(Command::Batch(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::batch::run(&args, &p));
        }
        Some(Command::Tag { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::tag::run(&sub, &p));
        }
        Some(Command::Export { sub, to }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            match sub {
                Some(ec) => std::process::exit(cmd::export_v2::run(&ec, &p)),
                None => cmd::export::run(to.as_deref().unwrap_or("hugo")),
            }
        }
        Some(Command::Workspace { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            match sub {
                WorkspaceSubcommand::Info(args) => {
                    std::process::exit(cmd::workspace_info::run(&args, &p));
                }
            }
        }
        Some(Command::Mcp { sub }) => match sub {
            McpSubcommand::Serve {} => {
                let p = resolve_path(cli.workspace.as_ref(), None)?;
                mcp::serve(p)
            }
        },
    }
}

/// Resolve which workspace path to operate on.
///
/// Precedence (first hit wins):
///
/// 1. **Subcommand-positional** path (`outl page get … <path>`).
/// 2. **Global `--workspace <DIR>`** flag.
/// 3. **`workspace.last`** from `~/.config/outl/config.toml`
///    (the same file the desktop's Settings modal writes when the
///    user picks a workspace — so `outl` with no args lands on the
///    workspace the user last opened in the GUI, no `--workspace`
///    flag needed).
/// 4. **Current directory** — final fallback (matches the
///    `cd ~/notes && outl` muscle memory).
///
/// A path stored in `config.toml` that no longer exists on disk is
/// skipped silently rather than failing the launch — the user
/// likely deleted / unmounted the folder and would be surprised by
/// a crash. The cwd fallback picks up.
fn resolve_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = local {
        return Ok(p.clone());
    }
    if let Some(p) = global {
        return Ok(p.clone());
    }
    if let Some(p) = outl_config::load().workspace.last {
        if p.exists() {
            return Ok(p);
        }
        tracing::warn!(
            "config.toml workspace.last = {} is no longer on disk; falling back to cwd",
            p.display()
        );
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
            "`outl init` needs an explicit path: pass a positional argument or `--workspace <DIR>`"
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
        // Logs MUST go to stderr; stdout carries the JSON envelope
        // that scripts/tests parse. Without this, every `INFO` line
        // from `JsonlStorage::reload` corrupts the response.
        .with_writer(std::io::stderr)
        .try_init();
}
