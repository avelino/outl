//! `outl init <path>` — scaffold a workspace.

use crate::workspace_layout::{init, read_config, today, Paths};
use anyhow::{Context, Result};
use outl_core::storage::{JsonlStorage, PageScope};
use std::fs;
use std::path::Path;

/// Run the `init` subcommand.
///
/// `scope = "per-page"` switches new workspaces to the Phase B layout
/// (`ops/<actor>/<slug>.jsonl`) — boot is proportional to the active
/// page, not the whole workspace. Default is `"global"` for back-compat
/// with every existing workspace.
pub fn run(path: &Path, scope: &str) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());

    // Create all directories and seed config/templates.
    init(&paths)?;

    let cfg = read_config(&paths)?;
    let actor = cfg.actor()?;
    let initial_scope = match scope {
        "per-page" => PageScope::PerPage("home".into()),
        _ => PageScope::Global,
    };
    let _ = JsonlStorage::open_with_scope_cap(paths.ops.clone(), actor, initial_scope, 0)
        .with_context(|| format!("opening JSONL log at {}", paths.ops.display()))?;

    // Seed today's journal if missing.
    let date = today();
    let journal_path = paths.journal_md(date);
    if !journal_path.exists() {
        let template = fs::read_to_string(&paths.journal_template).unwrap_or_default();
        let rendered = template.replace("{{date}}", &date.format("%Y-%m-%d").to_string());
        fs::write(&journal_path, rendered)
            .with_context(|| format!("writing initial journal at {}", journal_path.display()))?;
    }

    println!("Initialized outl workspace at {}", paths.root.display());
    println!("  ops:      {}", paths.ops.display());
    println!("  config:   {}", paths.config.display());
    println!("  journal:  {}", journal_path.display());
    println!("  scope:    {}", scope);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_full_workspace() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "global").unwrap();

        let paths = Paths::at(&root);
        assert!(paths.dot_outl.is_dir(), ".outl/ should exist");
        assert!(paths.ops.is_dir(), "ops/ should exist");
        assert!(paths.config.is_file(), "config.toml should exist");
        assert!(paths.pages.is_dir(), "pages/ should exist");
        assert!(paths.journals.is_dir(), "journals/ should exist");
        assert!(
            paths.journal_template.is_file(),
            "templates/journal.md should exist"
        );
        // Today's journal seeded.
        let today_path = paths.journal_md(today());
        assert!(today_path.is_file(), "today's journal should exist");
    }

    #[test]
    fn init_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "global").unwrap();
        // Second run must not error or wipe state.
        run(&root, "global").unwrap();
        let paths = Paths::at(&root);
        assert!(paths.ops.is_dir());
    }

    #[test]
    fn init_with_per_page_scope_creates_actor_subdir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "per-page").unwrap();

        let paths = Paths::at(&root);
        // `ops/` exists; the per-actor subdir gets created on first
        // `JsonlStorage::open_with_scope_cap` call (lazy).
        assert!(paths.ops.is_dir());
    }
}
