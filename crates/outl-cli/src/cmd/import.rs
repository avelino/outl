//! `outl import` — bring an existing graph in from Logseq, Roam, or
//! Obsidian.
//!
//! Each source format has its own quirks (Logseq stores `id::` lines
//! inline; Roam ships a JSON backup; Obsidian uses YAML frontmatter
//! and wiki-link variants). The shared output is the same: a populated
//! `pages/` and `journals/` directory in an outl workspace, plus an
//! initial reconcile so sidecars are stamped.
//!
//! Helpers shared by two or more importers (page writing, `((uid))`
//! resolution, sidecar seeding, …) live in [`common`]; each source
//! module owns only its source-specific transforms. See `CLAUDE.md`
//! in `import/` for the pipeline and step ownership.

use anyhow::{Context, Result};
use std::path::Path;

mod common;
pub mod logseq;
pub mod obsidian;
pub mod roam;

pub use common::ImportReport;

/// Dispatch on the source format chosen by the user.
pub fn run(source: &str, src: &Path, dst: &Path) -> Result<()> {
    let dst = dst.to_path_buf();
    if !dst.exists() {
        crate::cmd::init::run(&dst)?;
    }
    let paths = crate::workspace_layout::Paths::at(dst.clone());

    let report = match source {
        "logseq" => logseq::import(src, &paths)
            .with_context(|| format!("logseq import from {}", src.display()))?,
        "obsidian" => obsidian::import(src, &paths)
            .with_context(|| format!("obsidian import from {}", src.display()))?,
        "roam" => roam::import(src, &paths)
            .with_context(|| format!("roam import from {}", src.display()))?,
        other => anyhow::bail!("unknown import source: {other} (expected: logseq, obsidian, roam)"),
    };

    report.print();
    println!();
    println!(
        "Next: run `outl --path {}` to open the imported workspace.",
        dst.display()
    );
    Ok(())
}
