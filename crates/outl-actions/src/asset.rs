//! Asset import: copy an uploaded file into `<workspace>/assets/` and
//! hand back the markdown link that references it.
//!
//! This is the one action that deliberately touches the filesystem
//! outside `journal::write_md_atomic` (see `CLAUDE.md` → "Functions
//! never"). The reason: an asset's *bytes* are not workspace state and
//! must not enter the op log — a multi-MB PDF replayed through the CRDT
//! would bloat every device's log irreversibly. The file is a plain
//! blob replicated like the `.md` projections (file transport carries it
//! for free; the iroh transport ships it over the `outl-asset/1` stream).
//! Only the *link* — `[name](assets/<hash>.<ext>)` — is workspace state,
//! and that goes through the op log as an ordinary `Op::Edit` when the
//! caller inserts [`ImportedAsset::markdown`] into a block.
//!
//! Split of concerns:
//! - [`import_asset`] copies bytes in, returns the link. No `Workspace`.
//! - The caller inserts the link via `block::append_block` / `edit_text`.
//! - [`resolve_asset_path`] maps a link back to an on-disk path for the
//!   "open outside outl" handlers, rejecting anything outside `assets/`.

use std::path::{Component, Path, PathBuf};

use outl_md::asset::{asset_rel_path, hash_bytes, is_asset_link, ASSETS_DIR};

use crate::error::ActionError;

/// The `assets/` directory inside a workspace `root`.
pub fn assets_dir(root: &Path) -> PathBuf {
    root.join(ASSETS_DIR)
}

/// The result of importing a file: where it landed and the markdown to
/// insert.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportedAsset {
    /// Workspace-relative link target, e.g. `assets/<hash>.pdf`.
    pub rel_path: String,
    /// Original file name, used as the link's display text.
    pub display_name: String,
    /// Whether the asset is an image (drives `![]()` vs `[]()`).
    pub is_image: bool,
    /// Ready-to-insert markdown: `![name](rel)` for images, else
    /// `[name](rel)`.
    pub markdown: String,
}

/// Copy `source` into `<root>/assets/<hash>.<ext>` and return the link.
///
/// Content-addressed: the filename is the hex SHA-256 of the bytes, so
/// re-importing identical content is idempotent (the existing file is
/// left untouched) and two devices name the same content identically.
/// `max_bytes` is the `[assets] max_bytes` cap (`0` = unbounded); a file
/// over it is rejected before the copy. The write is atomic (tmp +
/// rename) so a crash mid-copy never leaves a half-written asset that
/// would hash-mismatch its name.
pub fn import_asset(
    root: &Path,
    source: &Path,
    max_bytes: u64,
) -> Result<ImportedAsset, ActionError> {
    let bytes = std::fs::read(source)?;
    if max_bytes > 0 && bytes.len() as u64 > max_bytes {
        return Err(ActionError::AssetTooLarge {
            size: bytes.len() as u64,
            limit: max_bytes,
        });
    }

    let hash = hash_bytes(&bytes);
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let rel_path = asset_rel_path(&hash, &ext);

    let dir = assets_dir(root);
    std::fs::create_dir_all(&dir)?;
    let dest = root.join(&rel_path);
    // Content-addressed: identical bytes already on disk need no rewrite.
    if !dest.exists() {
        // Build the tmp name from the basename directly — `with_extension`
        // mangles a hash that has no extension (empty `ext`).
        let file_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or(&hash);
        let tmp = dir.join(format!("{file_name}.tmp"));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &dest)?;
    }

    let display_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&rel_path)
        .to_string();
    let is_image = outl_md::wikilink::is_image_target(&rel_path);
    let markdown = link_markdown(&display_name, &rel_path, is_image);

    Ok(ImportedAsset {
        rel_path,
        display_name,
        is_image,
        markdown,
    })
}

/// Build the markdown link for an asset: `![name](rel)` when it's an
/// image (so clients that render images inline can), else `[name](rel)`.
pub fn link_markdown(display_name: &str, rel_path: &str, is_image: bool) -> String {
    if is_image {
        format!("![{display_name}]({rel_path})")
    } else {
        format!("[{display_name}]({rel_path})")
    }
}

