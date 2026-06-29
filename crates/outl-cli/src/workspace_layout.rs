//! Filesystem layout of an outl workspace.
//!
//! A workspace is a directory containing five subtrees:
//!
//! - `.outl/`     — config, peers, orphan log, lock.
//! - `ops/`       — per-actor JSONL op log (`ops-<actor>.jsonl`),
//!   the unit of cross-device sync.
//! - `pages/`     — user-named `.md` files (clean markdown).
//! - `journals/`  — daily-note `.md` files keyed by date.
//! - `templates/` — optional `.md` templates (e.g. `journal.md`).
//!
//! This module exposes helpers for constructing those paths and the
//! workspace config file.

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use outl_core::id::ActorId;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// On-disk workspace config (`<root>/.outl/config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Workspace metadata.
    pub workspace: WorkspaceConfig,
}

/// Workspace-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// This device's actor id. Persisted so HLCs stay coherent across runs.
    pub actor_id: String,
    /// When the workspace was initialized.
    pub created_at: DateTime<FixedOffset>,
}

impl Config {
    /// Build a fresh config with a new actor id.
    pub fn fresh() -> Self {
        Self {
            workspace: WorkspaceConfig {
                actor_id: ActorId::new().0.to_string(),
                created_at: Local::now().fixed_offset(),
            },
        }
    }

    /// Parse the actor id from the config string into a typed [`ActorId`].
    pub fn actor(&self) -> Result<ActorId> {
        let u = ulid::Ulid::from_string(&self.workspace.actor_id)
            .with_context(|| "actor_id in config.toml is not a valid ULID")?;
        Ok(ActorId(u))
    }
}

/// Paths inside a workspace, derived from its root.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Workspace root.
    pub root: PathBuf,
    /// `.outl/` directory.
    pub dot_outl: PathBuf,
    /// `pages/` directory.
    pub pages: PathBuf,
    /// `journals/` directory.
    pub journals: PathBuf,
    /// `templates/` directory.
    pub templates: PathBuf,
    /// `ops/` directory holding per-actor JSONL op logs.
    pub ops: PathBuf,
    /// `.outl/config.toml`.
    pub config: PathBuf,
    /// `.outl/orphans.log`.
    pub orphans: PathBuf,
    /// `.outl/peers.toml` (legacy peer-registry placeholder).
    pub peers: PathBuf,
    /// `templates/journal.md`.
    pub journal_template: PathBuf,
}

impl Paths {
    /// Build paths from a workspace root.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let dot_outl = root.join(".outl");
        Self {
            ops: root.join("ops"),
            config: dot_outl.join("config.toml"),
            orphans: dot_outl.join("orphans.log"),
            peers: dot_outl.join("peers.toml"),
            pages: root.join("pages"),
            journals: root.join("journals"),
            journal_template: root.join("templates").join("journal.md"),
            templates: root.join("templates"),
            dot_outl,
            root,
        }
    }

    /// Path for a given page name (no `.md` suffix expected from caller).
    ///
    /// Used by `outl-tui` (Step 5) and not yet referenced from the CLI
    /// itself, hence the explicit allow.
    #[allow(dead_code)]
    pub fn page_md(&self, name: &str) -> PathBuf {
        self.pages.join(format!("{name}.md"))
    }

    /// Path for a journal of a given date.
    pub fn journal_md(&self, date: NaiveDate) -> PathBuf {
        self.journals
            .join(format!("{}.md", date.format("%Y-%m-%d")))
    }
}

