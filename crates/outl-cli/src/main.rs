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

/// Subcommands for peer/device management.
#[derive(Debug, clap::Subcommand)]
enum PeerCommand {
    /// Pair with another device. Prints a ticket (QR + string); run on both devices.
    Pair {
        /// Accept a ticket from the other device instead of generating one.
        #[arg(long)]
        ticket: Option<String>,
        /// Human-readable name this device advertises to the other (shown in
        /// its `peer list`). Defaults to the machine hostname.
        #[arg(long)]
        name: Option<String>,
    },
    /// List all paired devices.
    List,
    /// Unpair a device by node-id prefix.
    Remove {
        /// Node-id prefix of the device to remove.
        id: String,
    },
    /// Show connection status of all paired devices.
    Status,
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
        /// Source format: `logseq` (directory), `roam` (JSON file), or
        /// `obsidian` (vault directory).
        format: String,
        /// Path to the Logseq graph directory, the Roam backup file,
        /// or the Obsidian vault directory.
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
    /// Manage peer devices for P2P sync.
    Peer {
        #[command(subcommand)]
        cmd: PeerCommand,
    },
    /// Force a one-shot P2P sync pass against every paired device, then exit.
    ///
    /// For scripts that mutate via the CLI and must flush to peers before the
    /// process dies — a normal `outl page/block/...` command is too short-lived
    /// to bind an iroh endpoint, so it relies on a long-lived peer (GUI / MCP)
    /// plus the catch-up re-sync instead. `outl sync` is the explicit flush.
    Sync,
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
        Some(Command::Peer { cmd }) => {
            // Identity is per-DEVICE → global `~/.outl/identity.key`.
            let outl_dir = dirs::home_dir().expect("home dir").join(".outl");
            std::fs::create_dir_all(&outl_dir)?;
            let identity =
                outl_sync_iroh::IrohIdentity::load_or_generate(&outl_dir.join("identity.key"))?;
            // The peer list is per-GRAPH → `<workspace>/.outl/peers.json`. Pairing
            // writes the new peer into the workspace the user is operating on, so
            // it needs the resolved workspace root (not the OS home).
            let ws_root = resolve_path(cli.workspace.as_ref(), None)?;
            outl_sync_iroh::migrate_global_peers_if_absent(&ws_root);
            let peers_path = outl_sync_iroh::workspace_peers_path(&ws_root);
            let mut peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path)?;

            match cmd {
                PeerCommand::Pair { ticket, name } => {
                    let peers_path = peers_path.clone();
                    let identity = std::sync::Arc::new(identity);
                    // The alias is the label THIS device advertises to the peer
                    // (it persists under our node id in the peer's `peers.json`).
                    // `--name` wins; otherwise fall back to the machine hostname
                    // so the peer list reads "macbook" instead of a node-id stub.
                    let alias = name.or_else(default_device_name);
                    let rt = tokio::runtime::Runtime::new()
                        .context("build tokio runtime for pairing")?;

                    if let Some(ticket_str) = ticket {
                        println!("Connecting to the other device…");
                        let entry = rt.block_on(outl_sync_iroh::join_pairing(
                            identity,
                            &ticket_str,
                            &peers_path,
                            alias,
                        ))?;
                        let prefix = &entry.node_id[..entry.node_id.len().min(12)];
                        println!("Paired with {prefix}");
                    } else {
                        println!("Node ID: {}", identity.node_id());
                        let entry = rt.block_on(outl_sync_iroh::host_pairing(
                            identity,
                            &peers_path,
                            alias,
                            |ticket, qr| {
                                println!();
                                println!("Scan this QR on the other device, or copy the ticket:");
                                println!();
                                println!("{qr}");
                                println!("Ticket:");
                                println!("{ticket}");
                                println!();
                                println!("On the other device, run:");
                                println!("  outl peer pair --ticket <ticket>");
                                println!();
                                println!("Waiting for the other device to connect…");
                            },
                        ))?;
                        let prefix = &entry.node_id[..entry.node_id.len().min(12)];
                        println!("Paired with {prefix}");
                    }
                }
                PeerCommand::List => {
                    let list = peers.list();
                    if list.is_empty() {
                        println!("No paired devices. Use `outl peer pair` to add one.");
                    } else {
                        println!("{:<20} {:<20} ADDED", "NODE ID (prefix)", "ALIAS");
                        for p in list {
                            let short = &p.node_id[..p.node_id.len().min(20)];
                            let alias = p.alias.as_deref().unwrap_or("-");
                            println!("{:<20} {:<20} {}", short, alias, p.added_at);
                        }
                    }
                }
                PeerCommand::Remove { id } => match peers.remove(&id)? {
                    true => println!("Removed peer {id}"),
                    false => println!("No peer matching '{id}' found."),
                },
                PeerCommand::Status => {
                    let statuses = outl_sync_iroh::probe_peers_blocking(&identity, &peers)?;
                    if statuses.is_empty() {
                        println!("No paired devices.");
                    } else {
                        println!("{:<22} {:<16} STATUS", "NODE ID (prefix)", "ALIAS");
                        for s in statuses {
                            let short = &s.node_id[..s.node_id.len().min(22)];
                            let alias = s.alias.as_deref().unwrap_or("-");
                            let state = if s.online {
                                match s.rtt_ms {
                                    Some(ms) => format!("online ({ms}ms)"),
                                    None => "online".into(),
                                }
                            } else {
                                "offline".into()
                            };
                            println!("{short:<22} {alias:<16} {state}");
                        }
                    }
                }
            }
            Ok(())
        }
        Some(Command::Sync) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            run_sync(&p)
        }
    }
}

