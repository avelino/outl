//! Asset command bodies: open an uploaded file outside outl, and import
//! a file from the OS into the workspace as a new block.
//!
//! Both delegate to `outl-actions` (`import_asset` / `resolve_asset_path`)
//! and follow the same `AppHost`-generic shape as the block commands.
//! outl deliberately does **not** render the asset — it copies the bytes
//! into `<root>/assets/<hash>.<ext>` and hands the OS the file so the
//! user's default viewer (Preview, an image viewer) opens it.

use std::path::Path;

use outl_actions::{
    append_block, create_after_or_append, import_asset, resolve_asset_path, ActionError,
    ImportedAsset,
};

use crate::helpers::{finish_in_page, parse_node_id, storage_root_or_err};
use crate::host::AppHost;
use crate::state::PageView;

/// Open an `assets/<hash>.<ext>` link in the OS default app.
///
/// The URL is a workspace-relative asset link the user clicked. We
/// resolve it to an absolute path under `<root>/assets/` (rejecting
/// traversal / external schemes upstream in `resolve_asset_path`) and
/// hand it to `open::that`, which launches the OS default handler.
///
/// Read-only: it touches no workspace lock, only a path resolution plus
/// the OS open. `Ok(None)` from the resolver means the asset hasn't
/// synced to this device yet — surfaced as a distinct error so the
/// client can tell "not here yet" from "bad link".
pub fn open_asset<S: AppHost>(state: &S, url: String) -> Result<(), String> {
    let root = storage_root_or_err(state)?;
    match resolve_asset_path(&root, &url) {
        Ok(Some(path)) => open::that(&path).map_err(|e| format!("failed to open asset: {e}")),
        Ok(None) => Err("asset not found on this device yet".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Import `source_path` into the workspace and return the ready-to-insert
/// link **without** touching the outline.
///
/// This is the drag-and-drop path: the file is dropped onto the *active
/// line*, and the client splices [`ImportedAsset::markdown`] straight into
/// that block's in-flight editor text at the caret — so the insertion
/// respects an uncommitted edit instead of racing it through a backend
/// `edit_text`. The copy is content-addressed / idempotent / size-capped,
/// exactly like [`attach_asset`], but no `Op` is emitted here; the client's
/// own commit (on blur / Enter) writes the block text through the normal
/// path. Pure filesystem side effect + a returned DTO, no workspace lock.
pub fn import_asset_file<S: AppHost>(
    state: &S,
    source_path: String,
) -> Result<ImportedAsset, String> {
    let root = storage_root_or_err(state)?;
    let max_bytes = outl_config::load().assets.max_bytes;
    import_asset(&root, Path::new(&source_path), max_bytes).map_err(|e| e.to_string())
}

/// Import `source_path` into the workspace and attach its link as a new
/// block.
///
/// `import_asset` copies the file into `<root>/assets/<hash>.<ext>`
/// (content-addressed, idempotent, atomic, size-capped by
/// `[assets] max_bytes`) and returns the ready-to-insert markdown
/// (`[name](assets/hash.pdf)`). The copy touches only the filesystem, not
/// the workspace, so it runs before the lock; the block insert then goes
/// through the same `finish_in_page` commit path every mutation uses.
///
/// `after_block_id` places the new block right after that block
/// (tolerating a stale anchor exactly like `create_block`); `None`
/// appends it at the end of the page.
pub fn attach_asset<S: AppHost>(
    state: &S,
    source_path: String,
    page_id: String,
    after_block_id: Option<String>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(state)?;
    let page = parse_node_id(&page_id)?;
    let max_bytes = outl_config::load().assets.max_bytes;
    let imported =
        import_asset(&root, Path::new(&source_path), max_bytes).map_err(|e| e.to_string())?;
    let markdown = imported.markdown;
    finish_in_page(state, page, |ws| {
        if let Some(id) = &after_block_id {
            let after = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_after_or_append(ws, state.hlc(), page, after, Some(&markdown)).map(|_| ())
        } else {
            append_block(ws, state.hlc(), Some(page), Some(&markdown)).map(|_| ())
        }
    })
}
