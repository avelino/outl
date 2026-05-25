//! Synthetic workspaces for benchmarks.
//!
//! Generates a `pages/` + `journals/` tree under a [`TempDir`] with
//! configurable size and shape. Every benchmark binary in `benches/`
//! includes this with `#[path = "common.rs"] mod common;`.

// Each bench binary only uses a subset of the helpers below — `parse`
// doesn't need the workspace generator, `index` doesn't need the
// per-page synthesizer. `dead_code` warnings on the unused-per-binary
// items aren't actionable; quieten them at module level.
#![allow(dead_code)]

use std::fs;
use std::path::Path;

use tempfile::TempDir;

/// Workspace shape. The defaults track real personal-note usage —
/// nudge them per-bench when stress-testing a specific axis.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceShape {
    /// Tier label, embedded in bench IDs so CI can filter by name
    /// (`cargo bench -- small` runs only the small tier).
    pub tier: &'static str,
    /// Pages under `pages/` (non-journal notes).
    pub pages: usize,
    /// Journal entries under `journals/`.
    pub journals: usize,
    /// Blocks per page (and per journal). Mix of plain bullets and
    /// `[[ref]]`s — see `synthesize_page`.
    pub blocks_per_page: usize,
    /// One in this many blocks contains a `[[ref]]` to another page.
    /// Higher = sparser cross-linking. `1` means every block has a ref.
    pub ref_density: usize,
}

impl WorkspaceShape {
    pub fn small() -> Self {
        Self {
            tier: "small",
            pages: 10,
            journals: 5,
            blocks_per_page: 8,
            ref_density: 4,
        }
    }
    pub fn medium() -> Self {
        Self {
            tier: "medium",
            pages: 100,
            journals: 30,
            blocks_per_page: 20,
            ref_density: 5,
        }
    }
    pub fn large() -> Self {
        Self {
            tier: "large",
            pages: 1000,
            journals: 200,
            blocks_per_page: 30,
            ref_density: 6,
        }
    }
    /// 10k+ files — used by the scheduled CI bench. Blocks per page
    /// kept modest (10) to keep total bytes in a sane range; the
    /// stress here is file count + walkdir + index pass-2 backlinks,
    /// not the parse loop.
    pub fn xlarge() -> Self {
        Self {
            tier: "xlarge",
            pages: 10_000,
            journals: 500,
            blocks_per_page: 10,
            ref_density: 6,
        }
    }
}

/// Build a fresh workspace on disk matching `shape`. Returns the
/// `TempDir` so the caller's lifetime keeps the files alive.
pub fn synth_workspace(shape: WorkspaceShape) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("journals")).unwrap();

    for i in 0..shape.pages {
        let path = dir.path().join("pages").join(format!("page-{i:05}.md"));
        fs::write(&path, synth_page(i, shape)).unwrap();
    }
    for i in 0..shape.journals {
        // Use a fake date string — slug stability doesn't matter for
        // the index benchmark; we just need N journal files.
        let path = dir
            .path()
            .join("journals")
            .join(format!("2026-01-{:02}.md", (i % 28) + 1));
        if !path.exists() {
            fs::write(&path, synth_journal(i, shape)).unwrap();
        }
    }
    dir
}

fn synth_page(i: usize, shape: WorkspaceShape) -> String {
    let mut out = format!("title:: Page {i}\nicon:: 📄\n\n");
    for b in 0..shape.blocks_per_page {
        if shape.ref_density > 0 && b % shape.ref_density == 0 && shape.pages > 1 {
            let other = (i + b + 1) % shape.pages;
            out.push_str(&format!(
                "- block {b} of page {i} mentions [[Page {other}]] and #project-{}\n",
                other % 10
            ));
        } else {
            out.push_str(&format!(
                "- block {b} of page {i} — plain content with **bold** and `code` tokens\n"
            ));
        }
    }
    out
}

fn synth_journal(i: usize, shape: WorkspaceShape) -> String {
    let mut out = String::from("type:: journal\n\n");
    for b in 0..shape.blocks_per_page {
        if shape.ref_density > 0 && b % shape.ref_density == 0 && shape.pages > 1 {
            let other = i % shape.pages;
            out.push_str(&format!("- saw [[Page {other}]] today\n"));
        } else {
            out.push_str(&format!("- entry {b} on journal {i}\n"));
        }
    }
    out
}

/// Walk a synthesized workspace and return total `.md` byte count —
/// useful for normalized throughput numbers in bench reports.
#[allow(dead_code)]
pub fn workspace_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    for sub in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(sub)).into_iter().flatten().flatten() {
            if let Ok(m) = entry.metadata() {
                total += m.len();
            }
        }
    }
    total
}
