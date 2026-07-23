//! Background `.md` + sidecar projection writer.
//!
//! The op log is the source of truth; the `.md` + `.outl` sidecar are
//! projections of it. Re-rendering a page and re-hashing its sidecar on
//! every keystroke-commit is real CPU work (SHA-256 per block + a
//! whole-page render), and doing it on the Tauri IPC thread made the
//! commit block the next keystroke. This moves it off that thread.
//!
//! **Async-by-default (see the outl async-writes principle).** A commit
//! now only mutates the op log (the truth), builds the reply view from
//! the tree, and *queues* the page here; the actual disk write lands a
//! beat later on this worker.
//!
//! ## Why it can't corrupt the `.md`↔sidecar pair
//!
//! A torn projection (a `.md` from write A next to a sidecar from write
//! B) breaks the 3-level matching algorithm and desyncs peers. Two rules
//! prevent it:
//!
//! 1. **One worker, serial.** A single thread drains the queue, so two
//!    projections never run concurrently.
//! 2. **Written under the workspace lock.** Every projection path in the
//!    app (`finish_in_page`, templates, exec, …) writes the `.md` +
//!    sidecar while holding the workspace `Mutex`. This worker takes the
//!    same lock, so its write is mutually exclusive with any synchronous
//!    projection too — the pair is always rendered + written from one
//!    consistent tree snapshot.
//!
//! ## Coalescing
//!
//! `apply_page_md_with_sidecar` re-renders from the *current* tree, so a
//! burst of edits to one page collapses to a single write of the final
//! state: the worker drains everything already queued into a dedup set
//! before writing, and a later edit that arrives mid-write just queues
//! another pass.
//!
//! ## Durability
//!
//! A crash with writes still queued leaves the `.md` briefly behind the
//! op log — never data loss, because the op log *is* the truth and the
//! next boot re-projects stale pages (`apply_page_md_with_sidecar_if_stale`
//! on open, plus the orphan scanner). Peers sync ops over iroh, not the
//! `.md`, so a lagging projection never ships a wrong tree.

use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use outl_actions::apply_page_md_with_sidecar;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tracing::warn;

use crate::host::StorageRootProvider;

/// Handle to the background projection worker. Cheap to hold in
/// `AppState`; cloning the `Sender` is how a command queues a write.
pub struct ProjectionWriter {
    tx: Sender<NodeId>,
}

impl ProjectionWriter {
    /// Spawn the worker. It owns clones of the same workspace slot every
    /// command locks, plus the client's storage-root provider (a fixed
    /// `PathBuf` on mobile, a swap-capable `Arc<Mutex<Option<PathBuf>>>`
    /// on desktop — both implement [`StorageRootProvider`]).
    pub fn spawn<R: StorageRootProvider>(
        workspace: Arc<Mutex<Option<Workspace>>>,
        root: R,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<NodeId>();
        thread::Builder::new()
            .name("outl-projection".into())
            .spawn(move || {
                // Block until something is queued, then coalesce every
                // other pending page into one batch.
                while let Ok(first) = rx.recv() {
                    let mut dirty: HashSet<NodeId> = HashSet::new();
                    dirty.insert(first);
                    while let Ok(more) = rx.try_recv() {
                        dirty.insert(more);
                    }
                    let Some(root) = root.current() else {
                        continue;
                    };
                    for page in dirty {
                        // Lock per page and drop between writes so a
                        // synchronous command isn't starved during a
                        // large batch. The write (render + sidecar) is
                        // atomic per file and happens under this lock, so
                        // it can't interleave with another projection.
                        let guard = workspace.lock();
                        let Some(ws) = guard.as_ref() else {
                            break;
                        };
                        if let Err(e) = apply_page_md_with_sidecar(ws, &root, page) {
                            warn!("background projection failed for {page}: {e}");
                        }
                    }
                }
            })
            .expect("spawning the projection writer thread should not fail");
        Self { tx }
    }

    /// Queue a page for background `.md` + sidecar projection.
    ///
    /// Coalesced: repeated queues of the same page before the worker
    /// catches up collapse into one write of the current tree. A dropped
    /// receiver (worker gone at shutdown) is a silent no-op — the next
    /// boot re-projects from the op log.
    pub fn queue(&self, page: NodeId) {
        let _ = self.tx.send(page);
    }
}
