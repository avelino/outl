//! `outl plugin …` — manage workspace plugins.
//!
//! Thin CLI wiring over `outl-plugins`: list installed plugins and the
//! commands they contribute, install one from a local directory (after
//! showing and confirming the permissions it asks for), run a plugin
//! command (re-rendering every `.md` so the mutation lands on disk), and
//! enable / disable a plugin in the lockfile.
//!
//! These are operator-facing, interactive-ish lifecycle commands, so they
//! use `anyhow` at the boundary (like `outl peer`) instead of the JSON
//! envelope the machine-shaped subcommands return.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;

use outl_actions::apply_all_pages_md;
use outl_plugins::{
    install_from_dir, load_installed, lockfile_path, plugins_dir, Capability, ClientCapabilities,
    InstalledPlugins, PluginHost, PluginManifest,
};

use crate::ws;

/// `outl plugin …` subcommands.
#[derive(Subcommand, Debug)]
pub enum PluginCommand {
    /// List installed plugins and the commands they contribute.
    List,
    /// Search the official plugin registry (plugins.outl.app) by name,
    /// id, description, or keyword. An empty query lists everything.
    Search {
        /// Filter text (optional).
        #[arg(default_value = "")]
        query: String,
    },
    /// Scaffold a new plugin project (manifest + build setup + a starter
    /// `src/index.ts`). Run `bun install && bun run build` inside it to get
    /// an installable bundle.
    Init {
        /// Plugin name — also the directory and display name.
        name: String,
        /// Reverse-DNS plugin id (default: `com.example.<slug>`).
        #[arg(long)]
        id: Option<String>,
        /// Output directory (default: `./<slug>`).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Install a plugin from a local directory or a `github:` source.
    ///
    /// Local: a directory holding `plugin.json` + bundle. GitHub:
    /// `github:owner/repo[/subdir][#tag]`, cloned at the newest semver tag
    /// (or the pinned one). Prints the permissions it requests and asks for
    /// approval before installing.
    Install {
        /// Local directory, or `github:owner/repo[/subdir][#tag]`.
        source: String,
        /// Approve every requested permission without prompting.
        #[arg(long)]
        yes: bool,
    },
    /// Run a plugin command, then re-render every `.md` so the effect
    /// shows up on disk.
    Run {
        /// Plugin id (reverse-DNS, e.g. `run.avelino.todo-archiver`).
        plugin_id: String,
        /// Command id contributed by the plugin.
        command_id: String,
    },
    /// Enable a previously installed plugin.
    Enable {
        /// Plugin id.
        id: String,
    },
    /// Disable an installed plugin without uninstalling it.
    Disable {
        /// Plugin id.
        id: String,
    },
    /// Remove (uninstall) a plugin: delete its files and lockfile entry.
    #[command(visible_alias = "uninstall", visible_alias = "rm")]
    Remove {
        /// Plugin id.
        id: String,
    },
}

/// Capabilities the CLI client implements. The CLI can run slash-style
/// commands and dispatch op hooks; it has no rich render surface.
fn cli_capabilities() -> ClientCapabilities {
    [Capability::SlashCommand, Capability::OpHook]
        .into_iter()
        .collect()
}

/// Run a `outl plugin …` invocation.
pub fn run(cmd: &PluginCommand, path: &Path) -> Result<()> {
    match cmd {
        // `init` scaffolds files in its own output dir — it doesn't touch
        // the workspace, so it ignores `path`.
        PluginCommand::Init { name, id, dir } => {
            super::plugin_init::run(name, id.as_deref(), dir.as_deref())
        }
        PluginCommand::List => list(path),
        // `search` hits the network registry, not the workspace.
        PluginCommand::Search { query } => search(query),
        PluginCommand::Install { source, yes } => install(path, source, *yes),
        PluginCommand::Run {
            plugin_id,
            command_id,
        } => run_command(path, plugin_id, command_id),
        PluginCommand::Enable { id } => set_enabled(path, id, true),
        PluginCommand::Disable { id } => set_enabled(path, id, false),
        PluginCommand::Remove { id } => remove(path, id),
    }
}

/// List every installed plugin (with its lockfile metadata) and the
/// commands each contributes on this client. Failed loads surface as
/// warnings so a broken plugin never hides the working ones.
fn list(path: &Path) -> Result<()> {
    let wc = open_ws(path)?;
    let pdir = plugins_dir(&wc.root);

    let mut host = PluginHost::new(cli_capabilities());
    let report = load_installed(&mut host, &pdir);
    let lock = InstalledPlugins::load(&lockfile_path(&pdir)).unwrap_or_default();

    if lock.plugins.is_empty() && report.loaded.is_empty() {
        println!("No plugins installed. Use `outl plugin install <dir>` to add one.");
        return Ok(());
    }

    let commands = host.commands();
    for (id, entry) in &lock.plugins {
        let state = if entry.enabled { "enabled" } else { "disabled" };
        println!("{id}  {}  ({state})", entry.version);
        for c in commands.iter().filter(|c| &c.plugin_id == id) {
            println!("    /{}  {}", c.command_id, c.title);
        }
    }

    for (id, err) in &report.failed {
        eprintln!("warning: plugin `{id}` failed to load: {err}");
    }
    Ok(())
}

/// Search the official registry and print matches with their install
/// command. Network-only — no workspace needed.
fn search(query: &str) -> Result<()> {
    let index = outl_plugins::registry::fetch(outl_plugins::DEFAULT_REGISTRY_URL)
        .map_err(|e| anyhow::anyhow!("fetching the registry: {e}"))?;
    let hits = index.search(query);
    if hits.is_empty() {
        println!("No plugins match `{query}`.");
        return Ok(());
    }
    for e in hits {
        println!("{}  {}", e.id, e.name);
        if !e.description.is_empty() {
            println!("    {}", e.description);
        }
        if !e.capabilities.is_empty() {
            println!("    capabilities: {}", e.capabilities.join(", "));
        }
        println!("    install: outl plugin install {}", e.repository);
    }
    Ok(())
}

/// Install a plugin from a local directory. Parses the manifest, shows the
/// permissions it requests, asks for confirmation (unless `--yes`), then
/// copies it into `.outl/plugins/<id>/` and records the approved
/// permissions in the lockfile.
fn install(path: &Path, source: &str, assume_yes: bool) -> Result<()> {
    // Resolve the source to a local directory: a local path stays put; a
    // `github:owner/repo[/subdir][#tag]` source is cloned at an immutable
    // tag (newest semver if not pinned). `resolved` owns the temp clone.
    let resolved = super::plugin_source::resolve(source)?;
    let source_dir = resolved.dir();

    let manifest_bytes = std::fs::read(source_dir.join("plugin.json"))
        .with_context(|| format!("reading plugin.json from {source}"))?;
    let manifest = PluginManifest::parse(&manifest_bytes)
        .with_context(|| format!("parsing plugin.json from {source}"))?;

    println!(
        "Installing {} ({}) v{}",
        manifest.name, manifest.id, manifest.version
    );
    if manifest.permissions.is_empty() {
        println!("Requests no permissions.");
    } else {
        println!("Requests these permissions:");
        for p in &manifest.permissions {
            println!("  - {p}");
        }
    }

    if !assume_yes && !confirm("Grant these permissions and install?")? {
        println!("Aborted.");
        return Ok(());
    }

    let wc = open_ws(path)?;
    let pdir = plugins_dir(&wc.root);
    std::fs::create_dir_all(&pdir).with_context(|| format!("creating {}", pdir.display()))?;

    let installed = install_from_dir(
        &pdir,
        source_dir,
        &resolved.source_ref,
        manifest.permissions.clone(),
        Some(wc.actor.to_string()),
    )
    .map_err(|e| anyhow::anyhow!("install failed: {e}"))?;

    println!("Installed {} v{}.", installed.id, installed.version);
    Ok(())
}

/// Run a plugin command and project the result back to disk.
///
/// Mutation goes through `host.run_command`, which routes every intent
/// through `outl-actions` → `Workspace::apply`. We then re-render every
/// page's `.md` — without it the op log changes but the files on disk
/// stay stale.
fn run_command(path: &Path, plugin_id: &str, command_id: &str) -> Result<()> {
    let mut wc = open_ws(path)?;
    let pdir = plugins_dir(&wc.root);

    let mut host = PluginHost::new(cli_capabilities());
    let report = load_installed(&mut host, &pdir);
    for (id, err) in &report.failed {
        eprintln!("warning: plugin `{id}` failed to load: {err}");
    }

    let outcome = host
        .run_command(&mut wc.workspace, &wc.hlc, plugin_id, command_id)
        .map_err(|e| anyhow::anyhow!("running {plugin_id}/{command_id}: {e}"))?;

    // Re-render every `.md`: the op log is the source of truth, the files
    // are a projection. Skipping this leaves the change invisible on disk.
    apply_all_pages_md(&wc.workspace, &wc.root)
        .map_err(|e| anyhow::anyhow!("re-rendering pages after plugin run: {e}"))?;

    for line in &outcome.logs {
        println!("[log] {line}");
    }
    for note in &outcome.notifications {
        println!("{note}");
    }
    println!("Applied {} change(s).", outcome.applied);
    for err in &outcome.errors {
        eprintln!("error: {err}");
    }
    Ok(())
}

/// Flip the `enabled` flag for an installed plugin in the lockfile.
fn set_enabled(path: &Path, id: &str, enabled: bool) -> Result<()> {
    let wc = open_ws(path)?;
    let lock_path = lockfile_path(&plugins_dir(&wc.root));
    let mut lock =
        InstalledPlugins::load(&lock_path).map_err(|e| anyhow::anyhow!("reading lockfile: {e}"))?;

    let entry = lock
        .plugins
        .get_mut(id)
        .with_context(|| format!("plugin `{id}` is not installed"))?;
    entry.enabled = enabled;
    lock.save(&lock_path)
        .map_err(|e| anyhow::anyhow!("writing lockfile: {e}"))?;

    println!("{} {id}.", if enabled { "Enabled" } else { "Disabled" });
    Ok(())
}

/// Uninstall a plugin: delete its files and drop its lockfile entry.
fn remove(path: &Path, id: &str) -> Result<()> {
    let wc = open_ws(path)?;
    let removed = outl_plugins::uninstall(&plugins_dir(&wc.root), id)
        .map_err(|e| anyhow::anyhow!("removing `{id}`: {e}"))?;
    if removed {
        println!("Removed {id}.");
    } else {
        println!("Plugin `{id}` is not installed.");
    }
    Ok(())
}

/// Open the workspace, mapping the structured `ApiError` to `anyhow`.
fn open_ws(path: &Path) -> Result<ws::WsCtx> {
    ws::open(path).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))
}

/// Yes/no prompt on stderr. Defaults to "no". When stdin isn't a TTY we
/// refuse rather than silently granting permissions to a scripted caller.
fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("not a TTY — re-run with `--yes` to approve permissions non-interactively");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading prompt response")?;
    let a = line.trim().to_lowercase();
    Ok(a == "y" || a == "yes")
}
