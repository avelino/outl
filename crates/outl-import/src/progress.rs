//! Import progress events — purely cosmetic, never load-bearing.
//!
//! Same policy as `outl_actions::SyncProgress`: the pipeline pushes
//! events through a caller-supplied sink so a client can paint a
//! status line / progress bar, and a dropped or ignored event can
//! never affect what actually gets imported. The library ships a
//! no-op sink by default; `run_import_with_progress` opts in.

/// One progress event. `done`/`total` count **pages**, not blocks —
/// pages are the unit both slow phases iterate over.
#[derive(Debug, Clone, Copy)]
pub enum ImportProgress<'a> {
    /// Adapter is reading + translating the source.
    Parsing,
    /// IR rendered; `pages` files are about to hit the disk.
    Rendered {
        /// Total pages the pipeline will write.
        pages: usize,
    },
    /// Writing `.md` files (fast).
    Writing {
        /// Pages written so far.
        done: usize,
        /// Total pages.
        total: usize,
    },
    /// Reconciling every written file into the op log — the long
    /// phase (mints NodeIds + sidecars for every block).
    Reconciling {
        /// Pages reconciled so far.
        done: usize,
        /// Total pages.
        total: usize,
        /// Page currently being reconciled.
        page: &'a str,
    },
    /// Pass B: rewriting placeholder refs into real handles.
    Resolving {
        /// Pages processed so far.
        done: usize,
        /// Pages with resolve work.
        total: usize,
    },
    /// Final sweeps (collapsed flags, file fallback).
    Finishing,
}

/// Callback the pipeline pushes [`ImportProgress`] events through.
pub type ProgressSink<'a> = &'a mut dyn FnMut(ImportProgress<'_>);
