//! `outl init <path>` — scaffold a workspace.

use crate::workspace_layout::{init, read_config, today, Paths};
use anyhow::{Context, Result};
use outl_core::storage::JsonlStorage;
use std::fs;
use std::path::Path;

/// Run the `init` subcommand.
pub fn run(path: &Path) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());

    // Create all directories and seed config/templates.
    init(&paths)?;

    // Touch the per-actor JSONL so the workspace has a writable
    // storage from the first op onwards.
    let cfg = read_config(&paths)?;
    let actor = cfg.actor()?;
    let _storage = JsonlStorage::open(paths.ops.clone(), actor)
        .with_context(|| format!("opening JSONL log at {}", paths.ops.display()))?;
    drop(_storage);

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
        run(&root).unwrap();

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
        run(&root).unwrap();
        // Second run must not error or wipe state.
        run(&root).unwrap();
        let paths = Paths::at(&root);
        assert!(paths.ops.is_dir());
    }
}
