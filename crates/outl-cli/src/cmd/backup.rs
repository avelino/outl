//! `outl backup` — local, versioned snapshots of a workspace.
//!
//! Thin CLI over [`outl_actions::backup`]; every decision about what is
//! captured, what is excluded, and why `restore` never writes in place
//! lives in that module, not here.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use outl_actions::backup;

#[derive(Subcommand, Debug, Clone)]
pub enum BackupSubcommand {
    /// Set up (or refresh) the backup repository for this workspace.
    ///
    /// Idempotent. Adopts a workspace you already keep in git rather
    /// than fighting it — exclusions go in `.git/info/exclude`, which
    /// is local-only and never shows up in your commits.
    Init,

    /// Take a snapshot now.
    Now {
        /// Commit message. Defaults to a timestamped one.
        #[arg(short, long)]
        message: Option<String>,
    },

    /// List snapshots, newest first.
    List {
        /// How many to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Extract a past snapshot into a directory for inspection.
    ///
    /// **Never writes into the live workspace.** You diff what came
    /// back and copy over what you actually want — a recovery tool
    /// that overwrites the op log is the last thing you want when
    /// something has already gone wrong.
    Restore {
        /// Snapshot id from `outl backup list`.
        id: String,
        /// Where to extract it.
        #[arg(long)]
        into: std::path::PathBuf,
    },

    /// Show whether backups are set up and when the last one ran.
    Status,
}

pub fn run(root: &Path, sub: &BackupSubcommand) -> Result<()> {
    match sub {
        BackupSubcommand::Init => {
            backup::init(root).context("setting up the backup repository")?;
            println!("backup repository ready at {}", root.display());
            println!("run `outl backup now` to take the first snapshot");
            Ok(())
        }

        BackupSubcommand::Now { message } => {
            let msg = message.clone().unwrap_or_else(default_message);
            match backup::snapshot(root, &msg).context("taking a snapshot")? {
                Some(entry) => {
                    println!("{}  {}", entry.id, entry.message);
                    Ok(())
                }
                None => {
                    println!("nothing changed since the last snapshot");
                    Ok(())
                }
            }
        }

        BackupSubcommand::List { limit } => {
            let entries = backup::list(root, *limit).context("reading the backup history")?;
            if entries.is_empty() {
                println!("no snapshots yet — run `outl backup now`");
                return Ok(());
            }
            for e in entries {
                println!("{}  {}  {}", e.id, e.at, e.message);
            }
            Ok(())
        }

        BackupSubcommand::Restore { id, into } => {
            backup::restore(root, id, into)
                .with_context(|| format!("restoring snapshot {id} into {}", into.display()))?;
            println!("snapshot {id} extracted to {}", into.display());
            println!();
            println!("Your live workspace was NOT modified.");
            println!("Compare, then copy back only what you want:");
            println!("  diff -ru {} {}", into.display(), root.display());
            Ok(())
        }

        BackupSubcommand::Status => {
            if !backup::git_available() {
                println!("git is not on PATH — backups are unavailable");
                return Ok(());
            }
            if !backup::is_initialized(root) {
                println!("no backup repository — run `outl backup init`");
                return Ok(());
            }
            match backup::list(root, 1)?.first() {
                Some(e) => println!("last snapshot: {}  {}  {}", e.id, e.at, e.message),
                None => println!("repository ready, no snapshots yet"),
            }
            Ok(())
        }
    }
}

fn default_message() -> String {
    format!(
        "outl snapshot {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    )
}
