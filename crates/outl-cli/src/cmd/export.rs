//! `outl export --to <fmt>` — legacy placeholder, superseded by
//! `outl export {hugo,md,json}` (see `export_v2.rs`).

use anyhow::Result;

/// Legacy `--to` entry point. The real exporters (Hugo / Markdown / JSON)
/// live in `export_v2.rs`; static HTML / PDF targets are not yet implemented.
pub fn run(_format: &str) -> Result<()> {
    println!("use `outl export hugo|md|json` instead; tracking issue: #2 (Hugo target)");
    Ok(())
}