/// Resolve a `[name](assets/...)` link target to an absolute on-disk
/// path, or `None` when the file doesn't exist.
///
/// Every "open the asset outside outl" handler (TUI, desktop, mobile)
/// routes through here so the traversal guard lives in exactly one
/// place. Rejects — via [`ActionError::InvalidAssetPath`] — anything
/// that isn't a plain relative link under `assets/`: an external scheme,
/// an absolute OS path, or any `..` component that would climb out of
/// the assets dir. `.md` arrives from untrusted peers, so a crafted
/// `[x](assets/../../etc/passwd)` must never reach the filesystem.
pub fn resolve_asset_path(root: &Path, url: &str) -> Result<Option<PathBuf>, ActionError> {
    if !is_asset_link(url) {
        return Err(ActionError::InvalidAssetPath(url.to_string()));
    }
    // Normalize to the form under the assets dir: drop `./` and a single
    // leading `/` (the `/assets/...` variant is workspace-root-relative,
    // never a real absolute path).
    let rel = url.strip_prefix("./").unwrap_or(url);
    let rel = rel.strip_prefix('/').unwrap_or(rel);
    let rel_path = Path::new(rel);

    // Reject any component that could escape the assets dir. Only plain
    // names and forward path separators are allowed.
    for comp in rel_path.components() {
        match comp {
            Component::Normal(_) => {}
            _ => return Err(ActionError::InvalidAssetPath(url.to_string())),
        }
    }

    let dir = assets_dir(root);
    let abs = root.join(rel_path);
    // Defense in depth: the resolved path must stay under assets/.
    if !abs.starts_with(&dir) {
        return Err(ActionError::InvalidAssetPath(url.to_string()));
    }

    Ok(abs.exists().then_some(abs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_source(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn import_copies_and_content_addresses() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let file = write_source(src.path(), "report.pdf", b"%PDF-1.7 fake");

        let a = import_asset(ws.path(), &file, 0).unwrap();
        assert!(a.rel_path.starts_with("assets/"));
        assert!(a.rel_path.ends_with(".pdf"));
        assert_eq!(a.display_name, "report.pdf");
        assert!(!a.is_image);
        assert_eq!(a.markdown, format!("[report.pdf]({})", a.rel_path));
        assert!(ws.path().join(&a.rel_path).exists());
    }

    #[test]
    fn identical_bytes_dedupe_to_one_path() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let f1 = write_source(src.path(), "a.pdf", b"same bytes");
        let f2 = write_source(src.path(), "b.pdf", b"same bytes");

        let a1 = import_asset(ws.path(), &f1, 0).unwrap();
        let a2 = import_asset(ws.path(), &f2, 0).unwrap();
        assert_eq!(a1.rel_path, a2.rel_path);
    }

    #[test]
    fn extensionless_file_imports_cleanly() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let file = write_source(src.path(), "LICENSE", b"MIT bytes");
        let a = import_asset(ws.path(), &file, 0).unwrap();
        // No extension → bare `assets/<hash>`, no trailing dot.
        assert!(a.rel_path.starts_with("assets/"));
        assert!(!a.rel_path.contains('.'));
        assert!(ws.path().join(&a.rel_path).exists());
        assert_eq!(a.display_name, "LICENSE");
    }

    #[test]
    fn image_uses_embed_markdown() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let file = write_source(src.path(), "pic.png", b"\x89PNG fake");
        let a = import_asset(ws.path(), &file, 0).unwrap();
        assert!(a.is_image);
        assert!(a.markdown.starts_with("!["));
    }

    #[test]
    fn oversize_is_rejected_before_copy() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let file = write_source(src.path(), "big.bin", &[0u8; 100]);
        let err = import_asset(ws.path(), &file, 10).unwrap_err();
        assert!(matches!(
            err,
            ActionError::AssetTooLarge {
                size: 100,
                limit: 10
            }
        ));
        // Nothing was copied.
        assert!(
            !assets_dir(ws.path()).exists()
                || std::fs::read_dir(assets_dir(ws.path())).unwrap().count() == 0
        );
    }

    #[test]
    fn resolve_finds_existing_asset() {
        let ws = tempdir().unwrap();
        let src = tempdir().unwrap();
        let file = write_source(src.path(), "doc.pdf", b"bytes");
        let a = import_asset(ws.path(), &file, 0).unwrap();

        let resolved = resolve_asset_path(ws.path(), &a.rel_path).unwrap();
        assert_eq!(resolved, Some(ws.path().join(&a.rel_path)));
    }

    #[test]
    fn resolve_missing_asset_is_none() {
        let ws = tempdir().unwrap();
        let resolved = resolve_asset_path(ws.path(), "assets/deadbeef.pdf").unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_rejects_traversal_and_external() {
        let ws = tempdir().unwrap();
        for bad in [
            "assets/../../etc/passwd",
            "https://example.com/x.pdf",
            "pages/secret.md",
            "assets/../ops/ops.jsonl",
        ] {
            assert!(
                matches!(
                    resolve_asset_path(ws.path(), bad),
                    Err(ActionError::InvalidAssetPath(_))
                ),
                "should reject {bad}"
            );
        }
    }
}
