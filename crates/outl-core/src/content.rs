//! Per-block text materialization.
//!
//! The structural CRDT (`Tree`) is text-free; block text convergence rides
//! on Yrs. [`ContentStore`] owns that side of a [`Workspace`], and it is
//! deliberately the *only* thing that writes the two text tiers so they
//! can't drift:
//!
//! - `text`: the materialized string of every block, the hot read path.
//!   Cheap, roughly the text size.
//! - `cache`: a bounded LRU of live Yrs `Doc`s, only for blocks being
//!   edited or merged right now. Rebuilt on demand from the op log.
//!
//! Holding a live `Doc` for every block is what pushed large vaults past
//! the iOS memory limit (issue #108); the materialized string keeps
//! steady-state RAM flat regardless of vault size.
//!
//! [`Workspace`]: crate::workspace::Workspace

use crate::id::NodeId;
use crate::log::OpLog;
use crate::op::Op;
use std::collections::{HashMap, VecDeque};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, Text, Transact};

/// How many live Yrs documents stay resident at once.
///
/// Only blocks being actively edited (or merging an incoming update) need
/// a live `Doc`; every read is served from the materialized-text map. A
/// few hundred live docs is a couple of MB, far under the iOS memory
/// limit even on vaults in the hundreds of thousands of blocks.
pub(crate) const DOC_CACHE_CAP: usize = 512;

/// Build a fresh Yrs `Doc` by replaying a block's `Edit` updates.
///
/// Yrs is a CRDT, so the order updates are applied in does not change the
/// resulting document. Malformed updates (corrupted peers) are skipped.
fn build_doc<'a>(updates: impl Iterator<Item = &'a [u8]>) -> Doc {
    let doc = Doc::new();
    {
        let _text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        for update in updates {
            if let Ok(decoded) = yrs::Update::decode_v1(update) {
                let _ = txn.apply_update(decoded);
            }
        }
    }
    doc
}

/// Materialize a `Doc`'s `content` text into a plain string.
fn doc_string(doc: &Doc) -> String {
    let text = doc.get_or_insert_text("content");
    let txn = doc.transact();
    text.get_string(&txn)
}

/// Bounded LRU of live Yrs documents.
///
/// A `Doc` is only needed to *mutate* a block (encode an edit delta) or to
/// merge an incoming remote update. Reads come from the materialized
/// string instead, so at most [`DOC_CACHE_CAP`] docs stay alive; a cold
/// block being edited is rebuilt on demand from the op log.
struct DocCache {
    docs: HashMap<NodeId, Doc>,
    /// Recency queue; the front is the least-recently-used node.
    order: VecDeque<NodeId>,
    cap: usize,
}

