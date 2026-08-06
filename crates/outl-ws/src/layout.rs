//! Filesystem layout of an outl workspace.
//!
//! A workspace is a directory containing six subtrees:
//!
//! - `.outl/`     — config, peers, orphan log, lock.
//! - `ops/`       — per-actor JSONL op log (`ops-<actor>.jsonl`),
//!   the unit of cross-device sync.
//! - `pages/`     — user-named `.md` files (clean markdown).
//! - `journals/`  — daily-note `.md` files keyed by date.
//! - `templates/` — optional `.md` templates (e.g. `journal.md`).
//! - `assets/`    — uploaded files (PDFs, images) referenced by
//!   `[name](assets/<hash>.<ext>)` links; content-addressed, replicated
//!   between devices alongside the `.md` (file transport) or over the
//!   `outl-asset/1` iroh stream (P2P transport).
//!
//! This module exposes helpers for constructing those paths and the
//! workspace config file.

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use outl_core::device::DeviceStore;
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
    /// Actor id seeded when this workspace was created.
    ///
    /// **Not** the source of truth for which actor a device writes
    /// under — this file lives inside the workspace, so any transport
    /// that replicates `.outl/` (Syncthing, Dropbox, NFS, git) hands the
    /// same id to two machines. [`outl_core::DeviceStore`] owns that
    /// answer; this field is the value one device may still *adopt*, and
    /// [`WorkspaceConfig::actor_claimed_by`] records which one did.
    /// See [`crate::actor`].
    pub actor_id: String,
    /// When the workspace was initialized.
    pub created_at: DateTime<FixedOffset>,
    /// [`outl_core::MachineId`] of the device that owns
    /// [`WorkspaceConfig::actor_id`], if any.
    ///
    /// Stamped **when the config is created** ([`Config::claimed_by`]),
    /// not on first open. That timing is the whole point: the marker is
    /// only trustworthy if it is already in the bytes a copy carries
    /// away. Stamping it on first open works under a transport that keeps
    /// replicating `config.toml` (Syncthing, Dropbox, NFS) and does
    /// nothing under the default one — iroh ships ops, `workspace-id` and
    /// snapshots, never the config — so two machines that copied a
    /// pre-upgrade workspace would each stamp their own local file and
    /// collide permanently. See [`crate::actor`].
    ///
    /// Absent on any workspace created before this existed. A device
    /// never adopts `actor_id` without seeing its own id here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_claimed_by: Option<String>,
}

impl Config {
    /// Build a fresh config with a new actor id, unclaimed.
    ///
    /// Prefer [`Config::claimed_by`]: an `actor_id` nobody claims is an
    /// `actor_id` no device will ever adopt.
    pub fn fresh() -> Self {
        Self {
            workspace: WorkspaceConfig {
                actor_id: ActorId::new().0.to_string(),
                created_at: Local::now().fixed_offset(),
                actor_claimed_by: None,
            },
        }
    }

    /// Build a fresh config whose brand-new `actor_id` is claimed by
    /// `store`'s device.
    ///
    /// Safe by construction: the ULID was generated on this machine
    /// microseconds ago, so no other device can already be writing under
    /// it. Every later copy of these bytes reads the claim as foreign and
    /// mints its own actor.
    pub fn claimed_by(store: &DeviceStore) -> Self {
        let mut cfg = Self::fresh();
        match store.machine_id() {
            Ok(machine) => cfg.workspace.actor_claimed_by = Some(machine.to_string()),
            // Leaving it unclaimed only costs one extra ops file on the
            // first open; a wrong claim would cost ops.
            Err(e) => tracing::warn!("could not stamp the actor claim: {e}"),
        }
        cfg
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
    /// `assets/` directory holding uploaded files (content-addressed).
    pub assets: PathBuf,
    /// `ops/` directory holding per-actor JSONL op logs.
    pub ops: PathBuf,
    /// `.outl/config.toml`.
    pub config: PathBuf,
    /// `.outl/orphans.log`, derived by
    /// [`outl_actions::sync::orphans_log_path`].
    ///
    /// Not re-joined here: `outl-md`'s rule is that a block dropping to
    /// matching level 3 is recorded in this file *before* it is trashed,
    /// and the CLI reaches that file exclusively through this field. A
    /// second derivation is a second owner of where those records land.
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
            orphans: outl_actions::sync::orphans_log_path(&root),
            peers: dot_outl.join("peers.toml"),
            pages: root.join("pages"),
            journals: root.join("journals"),
            journal_template: root.join("templates").join("journal.md"),
            templates: root.join("templates"),
            assets: root.join("assets"),
            dot_outl,
            root,
        }
    }

    /// Path for a given page name (no `.md` suffix expected from caller).
    pub fn page_md(&self, name: &str) -> PathBuf {
        self.pages.join(format!("{name}.md"))
    }

    /// Path for a journal of a given date.
    pub fn journal_md(&self, date: NaiveDate) -> PathBuf {
        self.journals
            .join(format!("{}.md", date.format("%Y-%m-%d")))
    }
}

