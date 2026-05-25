//! `outl-tui` — the terminal UI binary.
//!
//! Thin wrapper around the library `outl_tui::run`. The `outl` binary
//! (in `outl-cli`) reuses the same library so that `outl` with no
//! subcommand opens this TUI in the current directory. See
//! `crates/outl-tui/CLAUDE.md`.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "outl-tui",
    about = "Terminal UI for the outl outliner.",
    version
)]
struct Cli {
    /// Workspace path. Defaults to the current directory.
    #[arg(default_value = ".")]
    path: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    outl_tui::run(&cli.path)
}
