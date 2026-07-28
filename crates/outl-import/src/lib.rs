//! Adapter-based import pipeline: source graph → typed IR → outl workspace.
//!
//! Three stages, one owner each:
//!
//! 1. A [`SourceAdapter`] parses the source (Roam JSON backup, Logseq
//!    graph dir, …) into the typed IR in [`ir`]. All dialect knowledge
//!    (what `__x__` means, how `{{embed}}` is spelled) lives in the
//!    adapter — nothing downstream ever sees source syntax.
//! 2. [`emit`]'s render stage turns the IR into outl markdown. Block
//!    refs and embeds are emitted as inert placeholders
//!    (`((outl-import:<uid>))`) because `((blk-XXXXXX))` handles don't
//!    exist until the sidecars are stamped.
//! 3. [`emit`]'s resolve stage reconciles every written file (minting
//!    `NodeId`s + sidecars), maps each source UID to its freshly
//!    derived handle through the sidecar's depth-first block list, and
//!    applies the placeholder rewrite **through the op log**
//!    (`outl_actions::block::edit_text` + re-render), never by editing
//!    `.md` behind the workspace's back. Collapsed flags land as
//!    `Op::SetCollapsed` in the same pass.
//!
//! The [`ImportReport`] is the contract with the user: everything the
//! pipeline translates, falls back on, or drops is counted there —
//! silent loss is a bug.

pub mod adapter;
pub mod adapters;
pub mod emit;
pub mod ir;
pub mod progress;
pub mod report;

pub use adapter::{ImportError, SourceAdapter};
pub use progress::{ImportProgress, ProgressSink};
pub use report::ImportReport;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use std::path::{Path, PathBuf};

/// Where an import lands: an open workspace plus its on-disk layout.
///
/// The caller (CLI, future GUI import flow) owns lock acquisition and
/// storage opening — this crate only mutates through the provided
/// `Workspace`, mirroring the `outl-actions` contract.
pub struct ImportDest<'a> {
    /// Open workspace the import writes ops through.
    pub workspace: &'a mut Workspace,
    /// HLC generator bound to the writing actor.
    pub hlc: &'a HlcGenerator,
    /// Workspace root directory.
    pub root: PathBuf,
    /// `pages/` directory.
    pub pages_dir: PathBuf,
    /// `journals/` directory.
    pub journals_dir: PathBuf,
    /// `.outl/orphans.log`, handed to `reconcile_md`.
    pub orphans: PathBuf,
}

/// Import tuning knobs. `Default` matches the CLI defaults.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Write `created::` / `edited::` block properties when the source
    /// carries per-block timestamps. Off by default — two extra
    /// property lines per block pollute the markdown for metadata most
    /// users never read. Dropped timestamps are counted in the report.
    pub preserve_timestamps: bool,
}

/// Parse + render in memory, write nothing.
///
/// The returned report answers "what would this import do" — including
/// how many block refs would resolve vs. dangle — without touching the
/// destination. Backs `outl import … --dry-run`.
///
/// One asymmetry vs. a real run: `refs_page_fallback` needs the
/// reconciled sidecars to detect, so a dry-run counts those refs as
/// resolved. Every other number matches.
pub fn dry_run(
    adapter: &dyn SourceAdapter,
    src: &Path,
    opts: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let mut report = ImportReport::new(adapter.id());
    let graph = adapter.parse(src, &mut report)?;
    emit::dry_run(&graph, opts, &mut report);
    Ok(report)
}

/// Full import of `src` into `dest`.
pub fn run_import(
    adapter: &dyn SourceAdapter,
    src: &Path,
    dest: ImportDest<'_>,
    opts: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    run_import_with_progress(adapter, src, dest, opts, &mut |_| {})
}

/// Like [`run_import`], but pushes [`ImportProgress`] events through
/// `sink` so the caller can paint a status line. Events are purely
/// cosmetic — ignoring them never affects the import.
pub fn run_import_with_progress(
    adapter: &dyn SourceAdapter,
    src: &Path,
    dest: ImportDest<'_>,
    opts: &ImportOptions,
    sink: ProgressSink<'_>,
) -> Result<ImportReport, ImportError> {
    let mut report = ImportReport::new(adapter.id());
    sink(ImportProgress::Parsing);
    let graph = adapter.parse(src, &mut report)?;
    emit::run(&graph, dest, opts, &mut report, sink)?;
    Ok(report)
}