/// Create every directory and write the seed files for a fresh workspace.
///
/// Idempotent up to the config file: if `config.toml` already exists, the
/// caller's existing config is preserved.
pub fn init(paths: &Paths) -> Result<()> {
    for dir in [
        &paths.root,
        &paths.dot_outl,
        &paths.ops,
        &paths.pages,
        &paths.journals,
        &paths.templates,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    if !paths.config.exists() {
        let cfg = Config::fresh();
        write_config(paths, &cfg)?;
    }

    if !paths.journal_template.exists() {
        // Seed with a single empty bullet — outl's parser only
        // recognizes `- ` as a block marker, so a leading `# {{date}}`
        // heading would silently make the page parse to zero blocks
        // (see issue #55). Journals are slug-keyed by date already; no
        // heading needed.
        fs::write(&paths.journal_template, "- \n")
            .with_context(|| format!("writing {}", paths.journal_template.display()))?;
    }

    // Touch orphan log so `doctor` can rely on it existing.
    if !paths.orphans.exists() {
        fs::write(&paths.orphans, "")
            .with_context(|| format!("writing {}", paths.orphans.display()))?;
    }

    // An empty peers.toml placeholder is fine; the live P2P peer registry
    // is `.outl/peers.json`, written by `outl peer pair`.
    if !paths.peers.exists() {
        fs::write(&paths.peers, "# Sync peers live in .outl/peers.json\n")
            .with_context(|| format!("writing {}", paths.peers.display()))?;
    }

    Ok(())
}

/// Ensure `<root>/ops/` exists.
///
/// Both `workspace_layout::init` and every workspace opener (CLI
/// `ws::open`, TUI `open_workspace`, `outl serve`, importers) need
/// this directory present before touching `JsonlStorage`. Sync
/// transports (iCloud Drive, Syncthing) sometimes garbage-collect
/// empty directories, so the openers can't assume it survived since
/// `outl init`. Centralising the `create_dir_all` here keeps the
/// error message uniform across surfaces.
pub fn ensure_ops_dir(paths: &Paths) -> Result<()> {
    fs::create_dir_all(&paths.ops)
        .with_context(|| format!("creating ops dir at {}", paths.ops.display()))
}

/// Read the workspace config.
pub fn read_config(paths: &Paths) -> Result<Config> {
    let s = fs::read_to_string(&paths.config)
        .with_context(|| format!("reading {}", paths.config.display()))?;
    let cfg: Config = toml::from_str(&s).with_context(|| "parsing config.toml")?;
    Ok(cfg)
}

/// Read the workspace config, lazily seeding it when absent.
///
/// A workspace created by a GUI client (desktop / mobile) or by P2P sync only
/// seeds `.outl/workspace-id`, never the per-workspace `config.toml` the CLI /
/// TUI / MCP read the device actor from. When the `.outl/` dir exists but the
/// config doesn't, write a fresh one (new actor) and return it, so opening such
/// a workspace doesn't demand `outl init`. A parse error on an existing file
/// still propagates, and a genuinely-missing `.outl/` is rejected by the opener
/// (`ws::open` checks `dot_outl.exists()` first).
pub fn read_or_init_config(paths: &Paths) -> Result<Config> {
    match read_config(paths) {
        Ok(cfg) => Ok(cfg),
        Err(_) if !paths.config.exists() && paths.dot_outl.is_dir() => {
            let cfg = Config::fresh();
            write_config(paths, &cfg)?;
            Ok(cfg)
        }
        Err(e) => Err(e),
    }
}

/// Write the workspace config.
pub fn write_config(paths: &Paths, cfg: &Config) -> Result<()> {
    let s = toml::to_string_pretty(cfg).with_context(|| "serializing config.toml")?;
    fs::write(&paths.config, s).with_context(|| format!("writing {}", paths.config.display()))?;
    Ok(())
}

/// Today's date in the local timezone, as a `NaiveDate`.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Whether `path` looks like an outl-managed `.md` file (under pages/ or
/// journals/). Sidecars (`.foo.outl`) and template files are excluded.
pub fn is_workspace_md(paths: &Paths, path: &Path) -> bool {
    let Some(ext) = path.extension() else {
        return false;
    };
    if ext != "md" {
        return false;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') {
            return false;
        }
    }
    path.starts_with(&paths.pages) || path.starts_with(&paths.journals)
}