/// Force a one-shot P2P sync pass: bring a transport up, let the boot-time +
/// catch-up sync exchange ops with every paired device, then shut down.
///
/// An ephemeral CLI mutation can't keep a QUIC connection alive long enough to
/// push, so this is the explicit flush. It binds the device identity briefly;
/// the relay-route hijack against a co-resident GUI/MCP is benign (both serve
/// the sync ALPN), and the route returns to the long-lived holder on shutdown.
fn run_sync(path: &std::path::Path) -> anyhow::Result<()> {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    use outl_actions::SyncTransport;

    let wc = ws::open(path).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;
    let outl_dir = dirs::home_dir().expect("home dir").join(".outl");
    // Peer list is per-GRAPH (`<workspace>/.outl/peers.json`); identity is global.
    outl_sync_iroh::migrate_global_peers_if_absent(path);
    let peers =
        outl_sync_iroh::PeersStore::load_or_default(&outl_sync_iroh::workspace_peers_path(path))?;
    if peers.list().is_empty() {
        println!("No paired devices. Use `outl peer pair` to add one.");
        return Ok(());
    }
    let identity = outl_sync_iroh::IrohIdentity::load_or_generate(&outl_dir.join("identity.key"))?;
    // `[sync] relay_url` from the global config: `None` (or empty) keeps iroh's
    // n0 default relay, `Some(url)` points the sync endpoint at a custom relay.
    let relay_url = outl_config::load().sync.relay_url().map(str::to_string);
    let transport = outl_sync_iroh::IrohSyncTransport::new(identity, peers, relay_url);

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    transport.start(wc.root.clone(), wc.actor, tx);
    println!("Syncing with paired devices…");

    // Cross-network connects can take ~20s (iroh multipath), so wait up to
    // `MAX`; but return early once a baseline has passed with no new peer ops
    // (the exchange has gone quiet → converged or nothing to pull).
    const MAX: Duration = Duration::from_secs(25);
    const BASELINE: Duration = Duration::from_secs(6);
    const QUIET: Duration = Duration::from_secs(4);
    let start = Instant::now();
    let mut last_activity = start;
    while start.elapsed() < MAX {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => last_activity = Instant::now(),
            Err(RecvTimeoutError::Timeout) => {
                if start.elapsed() >= BASELINE && last_activity.elapsed() >= QUIET {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let health = transport.peer_health();
    transport.shutdown();

    let online = health.iter().filter(|h| h.reachable).count();
    println!(
        "Sync pass complete — {online}/{} peer(s) reachable.",
        health.len()
    );
    Ok(())
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
/// Best-effort device label for `outl peer pair` when `--name` is omitted.
///
/// Shells out to the `hostname` command (present on macOS + Linux) so the
/// peer's device list reads "macbook" instead of a node-id stub, trimming the
/// macOS `.local` suffix. Returns `None` if the command is unavailable or
/// empty — pairing then advertises no alias, exactly as before this flag.
/// Kept dependency-free on purpose; `--name` is the explicit override.
fn default_device_name() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let raw = String::from_utf8(out.stdout).ok()?;
    let name = raw.trim();
    let name = name.strip_suffix(".local").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

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
