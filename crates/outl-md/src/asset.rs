//! Asset links: content-addressed uploaded files referenced from
//! markdown as `[name](assets/<hash>.<ext>)`.
//!
//! An "asset" is any file the user uploads (a PDF, an image) that outl
//! copies into `<workspace>/assets/` and links to from a block. The file
//! is named by the hex SHA-256 of its bytes, so two uploads of the same
//! content deduplicate to one file and the link stays stable across
//! devices. This module owns only the pure text/bytes helpers; the
//! filesystem copy lives in `outl_actions::asset` and cross-device
//! transfer in `outl-sync-iroh`.

use sha2::{Digest, Sha256};

/// Directory, relative to the workspace root, where uploaded assets live.
pub const ASSETS_DIR: &str = "assets";

/// Content hash of raw file bytes, as lowercase hex (no `sha256:`
/// prefix). Used verbatim as the asset filename stem so identical
/// uploads collapse to one file and one stable link everywhere.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Build the workspace-relative link target for an asset. `ext` is the
/// lowercased extension without a leading dot (`"pdf"`); an empty `ext`
/// yields a bare `assets/<hash>`.
pub fn asset_rel_path(hash: &str, ext: &str) -> String {
    if ext.is_empty() {
        format!("{ASSETS_DIR}/{hash}")
    } else {
        format!("{ASSETS_DIR}/{hash}.{ext}")
    }
}

/// True when a markdown link URL points at a workspace asset rather than
/// an external resource.
///
/// Assets are workspace-relative links under `assets/` (the canonical
/// form, plus the `./assets/` and `/assets/` variants a hand-typed or
/// imported link might carry). External links always carry a scheme
/// (`http://`, `mailto:`) and never match — the `://` guard rejects an
/// `http://assets/...` lookalike.
pub fn is_asset_link(url: &str) -> bool {
    if url.contains("://") {
        return false;
    }
    let trimmed = url.strip_prefix("./").unwrap_or(url);
    let trimmed = trimmed.strip_prefix('/').unwrap_or(trimmed);
    trimmed.starts_with(&format!("{ASSETS_DIR}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_hex() {
        let h = hash_bytes(b"hello world");
        assert_eq!(
            h,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        // 32-byte digest → 64 hex chars.
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn rel_path_joins_hash_and_ext() {
        assert_eq!(asset_rel_path("abc123", "pdf"), "assets/abc123.pdf");
        assert_eq!(asset_rel_path("abc123", ""), "assets/abc123");
    }

    #[test]
    fn detects_asset_links() {
        assert!(is_asset_link("assets/abc.pdf"));
        assert!(is_asset_link("./assets/abc.pdf"));
        assert!(is_asset_link("/assets/abc.pdf"));
    }

    #[test]
    fn rejects_external_and_unrelated_links() {
        assert!(!is_asset_link("https://example.com/x.pdf"));
        assert!(!is_asset_link("http://assets/x.pdf"));
        assert!(!is_asset_link("mailto:a@b.com"));
        assert!(!is_asset_link("pages/other.md"));
        assert!(!is_asset_link("my-assets/x.pdf"));
    }
}
