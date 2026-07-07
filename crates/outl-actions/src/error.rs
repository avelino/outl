//! Errors surfaced by action functions.

use outl_core::workspace::WorkspaceError;
use thiserror::Error;

/// Reasons an action may fail. UI layers convert these to their own
/// error surface (toasts, returned strings, panics in tests).
#[derive(Debug, Error)]
pub enum ActionError {
    /// The referenced block is not part of the materialised tree.
    /// Either it was never created, was already moved to trash, or
    /// the caller passed a stale id.
    #[error("block {0} is not in the tree")]
    NotInTree(String),

    /// The block is in the tree but is missing a position record.
    /// Should never happen; if it does, the tree state is corrupt.
    #[error("block {0} has no position in the tree")]
    MissingPosition(String),

    /// The block is at the top of its sibling list and cannot be
    /// indented under a previous sibling.
    #[error("cannot indent {0}: no previous sibling")]
    NoPreviousSibling(String),

    /// The block is already at the root level and cannot be promoted
    /// further.
    #[error("cannot outdent {0}: already at root level")]
    AlreadyAtRoot(String),

    /// The parent does not have a parent of its own (used by outdent
    /// when walking up two levels).
    #[error("cannot outdent {0}: parent has no grandparent")]
    NoGrandparent(String),

    /// A move (e.g. cut-and-paste of a block) would drop the node
    /// inside its own subtree, creating a cycle. The CRDT treats such
    /// a move as a deterministic no-op on the materialised tree, so
    /// we reject it up front and let the client nudge the user
    /// instead of emitting an op that does nothing visible.
    #[error("cannot move {0}: target is inside the block's own subtree")]
    WouldCreateCycle(String),

    /// The page slug failed validation (empty, too long, contains a
    /// path separator, `..`, or a control character). The slug ends
    /// up joined into a filesystem path, so we reject anything that
    /// could escape its directory before it reaches storage.
    #[error("invalid page slug `{0}`")]
    InvalidSlug(String),

    /// No page in the workspace carries the requested slug. Returned
    /// by `page::delete` (and any caller that resolves a slug before
    /// acting on a page that must already exist) so the UI can surface
    /// "page not found" instead of a generic `NotInTree`.
    #[error("page `{0}` not found")]
    PageNotFound(String),

    /// Underlying workspace failure (storage, etc).
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),

    /// I/O while rendering the journal markdown.
    #[error("journal io: {0}")]
    Io(#[from] std::io::Error),

    /// Sidecar (`.outl`) read/write failure when keeping it in sync
    /// with the rendered `.md` projection.
    #[error("sidecar: {0}")]
    Sidecar(#[from] outl_md::sidecar::SidecarError),

    /// `.md` ↔ ops reconcile failure while restoring an undo / redo
    /// snapshot (`history::restore_page_md`).
    #[error("reconcile: {0}")]
    Reconcile(#[from] outl_md::ReconcileError),

    /// Code-block execution orchestration failed (sidecar IO, op log
    /// apply, `.md` reconcile during the run). Runtime-level failures
    /// (`unknown language`, timeout) come back through the success
    /// payload's `error` field instead — they are user-visible
    /// diagnostics, not bugs.
    #[error("exec: {0}")]
    Exec(String),
}