/// Create every directory and write the seed files for a fresh
/// workspace, claiming its `actor_id` for this device.
///
/// Idempotent up to the config file: if `config.toml` already exists, the
/// caller's existing config is preserved.
pub fn init(paths: &Paths) -> Result<()> {
    init_with_device(paths, &DeviceStore::open_default())
}

/// [`init`] against an explicit device store — the seam that lets a test
/// (or an embedder with its own identity directory) act as a *different*
/// machine than the one that will open the workspace.
pub fn init_with_device(paths: &Paths, store: &DeviceStore) -> Result<()> {
    for dir in [
        &paths.root,
        &paths.dot_outl,
        &paths.ops,
        &paths.pages,
        &paths.journals,
        &paths.templates,
        &paths.assets,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    if !paths.config.exists() {
        write_config(paths, &Config::claimed_by(store))?;
    }

    // The journal template is a **page** now (`templates/journal` with
    // `template:: journal`), seeded through the op log in the CLI's
    // `cmd::init`, not a `templates/journal.md` file — see issue #146.
    // `cmd::init` still reads `paths.journal_template` best-effort to
    // migrate a legacy file if one exists.

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
/// Both [`init`] and every workspace opener (CLI `ws::open`, TUI
/// `open_workspace`, `outl serve`, importers, embedders) need this
/// directory present before touching `JsonlStorage`. Sync transports
/// (iCloud Drive, Syncthing) sometimes garbage-collect empty
/// directories, so the openers can't assume it survived since
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
/// ([`crate::open`] checks `dot_outl.exists()` first).
pub fn read_or_init_config(paths: &Paths) -> Result<Config> {
    read_or_init_config_with_device(paths, &DeviceStore::open_default())
}

/// [`read_or_init_config`] against an explicit device store — same seam
/// as [`init_with_device`].
pub fn read_or_init_config_with_device(paths: &Paths, store: &DeviceStore) -> Result<Config> {
    match read_config(paths) {
        Ok(cfg) => Ok(cfg),
        Err(_) if !paths.config.exists() && paths.dot_outl.is_dir() => {
            let cfg = Config::claimed_by(store);
            write_config(paths, &cfg)?;
            Ok(cfg)
        }
        Err(e) => Err(e),
    }
}

/// Write the workspace config, **preserving every key this struct does
/// not model**.
///
/// [`Config`] covers `[workspace]` only, and serializing it wholesale
/// erased the rest of the file: `[theme] preset` and `[workspace]
/// storage` vanished on the first write, silently reverting a
/// per-workspace theme to the global one. So the on-disk document is read
/// back and the modelled keys are merged over it.
///
/// The merge never *removes* a key. Clearing
/// [`WorkspaceConfig::actor_claimed_by`] by writing `None` therefore does
/// nothing — no caller does, and dropping a claim by accident is exactly
/// the failure this file guards against.
///
/// The write itself is temp-file + rename. Two processes racing a first
/// open used to be able to leave a half-written `config.toml`, which
/// fails every later open with "parsing config.toml" and is recoverable
/// only by hand.
pub fn write_config(paths: &Paths, cfg: &Config) -> Result<()> {
    let mut doc: toml::Table = match fs::read_to_string(&paths.config) {
        Ok(s) => toml::from_str(&s)
            .with_context(|| format!("parsing {} before rewriting it", paths.config.display()))?,
        Err(_) => toml::Table::new(),
    };
    let rendered = toml::to_string_pretty(cfg).with_context(|| "serializing config.toml")?;
    let modelled: toml::Table =
        toml::from_str(&rendered).with_context(|| "re-reading the serialized config")?;
    merge_tables(&mut doc, modelled);

    let out = toml::to_string_pretty(&doc).with_context(|| "serializing config.toml")?;
    // `outl_md::write_atomic`, not a local copy: it fsyncs the temp file
    // *and* the parent directory after the rename. This file carries
    // `actor_claimed_by`, which decides which device owns which op log —
    // a rename that survives a crash without its bytes is exactly the
    // shape of the collision the claim exists to prevent.
    outl_md::write_atomic(&paths.config, out.as_bytes())
        .with_context(|| format!("writing {}", paths.config.display()))
}

/// Overlay `over` onto `base`, recursing into sub-tables so an unmodelled
/// sibling key survives.
fn merge_tables(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
                merge_tables(existing, incoming);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Today's date in the user's configured timezone (falling back to the
/// OS local timezone), as a `NaiveDate`. Routes through
/// [`outl_actions::clock`] so `[calendar] timezone` applies — see #107.
pub fn today() -> NaiveDate {
    outl_actions::clock::today()
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

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::device::DeviceStore;
    use tempfile::TempDir;

    /// A workspace created by its own throwaway device, with both temp
    /// dirs handed back so neither is dropped mid-test.
    fn fresh_workspace() -> (TempDir, TempDir, Paths, DeviceStore) {
        let home = TempDir::new().unwrap();
        let store = DeviceStore::at(home.path());
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        init_with_device(&paths, &store).unwrap();
        (home, dir, paths, store)
    }

    /// Defect: [`Config`] models `[workspace]` only, so serializing it
    /// wholesale deleted every other section. A per-workspace `[theme]
    /// preset` silently reverted to the global one on the first write.
    #[test]
    fn writing_the_config_preserves_sections_it_does_not_model() {
        let (_home, _ws, paths, _store) = fresh_workspace();
        let mut raw = std::fs::read_to_string(&paths.config).unwrap();
        raw.push_str("storage = \"jsonl\"\n\n[theme]\npreset = \"monokai\"\n");
        std::fs::write(&paths.config, &raw).unwrap();

        let cfg = read_config(&paths).unwrap();
        write_config(&paths, &cfg).unwrap();

        let doc: toml::Table = toml::from_str(&std::fs::read_to_string(&paths.config).unwrap())
            .expect("still valid toml");
        assert_eq!(
            doc["theme"]["preset"].as_str(),
            Some("monokai"),
            "the workspace theme override must survive: {doc:#?}"
        );
        assert_eq!(
            doc["workspace"]["storage"].as_str(),
            Some("jsonl"),
            "unmodelled keys inside [workspace] too: {doc:#?}"
        );
        // And the modelled keys are still the ones we wrote.
        assert_eq!(
            doc["workspace"]["actor_id"].as_str(),
            Some(cfg.workspace.actor_id.as_str())
        );
    }

    /// Defect: `fs::write` truncates first, so a crash (or a second
    /// process) between truncate and write leaves a `config.toml` that
    /// fails every later open with "parsing config.toml".
    #[test]
    fn writing_the_config_leaves_no_partial_file_behind() {
        let (_home, _ws, paths, _store) = fresh_workspace();
        let cfg = read_config(&paths).unwrap();
        write_config(&paths, &cfg).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&paths.dot_outl)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file not renamed: {leftovers:?}");
        read_config(&paths).expect("the config is readable after a rewrite");
    }

    /// A brand-new `actor_id` is claimed by the device that minted it, so
    /// every later copy of those bytes reads the claim as foreign. Without
    /// this, nothing ever claims it and the creating device forks off its
    /// own workspace on the very first open.
    #[test]
    fn a_fresh_config_claims_its_actor_for_the_creating_device() {
        let (_home, _ws, paths, store) = fresh_workspace();
        let cfg = read_config(&paths).unwrap();
        assert_eq!(
            cfg.workspace.actor_claimed_by.as_deref(),
            Some(store.machine_id().unwrap().as_str())
        );
    }
}
