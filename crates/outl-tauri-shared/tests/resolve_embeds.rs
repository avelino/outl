//! `resolve_embeds` is the one round-trip the GUI clients use to turn a
//! `((blk-…))` / `!((blk-…))` handle into renderable content (issue #147).
//! This guards that a resolved handle carries both the source block's
//! `text` (what an inline `((…))` ref renders) and its `children`
//! subtree with inline tokens (what a `!((…))` embed expands) — the two
//! surfaces the desktop drives off one call.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::sidecar::derive_ref_handle;
use outl_tauri_shared::commands::exec::resolve_embeds;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`] for exercising a read-only command: `resolve_embeds`
/// only reads `storage_root()`, so the rest is boilerplate the trait
/// requires. Mirrors the shape both real clients implement.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
}

#[test]
fn resolves_handle_to_text_and_subtree() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // The exact shape from the issue repro: a source page with a parent
    // block carrying inline markup and a nested child.
    let src = open_or_create_page(&mut ws, &hlc, "src", "Src", PageKind::Page).unwrap();
    let parent = append_block(&mut ws, &hlc, Some(src), Some("parent **block**")).unwrap();
    append_block(&mut ws, &hlc, Some(parent), Some("child block")).unwrap();

    // The handle is deterministic from the block's NodeId — the same one
    // the sidecar records and the frontend types into `!((…))`.
    let handle = derive_ref_handle(parent);

    // `WorkspaceIndex::build` reads the `.md` projection, so it must exist.
    apply_page_md_with_sidecar(&ws, &root, src).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved: HashMap<_, _> = resolve_embeds(&host, vec![handle.clone()]).unwrap();

    let entry = resolved
        .get(&handle)
        .expect("handle should resolve to its source block");

    // Inline `((…))` ref surface: the source text, prefix-free.
    assert_eq!(entry.text, "parent **block**");
    assert!(entry.status.is_none());

    // Embed `!((…))` surface: the child subtree, projected WITH tokens so
    // the client renders it instead of an empty row. This is the field the
    // pre-#147 desktop lacked, so the embed showed only `↳ parent block`.
    assert_eq!(entry.children.len(), 1, "the child block must come through");
    let child = &entry.children[0];
    assert_eq!(child.text, "child block");
    assert!(
        !child.tokens.is_empty(),
        "subtree nodes must carry inline tokens"
    );

    // An unknown handle is simply absent (orphan → client shows raw chip).
    let empty = resolve_embeds(&host, vec![derive_ref_handle(NodeId::new()).clone()]).unwrap();
    assert!(empty.is_empty());
}