impl DocCache {
    fn new(cap: usize) -> Self {
        Self {
            docs: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn contains(&self, node: NodeId) -> bool {
        self.docs.contains_key(&node)
    }

    fn touch(&mut self, node: NodeId) {
        if let Some(pos) = self.order.iter().position(|n| *n == node) {
            self.order.remove(pos);
        }
        self.order.push_back(node);
    }

    fn get_mut(&mut self, node: NodeId) -> Option<&mut Doc> {
        if self.docs.contains_key(&node) {
            self.touch(node);
            self.docs.get_mut(&node)
        } else {
            None
        }
    }

    fn insert(&mut self, node: NodeId, doc: Doc) {
        if !self.docs.contains_key(&node) && self.docs.len() >= self.cap {
            if let Some(evicted) = self.order.pop_front() {
                self.docs.remove(&evicted);
            }
        }
        self.docs.insert(node, doc);
        self.touch(node);
    }
}

/// Per-node block text, the two-tier store behind `Workspace`'s text API.
pub(crate) struct ContentStore {
    text: HashMap<NodeId, String>,
    cache: DocCache,
}

impl Default for ContentStore {
    fn default() -> Self {
        Self {
            text: HashMap::new(),
            cache: DocCache::new(DOC_CACHE_CAP),
        }
    }
}

impl ContentStore {
    /// Materialized text of a block, if it has any.
    pub(crate) fn text(&self, node: NodeId) -> Option<String> {
        self.text.get(&node).cloned()
    }

    /// Materialize a node's text from its `Edit` updates without caching
    /// the `Doc`. Used during open so the memory peak stays at a single
    /// live doc instead of one per block.
    pub(crate) fn materialize<'a>(
        &mut self,
        node: NodeId,
        updates: impl Iterator<Item = &'a [u8]>,
    ) {
        let doc = build_doc(updates);
        self.text.insert(node, doc_string(&doc));
    }

    /// Ensure `node`'s `Doc` is resident in the cache, rebuilding it from
    /// the block's `Edit` ops in `log` if it was evicted (or never loaded).
    /// No-op when already cached.
    fn ensure_doc(&mut self, node: NodeId, log: &OpLog) {
        if self.cache.contains(node) {
            return;
        }
        // Replay the block's edits straight from the log. `log` is a
        // separate borrow from `self.cache`, so we can hand `build_doc`
        // borrowed slices instead of cloning the block's whole history
        // into a transient `Vec<Vec<u8>>` first.
        let doc = build_doc(log.iter().filter_map(|logged| match &logged.op {
            Op::Edit { node: n, text_op } if *n == node => Some(text_op.as_slice()),
            _ => None,
        }));
        self.cache.insert(node, doc);
    }

    /// Merge a raw Yrs update into a node's live `Doc` and refresh its
    /// materialized string. Rebuilds the doc from `log` first if it was
    /// cold. Both tiers stay in sync because only this method writes them.
    pub(crate) fn merge_update(&mut self, node: NodeId, log: &OpLog, update: &[u8]) {
        self.ensure_doc(node, log);
        if let Some(doc) = self.cache.get_mut(node) {
            {
                let _text = doc.get_or_insert_text("content");
                let mut txn = doc.transact_mut();
                if let Ok(decoded) = yrs::Update::decode_v1(update) {
                    let _ = txn.apply_update(decoded);
                }
            }
            self.text.insert(node, doc_string(doc));
        }
    }

    /// Rewrite a node's text to `new_text` and return the Yrs delta that
    /// encodes the change. Rebuilds the doc from `log` first if it was
    /// cold, and keeps the materialized string in sync.
    pub(crate) fn replace_text(&mut self, node: NodeId, log: &OpLog, new_text: &str) -> Vec<u8> {
        self.ensure_doc(node, log);
        // `ensure_doc` just cached this node, so the lookup can't miss. If
        // it ever did, returning an empty update would silently drop the
        // edit, so fail loud instead.
        let doc = self
            .cache
            .get_mut(node)
            .expect("ensure_doc guarantees the node's doc is cached");
        let text = doc.get_or_insert_text("content");

        // Snapshot the state vector before our mutation so we can encode
        // only the resulting delta.
        let sv_before = {
            let txn = doc.transact();
            txn.state_vector()
        };

        {
            let mut txn = doc.transact_mut();
            let len = text.len(&txn);
            if len > 0 {
                text.remove_range(&mut txn, 0, len);
            }
            if !new_text.is_empty() {
                text.insert(&mut txn, 0, new_text);
            }
        }

        let update = {
            let txn = doc.transact();
            txn.encode_state_as_update_v1(&sv_before)
        };
        self.text.insert(node, new_text.to_string());
        update
    }

    /// Number of live `Doc`s currently resident. Test-only window into the
    /// bound that keeps large vaults under the iOS memory limit (#108).
    #[cfg(test)]
    pub(crate) fn live_doc_count(&self) -> usize {
        self.cache.docs.len()
    }

    /// Whether `node`'s `Doc` is currently resident in the cache.
    pub(crate) fn is_cached(&self, node: NodeId) -> bool {
        self.cache.contains(node)
    }

    /// Build and cache a `Doc` from the given updates, also refreshing
    /// the materialized text. Used when the in-memory log is incomplete
    /// (snapshot boot) and the full `Edit` history must be loaded from
    /// storage to rebuild a correct Doc (#129).
    pub(crate) fn cache_doc<'a>(&mut self, node: NodeId, updates: impl Iterator<Item = &'a [u8]>) {
        if self.cache.contains(node) {
            return;
        }
        let doc = build_doc(updates);
        let s = doc_string(&doc);
        self.text.insert(node, s);
        self.cache.insert(node, doc);
    }

    /// Borrow the materialized text map. The snapshot path serializes it
    /// directly so a snapshot-opened workspace can serve reads without
    /// replaying every `Edit` op.
    pub(crate) fn text_map(&self) -> &HashMap<NodeId, String> {
        &self.text
    }

    /// Rebuild a `ContentStore` from a pre-materialized text map. The
    /// LRU `Doc` cache starts empty; cold blocks rebuild on demand from
    /// the op log via `ensure_doc`. Used by the snapshot boot path.
    pub(crate) fn from_text_map(text: HashMap<NodeId, String>) -> Self {
        Self {
            text,
            cache: DocCache::new(DOC_CACHE_CAP),
        }
    }
}
