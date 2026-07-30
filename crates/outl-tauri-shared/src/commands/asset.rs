//! Asset command bodies: open an uploaded file outside outl, and import
//! a file from the OS into the workspace as a new block.
//!
//! Both delegate to `outl-actions` (`import_asset` / `resolve_asset_path`)
//! and follow the same `AppHost`-generic shape as the block commands.
//! outl deliberately does **not** render the asset — it copies the bytes
//! into `<root>/assets/<hash>.<ext>` and hands the OS the file so the
//! user's default viewer (Preview, an image viewer) opens it.

use std::path::Path;

use base64::Engine as _;
use outl_actions::{
    append_block, create_after_or_append, import_asset, resolve_asset_path, ActionError,
    ImportedAsset,
};

use crate::helpers::{finish_in_page, parse_node_id, storage_root_or_err};
use crate::host::AppHost;
use crate::state::PageView;

/// Normalize a path the OS file picker handed back into a plain filesystem
/// path `std::fs` can open.
///
/// On desktop the Tauri dialog returns a bare path and this is a no-op. On
/// iOS it returns a `file://` URL (often percent-encoded), so we strip the
/// scheme and decode `%XX` before `import_asset` reads it. Android's
/// `content://` URIs are not filesystem paths and can't be handled here —
/// they're left as-is and fail loudly downstream (Android is not a target
/// platform yet).
fn normalize_picker_path(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    // `file:///path` → `/path`; drop an empty authority if present.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned()
}

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

/// Ceiling on the bytes we're willing to inline into a `data:` URL. A
/// base64 payload is ~4/3 the file size and lives entirely in the
/// webview's memory, so an unbounded read would let one giant asset wedge
/// the renderer. Images / small PDFs (the only kinds the frontend fetches)
/// sit well under this.
const MAX_DATA_URL_BYTES: u64 = 25 * 1024 * 1024;

/// Guess a MIME type from a file extension, covering the kinds the
/// frontend inlines (images + pdf). Anything else falls back to the
/// generic binary type — the webview still renders images/pdf correctly,
/// and the frontend only asks for kinds it knows how to show.
fn mime_from_ext(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Resolve an `assets/<hash>.<ext>` link to a `data:<mime>;base64,<…>`
/// URL the webview loads directly (an `<img src>` or a PDF viewer).
///
/// The Tauri asset protocol is deliberately **not** used: it needs a
/// static `assetProtocol.scope`, but outl's workspace root is picked at
/// runtime, so there's nothing to scope ahead of time. Encoding the bytes
/// into a `data:` URL sidesteps the protocol entirely — same code path on
/// desktop and mobile, zero Tauri config.
///
/// The path resolves through the shared [`resolve_asset_path`] guard
/// (rejects traversal / external schemes), and reads are capped at
/// `MAX_DATA_URL_BYTES` so a huge file can never be base64'd into the
/// webview. `Ok(None)` from the resolver means the asset hasn't synced to
/// this device yet — surfaced as a distinct error, mirroring
/// [`open_asset`].
pub fn read_asset_data_url<S: AppHost>(state: &S, url: String) -> Result<String, String> {
    let root = storage_root_or_err(state)?;
    let path = match resolve_asset_path(&root, &url) {
        Ok(Some(path)) => path,
        Ok(None) => return Err("asset not found on this device yet".to_string()),
        Err(e) => return Err(e.to_string()),
    };

    let size = std::fs::metadata(&path)
        .map_err(|e| format!("failed to read asset: {e}"))?
        .len();
    if size > MAX_DATA_URL_BYTES {
        return Err(format!(
            "asset too large to display inline ({size} bytes, limit {MAX_DATA_URL_BYTES})"
        ));
    }

    let bytes = std::fs::read(&path).map_err(|e| format!("failed to read asset: {e}"))?;
    let mime = mime_from_ext(&path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
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
    let source = normalize_picker_path(&source_path);
    import_asset(&root, Path::new(&source), max_bytes).map_err(|e| e.to_string())
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
    let source = normalize_picker_path(&source_path);
    let imported = import_asset(&root, Path::new(&source), max_bytes).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::normalize_picker_path;

    #[test]
    fn desktop_plain_path_is_untouched() {
        assert_eq!(
            normalize_picker_path("/Users/me/report.pdf"),
            "/Users/me/report.pdf"
        );
    }

    #[test]
    fn ios_file_url_is_stripped_and_decoded() {
        assert_eq!(
            normalize_picker_path("file:///private/var/My%20File.pdf"),
            "/private/var/My File.pdf"
        );
        assert_eq!(
            normalize_picker_path("file://localhost/tmp/a.pdf"),
            "/tmp/a.pdf"
        );
    }
}
